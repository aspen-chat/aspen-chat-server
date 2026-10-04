//! Waking phones (`spec/push.md`): the deployment's push key, the phones that asked to be woken
//! (one subscription per sign-in), and what wakes them. Every push is a small pointer, encrypted
//! to the phone ([`webpush`]), naming a channel and a message; the phone fetches the rest with
//! its own session.
//!
//! One task per API server ([`spawn_dispatcher`]) reads the event stream through a durable
//! JetStream consumer they share, so each event is handled once however many servers run, and
//! only after the transaction that published it has written it. It wakes the people a new
//! message is for, and, for the phones it woke, says when that channel was read elsewhere or the
//! message deleted, so the phone can take its notification down. What it woke whom for is kept
//! in Valkey for [`REMEMBERED`].

pub mod webpush;

use crate::app::channel::Channel;
use crate::app::context::GlobalServerContext;
use crate::app::events::{SubjectOwner, subject_owner};
use crate::app::message::Message;
use crate::app::message::MessageKind;
use crate::app::notification_setting::{NotificationLevel, default_level};
use crate::app::two_factor::Caller;
use crate::app::visibility::viewers;
use crate::app::{
    self, ASPEN_NATS_STREAM_NAME, ChannelId, CommunityId, MessageId, PushKeyId, PushSubscriptionId,
    UserId,
};
use crate::database::schema::{
    channel, channel_mute, community_member_role, community_user, dm_recipient, message,
    notification_setting, push_key, push_subscription, read_state, user_block,
};
use crate::t;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use fred::prelude::{KeysInterface as _, SetsInterface as _};
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OnceCell;
use webpush::PushKey;

/// The name of the JetStream consumer every API server's dispatcher shares.
const CONSUMER: &str = "aspen_push";
/// How many events one server handles at once.
const CONCURRENCY: usize = 16;
/// How many people one event wakes at once: a message tagging everyone in a large community
/// wakes thousands, each a request to a push service that spends most of its time waiting.
const FAN_OUT: usize = 128;
/// How long whom a message woke, and for what, is remembered: longer than anyone leaves a
/// notification unread on a phone that has not been turned on meanwhile.
const REMEMBERED: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How long a push is worth delivering to a phone that cannot be reached now.
const TIME_TO_LIVE: Duration = Duration::from_secs(24 * 60 * 60);
/// Above this many people woken by one message, the badge count is left out: working it out
/// is a query per person.
const MAX_BADGED: usize = 100;
/// The longest endpoint URL taken.
const MAX_ENDPOINT_CHARS: usize = 2048;

/// The key this server signs with, loaded once it exists.
static KEY: OnceCell<(PushKeyId, Arc<PushKey>)> = OnceCell::const_new();

fn key_error(error: impl std::fmt::Display) -> app::Error {
    app::Error::Config(config::ConfigError::Message(format!("push key: {error}")))
}

/// Makes the deployment's push key when it has none, as its first server starts, and loads it.
/// Servers starting together make one: the current key is unique.
pub async fn ensure_key(conn: &mut AsyncPgConnection) -> app::Result<()> {
    let (document, public) = PushKey::generate().map_err(key_error)?;
    diesel::insert_into(push_key::table)
        .values((
            push_key::id.eq(PushKeyId::new()),
            push_key::private_key.eq(document),
            push_key::public_key.eq(public),
        ))
        .on_conflict_do_nothing()
        .execute(conn)
        .await?;
    let (id, document): (PushKeyId, Vec<u8>) = push_key::table
        .select((push_key::id, push_key::private_key))
        .filter(push_key::retired_at.is_null())
        .first(conn)
        .await?;
    let key = PushKey::from_pkcs8(&document).map_err(key_error)?;
    let _ = KEY.set((id, Arc::new(key)));
    Ok(())
}

fn current_key() -> Option<&'static (PushKeyId, Arc<PushKey>)> {
    KEY.get()
}

/// The deployment's push key as apps give it to relays (RFC 8292: the uncompressed point,
/// base64url); `None` where push is off.
pub fn application_server_key(state: &GlobalServerContext) -> Option<String> {
    if !state.config.push.enabled {
        return None;
    }
    current_key().map(|(_, key)| URL_SAFE_NO_PAD.encode(key.public_key()))
}

