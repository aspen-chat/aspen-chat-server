//! Where each server event goes. Every event is published on a NATS subject that names whose
//! it is, and an event stream connection receives only the subjects its user is entitled to:
//! their own private subject and every community they belong to. Each API server reads every
//! subject once and routes by owner (`subject_owner`, `app::event_feed`).
//!
//! Subjects:
//! - `aspen.events.c.{community}.ch.{channel}` for what happens in a channel: messages,
//!   reactions, polls, and calls;
//! - `aspen.events.c.{community}.all` for the community itself, its channels, categories,
//!   invites, and memberships;
//! - `aspen.events.u.{user}` for what is the user's alone: their preferences, and their own
//!   membership changes, which are how their connection learns to change what it reads.
//!
//! The channel level is in the subject so that private channels, once permissions exist, can
//! be routed by channel rather than by changing every publisher.
//!
//! A DM or group DM belongs to no community: what happens in it, and the channel itself, is
//! published to each recipient's user subject, so it reaches exactly its recipients with no
//! change to what their connections read. A thread belongs wherever its parent channel does.
//! Where a channel belongs is its `ChannelHome`, which never changes and is cached.
//!
//! An event about a user (their profile) is wanted by everyone who shares a community with
//! them, so it is published once to every community they are in, and once to themselves. A
//! reader in several of those communities receives it more than once; every copy carries the
//! same `Aspen-Event-Id` header, which the client uses to drop repeats. The number of copies
//! is bounded by the membership cap (`limits.max_communities_per_user`).
//!
//! `expected_kind` decides the kind of scope every event must be published with, in a match
//! with no wildcard arm, so adding an entity without deciding its routing does not compile,
//! and `publish_event` refuses a scope of the wrong kind.

use crate::api::message_enum::server_event::ServerEvent;
use crate::app::{
    self, CategoryId, ChannelId, CommunityId, GlobalServerContext, MessageId, UserId,
    VoiceSessionId,
};
use crate::database::schema::{
    category, channel, community_user, dm_recipient, invite, message, voice_session,
};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use futures_util::future::try_join_all;
use uuid::Uuid;

/// The subject prefix of every event; the stream captures `aspen.events.>`.
pub const SUBJECT_ROOT: &str = "aspen.events";
/// The header every copy of an event carries, the same for all its copies.
pub const EVENT_ID_HEADER: &str = "Aspen-Event-Id";

/// Whose an event is, as the publishing code knows it. The ids it does not have in hand are
/// looked up on the caller's connection, so an event published inside a transaction can name
/// rows that transaction wrote.
#[derive(Debug, Clone)]
pub enum EventScope {
    /// What happens in a channel, routed to its community's channel subject, or for a DM (or
    /// a thread in one) to each recipient.
    Channel(ChannelId),
    /// The same, for events that know only their message.
    Message(MessageId),
    /// The same, for events that know only their call.
    Session(VoiceSessionId),
    /// The community itself and what is defined in it.
    Community(CommunityId),
    /// A channel's own events (not what happens in it): its community's subject, or for a DM
    /// (or a thread in one) each recipient's, and `departed`'s, a recipient who has just left
    /// and must learn so.
    ChannelDefinition {
        channel: ChannelId,
        departed: Option<UserId>,
    },
    CommunityOfCategory(CategoryId),
    CommunityOfInvite(String),
    /// A membership: the community sees it, and so does the user, privately.
    Membership {
        community: CommunityId,
        user: UserId,
    },
    /// The user's alone.
    User(UserId),
    /// About the user, for everyone who shares a community with them and for themself.
    UserEverywhere(UserId),
}

/// The kinds of scope, which `expected_kind` pairs with events. `Community` covers a
/// community and what is defined in it, and the definition of a DM, which is its recipients'.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Channel,
    Community,
    Membership,
    User,
    UserEverywhere,
}

