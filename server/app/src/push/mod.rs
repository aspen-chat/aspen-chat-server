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
//! in Valkey for [`REMEMBERED`]. A DM's call ringing someone (`app::voice::ring`) wakes every
//! phone of theirs with a `call` pointer, which carries what a phone must show before it reaches
//! the network; nothing is pushed when the ring ends, which the phone learns of through the
//! event stream and the ring's own end by the clock. An event is acknowledged once its pushes
//! are decided and queued ([`queue`]); push services' answers are waited for there, never by the
//! dispatcher.

pub use aspen_webpush as webpush;

mod queue;

use crate::channel::Channel;
use crate::context::GlobalServerContext;
use crate::events::{SubjectOwner, subject_owner};
use crate::message::Message;
use crate::notification_setting::default_level;
use crate::t;
use crate::two_factor::Caller;
use crate::visibility::viewers;
use crate::{
    ASPEN_NATS_STREAM_NAME, ChannelId, CommunityId, MessageId, PluginNoticeId, PushKeyId,
    PushSubscriptionId, UserId, VoiceSessionId,
};
use aspen_schema::{
    self as schema, channel, community_user, dm_recipient, message, push_key, push_subscription,
    read_state, refresh_token, voice_ring, voice_session,
};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, Bool, Nullable, Text, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
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
/// How many people one event's pushes are prepared for at once: a message tagging everyone in a
/// large community wakes thousands, each needing a Valkey write and, for a badge, database
/// reads, before their pushes are queued.
const FAN_OUT: usize = 128;
/// How long the dispatcher waits for what an event announces to be committed: events are
/// published before their transaction commits, and one whose change is not there by then was
/// rolled back.
const COMMIT_WAIT: Duration = Duration::from_secs(10);
/// How long it waits before it first looks again; each wait after doubles, up to a second.
const FIRST_LOOK_AGAIN: Duration = Duration::from_millis(50);
/// How long whom a message woke, and for what, is remembered: longer than anyone leaves a
/// notification unread on a phone that has not been turned on meanwhile.
const REMEMBERED: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How long a push is worth delivering to a phone that cannot be reached now. A `call` pointer
/// is worth delivering only until its ring ends (`Pointer::time_to_live`).
const TIME_TO_LIVE: Duration = Duration::from_secs(24 * 60 * 60);
/// The header that tells the relay a push is a call, which it cannot read from the ciphertext,
/// so that it sends the push as the platform's call push (`spec/push.md`, Calls). Sent on `call`
/// pointers alone; the relay learns that a call is ringing the phone, and nothing else.
const KIND_HEADER: &str = "Aspen-Push-Kind";
/// Above this many people woken by one message, the badge count is left out: working out one
/// not kept (`badge_of`) is several queries per person.
const MAX_BADGED: usize = 100;
/// How long a person's badge, once counted, is kept and adjusted rather than counted again
/// (`badge_of`). A change no push follows (a role taken away, a channel hidden from them) shows
/// on their phone within it.
const BADGE_KEPT: Duration = Duration::from_secs(300);
/// The longest endpoint URL taken.
const MAX_ENDPOINT_CHARS: usize = 2048;
/// The most phones one account has woken. Subscribing another lets go of the oldest beyond it,
/// and of any whose sign-in has expired, so a message tagging someone costs a bounded number
/// of pushes however often they sign in.
const MAX_SUBSCRIPTIONS_PER_USER: i64 = 10;
/// How much of a push service's refusal is read, for the log.
const MAX_REFUSAL_BYTES: usize = 4096;

/// The key this server signs with, loaded once it exists.
static KEY: OnceCell<(PushKeyId, Arc<PushKey>)> = OnceCell::const_new();

fn key_error(error: impl std::fmt::Display) -> crate::Error {
    crate::Error::Config(config::ConfigError::Message(format!("push key: {error}")))
}