/// A phone to wake for one sign-in.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = push_subscription)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PushSubscription {
    pub id: PushSubscriptionId,
    pub endpoint: String,
    pub p256dh: Vec<u8>,
    pub auth: Vec<u8>,
    pub push_key: PushKeyId,
    pub created_at: DateTime<Utc>,
}

/// What an app gives to be woken: the endpoint its relay or distributor gave it, and its keys.
pub struct NewSubscription {
    pub endpoint: String,
    pub p256dh: Vec<u8>,
    pub auth: Vec<u8>,
}

/// Registers the calling sign-in's phone, replacing any it registered before.
pub async fn subscribe(
    state: &GlobalServerContext,
    caller: &Caller,
    new: NewSubscription,
) -> app::Result<PushSubscription> {
    let Some((key, _)) = current_key().filter(|_| state.config.push.enabled) else {
        return Err(app::Error::Validation(t!("pushOff")));
    };
    if caller.bot {
        return Err(app::Error::Validation(t!("pushBots")));
    }
    let endpoint = reqwest::Url::parse(&new.endpoint).ok().filter(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && new.endpoint.len() <= MAX_ENDPOINT_CHARS
    });
    if endpoint.is_none() {
        return Err(app::Error::Validation(t!(
            "pushEndpoint",
            max = MAX_ENDPOINT_CHARS
        )));
    }
    if new.p256dh.len() != 65 || new.p256dh[0] != 4 || new.auth.len() != 16 {
        return Err(app::Error::Validation(t!("pushKeys")));
    }
    let mut conn = state.connection_pool.get().await?;
    Ok(diesel::insert_into(push_subscription::table)
        .values((
            push_subscription::id.eq(PushSubscriptionId::new()),
            push_subscription::user.eq(caller.user),
            push_subscription::refresh_token.eq(&caller.refresh_token),
            push_subscription::endpoint.eq(&new.endpoint),
            push_subscription::p256dh.eq(&new.p256dh),
            push_subscription::auth.eq(&new.auth),
            push_subscription::push_key.eq(key),
        ))
        .on_conflict(push_subscription::refresh_token)
        .do_update()
        .set((
            push_subscription::endpoint.eq(&new.endpoint),
            push_subscription::p256dh.eq(&new.p256dh),
            push_subscription::auth.eq(&new.auth),
            push_subscription::push_key.eq(key),
            push_subscription::created_at.eq(diesel::dsl::now),
        ))
        .returning(PushSubscription::as_returning())
        .get_result(conn.as_mut())
        .await?)
}

/// Stops waking one of the caller's phones.
pub async fn unsubscribe(
    state: &GlobalServerContext,
    user: UserId,
    id: PushSubscriptionId,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let deleted = diesel::delete(
        push_subscription::table.filter(
            push_subscription::id
                .eq(id)
                .and(push_subscription::user.eq(user)),
        ),
    )
    .execute(conn.as_mut())
    .await?;
    if deleted == 0 {
        return Err(diesel::result::Error::NotFound.into());
    }
    Ok(())
}

/// What a push says (`spec/push.md`, The pointer).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Pointer {
    /// A message the person should be told of.
    Message {
        channel: ChannelId,
        message: MessageId,
    },
    /// They read the channel up to `message` elsewhere.
    Read {
        channel: ChannelId,
        message: MessageId,
    },
    /// The message is gone.
    Deleted {
        channel: ChannelId,
        message: MessageId,
    },
}

impl Pointer {
    fn channel(&self) -> ChannelId {
        match self {
            Pointer::Message { channel, .. }
            | Pointer::Read { channel, .. }
            | Pointer::Deleted { channel, .. } => *channel,
        }
    }

    /// RFC 8030's urgency: a message is for now, and a pointer that shows nothing is `low`,
    /// which a relay whose app cannot hide a notification leaves undelivered (`spec/push.md`).
    fn urgency(&self) -> &'static str {
        match self {
            Pointer::Message { .. } => "high",
            Pointer::Read { .. } | Pointer::Deleted { .. } => "low",
        }
    }
}