impl EventScope {
    pub fn kind(&self) -> ScopeKind {
        match self {
            EventScope::Channel(_) | EventScope::Message(_) | EventScope::Session(_) => {
                ScopeKind::Channel
            }
            EventScope::Community(_)
            | EventScope::ChannelDefinition { .. }
            | EventScope::CommunityOfCategory(_)
            | EventScope::CommunityOfInvite(_) => ScopeKind::Community,
            EventScope::Membership { .. } => ScopeKind::Membership,
            EventScope::User(_) => ScopeKind::User,
            EventScope::UserEverywhere(_) => ScopeKind::UserEverywhere,
        }
    }
}

/// The scope kind each event must be published with. No wildcard arm: a new entity must be
/// placed here before it can be published.
pub fn expected_kind(event: &ServerEvent) -> ScopeKind {
    match event {
        ServerEvent::Message(_)
        | ServerEvent::Poll(_)
        | ServerEvent::Pin(_)
        | ServerEvent::React(_)
        | ServerEvent::VoiceSession(_)
        | ServerEvent::VoiceParticipant(_)
        | ServerEvent::VoiceSessionEnded { .. }
        | ServerEvent::VoiceSpeaking { .. } => ScopeKind::Channel,
        ServerEvent::Community(_)
        | ServerEvent::Channel(_)
        | ServerEvent::Category(_)
        | ServerEvent::Invite(_) => ScopeKind::Community,
        ServerEvent::UserCommunity(_) => ScopeKind::Membership,
        ServerEvent::UserPreferencesChanged { .. } => ScopeKind::User,
        ServerEvent::User(_) => ScopeKind::UserEverywhere,
    }
}

pub fn channel_subject(community: CommunityId, channel: ChannelId) -> String {
    format!("{SUBJECT_ROOT}.c.{}.ch.{}", community.0, channel.0)
}

pub fn community_subject(community: CommunityId) -> String {
    format!("{SUBJECT_ROOT}.c.{}.all", community.0)
}

pub fn user_subject(user: UserId) -> String {
    format!("{SUBJECT_ROOT}.u.{}", user.0)
}

/// Whose a subject is: every event subject belongs to one user or one community.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubjectOwner {
    User(UserId),
    Community(CommunityId),
}