/// Makes the deployment's push key when it has none, as its first server starts, and loads it.
/// Servers starting together make one: the current key is unique.
pub async fn ensure_key(conn: &mut AsyncPgConnection) -> crate::Result<()> {
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
) -> crate::Result<PushSubscription> {
    let Some((key, _)) = current_key().filter(|_| state.config.push.enabled) else {
        return Err(crate::Error::Validation(t!("pushOff")));
    };
    if caller.bot {
        return Err(crate::Error::Validation(t!("pushBots")));
    }
    let endpoint = reqwest::Url::parse(&new.endpoint).ok().filter(|url| {
        url.scheme() == "https"
            && url.host_str().is_some()
            && new.endpoint.len() <= MAX_ENDPOINT_CHARS
    });
    let Some(endpoint) = endpoint else {
        return Err(crate::Error::Validation(t!(
            "pushEndpoint",
            max = MAX_ENDPOINT_CHARS
        )));
    };
    if reaches_inside(state, &endpoint) {
        return Err(crate::Error::Validation(t!(
            "pushEndpointInside",
            host = endpoint.host_str().unwrap_or_default().to_string()
        )));
    }
    if new.p256dh.len() != 65 || new.p256dh[0] != 4 || new.auth.len() != 16 {
        return Err(crate::Error::Validation(t!("pushKeys")));
    }
    let mut conn = state.connection_pool.get().await?;
    let user = caller.user;
    conn.transaction(|conn| {
        async move {
            // Subscriptions of one account are made one at a time, so the cap holds however
            // many arrive at once.
            schema::user::table
                .select(schema::user::id)
                .filter(schema::user::id.eq(user))
                .for_no_key_update()
                .first::<UserId>(conn)
                .await?;
            let made = diesel::insert_into(push_subscription::table)
                .values((
                    push_subscription::id.eq(PushSubscriptionId::new()),
                    push_subscription::user.eq(user),
                    push_subscription::refresh_token.eq(&caller.refresh_digest),
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
                .get_result(conn)
                .await?;
            let kept: Vec<PushSubscriptionId> = push_subscription::table
                .inner_join(refresh_token::table)
                .select(push_subscription::id)
                .filter(push_subscription::user.eq(user))
                .filter(refresh_token::expires.gt(diesel::dsl::now))
                .order((
                    push_subscription::created_at.desc(),
                    push_subscription::id.desc(),
                ))
                .limit(MAX_SUBSCRIPTIONS_PER_USER)
                .load(conn)
                .await?;
            diesel::delete(
                push_subscription::table
                    .filter(push_subscription::user.eq(user))
                    .filter(push_subscription::id.ne_all(kept)),
            )
            .execute(conn)
            .await?;
            Ok(made)
        }
        .scope_boxed()
    })
    .await
}

/// Stops waking one of the caller's phones.
pub async fn unsubscribe(
    state: &GlobalServerContext,
    user: UserId,
    id: PushSubscriptionId,
) -> crate::Result<()> {
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
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
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
    /// A plugin tells the person of something in the channel (`app::plugin::notice`).
    Notice {
        channel: ChannelId,
        notice: PluginNoticeId,
    },
    /// A DM's call is ringing the person (`app::voice::ring`). The phone shows the call before
    /// it reaches the network, so this one pointer names a person: the caller, as they are
    /// shown (`callerName`, their display name or else their username, at most
    /// `user::DISPLAY_NAME_MAX_CHARS` characters). `until` is when the ring ends, on the
    /// server's clock; the phone stops ringing then, or sooner when the event stream says the
    /// ring ended.
    #[serde(rename_all = "camelCase")]
    Call {
        channel: ChannelId,
        session: VoiceSessionId,
        caller: UserId,
        caller_name: String,
        until: DateTime<Utc>,
    },
}

impl Pointer {
    fn channel(&self) -> ChannelId {
        match self {
            Pointer::Message { channel, .. }
            | Pointer::Read { channel, .. }
            | Pointer::Deleted { channel, .. }
            | Pointer::Notice { channel, .. }
            | Pointer::Call { channel, .. } => *channel,
        }
    }

    /// RFC 8030's urgency: a message is for now, and a pointer that shows nothing is `low`,
    /// which a relay whose app cannot hide a notification leaves undelivered (`spec/push.md`).
    fn urgency(&self) -> &'static str {
        match self {
            Pointer::Message { .. } | Pointer::Notice { .. } | Pointer::Call { .. } => "high",
            Pointer::Read { .. } | Pointer::Deleted { .. } => "low",
        }
    }

    /// RFC 8030's topic: a later push with the same one replaces this one at the push service if
    /// it has not been delivered. A channel's pushes share one, so a burst in one channel wakes
    /// the phone once; a call's is its session's, so a ring never replaces a message's push, nor
    /// the other way round. An id without its hyphens is 32 characters of the alphabet RFC 8030
    /// allows.
    fn topic(&self) -> String {
        match self {
            Pointer::Message { .. }
            | Pointer::Read { .. }
            | Pointer::Deleted { .. }
            | Pointer::Notice { .. } => self.channel().0.simple().to_string(),
            Pointer::Call { session, .. } => session.0.simple().to_string(),
        }
    }

    /// How long the push is worth delivering to a phone that cannot be reached now: a ring, only
    /// until it ends (`now` is the clock it is judged by; a ring already over is worth nothing);
    /// everything else, [`TIME_TO_LIVE`].
    fn time_to_live(&self, now: DateTime<Utc>) -> Duration {
        match self {
            Pointer::Message { .. }
            | Pointer::Read { .. }
            | Pointer::Deleted { .. }
            | Pointer::Notice { .. } => TIME_TO_LIVE,
            Pointer::Call { until, .. } => (*until - now).to_std().unwrap_or(Duration::ZERO),
        }
    }

    /// What the relay is told of the push besides the Web Push headers: that it is a call, for
    /// the one kind the relay must send differently (`spec/push.md`, Calls); nothing of any
    /// other.
    fn kind_header(&self) -> Option<&'static str> {
        match self {
            Pointer::Message { .. }
            | Pointer::Read { .. }
            | Pointer::Deleted { .. }
            | Pointer::Notice { .. } => None,
            Pointer::Call { .. } => Some("call"),
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
) -> crate::Result<HashMap<UserId, Vec<PushSubscription>>> {
    use aspen_schema::{refresh_token, user};
    let mut conn = state.connection_pool.get().await?;
    let mut phones: HashMap<UserId, Vec<PushSubscription>> = HashMap::new();
    for (user, subscription) in push_subscription::table
        .inner_join(refresh_token::table)
        .inner_join(user::table)
        .select((push_subscription::user, PushSubscription::as_select()))
        .filter(push_subscription::user.eq_any(users))
        .filter(refresh_token::expires.gt(diesel::dsl::now))
        .filter(user::deleted_at.is_null())
        .filter(diesel::dsl::not(crate::user_ban::banned()))
        .load::<(UserId, PushSubscription)>(conn.as_mut())
        .await?
    {
        phones.entry(user).or_default().push(subscription);
    }
    Ok(phones)
}

/// Queues `pointer` for each of `phones` ([`queue`]), which sends it without the caller waiting
/// for push services to answer.
async fn wake(
    state: &GlobalServerContext,
    phones: &[PushSubscription],
    pointer: &Pointer,
    badge: Option<i64>,
) {
    for subscription in phones {
        queue::enqueue(state, subscription.clone(), pointer.clone(), badge).await;
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
    /// Held without its URL, the endpoint, which is a capability to wake the phone and so is
    /// kept out of logs.
    #[error("{0}")]
    Http(reqwest::Error),
    #[error("the push service answered {0}: {1}")]
    Refused(reqwest::StatusCode, String),
}

impl From<reqwest::Error> for SendError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error.without_url())
    }
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
    let now = Utc::now();
    let time_to_live = pointer.time_to_live(now);
    if time_to_live.is_zero() {
        // A ring already over wakes no one.
        return Ok(Delivery::Accepted);
    }
    let kind_header = pointer.kind_header();
    let urgency = pointer.urgency();
    let topic = pointer.topic();
    let payload = serde_json::to_vec(&Payload {
        v: 1,
        pointer,
        badge,
    })
    .expect("pointers serialize");
    let body = webpush::encrypt(&payload, &subscription.p256dh, &subscription.auth)?;
    let url = reqwest::Url::parse(&subscription.endpoint)
        .map_err(|_| SendError::Refused(reqwest::StatusCode::BAD_REQUEST, "endpoint".into()))?;
    if reaches_inside(state, &url) {
        return Ok(Delivery::Gone);
    }
    let audience = url.origin().ascii_serialization();
    let subject = state
        .config
        .federation
        .domain
        .as_ref()
        .map(|domain| format!("https://{domain}"));
    let authorization = key.authorization(&audience, subject.as_deref(), now.timestamp())?;
    let mut request = state
        .push_client
        .post(url)
        .header("Content-Encoding", "aes128gcm")
        .header("Content-Type", "application/octet-stream")
        .header("TTL", time_to_live.as_secs().max(1).to_string())
        .header("Urgency", urgency)
        .header("Topic", topic)
        .header("Authorization", authorization);
    if let Some(kind) = kind_header {
        request = request.header(KIND_HEADER, kind);
    }
    let response = request
        .timeout(queue::SEND_TIMEOUT)
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
    // A push service is a host a client chose, so only the start of what it says is read.
    let mut detail = Vec::new();
    let mut body = response.bytes_stream();
    while detail.len() < MAX_REFUSAL_BYTES {
        match body.next().await {
            Some(Ok(chunk)) => detail.extend_from_slice(&chunk),
            _ => break,
        }
    }
    detail.truncate(MAX_REFUSAL_BYTES);
    Err(SendError::Refused(
        status,
        String::from_utf8_lossy(&detail).chars().take(500).collect(),
    ))
}