#[derive(Serialize)]
struct Payload {
    v: u32,
    #[serde(flatten)]
    pointer: Pointer,
    #[serde(skip_serializing_if = "Option::is_none")]
    badge: Option<i64>,
}

/// The phones of each of `users`: those of their sign-ins still live, of accounts neither
/// deleted nor banned from the deployment. A phone whose sign-in was revoked (a password
/// change, a ban) is not woken, though its subscription stays until the sign-in is deleted.
async fn phones_of(
    state: &GlobalServerContext,
    users: &[UserId],
) -> app::Result<HashMap<UserId, Vec<PushSubscription>>> {
    use crate::database::schema::{refresh_token, user};
    let mut conn = state.connection_pool.get().await?;
    let mut phones: HashMap<UserId, Vec<PushSubscription>> = HashMap::new();
    for (user, subscription) in push_subscription::table
        .inner_join(refresh_token::table)
        .inner_join(user::table)
        .select((push_subscription::user, PushSubscription::as_select()))
        .filter(push_subscription::user.eq_any(users))
        .filter(refresh_token::expires.gt(diesel::dsl::now))
        .filter(user::deleted_at.is_null())
        .filter(diesel::dsl::not(app::user_ban::banned()))
        .load::<(UserId, PushSubscription)>(conn.as_mut())
        .await?
    {
        phones.entry(user).or_default().push(subscription);
    }
    Ok(phones)
}

/// Sends `pointer` to each of `phones`, dropping those their relay says are gone.
async fn wake(
    state: &GlobalServerContext,
    phones: &[PushSubscription],
    pointer: Pointer,
    badge: Option<i64>,
) {
    for subscription in phones {
        match send(state, subscription, pointer, badge).await {
            Ok(Delivery::Accepted) => {}
            Ok(Delivery::Gone) => {
                tracing::debug!(subscription = %subscription.id.0, "push subscription is gone");
                let _ = async {
                    let mut conn = state.connection_pool.get().await?;
                    diesel::delete(push_subscription::table.find(subscription.id))
                        .execute(conn.as_mut())
                        .await?;
                    Ok::<_, app::Error>(())
                }
                .await;
            }
            Err(e) => tracing::warn!(endpoint = subscription.endpoint, "a push failed: {e}"),
        }
    }
}

enum Delivery {
    Accepted,
    /// The subscription no longer reaches anything: the relay says it is unknown or gone, or
    /// that it belongs to another key.
    Gone,
}