/// The owner named by an event subject, the inverse of `user_subject`, `community_subject`,
/// and `channel_subject`; `None` for anything else.
pub fn subject_owner(subject: &str) -> Option<SubjectOwner> {
    let rest = subject.strip_prefix(SUBJECT_ROOT)?.strip_prefix('.')?;
    let mut tokens = rest.split('.');
    let owner = match (tokens.next()?, tokens.next()?) {
        ("u", id) if tokens.next().is_none() => SubjectOwner::User(UserId(id.parse().ok()?)),
        ("c", id) => {
            let community = CommunityId(id.parse().ok()?);
            match (tokens.next()?, tokens.next(), tokens.next()) {
                ("all", None, None) | ("ch", Some(_), None) => SubjectOwner::Community(community),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some(owner)
}

/// The communities a user belongs to, which is what their event stream reads.
pub async fn memberships(
    conn: &mut AsyncPgConnection,
    user: UserId,
) -> app::Result<Vec<CommunityId>> {
    Ok(community_user::table
        .select(community_user::community)
        .filter(community_user::user.eq(user))
        .load(conn)
        .await?)
}

/// Where a channel belongs: a community, or a DM whose recipients are its only audience. A
/// thread belongs where its parent channel does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelHome {
    Community(CommunityId),
    /// The DM or group DM, which is the channel itself or a thread's parent.
    Direct(ChannelId),
}

/// Where a channel belongs. It never changes, so the answer is kept for the process's life.
pub async fn channel_home(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    channel_id: ChannelId,
) -> app::Result<ChannelHome> {
    let cached = |id: ChannelId| {
        state
            .channel_homes
            .lock()
            .expect("channel home cache")
            .get(&id)
            .copied()
    };
    let remember = |ids: &[ChannelId], home: ChannelHome| {
        let mut homes = state.channel_homes.lock().expect("channel home cache");
        for id in ids {
            homes.insert(*id, home);
        }
    };
    // A thread's parent is a community channel or a DM, never another thread, so at most two
    // rows are read.
    let mut current = channel_id;
    for _ in 0..2 {
        if let Some(home) = cached(current) {
            remember(&[channel_id], home);
            return Ok(home);
        }
        let (community, parent): (Option<CommunityId>, Option<ChannelId>) = channel::table
            .select((channel::community, channel::parent_channel))
            .filter(channel::id.eq(current))
            .first(conn)
            .await?;
        let home = match (community, parent) {
            (Some(community), _) => ChannelHome::Community(community),
            (None, Some(parent)) => {
                current = parent;
                continue;
            }
            (None, None) => ChannelHome::Direct(current),
        };
        remember(&[channel_id, current], home);
        return Ok(home);
    }
    Err(app::Error::EventRouting(format!(
        "channel {channel_id} is a thread of a thread"
    )))
}

/// The people in a DM or group DM.
pub async fn dm_recipients(
    conn: &mut AsyncPgConnection,
    dm: ChannelId,
) -> app::Result<Vec<UserId>> {
    Ok(dm_recipient::table
        .select(dm_recipient::user)
        .filter(dm_recipient::channel.eq(dm))
        .load(conn)
        .await?)
}

async fn recipient_subjects(
    conn: &mut AsyncPgConnection,
    dm: ChannelId,
    departed: Option<UserId>,
) -> app::Result<Vec<String>> {
    let mut users = dm_recipients(conn, dm).await?;
    if let Some(departed) = departed
        && !users.contains(&departed)
    {
        users.push(departed);
    }
    Ok(users.into_iter().map(user_subject).collect())
}

/// Where what happens in a channel is published.
async fn channel_subjects(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    channel_id: ChannelId,
) -> app::Result<Vec<String>> {
    Ok(match channel_home(state, conn, channel_id).await? {
        ChannelHome::Community(community) => vec![channel_subject(community, channel_id)],
        ChannelHome::Direct(dm) => recipient_subjects(conn, dm, None).await?,
    })
}

/// The subjects an event with this scope is published on.
async fn subjects(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    scope: EventScope,
) -> app::Result<Vec<String>> {
    Ok(match scope {
        EventScope::Channel(channel_id) => channel_subjects(state, conn, channel_id).await?,
        EventScope::Message(message_id) => {
            let channel_id: ChannelId = message::table
                .select(message::channel)
                .filter(message::id.eq(message_id))
                .first(conn)
                .await?;
            channel_subjects(state, conn, channel_id).await?
        }
        EventScope::Session(session_id) => {
            let channel_id: ChannelId = voice_session::table
                .select(voice_session::channel)
                .filter(voice_session::id.eq(session_id))
                .first(conn)
                .await?;
            channel_subjects(state, conn, channel_id).await?
        }
        EventScope::Community(community) => vec![community_subject(community)],
        EventScope::ChannelDefinition {
            channel: channel_id,
            departed,
        } => match channel_home(state, conn, channel_id).await? {
            ChannelHome::Community(community) => vec![community_subject(community)],
            ChannelHome::Direct(dm) => recipient_subjects(conn, dm, departed).await?,
        },
        EventScope::CommunityOfCategory(category_id) => {
            let community: CommunityId = category::table
                .select(category::community)
                .filter(category::id.eq(category_id))
                .first(conn)
                .await?;
            vec![community_subject(community)]
        }
        EventScope::CommunityOfInvite(code) => {
            let community: CommunityId = invite::table
                .select(invite::community)
                .filter(invite::code.eq(code))
                .first(conn)
                .await?;
            vec![community_subject(community)]
        }
        EventScope::Membership { community, user } => {
            vec![community_subject(community), user_subject(user)]
        }
        EventScope::User(user) => vec![user_subject(user)],
        EventScope::UserEverywhere(user) => {
            let mut all: Vec<String> = memberships(conn, user)
                .await?
                .into_iter()
                .map(community_subject)
                .collect();
            all.push(user_subject(user));
            all
        }
    })
}

/// Publishes an event to everyone its scope names, and waits for the stream to hold every
/// copy. Called before the transaction that made the change commits, so the stream's order is
/// the database's order and a refused publish rolls the change back.
pub async fn publish_event(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    scope: EventScope,
    event: &ServerEvent,
) -> app::Result<()> {
    let expected = expected_kind(event);
    if scope.kind() != expected {
        return Err(app::Error::EventRouting(format!(
            "event routed with a {:?} scope but needs {:?}",
            scope.kind(),
            expected
        )));
    }
    let subjects = subjects(state, conn, scope).await?;
    let payload: bytes::Bytes = serde_json::to_string(event)?.into_bytes().into();
    let event_id = Uuid::now_v7().to_string();
    let publishes = subjects.into_iter().map(|subject| {
        let mut headers = async_nats::HeaderMap::new();
        headers.insert(EVENT_ID_HEADER, event_id.as_str());
        let payload = payload.clone();
        async move {
            let started = std::time::Instant::now();
            state
                .nats_context
                .publish_with_headers(subject, headers, payload)
                .await?
                .await?;
            metrics::histogram!(aspen_metrics::api::EVENT_PUBLISH_DURATION)
                .record(started.elapsed().as_secs_f64());
            metrics::counter!(aspen_metrics::api::EVENTS_PUBLISHED).increment(1);
            Ok::<(), app::Error>(())
        }
    });
    try_join_all(publishes).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::message_enum::server_event::{
        CommunityEvent, MessageEvent, UserCommunityEvent,
    };

    #[test]
    fn subjects_name_their_owner() {
        let community = CommunityId::new();
        let user = UserId::new();
        assert_eq!(
            subject_owner(&channel_subject(community, ChannelId::new())),
            Some(SubjectOwner::Community(community))
        );
        assert_eq!(
            subject_owner(&community_subject(community)),
            Some(SubjectOwner::Community(community))
        );
        assert_eq!(
            subject_owner(&user_subject(user)),
            Some(SubjectOwner::User(user))
        );
        assert_eq!(subject_owner("aspen.events.u.nope"), None);
        assert_eq!(
            subject_owner(&format!("aspen.events.c.{}.other", community.0)),
            None
        );
        assert_eq!(subject_owner(&format!("aspen.voice.u.{}", user.0)), None);
    }

    #[test]
    fn every_scope_reports_its_kind() {
        let community = CommunityId::new();
        let user = UserId::new();
        assert_eq!(
            EventScope::Channel(ChannelId::new()).kind(),
            ScopeKind::Channel
        );
        assert_eq!(
            EventScope::Message(MessageId::new()).kind(),
            ScopeKind::Channel
        );
        assert_eq!(
            EventScope::Community(community).kind(),
            ScopeKind::Community
        );
        assert_eq!(
            EventScope::CommunityOfInvite("x".into()).kind(),
            ScopeKind::Community
        );
        assert_eq!(
            EventScope::Membership { community, user }.kind(),
            ScopeKind::Membership
        );
        assert_eq!(EventScope::User(user).kind(), ScopeKind::User);
        assert_eq!(
            EventScope::UserEverywhere(user).kind(),
            ScopeKind::UserEverywhere
        );
    }

    #[test]
    fn events_expect_the_scope_of_their_entity() {
        let community = CommunityId::new();
        assert_eq!(
            expected_kind(&ServerEvent::Community(CommunityEvent::Delete {
                id: community
            })),
            ScopeKind::Community
        );
        assert_eq!(
            expected_kind(&ServerEvent::Message(MessageEvent::Delete {
                id: MessageId::new()
            })),
            ScopeKind::Channel
        );
        assert_eq!(
            expected_kind(&ServerEvent::UserCommunity(UserCommunityEvent::Delete {
                community,
                user: UserId::new()
            })),
            ScopeKind::Membership
        );
    }
}