/// The client every push is made with: like the federation client, but reaching only public
/// addresses, and this machine's when `[federation.development]` allows private addresses (which
/// only a deployment at `localhost` may), so a push endpoint someone registers can never reach
/// into this server's network.
pub fn client(config: &crate::aspen_config::FederationConfig) -> crate::Result<reqwest::Client> {
    crate::federation::fetch::builder(
        config,
        crate::outbound::PublicResolver {
            allow_private: false,
            allow_loopback: config.development.allow_private_addresses,
        },
    )?
    .user_agent(concat!("Aspen/", env!("CARGO_PKG_VERSION"), " (push)"))
    .build()
    .map_err(|e| {
        crate::Error::Config(config::ConfigError::Message(format!(
            "building the push client: {e}"
        )))
    })
}

/// Whether `endpoint` names an address inside a network, which pushes never go to, but for this
/// machine's when `[federation.development]` allows private addresses. A name is checked as it is
/// connected to (`push_client`'s `app::outbound::PublicResolver`); an address is connected to
/// without being resolved, so it is checked here, when a subscription is made and again before
/// each push.
fn reaches_inside(state: &GlobalServerContext, endpoint: &reqwest::Url) -> bool {
    let loopback = match endpoint.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    };
    crate::outbound::names_inside_address(endpoint)
        && !(loopback && state.config.federation.development.allow_private_addresses)
}