#[derive(Debug, thiserror::Error)]
enum SendError {
    #[error("{0}")]
    Encrypt(#[from] webpush::WebPushError),
    #[error("{0}")]
    Http(#[from] reqwest::Error),
    #[error("the push service answered {0}: {1}")]
    Refused(reqwest::StatusCode, String),
}

async fn send(
    state: &GlobalServerContext,
    subscription: &PushSubscription,
    pointer: Pointer,
    badge: Option<i64>,
) -> Result<Delivery, SendError> {
    let Some((key_id, key)) = current_key() else {
        return Ok(Delivery::Accepted);
    };
    // A subscription made for a key since replaced reaches nothing; its app subscribes again
    // when it sees the new key.
    if subscription.push_key != *key_id {
        return Ok(Delivery::Gone);
    }
    let payload = serde_json::to_vec(&Payload {
        v: 1,
        pointer,
        badge,
    })
    .expect("pointers serialize");
    let body = webpush::encrypt(&payload, &subscription.p256dh, &subscription.auth)?;
    let url = reqwest::Url::parse(&subscription.endpoint)
        .map_err(|_| SendError::Refused(reqwest::StatusCode::BAD_REQUEST, "endpoint".into()))?;
    let audience = url.origin().ascii_serialization();
    let subject = state
        .config
        .federation
        .domain
        .as_ref()
        .map(|domain| format!("https://{domain}"));
    let authorization = key.authorization(&audience, subject.as_deref(), Utc::now().timestamp())?;
    let response = state
        .federation_client
        .post(url)
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("TTL", TIME_TO_LIVE.as_secs().to_string())
        .header("Urgency", pointer.urgency())
        // A channel's id without its hyphens is 32 characters of the alphabet RFC 8030 allows,
        // so a burst in one channel replaces what the phone has not yet received.
        .header("Topic", pointer.channel().0.simple().to_string())
        .header("Authorization", authorization)
        .body(body)
        .send()
        .await?;
    let status = response.status();
    if status.is_success() {
        return Ok(Delivery::Accepted);
    }
    if matches!(status.as_u16(), 403 | 404 | 410) {
        return Ok(Delivery::Gone);
    }
    let detail = response.text().await.unwrap_or_default();
    Err(SendError::Refused(
        status,
        detail.chars().take(500).collect(),
    ))
}

/// Starts this server's share of handling events for push.
pub fn spawn_dispatcher(state: GlobalServerContext) {
    if !state.config.push.enabled {
        return;
    }
    tokio::spawn(async move {
        loop {
            if let Err(e) = dispatch(&state).await {
                tracing::error!("the push dispatcher stopped: {e}; starting it again");
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn dispatch(state: &GlobalServerContext) -> anyhow::Result<()> {
    use async_nats::jetstream::consumer::{AckPolicy, DeliverPolicy, pull};
    let stream = state
        .nats_context
        .get_stream(ASPEN_NATS_STREAM_NAME)
        .await?;
    let consumer = stream
        .get_or_create_consumer(
            CONSUMER,
            pull::Config {
                durable_name: Some(CONSUMER.into()),
                deliver_policy: DeliverPolicy::New,
                ack_policy: AckPolicy::Explicit,
                ack_wait: Duration::from_secs(60),
                max_deliver: 3,
                filter_subject: format!("{}.>", app::events::SUBJECT_ROOT),
                ..Default::default()
            },
        )
        .await?;
    consumer
        .messages()
        .await?
        .for_each_concurrent(CONCURRENCY, |received| async move {
            let Ok(received) = received else {
                return;
            };
            if let Err(e) = handle(state, &received.subject, &received.payload).await {
                tracing::warn!("handling an event for push failed: {e}");
            }
            if let Err(e) = received.ack().await {
                tracing::warn!("acknowledging an event for push failed: {e}");
            }
        })
        .await;
    Ok(())
}

/// What push reads of an event: its names and ids, never its content.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Seen {
    server_event: String,
    #[serde(rename = "type")]
    change: Option<String>,
    id: Option<MessageId>,
    channel: Option<ChannelId>,
    last_read: Option<MessageId>,
}

async fn handle(state: &GlobalServerContext, subject: &str, payload: &[u8]) -> app::Result<()> {
    let Ok(seen) = serde_json::from_slice::<Seen>(payload) else {
        return Ok(());
    };
    match (seen.server_event.as_str(), seen.change.as_deref()) {
        ("message", Some("create")) => {
            if let Some(id) = seen.id
                && first_time(state, &format!("push:created:{}", id.0)).await?
            {
                message_created(state, id).await?;
            }
        }
        ("message", Some("delete")) => {
            if let Some(id) = seen.id
                && first_time(state, &format!("push:deleted:{}", id.0)).await?
            {
                message_deleted(state, id).await?;
            }
        }
        ("channelRead", _) => {
            if let (Some(SubjectOwner::User(user)), Some(channel), Some(last_read)) =
                (subject_owner(subject), seen.channel, seen.last_read)
            {
                channel_read(state, user, channel, last_read).await?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Whether this is the first time `key` is seen: a DM's events are published once for each of
/// its people, and each copy must wake them once.
async fn first_time(state: &GlobalServerContext, key: &str) -> app::Result<bool> {
    let set: Option<String> = state
        .valkey
        .set(
            key,
            1,
            Some(fred::types::Expiration::EX(600)),
            Some(fred::types::SetOptions::NX),
            false,
        )
        .await?;
    Ok(set.is_some())
}

fn woken_key(user: UserId, channel: ChannelId) -> String {
    format!("push:woken:{}:{}", user.0, channel.0)
}

fn message_key(message: MessageId) -> String {
    format!("push:message:{}", message.0)
}

async fn message_created(state: &GlobalServerContext, id: MessageId) -> app::Result<()> {
    // The event is published before the transaction that wrote the message commits; it is
    // there moments later.
    let mut found = None;
    for _ in 0..5 {
        let mut conn = state.connection_pool.get().await?;
        found = message::table
            .select(Message::as_select())
            .filter(message::id.eq(id).and(message::deleted_at.is_null()))
            .first(conn.as_mut())
            .await
            .optional()?;
        if found.is_some() {
            break;
        }
        drop(conn);
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let Some(found) = found else {
        return Ok(());
    };
    // An echo and a poll's result say nothing of their own, a call's record follows the ring
    // that already told everyone of the call, and a command is for its bot, whose answer is
    // what tells.
    if matches!(
        found.kind,
        MessageKind::ThreadEcho
            | MessageKind::PollClosed
            | MessageKind::Call
            | MessageKind::MissedCall
            | MessageKind::Command
    ) {
        return Ok(());
    }
    let channel_id = *found.channel.id();
    let author = *found.author.id();
    let recipients = recipients(state, &found, channel_id, author).await?;
    if recipients.is_empty() {
        return Ok(());
    }
    let pointer = Pointer::Message {
        channel: channel_id,
        message: id,
    };
    let badge = recipients.len() <= MAX_BADGED;
    let _: () = state
        .valkey
        .sadd(
            message_key(id),
            recipients
                .iter()
                .map(|u| u.0.to_string())
                .collect::<Vec<_>>(),
        )
        .await?;
    let _: () = state
        .valkey
        .expire(message_key(id), REMEMBERED.as_secs() as i64, None)
        .await?;
    let phones = &phones_of(state, &recipients).await?;
    futures_util::stream::iter(recipients)
        .for_each_concurrent(FAN_OUT, |user| async move {
            let _: Result<(), _> = state
                .valkey
                .set(
                    woken_key(user, channel_id),
                    id.0.to_string(),
                    Some(fred::types::Expiration::EX(REMEMBERED.as_secs() as i64)),
                    None,
                    false,
                )
                .await;
            let badge = if badge {
                badge_of(state, user).await.ok()
            } else {
                None
            };
            wake(
                state,
                phones.get(&user).map_or(&[], Vec::as_slice),
                pointer,
                badge,
            )
            .await;
        })
        .await;
    Ok(())
}

async fn message_deleted(state: &GlobalServerContext, id: MessageId) -> app::Result<()> {
    let woken: Vec<String> = state.valkey.smembers(message_key(id)).await?;
    if woken.is_empty() {
        return Ok(());
    }
    let _: () = state.valkey.del(message_key(id)).await?;
    let mut conn = state.connection_pool.get().await?;
    let Some(channel) = message::table
        .select(message::channel)
        .filter(message::id.eq(id))
        .first::<ChannelId>(conn.as_mut())
        .await
        .optional()?
    else {
        return Ok(());
    };
    drop(conn);
    let woken: Vec<UserId> = woken
        .iter()
        .filter_map(|u| u.parse().ok().map(UserId))
        .collect();
    let pointer = Pointer::Deleted {
        channel,
        message: id,
    };
    let phones = phones_of(state, &woken).await?;
    futures_util::stream::iter(phones.values())
        .for_each_concurrent(FAN_OUT, |phones| wake(state, phones, pointer, None))
        .await;
    Ok(())
}

async fn channel_read(
    state: &GlobalServerContext,
    user: UserId,
    channel: ChannelId,
    last_read: MessageId,
) -> app::Result<()> {
    let woken: Option<String> = state.valkey.get(woken_key(user, channel)).await?;
    let Some(woken) = woken.and_then(|w| w.parse().ok().map(MessageId)) else {
        return Ok(());
    };
    // Ids are UUIDv7, so a later message has a greater id: reading up to or past the message
    // that woke the phone takes its notifications down.
    if woken > last_read {
        return Ok(());
    }
    let _: () = state.valkey.del(woken_key(user, channel)).await?;
    // The event is published before the transaction that moved the position commits; the badge
    // is counted once it has.
    for _ in 0..10 {
        let mut conn = state.connection_pool.get().await?;
        let recorded: Option<MessageId> = read_state::table
            .select(read_state::message)
            .filter(
                read_state::user
                    .eq(user)
                    .and(read_state::channel.eq(channel)),
            )
            .first(conn.as_mut())
            .await
            .optional()?;
        if recorded.is_some_and(|recorded| recorded >= last_read) {
            break;
        }
        drop(conn);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let phones = phones_of(state, &[user]).await?;
    let Some(phones) = phones.get(&user) else {
        return Ok(());
    };
    wake(
        state,
        phones,
        Pointer::Read {
            channel,
            message: last_read,
        },
        badge_of(state, user).await.ok(),
    )
    .await;
    Ok(())
}

/// Whom a new message is for, among those with a phone to wake (`spec/push.md`, Who is woken):
/// the people of a DM, or the members of a community who may read it, as far as each one's
/// notification setting for the channel asks (`app::notification_setting`: every message, or
/// only those that tag them); never its author, anyone who blocked them, anyone who muted the
/// channel (a thread counting as its parent), or anyone using Aspen right now.
async fn recipients(
    state: &GlobalServerContext,
    found: &Message,
    channel_id: ChannelId,
    author: UserId,
) -> app::Result<Vec<UserId>> {
    let mut conn = state.connection_pool.get().await?;
    let posted_in: Channel = channel::table
        .select(Channel::as_select())
        .find(channel_id)
        .first(conn.as_mut())
        .await?;
    // A thread's messages are its parent's, for who may read them, settings, and muting.
    let place = match posted_in.parent_channel {
        Some(parent) => {
            channel::table
                .select(Channel::as_select())
                .find(parent)
                .first(conn.as_mut())
                .await?
        }
        None => posted_in,
    };
    let community = place.community.as_ref().map(|c| *c.id());
    let subscribed = push_subscription::table.select(push_subscription::user);
    // Everyone who might be told: a DM's people, or a community's members who are tagged or
    // want every message.
    let (candidates, tagged_users): (HashSet<UserId>, HashSet<UserId>) = match community {
        None => {
            let people: HashSet<UserId> = dm_recipient::table
                .select(dm_recipient::user)
                .filter(dm_recipient::channel.eq(place.id))
                .filter(dm_recipient::user.eq_any(subscribed))
                .load::<UserId>(conn.as_mut())
                .await?
                .into_iter()
                .collect();
            let tagged_users = people
                .iter()
                .filter(|u| found.mentions.everyone || found.mentions.users.contains(u))
                .copied()
                .collect();
            (people, tagged_users)
        }
        Some(community) => {
            let tagged_users = tagged(conn.as_mut(), community, &found.mentions).await?;
            let everything: HashSet<UserId> = notification_setting::table
                .select(notification_setting::user)
                .filter(notification_setting::level.eq(NotificationLevel::All))
                .filter(
                    notification_setting::channel
                        .eq(place.id)
                        .or(notification_setting::community.eq(community)),
                )
                .filter(notification_setting::user.eq_any(subscribed))
                .load::<UserId>(conn.as_mut())
                .await?
                .into_iter()
                .collect();
            (&tagged_users | &everything, tagged_users)
        }
    };
    let mut candidates = candidates;
    candidates.remove(&author);
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let listed: Vec<UserId> = candidates.iter().copied().collect();
    // Each one's level here: the channel's own setting, else the community's, else the default.
    let settings: Vec<(UserId, Option<CommunityId>, NotificationLevel)> =
        notification_setting::table
            .select((
                notification_setting::user,
                notification_setting::community,
                notification_setting::level,
            ))
            .filter(notification_setting::user.eq_any(&listed))
            .filter(
                notification_setting::channel
                    .eq(place.id)
                    .or(notification_setting::community.nullable().eq(community)),
            )
            .load(conn.as_mut())
            .await?;
    let mut for_channel = HashMap::new();
    let mut for_community = HashMap::new();
    for (user, community, level) in settings {
        match community {
            Some(_) => for_community.insert(user, level),
            None => for_channel.insert(user, level),
        };
    }
    let level_of = |user: UserId| {
        for_channel
            .get(&user)
            .or_else(|| for_community.get(&user))
            .copied()
            .unwrap_or_else(|| default_level(place.ty))
    };
    candidates.retain(|user| match level_of(*user) {
        NotificationLevel::All => true,
        NotificationLevel::Tags => tagged_users.contains(user),
        NotificationLevel::Nothing => false,
    });
    if let Some(community) = community {
        let viewers = viewers(conn.as_mut(), community, &candidates, place.id).await?;
        candidates.retain(|user| viewers.contains(user));
    }
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let listed: Vec<UserId> = candidates.iter().copied().collect();
    let blocking: Vec<UserId> = user_block::table
        .select(user_block::blocker)
        .filter(user_block::blocked.eq(author))
        .filter(user_block::blocker.eq_any(&listed))
        .load(conn.as_mut())
        .await?;
    let muting: Vec<UserId> = channel_mute::table
        .select(channel_mute::user)
        .filter(channel_mute::channel.eq(place.id))
        .filter(channel_mute::user.eq_any(&listed))
        .filter(
            channel_mute::until
                .is_null()
                .or(channel_mute::until.gt(diesel::dsl::now)),
        )
        .load(conn.as_mut())
        .await?;
    drop(conn);
    for user in blocking.into_iter().chain(muting) {
        candidates.remove(&user);
    }
    let listed: Vec<UserId> = candidates.into_iter().collect();
    if listed.is_empty() {
        return Ok(listed);
    }
    let keys: Vec<String> = listed
        .iter()
        .map(|user| app::user_status::active_key(*user))
        .collect();
    let active: Vec<Option<i64>> = state.valkey.mget(keys).await?;
    Ok(listed
        .into_iter()
        .zip(active)
        .filter_map(|(user, active)| active.is_none().then_some(user))
        .collect())
}

/// The members of `community` with a phone to wake whom `mentions` tags.
async fn tagged(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    mentions: &app::mention::Mentions,
) -> app::Result<HashSet<UserId>> {
    if mentions.users.is_empty() && mentions.roles.is_empty() && !mentions.everyone {
        return Ok(HashSet::new());
    }
    let subscribed = push_subscription::table.select(push_subscription::user);
    let members = community_user::table
        .select(community_user::user)
        .filter(community_user::community.eq(community))
        .filter(community_user::user.eq_any(subscribed))
        .into_boxed();
    let members: Vec<UserId> = if mentions.everyone {
        members.load(conn).await?
    } else {
        let holders = community_member_role::table
            .select(community_member_role::user)
            .filter(community_member_role::role.eq_any(mentions.roles.clone()));
        members
            .filter(
                community_user::user
                    .eq_any(mentions.users.clone())
                    .or(community_user::user.eq_any(holders)),
            )
            .load(conn)
            .await?
    };
    Ok(members.into_iter().collect())
}

/// What the phone's badge shows for this deployment: the unread messages tagging the person,
/// and their unread DMs.
async fn badge_of(state: &GlobalServerContext, user: UserId) -> app::Result<i64> {
    let mut conn = state.connection_pool.get().await?;
    let communities: Vec<CommunityId> = community_user::table
        .select(community_user::community)
        .filter(community_user::user.eq(user))
        .load(conn.as_mut())
        .await?;
    let dms: Vec<ChannelId> = dm_recipient::table
        .select(dm_recipient::channel)
        .filter(dm_recipient::user.eq(user))
        .load(conn.as_mut())
        .await?;
    drop(conn);
    let visible = app::visibility::Visibility::load(state, user, &communities).await?;
    let (in_communities, in_dms) = tokio::try_join!(
        app::read_state::read_communities_read_states(state, &visible),
        app::read_state::read_channels_read_states(state, user, &dms),
    )?;
    let tags: i64 = in_communities.iter().map(|s| i64::from(s.mentions)).sum();
    let unread_dms = in_dms
        .iter()
        .filter(|s| s.last_message.is_some_and(|last| last > s.last_read))
        .count() as i64;
    Ok(tags + unread_dms)
}