/// Wakes `user`'s phones for a plugin's notice to them, unless they are in front of Aspen now
/// (`app::attention`) or in do not disturb, as for a message (`spec/push.md`).
pub async fn notice(
    state: &GlobalServerContext,
    user: UserId,
    channel: ChannelId,
    notice: PluginNoticeId,
) {
    if !state.config.push.enabled {
        return;
    }
    let woken = async {
        if crate::attention::attended(state, &[user]).await?[0]
            || crate::presence_override::do_not_disturb(
                state.connection_pool.get().await?.as_mut(),
                user,
            )
            .await?
        {
            return Ok(());
        }
        let phones = phones_of(state, &[user]).await?;
        let badge = badge_of(state, user).await.ok();
        wake(
            state,
            phones.get(&user).map_or(&[], Vec::as_slice),
            &Pointer::Notice { channel, notice },
            badge,
        )
        .await;
        Ok::<_, crate::Error>(())
    }
    .await;
    if let Err(e) = woken {
        tracing::warn!("waking phones for a plugin's notice failed: {e}");
    }
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

/// How long an event whose handling failed waits to be delivered again.
const RETRY_AFTER: Duration = Duration::from_secs(5);

async fn dispatch(state: &GlobalServerContext) -> anyhow::Result<()> {
    state
        .bus
        .durable(
            ASPEN_NATS_STREAM_NAME,
            aspen_bus::DurableConfig {
                name: CONSUMER.into(),
                filter: format!("{}.>", crate::events::SUBJECT_ROOT),
                deliver: aspen_bus::Deliver::New,
                ack_wait: Duration::from_secs(60),
                max_deliver: Some(3),
                max_ack_pending: None,
            },
        )
        .await?
        .for_each_concurrent(CONCURRENCY, |received| async move {
            let Ok(received) = received else {
                return;
            };
            let event = &received.message;
            // A failure is delivered again, up to `max_deliver` times in all.
            let acknowledged = match handle(state, &event.subject, &event.payload).await {
                Ok(()) => received.ack().await,
                Err(e) => {
                    tracing::warn!("handling an event for push failed: {e}");
                    received.nak(Some(RETRY_AFTER)).await
                }
            };
            if let Err(e) = acknowledged {
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
    last_message: Option<MessageId>,
    /// A `voiceRing`'s call, and whom it rings.
    session: Option<VoiceSessionId>,
    user: Option<UserId>,
}

async fn handle(state: &GlobalServerContext, subject: &str, payload: &[u8]) -> crate::Result<()> {
    let Ok(seen) = serde_json::from_slice::<Seen>(payload) else {
        return Ok(());
    };
    match (seen.server_event.as_str(), seen.change.as_deref()) {
        ("message", Some("create")) => {
            if let Some(id) = seen.id {
                let key = format!("push:created:{}", id.0);
                if first_time(state, &key).await? {
                    released_on_failure(state, &key, message_created(state, id).await).await?;
                }
            }
        }
        ("message", Some("delete")) => {
            if let Some(id) = seen.id {
                let key = format!("push:deleted:{}", id.0);
                if first_time(state, &key).await? {
                    released_on_failure(state, &key, message_deleted(state, id).await).await?;
                }
            }
        }
        ("channelRead", _) => {
            if let (Some(SubjectOwner::User(user)), Some(channel), Some(last_read)) =
                (subject_owner(subject), seen.channel, seen.last_read)
            {
                channel_read(state, user, channel, last_read).await?;
            }
        }
        ("channelMarkedUnread", _) => {
            if let (Some(SubjectOwner::User(user)), Some(channel), Some(last_read)) =
                (subject_owner(subject), seen.channel, seen.last_read)
            {
                channel_marked_unread(state, user, channel, last_read, seen.last_message).await?;
            }
        }
        // A ring's `create` is published once to each person of the DM, and only the one rung
        // is woken; its `delete` wakes no one, as Apple's rules for call pushes ask.
        ("voiceRing", Some("create")) => {
            if let (Some(session), Some(user)) = (seen.session, seen.user) {
                let key = format!("push:ring:{}:{}", session.0, user.0);
                if first_time(state, &key).await? {
                    released_on_failure(state, &key, ring_made(state, session, user).await).await?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Whether this is the first time `key` is seen: a DM's events are published once for each of
/// its people, and each copy must wake them once.
async fn first_time(state: &GlobalServerContext, key: &str) -> crate::Result<bool> {
    Ok(state
        .ephemeral
        .set(
            key,
            1,
            Some(aspen_ephemeral::Expiry::Seconds(600)),
            Some(aspen_ephemeral::Condition::Absent),
        )
        .await?)
}

/// `result`, of handling what `first_time` claimed as `key`; a failure lets the claim go, so
/// that the event delivered again is handled. Handling fails only before it wakes anyone.
async fn released_on_failure(
    state: &GlobalServerContext,
    key: &str,
    result: crate::Result<()>,
) -> crate::Result<()> {
    if result.is_err()
        && let Err(e) = state.ephemeral.del(key).await
    {
        tracing::warn!("letting go of a push claim failed: {e}");
    }
    result
}

/// What `look` finds once it finds anything, or `None` if it has found nothing within
/// `COMMIT_WAIT`: what an event announces is committed moments after the event is published.
async fn once_committed<T, Looking>(mut look: impl FnMut() -> Looking) -> crate::Result<Option<T>>
where
    Looking: std::future::Future<Output = crate::Result<Option<T>>>,
{
    let deadline = tokio::time::Instant::now() + COMMIT_WAIT;
    let mut wait = FIRST_LOOK_AGAIN;
    loop {
        if let Some(found) = look().await? {
            return Ok(Some(found));
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(wait.min(deadline - now)).await;
        wait = (wait * 2).min(Duration::from_secs(1));
    }
}

fn woken_key(user: UserId, channel: ChannelId) -> String {
    format!("push:woken:{}:{}", user.0, channel.0)
}

fn badge_key(user: UserId) -> String {
    format!("push:badge:{}", user.0)
}

fn message_key(message: MessageId) -> String {
    format!("push:message:{}", message.0)
}

async fn message_created(state: &GlobalServerContext, id: MessageId) -> crate::Result<()> {
    // A message deleted since is found too, so that waiting ends there.
    let found = once_committed(|| async move {
        let mut conn = state.connection_pool.get().await?;
        Ok(message::table
            .select(Message::as_select())
            .filter(message::id.eq(id))
            .first(conn.as_mut())
            .await
            .optional()?)
    })
    .await?;
    let Some(found) = found.filter(|found| found.deleted_at.is_none()) else {
        return Ok(());
    };
    if !found.kind.notifies() {
        return Ok(());
    }
    let channel_id = *found.channel.id();
    let author = *found.author.id();
    let (recipients, in_community) = recipients(state, &found, channel_id, author).await?;
    if recipients.is_empty() {
        return Ok(());
    }
    // Whom it tags, in a community: their kept badges grow by one, the message being unread
    // and tagging them. A DM's people's are counted afresh, since a DM already unread counts
    // once however many of its messages are.
    let tagged: HashSet<UserId> = recipients
        .iter()
        .filter(|(_, tagged)| *tagged && in_community)
        .map(|(user, _)| *user)
        .collect();
    let recipients: Vec<UserId> = recipients.into_iter().map(|(user, _)| user).collect();
    let pointer = Pointer::Message {
        channel: channel_id,
        message: id,
    };
    let badge = recipients.len() <= MAX_BADGED;
    state
        .ephemeral
        .sadd(
            &message_key(id),
            recipients
                .iter()
                .map(|u| u.0.to_string())
                .collect::<Vec<_>>(),
        )
        .await?;
    state
        .ephemeral
        .expire(&message_key(id), REMEMBERED.as_secs() as i64)
        .await?;
    let phones = &phones_of(state, &recipients).await?;
    let tagged = &tagged;
    let pointer = &pointer;
    futures_util::stream::iter(recipients)
        .for_each_concurrent(FAN_OUT, |user| async move {
            let _ = state
                .ephemeral
                .set(
                    &woken_key(user, channel_id),
                    id.0.to_string(),
                    Some(aspen_ephemeral::Expiry::Seconds(REMEMBERED.as_secs() as i64)),
                    None,
                )
                .await;
            // A kept badge follows the message whether or not this push carries it.
            let grown = if tagged.contains(&user) {
                grow_badge(state, user).await
            } else {
                if !in_community {
                    forget_badge(state, user).await;
                }
                None
            };
            let badge = match (badge, grown) {
                (false, _) => None,
                (true, Some(grown)) => Some(grown),
                (true, None) => badge_of(state, user).await.ok(),
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

/// Wakes every phone of `user` for the call of `session` ringing them: all of them, whether or
/// not they are using Aspen elsewhere, since a ring reaches every device, as a phone call does.
/// Who is rung at all (not those in do not disturb) is `app::voice::ring`'s decision, made before
/// the ring exists; this only carries it to the phones.
async fn ring_made(
    state: &GlobalServerContext,
    session: VoiceSessionId,
    user: UserId,
) -> crate::Result<()> {
    use aspen_schema::user as user_table;
    // The ring, with its channel and its caller's shown name, once the transaction that rang has
    // committed. A ring already ended (`end_ring` deletes its row) is not found, and wakes no
    // one: there is nothing left to answer. Waiting for it costs `COMMIT_WAIT` at most.
    let found = once_committed(|| async move {
        let mut conn = state.connection_pool.get().await?;
        Ok(voice_ring::table
            .inner_join(voice_session::table)
            .inner_join(user_table::table.on(user_table::id.eq(voice_ring::caller)))
            .select((
                voice_session::channel,
                voice_ring::caller,
                user_table::name,
                user_table::display_name,
                voice_ring::until,
            ))
            .filter(voice_ring::session.eq(session))
            .filter(voice_ring::user.eq(user))
            .first::<(ChannelId, UserId, String, Option<String>, DateTime<Utc>)>(conn.as_mut())
            .await
            .optional()?)
    })
    .await?;
    let Some((channel, caller, name, display_name, until)) = found else {
        return Ok(());
    };
    let phones = phones_of(state, &[user]).await?;
    wake(
        state,
        phones.get(&user).map_or(&[], Vec::as_slice),
        &Pointer::Call {
            channel,
            session,
            caller,
            caller_name: display_name.unwrap_or(name),
            until,
        },
        None,
    )
    .await;
    Ok(())
}

async fn message_deleted(state: &GlobalServerContext, id: MessageId) -> crate::Result<()> {
    let woken: Vec<String> = state.ephemeral.smembers(&message_key(id)).await?;
    if woken.is_empty() {
        return Ok(());
    }
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
    // The message no longer counts toward their badges, which are counted afresh when next
    // shown.
    state
        .ephemeral
        .del(
            woken
                .iter()
                .map(|user| badge_key(*user))
                .collect::<Vec<_>>(),
        )
        .await?;
    // Forgotten only now, so that a failure before here leaves it for the event's next delivery.
    state.ephemeral.del(message_key(id)).await?;
    futures_util::stream::iter(phones.values())
        .for_each_concurrent(FAN_OUT, |phones| wake(state, phones, &pointer, None))
        .await;
    Ok(())
}

async fn channel_read(
    state: &GlobalServerContext,
    user: UserId,
    channel: ChannelId,
    last_read: MessageId,
) -> crate::Result<()> {
    let woken: Option<String> = state.ephemeral.get(&woken_key(user, channel)).await?;
    let Some(woken) = woken.and_then(|w| w.parse().ok().map(MessageId)) else {
        return Ok(());
    };
    // Ids are UUIDv7, so a later message has a greater id: reading up to or past the message
    // that woke the phone takes its notifications down.
    if woken > last_read {
        return Ok(());
    }
    state.ephemeral.del(woken_key(user, channel)).await?;
    forget_badge(state, user).await;
    // The badge is counted once the position has moved.
    once_committed(|| async move {
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
        Ok(recorded
            .is_some_and(|recorded| recorded >= last_read)
            .then_some(()))
    })
    .await?;
    let phones = phones_of(state, &[user]).await?;
    let Some(phones) = phones.get(&user) else {
        return Ok(());
    };
    wake(
        state,
        phones,
        &Pointer::Read {
            channel,
            message: last_read,
        },
        shown_badge(state, user).await.ok(),
    )
    .await;
    Ok(())
}

/// Tells `user`'s phones their badge after they marked `channel` unread on any device, as a
/// `read` pointer at the new position, which takes down no notification the position had not
/// already passed. A channel that is still read once marked (`last_message` is `None`) changes no
/// badge, and wakes nothing.
async fn channel_marked_unread(
    state: &GlobalServerContext,
    user: UserId,
    channel: ChannelId,
    last_read: MessageId,
    last_message: Option<MessageId>,
) -> crate::Result<()> {
    if last_message.is_none() {
        return Ok(());
    }
    // The badge is counted afresh once the position has moved back. A position the join bounds is
    // recorded before it, so the recorded one is at most `last_read` once committed.
    once_committed(|| async move {
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
        Ok(recorded
            .is_some_and(|recorded| recorded <= last_read)
            .then_some(()))
    })
    .await?;
    forget_badge(state, user).await;
    let phones = phones_of(state, &[user]).await?;
    let Some(phones) = phones.get(&user) else {
        return Ok(());
    };
    wake(
        state,
        phones,
        &Pointer::Read {
            channel,
            message: last_read,
        },
        shown_badge(state, user).await.ok(),
    )
    .await;
    Ok(())
}

/// Whom a new message is for, among those with a phone to wake (`spec/push.md`, Who is woken):
/// the people of a DM, or the members of a community who may read it, as far as each one's
/// notification setting for the channel asks (`app::notification_setting`: every message, or
/// only those that tag them), and those following the thread it is a reply in
/// (`app::thread_follow`); never its author, anyone who blocked them, anyone who muted the
/// channel or its category (a thread counting as its parent), or anyone using Aspen right now.
async fn recipients(
    state: &GlobalServerContext,
    found: &Message,
    channel_id: ChannelId,
    author: UserId,
) -> crate::Result<(Vec<(UserId, bool)>, bool)> {
    let mut conn = state.connection_pool.get().await?;
    let posted_in: Channel = channel::table
        .select(Channel::as_select())
        .find(channel_id)
        .first(conn.as_mut())
        .await?;
    // A thread's messages are its parent's, for who may read them, settings, and muting.
    let posted_in_thread = posted_in.parent_channel.is_some();
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
    let small = match community {
        Some(community) => {
            crate::large_community::is_small(state, conn.as_mut(), community).await?
        }
        None => false,
    };
    let mentions = &found.mentions;
    let mut candidates: HashMap<UserId, bool> = diesel::sql_query(RECIPIENTS_SQL)
        .bind::<Nullable<PgUuid>, _>(community.map(|c| c.0))
        .bind::<PgUuid, _>(place.id.0)
        .bind::<PgUuid, _>(author.0)
        .bind::<Array<PgUuid>, _>(mentions.users.iter().map(|u| u.0).collect::<Vec<_>>())
        .bind::<Array<PgUuid>, _>(mentions.roles.iter().map(|r| r.0).collect::<Vec<_>>())
        .bind::<Bool, _>(mentions.everyone)
        .bind::<Text, _>(default_level(place.ty, small))
        .bind::<Nullable<PgUuid>, _>(posted_in_thread.then_some(channel_id.0))
        .bind::<Nullable<PgUuid>, _>(place.parent_category.as_ref().map(|c| c.id().0))
        .load::<Recipient>(conn.as_mut())
        .await?
        .into_iter()
        .map(|r| (r.user, r.tagged))
        .collect();
    if let Some(community) = community
        && !candidates.is_empty()
    {
        let users: HashSet<UserId> = candidates.keys().copied().collect();
        let viewers = viewers(conn.as_mut(), community, &users, place.id).await?;
        candidates.retain(|user, _| viewers.contains(user));
    }
    drop(conn);
    let listed: Vec<(UserId, bool)> = candidates.into_iter().collect();
    // Those in front of Aspen right now are not woken: what they are using tells them.
    let users: Vec<UserId> = listed.iter().map(|(user, _)| *user).collect();
    let attended = crate::attention::attended(state, &users).await?;
    let woken = listed
        .into_iter()
        .zip(attended)
        .filter_map(|(recipient, attended)| (!attended).then_some(recipient))
        .collect();
    Ok((woken, community.is_some()))
}

#[derive(diesel::QueryableByName)]
struct Recipient {
    #[diesel(sql_type = PgUuid)]
    user: UserId,
    /// Whether the message tags them, by name, a role, or everyone.
    #[diesel(sql_type = Bool)]
    tagged: bool,
}

/// Who a message wakes, before who may view it and who is using Aspen are decided: `$1` the
/// community (`NULL` in a DM), `$2` the channel whose settings and mutes count (a thread's
/// parent), `$3` the author, `$4`/`$5`/`$6` whom it tags (users, roles, everyone), `$7` the level
/// with no setting, `$8` the thread it is a reply in (`NULL` when it is in none), `$9` the
/// channel's category (`NULL` when it is in none). Candidates come from indexed branches: the
/// members tagged by name (`community_user`'s key), holders of tagged roles
/// (`community_member_role_by_role`), every member when everyone is tagged or `$7` asks for
/// every message (only in a small community, which `app::large_community` bounds by the
/// deployment's threshold), those whose setting asks for every message
/// (`notification_setting_all_*`), a DM's people, and the thread's followers
/// (`thread_follow_by_thread`), a DM's thread's only while they are still its people. Each one's
/// level is the channel's setting, else its category's, else the community's, else `$7`, and a
/// follower is told whatever theirs; those with a phone who neither blocked the author, muted
/// the channel or its category (`app::channel_mute::muted_sql`), nor are in do not disturb
/// (`app::presence_override`) remain.
const RECIPIENTS_SQL: &str = concat!(
    r#"
    WITH candidate AS (
        SELECT cu."user", true AS tagged, false AS following FROM community_user cu
        WHERE cu.community = $1 AND cu."user" = ANY($4)
        UNION ALL
        SELECT r."user", true, false FROM community_member_role r
        WHERE r.community = $1 AND r.role = ANY($5)
        UNION ALL
        SELECT cu."user", true, false FROM community_user cu
        WHERE $6 AND cu.community = $1
        UNION ALL
        SELECT cu."user", false, false FROM community_user cu
        WHERE NOT $6 AND $7 = 'all' AND cu.community = $1
        UNION ALL
        SELECT ns."user", false, false FROM notification_setting ns
        WHERE $1 IS NOT NULL AND ns.level = 'all' AND ns.channel = $2
        UNION ALL
        SELECT ns."user", false, false FROM notification_setting ns
        WHERE ns.level = 'all' AND ns.category = $9
        UNION ALL
        SELECT ns."user", false, false FROM notification_setting ns
        WHERE ns.level = 'all' AND ns.community = $1
        UNION ALL
        SELECT dr."user", $6 OR dr."user" = ANY($4), false FROM dm_recipient dr
        WHERE $1 IS NULL AND dr.channel = $2
        UNION ALL
        SELECT tf."user", false, true FROM thread_follow tf
        WHERE tf.thread = $8
          AND (
              $1 IS NOT NULL
              OR EXISTS (
                  SELECT 1 FROM dm_recipient dr
                  WHERE dr.channel = $2 AND dr."user" = tf."user"
              )
          )
    ),
    person AS (
        SELECT "user", bool_or(tagged) AS tagged, bool_or(following) AS following
        FROM candidate
        WHERE "user" <> $3
        GROUP BY "user"
    )
    SELECT p."user", p.tagged FROM person p
    CROSS JOIN LATERAL (
        SELECT COALESCE(
            (SELECT level FROM notification_setting
             WHERE "user" = p."user" AND channel = $2),
            (SELECT level FROM notification_setting
             WHERE "user" = p."user" AND category = $9),
            (SELECT level FROM notification_setting
             WHERE "user" = p."user" AND community = $1),
            $7
        ) AS level
    ) l
    WHERE (p.following OR l.level = 'all' OR (l.level = 'tags' AND p.tagged))
      AND EXISTS (SELECT 1 FROM push_subscription s WHERE s."user" = p."user")
      AND NOT EXISTS (
          SELECT 1 FROM user_block b WHERE b.blocker = p."user" AND b.blocked = $3
      )
      AND NOT "#,
    crate::channel_mute::muted_sql!("p.\"user\"", "$2"),
    r#"
      AND NOT EXISTS (
          SELECT 1 FROM "user" u WHERE u.id = p."user" AND "#,
    crate::presence_override::do_not_disturb_sql!("u"),
    r#"
      )
"#
);

/// The badge a push that takes notifications down carries: none at all while the person is in
/// do not disturb, which hides badges (`app::presence_override`), and [`badge_of`] otherwise.
/// Pushes that tell of something new never reach them then, so only these need it.
async fn shown_badge(state: &GlobalServerContext, user: UserId) -> crate::Result<i64> {
    let chosen: Option<String> = state
        .ephemeral
        .get(&crate::presence_override::override_key(user))
        .await?;
    if chosen.and_then(|c| c.parse().ok())
        == Some(crate::presence_override::PresenceOverride::DoNotDisturb)
    {
        return Ok(0);
    }
    badge_of(state, user).await
}

/// What the phone's badge shows for this deployment: the unread messages tagging the person,
/// and their unread DMs. Once counted it is kept for [`BADGE_KEPT`] in Valkey, grown by one for
/// each new message tagging them in a community ([`grow_badge`]) and forgotten when they read,
/// when a DM speaks, or when a message that woke them is deleted ([`forget_badge`]), so a busy
/// community's tags cost a person one count, not one per message.
async fn badge_of(state: &GlobalServerContext, user: UserId) -> crate::Result<i64> {
    let kept: Option<i64> = state.ephemeral.get(&badge_key(user)).await?;
    if let Some(kept) = kept {
        return Ok(kept);
    }
    let counted = count_badge(state, user).await?;
    let _ = state
        .ephemeral
        .set(
            &badge_key(user),
            counted,
            Some(aspen_ephemeral::Expiry::Seconds(BADGE_KEPT.as_secs() as i64)),
            None,
        )
        .await;
    Ok(counted)
}

/// Grows `user`'s kept badge by one more unread message tagging them, answering it; `None` when
/// none is kept, and a count afresh then finds the message, already committed.
async fn grow_badge(state: &GlobalServerContext, user: UserId) -> Option<i64> {
    match state.ephemeral.badge_add(&badge_key(user)).await {
        Ok(grown) => grown,
        Err(e) => {
            tracing::warn!("growing a badge failed: {e}");
            forget_badge(state, user).await;
            None
        }
    }
}

/// Forgets `user`'s kept badge, so it is counted afresh when next shown.
async fn forget_badge(state: &GlobalServerContext, user: UserId) {
    if let Err(e) = state.ephemeral.del(badge_key(user)).await {
        tracing::warn!("forgetting a badge failed: {e}");
    }
}

/// `user`'s badge, counted from the database.
async fn count_badge(state: &GlobalServerContext, user: UserId) -> crate::Result<i64> {
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
    let visible = crate::visibility::Visibility::load(state, user, &communities).await?;
    let (in_communities, in_dms) = tokio::try_join!(
        crate::read_state::read_communities_read_states(state, &visible),
        crate::read_state::read_channels_read_states(state, user, &dms),
    )?;
    let tags: i64 = in_communities.iter().map(|s| i64::from(s.mentions)).sum();
    let unread_dms = in_dms
        .iter()
        .filter(|s| s.last_message.is_some_and(|last| last > s.last_read))
        .count() as i64;
    Ok(tags + unread_dms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_pointer_names_the_ring_and_fits_a_push() {
        let channel = ChannelId::new();
        let session = VoiceSessionId::new();
        let caller = UserId::new();
        let until = DateTime::parse_from_rfc3339("2026-11-15T12:00:30Z")
            .unwrap()
            .with_timezone(&Utc);
        let pointer = Pointer::Call {
            channel,
            session,
            caller,
            // The longest name a person can have, of the widest characters.
            caller_name: "\u{1F600}".repeat(crate::user::DISPLAY_NAME_MAX_CHARS),
            until,
        };
        let json = serde_json::to_value(Payload {
            v: 1,
            pointer: pointer.clone(),
            badge: None,
        })
        .unwrap();
        assert_eq!(json["kind"], "call");
        assert_eq!(json["channel"], channel.0.to_string());
        assert_eq!(json["session"], session.0.to_string());
        assert_eq!(json["caller"], caller.0.to_string());
        assert_eq!(json["callerName"].as_str().unwrap().chars().count(), 32);
        assert_eq!(json["until"], "2026-11-15T12:00:30Z");
        assert!(json.get("badge").is_none());
        // `spec/push.md`: a pointer is at most 1024 bytes, so its ciphertext fits an APNs payload.
        assert!(serde_json::to_vec(&json).unwrap().len() <= 1024);

        assert_eq!(pointer.urgency(), "high");
        assert_eq!(pointer.kind_header(), Some("call"));
        assert_eq!(pointer.topic(), session.0.simple().to_string());
        let before = until - chrono::Duration::seconds(12);
        assert_eq!(pointer.time_to_live(before), Duration::from_secs(12));
        assert_eq!(pointer.time_to_live(until), Duration::ZERO);
        assert_eq!(
            pointer.time_to_live(until + chrono::Duration::seconds(1)),
            Duration::ZERO
        );
    }

    #[test]
    fn every_other_pointer_is_topic_by_channel_with_a_day_to_live_and_no_kind_header() {
        let channel = ChannelId::new();
        let message = MessageId::new();
        let now = Utc::now();
        for pointer in [
            Pointer::Message { channel, message },
            Pointer::Read { channel, message },
            Pointer::Deleted { channel, message },
            Pointer::Notice {
                channel,
                notice: PluginNoticeId::new(),
            },
        ] {
            assert_eq!(pointer.topic(), channel.0.simple().to_string());
            assert_eq!(pointer.time_to_live(now), TIME_TO_LIVE);
            assert_eq!(pointer.kind_header(), None);
        }
        assert_eq!(Pointer::Read { channel, message }.urgency(), "low");
        assert_eq!(Pointer::Message { channel, message }.urgency(), "high");
    }
}
