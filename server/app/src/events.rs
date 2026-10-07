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
//! Every event about a community channel, or happening in one, also carries the
//! `Aspen-Channel` header naming the channel whose View channel permission decides who
//! receives it (a thread's parent), which the event feed filters by.
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

use crate::context::GlobalServerContext;
use crate::permissions::Permission;
use crate::voice::Recheck;
use crate::{CategoryId, ChannelId, CommunityId, MessageId, UserId, VoiceSessionId};
use aspen_schema::{
    category, channel, community_user, dm_recipient, invite, message, voice_session,
};
use aspen_wire::message_enum::server_event::{ServerEvent, VoiceMuteEvent};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use futures_util::future::try_join_all;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// The subject prefix of every event; the stream captures `aspen.events.>`.
pub const SUBJECT_ROOT: &str = "aspen.events";
/// The header every copy of an event carries, the same for all its copies.
pub const EVENT_ID_HEADER: &str = "Aspen-Event-Id";
/// The header naming the community channel an event is about or happens in: the channel whose
/// View channel permission decides who receives it, which for a thread is its parent's.
pub const CHANNEL_HEADER: &str = "Aspen-Channel";
/// The header naming the category an event is about (its own events and its overrides'): the
/// category whose own overrides decide who receives it, by whether they leave the reader View
/// channel there (`app::visibility::CommunityModel::can_view_category`).
pub const CATEGORY_HEADER: &str = "Aspen-Category";
/// The header naming the community permission an event needs besides membership, as its wire
/// name (`manageInvites`); with it, `CREATOR_HEADER` may name one member who receives it anyway.
pub const REQUIRES_HEADER: &str = "Aspen-Requires";
/// The header naming the member who made what an event is about, who receives it without the
/// permission `REQUIRES_HEADER` names.
pub const CREATOR_HEADER: &str = "Aspen-Creator";

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
        | ServerEvent::VoiceRing(_)
        | ServerEvent::VoiceSessionEnded { .. }
        | ServerEvent::VoiceSpeaking { .. }
        | ServerEvent::MessageAnnotation(_) => ScopeKind::Channel,
        // A plugin's event goes where the plugin said: a channel, a community, or one user.
        ServerEvent::PluginEvent {
            channel: Some(_), ..
        } => ScopeKind::Channel,
        // A preview goes where its attachment is seen: a message's channel, or its uploader.
        ServerEvent::AttachmentPreviewed {
            message: Some(_), ..
        } => ScopeKind::Channel,
        ServerEvent::AttachmentPreviewed { message: None, .. } => ScopeKind::User,
        ServerEvent::PluginEvent {
            channel: None,
            community: Some(_),
            ..
        } => ScopeKind::Community,
        ServerEvent::PluginEvent {
            channel: None,
            community: None,
            ..
        } => ScopeKind::User,
        ServerEvent::Community(_)
        | ServerEvent::Channel(_)
        | ServerEvent::Category(_)
        | ServerEvent::Invite(_)
        | ServerEvent::Role(_)
        | ServerEvent::CustomEmoji(_)
        | ServerEvent::CommunityBan(_)
        | ServerEvent::VoiceMute(_)
        | ServerEvent::ChannelOverride(_)
        | ServerEvent::CategoryOverride(_)
        | ServerEvent::CommunityPlugin(_)
        | ServerEvent::CommunityResync { .. } => ScopeKind::Community,
        ServerEvent::UserCommunity(_) => ScopeKind::Membership,
        ServerEvent::UserPreferencesChanged { .. }
        | ServerEvent::EmailAccountChanged { .. }
        | ServerEvent::ChannelRead { .. }
        | ServerEvent::ChannelMuteChanged { .. }
        | ServerEvent::NotificationSettingChanged { .. }
        | ServerEvent::ForeignDmJoined { .. }
        | ServerEvent::UserBlockChanged { .. }
        | ServerEvent::BotCommandInvoked { .. }
        | ServerEvent::CategoryCollapseChanged { .. }
        | ServerEvent::DeploymentAccessChanged { .. }
        | ServerEvent::AccountBanned { .. }
        | ServerEvent::SignInsEnded { .. }
        | ServerEvent::UserResync { .. }
        | ServerEvent::ReportsChanged { .. }
        | ServerEvent::PluginNotice { .. }
        | ServerEvent::HeldMessagePosted { .. }
        | ServerEvent::HeldMessageFailed { .. } => ScopeKind::User,
        ServerEvent::User(_)
        | ServerEvent::BotCommandsChanged { .. }
        | ServerEvent::UserAnnotation(_) => ScopeKind::UserEverywhere,
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

/// The communities a user belongs to that are not deleted, which is what their event stream
/// reads.
pub async fn memberships(
    conn: &mut AsyncPgConnection,
    user: UserId,
) -> crate::Result<Vec<CommunityId>> {
    Ok(community_user::table
        .select(community_user::community)
        .filter(community_user::user.eq(user))
        .filter(community_user::community.eq_any(crate::community::live()))
        .load(conn)
        .await?)
}

/// Where a channel belongs: a community, or a DM whose recipients are its only audience. A
/// thread belongs where its parent channel does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelHome {
    /// A community channel, and the channel whose permissions govern it: itself, or a
    /// thread's parent.
    Community {
        community: CommunityId,
        governing: ChannelId,
    },
    /// The DM or group DM, which is the channel itself or a thread's parent.
    Direct(ChannelId),
}

/// What publishing events needs: the event stream, and where each channel belongs, which is
/// kept as it is learned since a channel never moves. The server has it in its context; an
/// operator command, which has no server context, makes a `Publisher`.
pub trait Publishing: Sync {
    fn nats(&self) -> &async_nats::jetstream::Context;
    fn channel_homes(&self) -> &Mutex<HashMap<ChannelId, ChannelHome>>;
}

impl Publishing for GlobalServerContext {
    fn nats(&self) -> &async_nats::jetstream::Context {
        &self.nats_context
    }

    fn channel_homes(&self) -> &Mutex<HashMap<ChannelId, ChannelHome>> {
        &self.channel_homes
    }
}

/// Event publishing for an operator command run from the terminal, which changes the database
/// as the server does and announces it the same way.
pub struct Publisher {
    nats: async_nats::jetstream::Context,
    channel_homes: Mutex<HashMap<ChannelId, ChannelHome>>,
}

impl Publisher {
    /// Connects to the event stream `config` names.
    pub async fn connect(config: &crate::aspen_config::AspenConfig) -> crate::Result<Self> {
        let client =
            async_nats::connect_with_options(&config.nats_url, config.nats_options()).await?;
        Ok(Publisher {
            nats: async_nats::jetstream::new(client),
            channel_homes: Mutex::default(),
        })
    }
}

impl Publishing for Publisher {
    fn nats(&self) -> &async_nats::jetstream::Context {
        &self.nats
    }

    fn channel_homes(&self) -> &Mutex<HashMap<ChannelId, ChannelHome>> {
        &self.channel_homes
    }
}

/// Where a channel belongs. It never changes, so the answer is kept for the process's life.
pub async fn channel_home(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    channel_id: ChannelId,
) -> crate::Result<ChannelHome> {
    let cached = |id: ChannelId| {
        state
            .channel_homes()
            .lock()
            .expect("channel home cache")
            .get(&id)
            .copied()
    };
    let remember = |ids: &[ChannelId], home: ChannelHome| {
        let mut homes = state.channel_homes().lock().expect("channel home cache");
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
            (Some(community), parent) => ChannelHome::Community {
                community,
                governing: parent.unwrap_or(current),
            },
            (None, Some(parent)) => {
                current = parent;
                continue;
            }
            (None, None) => ChannelHome::Direct(current),
        };
        remember(&[channel_id, current], home);
        return Ok(home);
    }
    Err(crate::Error::EventRouting(format!(
        "channel {channel_id} is a thread of a thread"
    )))
}

/// The people in a DM or group DM.
pub async fn dm_recipients(
    conn: &mut AsyncPgConnection,
    dm: ChannelId,
) -> crate::Result<Vec<UserId>> {
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
) -> crate::Result<Vec<String>> {
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
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    channel_id: ChannelId,
) -> crate::Result<Vec<String>> {
    Ok(match channel_home(state, conn, channel_id).await? {
        ChannelHome::Community { community, .. } => vec![channel_subject(community, channel_id)],
        ChannelHome::Direct(dm) => recipient_subjects(conn, dm, None).await?,
    })
}

/// The scope with a message or call replaced by the channel it is in, which is what both its
/// subjects and its governing channel are decided by.
async fn to_channel(conn: &mut AsyncPgConnection, scope: EventScope) -> crate::Result<EventScope> {
    Ok(match scope {
        EventScope::Message(message_id) => EventScope::Channel(
            message::table
                .select(message::channel)
                .filter(message::id.eq(message_id))
                .first(conn)
                .await?,
        ),
        EventScope::Session(session_id) => EventScope::Channel(
            voice_session::table
                .select(voice_session::channel)
                .filter(voice_session::id.eq(session_id))
                .first(conn)
                .await?,
        ),
        scope => scope,
    })
}

/// The community channel whose View channel permission decides who receives an event with
/// this scope, if it is about one.
/// The category whose own overrides decide who receives `event` (`CATEGORY_HEADER`): the one a
/// category's own event, or one of its overrides', is about.
fn governing_category(event: &ServerEvent) -> Option<CategoryId> {
    use aspen_wire::message_enum::server_event::{CategoryEvent, CategoryOverrideEvent};
    match event {
        ServerEvent::Category(CategoryEvent::Create(category)) => Some(category.id),
        ServerEvent::Category(CategoryEvent::Update { id, .. } | CategoryEvent::Delete { id }) => {
            Some(*id)
        }
        ServerEvent::CategoryOverride(
            CategoryOverrideEvent::Create(aspen_wire::message_enum::CategoryOverride {
                category,
                ..
            })
            | CategoryOverrideEvent::Update { category, .. }
            | CategoryOverrideEvent::Delete { category, .. },
        ) => Some(*category),
        _ => None,
    }
}

async fn governing_channel(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    scope: &EventScope,
) -> crate::Result<Option<ChannelId>> {
    let (EventScope::Channel(channel_id)
    | EventScope::ChannelDefinition {
        channel: channel_id,
        ..
    }) = scope
    else {
        return Ok(None);
    };
    Ok(match channel_home(state, conn, *channel_id).await? {
        ChannelHome::Community { governing, .. } => Some(governing),
        ChannelHome::Direct(_) => None,
    })
}

/// Who besides the holders of a permission may receive an event: the permission it needs, and
/// the one member who receives it without. An invite is its code, so every event about one
/// reaches only those who may manage invites and whoever made it; a ban is a moderation
/// record, reaching those who may ban; a community's use of a plugin reaches those who may
/// manage plugins.
async fn audience(
    conn: &mut AsyncPgConnection,
    event: &ServerEvent,
) -> crate::Result<Option<(Permission, Option<UserId>)>> {
    use aspen_wire::message_enum::server_event::InviteEvent;
    if let ServerEvent::CommunityBan(_) = event {
        return Ok(Some((Permission::BanMembers, None)));
    }
    // A moderator's mute reaches those who may mute, and the person muted.
    if let ServerEvent::VoiceMute(
        VoiceMuteEvent::Create(aspen_wire::message_enum::VoiceMute { user, .. })
        | VoiceMuteEvent::Delete { user, .. },
    ) = event
    {
        return Ok(Some((Permission::ManageCalls, Some(*user))));
    }
    // A community's settings for a plugin are its managers' to read.
    if let ServerEvent::CommunityPlugin(_) = event {
        return Ok(Some((Permission::ManagePlugins, None)));
    }
    let ServerEvent::Invite(event) = event else {
        return Ok(None);
    };
    let creator = match event {
        InviteEvent::Create(invite) => Some(invite.created_by),
        InviteEvent::Update { code, .. } | InviteEvent::Delete { code } => invite::table
            .select(invite::created_by)
            .filter(invite::code.eq(code))
            .first(conn)
            .await
            .optional()?,
    };
    Ok(Some((Permission::ManageInvites, creator)))
}

/// The copy of a membership event the rest of the community receives: without the member's own
/// list position, which is theirs alone. `None` when nothing else is left to tell them, as for a
/// reorder alone.
fn for_community(event: &ServerEvent) -> Option<ServerEvent> {
    use aspen_wire::message_enum::server_event::UserCommunityEvent;
    let mut copy = event.clone();
    match &mut copy {
        ServerEvent::UserCommunity(UserCommunityEvent::Create(membership)) => {
            membership.sort_index = None;
        }
        ServerEvent::UserCommunity(UserCommunityEvent::Update {
            sort_index,
            roles,
            nickname,
            ..
        }) => {
            *sort_index = None;
            if roles.is_none() && nickname.is_none() {
                return None;
            }
        }
        _ => {}
    }
    Some(copy)
}

/// The subjects an event with this scope is published on.
async fn subjects(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    scope: EventScope,
) -> crate::Result<Vec<String>> {
    Ok(match scope {
        EventScope::Channel(channel_id) => channel_subjects(state, conn, channel_id).await?,
        EventScope::Message(_) | EventScope::Session(_) => {
            return Err(crate::Error::EventRouting(
                "a message or call scope reached subjects unresolved".to_string(),
            ));
        }
        EventScope::Community(community) => vec![community_subject(community)],
        EventScope::ChannelDefinition {
            channel: channel_id,
            departed,
        } => match channel_home(state, conn, channel_id).await? {
            ChannelHome::Community { community, .. } => vec![community_subject(community)],
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

/// Whose calls an event may change who may be in, or what they may do there: the event's
/// effect on access, which `publish_event` notes so the calls are rechecked once the work that
/// published it is done (`settle`). No wildcard arm: a new event must say whether it changes
/// access before it can be published.
pub fn rechecks_of(event: &ServerEvent, scope: &EventScope) -> Vec<Recheck> {
    use aspen_wire::message_enum::server_event::{
        CategoryOverrideEvent, ChannelEvent, ChannelOverrideEvent, CommunityEvent, RoleEvent,
        UserCommunityEvent, UserEvent,
    };
    let scoped_user = match scope {
        EventScope::User(user) | EventScope::UserEverywhere(user) => Some(*user),
        _ => None,
    };
    let scoped_community = match scope {
        EventScope::Community(community) => Some(*community),
        _ => None,
    };
    match event {
        // A role's permissions reach every holder; a new role has none yet.
        ServerEvent::Role(RoleEvent::Update { permissions, .. }) => match permissions {
            Some(_) => scoped_community
                .map(Recheck::Community)
                .into_iter()
                .collect(),
            None => Vec::new(),
        },
        ServerEvent::Role(RoleEvent::Delete { .. }) => scoped_community
            .map(Recheck::Community)
            .into_iter()
            .collect(),
        ServerEvent::Role(RoleEvent::Create(_)) => Vec::new(),
        ServerEvent::ChannelOverride(
            ChannelOverrideEvent::Create(aspen_wire::message_enum::ChannelOverride {
                channel, ..
            })
            | ChannelOverrideEvent::Update { channel, .. }
            | ChannelOverrideEvent::Delete { channel, .. },
        ) => vec![Recheck::Channel(*channel)],
        ServerEvent::CategoryOverride(
            CategoryOverrideEvent::Create(aspen_wire::message_enum::CategoryOverride {
                category,
                ..
            })
            | CategoryOverrideEvent::Update { category, .. }
            | CategoryOverrideEvent::Delete { category, .. },
        ) => vec![Recheck::Category(*category)],
        // A move changes the overrides that apply; a group DM's recipients, who is in it; a
        // deletion, everyone.
        ServerEvent::Channel(ChannelEvent::Update {
            id,
            parent_category,
            recipients,
            ..
        }) => {
            if parent_category.is_some() || recipients.is_some() {
                vec![Recheck::Channel(*id)]
            } else {
                Vec::new()
            }
        }
        ServerEvent::Channel(ChannelEvent::Delete { id }) => vec![Recheck::Channel(*id)],
        ServerEvent::Channel(ChannelEvent::Create(_)) => Vec::new(),
        ServerEvent::Community(CommunityEvent::Update { id, owner, .. }) => match owner {
            Some(_) => vec![Recheck::Community(*id)],
            None => Vec::new(),
        },
        ServerEvent::Community(CommunityEvent::Delete { id }) => vec![Recheck::Community(*id)],
        ServerEvent::Community(CommunityEvent::Create(_)) => Vec::new(),
        ServerEvent::UserCommunity(UserCommunityEvent::Update { user, roles, .. }) => match roles {
            Some(_) => vec![Recheck::User(*user)],
            None => Vec::new(),
        },
        ServerEvent::UserCommunity(UserCommunityEvent::Delete { user, .. }) => {
            vec![Recheck::User(*user)]
        }
        ServerEvent::UserCommunity(UserCommunityEvent::Create(_)) => Vec::new(),
        // A block ends what either may do in their DM.
        ServerEvent::UserBlockChanged { user, blocked } => match (blocked, scoped_user) {
            (true, Some(blocker)) => vec![Recheck::User(blocker), Recheck::User(*user)],
            _ => Vec::new(),
        },
        // Moderating the deployment reaches into calls; a ban or a deleted account ends them.
        ServerEvent::DeploymentAccessChanged { .. } | ServerEvent::AccountBanned { .. } => {
            scoped_user.map(Recheck::User).into_iter().collect()
        }
        ServerEvent::User(UserEvent::Delete { id }) => vec![Recheck::User(*id)],
        // A moderator's mute reaches the calls the person is in through their recheck.
        ServerEvent::VoiceMute(
            VoiceMuteEvent::Create(aspen_wire::message_enum::VoiceMute { user, .. })
            | VoiceMuteEvent::Delete { user, .. },
        ) => vec![Recheck::User(*user)],
        // A participant who joined on a token of an ended sign-in leaves the call with it.
        ServerEvent::SignInsEnded { ended, kept, .. } => scoped_user
            .map(|user| Recheck::SignIns {
                user,
                ended: ended.clone(),
                kept: kept.clone(),
            })
            .into_iter()
            .collect(),
        ServerEvent::User(UserEvent::Create(_) | UserEvent::Update { .. }) => Vec::new(),
        ServerEvent::Message(_)
        | ServerEvent::Poll(_)
        | ServerEvent::Pin(_)
        | ServerEvent::React(_)
        | ServerEvent::VoiceSession(_)
        | ServerEvent::VoiceParticipant(_)
        | ServerEvent::VoiceRing(_)
        | ServerEvent::VoiceSessionEnded { .. }
        | ServerEvent::VoiceSpeaking { .. }
        | ServerEvent::Category(_)
        | ServerEvent::Invite(_)
        | ServerEvent::CustomEmoji(_)
        | ServerEvent::CommunityBan(_)
        | ServerEvent::CommunityResync { .. }
        | ServerEvent::UserResync { .. }
        | ServerEvent::UserPreferencesChanged { .. }
        | ServerEvent::EmailAccountChanged { .. }
        | ServerEvent::ChannelRead { .. }
        | ServerEvent::ChannelMuteChanged { .. }
        | ServerEvent::NotificationSettingChanged { .. }
        | ServerEvent::ForeignDmJoined { .. }
        | ServerEvent::BotCommandInvoked { .. }
        | ServerEvent::CategoryCollapseChanged { .. }
        | ServerEvent::ReportsChanged { .. }
        | ServerEvent::BotCommandsChanged { .. }
        // What plugins say and publish never changes who may see or do anything, and a
        // community turning one on changes access only through the principal's membership,
        // which is announced as any is.
        | ServerEvent::MessageAnnotation(_)
        | ServerEvent::UserAnnotation(_)
        | ServerEvent::CommunityPlugin(_)
        | ServerEvent::PluginEvent { .. }
        | ServerEvent::PluginNotice { .. }
        | ServerEvent::AttachmentPreviewed { .. }
        | ServerEvent::HeldMessagePosted { .. }
        | ServerEvent::HeldMessageFailed { .. } => Vec::new(),
    }
}

/// What the work in `noting` published that matters once it is done.
#[derive(Debug, Default)]
pub struct Noted {
    /// What it published inside a transaction (not a savepoint within one), by the
    /// transaction's id (`pg_current_xact_id`): it stands or falls with that transaction, which
    /// the database says committed or not once the work is done.
    in_transactions: HashMap<String, Published>,
    /// What it published anywhere else (outside a transaction, inside a savepoint, or where the
    /// transaction's id could not be read): it stands or falls with the work.
    elsewhere: Published,
    /// The calls its events may have changed access to (`rechecks_of`).
    rechecks: HashSet<Recheck>,
}

/// Whom some events were published to.
#[derive(Debug, Default)]
struct Published {
    /// The communities they were about.
    communities: HashSet<CommunityId>,
    /// The users they were published to alone, on their own subjects.
    users: HashSet<UserId>,
}

impl Published {
    fn is_empty(&self) -> bool {
        self.communities.is_empty() && self.users.is_empty()
    }

    fn extend(&mut self, other: Published) {
        self.communities.extend(other.communities);
        self.users.extend(other.users);
    }
}

impl Noted {
    /// Whether nothing was noted, so there is nothing to settle.
    fn is_empty(&self) -> bool {
        self.in_transactions.is_empty() && self.elsewhere.is_empty() && self.rechecks.is_empty()
    }
}

tokio::task_local! {
    /// What the work in `noting` has published, shared with `settle_after` so that work dropped
    /// part way is still settled.
    static NOTED: Arc<Mutex<Noted>>;
}

/// Runs `work`, returning with its result what it published (`Noted`), for `settle` once it is
/// done. Every request runs in one (`settle_after`), and so must any other work that publishes
/// an event changing access; one published outside is logged as an error.
pub async fn noting<T>(work: impl std::future::Future<Output = T>) -> (T, Noted) {
    let noted = Arc::<Mutex<Noted>>::default();
    let result = NOTED.scope(noted.clone(), work).await;
    (result, take_noted(&noted))
}

fn take_noted(noted: &Mutex<Noted>) -> Noted {
    std::mem::take(
        &mut *noted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    )
}

/// Runs `work` inside `noting` and settles what it published (`settle`), as having failed when
/// `failed` says its result is a failure. Work dropped before it finishes (a panic, or a
/// shutdown; a request's goes on when its client goes away, `api::settle_after_request`) is
/// settled as failed in a task of its own: its open transactions were rolled back with it,
/// perhaps after it published.
pub async fn settle_after<T>(
    state: &GlobalServerContext,
    work: impl std::future::Future<Output = T>,
    failed: impl FnOnce(&T) -> bool,
) -> T {
    /// Settles what was noted as failed if dropped before it is disarmed.
    struct Unsettled {
        state: Option<GlobalServerContext>,
        noted: Arc<Mutex<Noted>>,
    }
    impl Drop for Unsettled {
        fn drop(&mut self) {
            if let Some(state) = self.state.take() {
                let noted = take_noted(&self.noted);
                if !noted.is_empty()
                    && let Ok(runtime) = tokio::runtime::Handle::try_current()
                {
                    runtime.spawn(async move { settle(&state, noted, true).await });
                }
            }
        }
    }
    let mut unsettled = Unsettled {
        state: Some(state.clone()),
        noted: Arc::default(),
    };
    let result = NOTED.scope(unsettled.noted.clone(), work).await;
    unsettled.state = None;
    let noted = take_noted(&unsettled.noted);
    if !noted.is_empty() {
        let failed = failed(&result);
        let state = state.clone();
        // In a task of its own, so that it finishes even if the work's caller is dropped
        // while it runs.
        if let Err(e) = tokio::spawn(async move { settle(&state, noted, failed).await }).await {
            tracing::error!("settling what a request published failed: {e}");
        }
    }
    result
}

/// Finishes what `noting` recorded, once the work is done and its transactions have committed
/// or rolled back: rechecks the calls its events may have changed access to
/// (`app::voice::recheck`, which changes nothing where nothing changed), and when the work
/// `failed`, announces that what it published may not have happened (`announce_resync`) for
/// what may indeed not have (`unborne`): a failure after the transaction that published
/// committed leaves what it published true.
pub async fn settle(state: &GlobalServerContext, noted: Noted, failed: bool) {
    let Noted {
        in_transactions,
        elsewhere,
        rechecks,
    } = noted;
    for which in rechecks {
        crate::voice::recheck(state, which);
    }
    if !failed || (in_transactions.is_empty() && elsewhere.is_empty()) {
        return;
    }
    match state.connection_pool.get().await {
        Ok(mut conn) => {
            let resync = unborne(conn.as_mut(), in_transactions, elsewhere).await;
            if !resync.is_empty() {
                announce_resync_in(state, conn.as_mut(), resync.communities, resync.users).await;
            }
        }
        Err(e) => tracing::error!("could not settle what failed work published: {e}"),
    }
}

/// `settle` for work with no server context, an operator command: on `conn`, waiting for each
/// recheck.
pub async fn settle_in(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    noted: Noted,
    failed: bool,
) {
    let Noted {
        in_transactions,
        elsewhere,
        rechecks,
    } = noted;
    for which in rechecks {
        if let Err(e) = crate::voice::recheck_in(state, conn, which.clone()).await {
            tracing::error!(?which, "could not recheck who may stay in calls: {e}");
        }
    }
    if failed {
        let resync = unborne(conn, in_transactions, elsewhere).await;
        if !resync.is_empty() {
            announce_resync_in(state, conn, resync.communities, resync.users).await;
        }
    }
}

/// Of what failed work published, what the database may not bear out: everything published
/// outside a transaction of its own, and what was published in each transaction that did not
/// commit. A transaction the database cannot answer for counts as not committed.
async fn unborne(
    conn: &mut AsyncPgConnection,
    in_transactions: HashMap<String, Published>,
    elsewhere: Published,
) -> Published {
    let mut resync = elsewhere;
    if in_transactions.is_empty() {
        return resync;
    }
    #[derive(QueryableByName)]
    struct Status {
        #[diesel(sql_type = diesel::sql_types::Text)]
        id: String,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
        status: Option<String>,
    }
    let ids: Vec<String> = in_transactions.keys().cloned().collect();
    let committed: HashSet<String> = match diesel::sql_query(
        "SELECT id, pg_xact_status(id::xid8) AS status FROM unnest($1::text[]) AS id",
    )
    .bind::<diesel::sql_types::Array<diesel::sql_types::Text>, _>(ids)
    .load::<Status>(conn)
    .await
    {
        Ok(rows) => rows
            .into_iter()
            .filter(|row| row.status.as_deref() == Some("committed"))
            .map(|row| row.id)
            .collect(),
        Err(e) => {
            tracing::error!("could not read whether failed work's transactions committed: {e}");
            HashSet::new()
        }
    };
    for (id, published) in in_transactions {
        if !committed.contains(&id) {
            resync.extend(published);
        }
    }
    resync
}

/// The id of the transaction `conn` is in (`pg_current_xact_id`), when it is in one and not in
/// a savepoint within it, whose rollback the id would not tell of.
async fn transaction_of(conn: &mut AsyncPgConnection) -> Option<String> {
    use diesel_async::TransactionManager as _;
    let depth =
        <AsyncPgConnection as diesel_async::AsyncConnection>::TransactionManager::transaction_manager_status_mut(conn)
            .transaction_depth()
            .ok()
            .flatten()?;
    if depth.get() != 1 {
        return None;
    }
    #[derive(QueryableByName)]
    struct Id {
        #[diesel(sql_type = diesel::sql_types::Text)]
        id: String,
    }
    match diesel::sql_query("SELECT pg_current_xact_id()::text AS id")
        .get_result::<Id>(conn)
        .await
    {
        Ok(row) => Some(row.id),
        Err(e) => {
            tracing::warn!("could not read the publishing transaction's id: {e}");
            None
        }
    }
}

/// Tells everyone reading `communities`, and each of `users`, that what was announced to them
/// may not have happened, for work that published events and then failed: events are
/// published before their transaction commits, so a transaction rolled back after publishing
/// leaves events in the stream that the database does not bear out. Each event feed drops what
/// it holds of the community and the connections reading it, or the user's connections, which
/// resume with it loaded afresh; clients read the community, or everything the user's own
/// subject told them of, again.
async fn announce_resync_in(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    communities: HashSet<CommunityId>,
    users: HashSet<UserId>,
) {
    for community in communities {
        if let Err(e) = publish_event(
            state,
            conn,
            EventScope::Community(community),
            &ServerEvent::CommunityResync { community },
        )
        .await
        {
            tracing::error!(%community, "could not announce a resync: {e}");
        }
    }
    for user in users {
        if let Err(e) = publish_event(
            state,
            conn,
            EventScope::User(user),
            &ServerEvent::UserResync { user },
        )
        .await
        {
            tracing::error!(user = %user.0, "could not announce a resync: {e}");
        }
    }
}

/// Publishes an event to everyone its scope names, and waits for the stream to hold every
/// copy. Called before the transaction that made the change commits, so the stream's order is
/// the database's order and a refused publish rolls the change back.
pub async fn publish_event(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    scope: EventScope,
    event: &ServerEvent,
) -> crate::Result<()> {
    let expected = expected_kind(event);
    if scope.kind() != expected {
        return Err(crate::Error::EventRouting(format!(
            "event routed with a {:?} scope but needs {:?}",
            scope.kind(),
            expected
        )));
    }
    let scope = to_channel(conn, scope).await?;
    let governing = governing_channel(state, conn, &scope).await?;
    let audience = audience(conn, event).await?;
    // A membership's community copy leaves out what is the member's alone.
    let community_copy = match &scope {
        EventScope::Membership { community, .. } => Some((
            community_subject(*community),
            for_community(event)
                .map(|copy| serde_json::to_string(&copy))
                .transpose()?,
        )),
        _ => None,
    };
    let rechecks = rechecks_of(event, &scope);
    let subjects = subjects(state, conn, scope).await?;
    let transaction = if NOTED.try_with(|_| ()).is_ok() {
        transaction_of(conn).await
    } else {
        None
    };
    // Noted before publishing, since a failure may come after some copies are out.
    let noted = NOTED.try_with(|noted| {
        let mut noted = noted
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let published = match transaction {
            Some(id) => noted.in_transactions.entry(id).or_default(),
            None => &mut noted.elsewhere,
        };
        for subject in &subjects {
            match subject_owner(subject) {
                Some(SubjectOwner::Community(community)) => {
                    published.communities.insert(community);
                }
                Some(SubjectOwner::User(user)) => {
                    published.users.insert(user);
                }
                None => {}
            }
        }
        noted.rechecks.extend(rechecks.iter().cloned());
    });
    if noted.is_err() && !rechecks.is_empty() {
        tracing::error!(
            ?rechecks,
            "an event changing access was published outside `crate::events::noting`, so no call \
             is rechecked for it"
        );
    }
    let payload: bytes::Bytes = serde_json::to_string(event)?.into_bytes().into();
    let event_id = Uuid::now_v7().to_string();
    let mut headers = async_nats::HeaderMap::new();
    headers.insert(EVENT_ID_HEADER, event_id.as_str());
    if let Some(channel) = governing {
        headers.insert(CHANNEL_HEADER, channel.0.to_string().as_str());
    }
    if let Some(category) = governing_category(event) {
        headers.insert(CATEGORY_HEADER, category.0.to_string().as_str());
    }
    if let Some((permission, creator)) = &audience {
        headers.insert(REQUIRES_HEADER, permission.to_string().as_str());
        if let Some(creator) = creator {
            headers.insert(CREATOR_HEADER, creator.0.to_string().as_str());
        }
    }
    let publishes = subjects.into_iter().filter_map(|subject| {
        let payload = match &community_copy {
            // A membership event with nothing left for the community is not sent to it.
            Some((community, copy)) if *community == subject => {
                bytes::Bytes::from(copy.clone()?.into_bytes())
            }
            _ => payload.clone(),
        };
        let headers = headers.clone();
        Some(async move {
            let started = std::time::Instant::now();
            state
                .nats()
                .publish_with_headers(subject, headers, payload)
                .await?
                .await?;
            metrics::histogram!(aspen_metrics::api::EVENT_PUBLISH_DURATION)
                .record(started.elapsed().as_secs_f64());
            metrics::counter!(aspen_metrics::api::EVENTS_PUBLISHED).increment(1);
            Ok::<(), crate::Error>(())
        })
    });
    try_join_all(publishes).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aspen_wire::message_enum::server_event::{
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
    fn events_that_change_access_name_the_calls_to_recheck() {
        use aspen_wire::message_enum::server_event::{ChannelOverrideEvent, RoleEvent};
        let community = CommunityId::new();
        let channel = ChannelId::new();
        let (blocker, blocked) = (UserId::new(), UserId::new());
        let role = crate::RoleId::new();
        let renamed = ServerEvent::Role(RoleEvent::Update {
            id: role,
            name: Some("Renamed".to_string()),
            position: None,
            permissions: None,
            hue: None,
            hoist: None,
        });
        assert!(rechecks_of(&renamed, &EventScope::Community(community)).is_empty());
        assert_eq!(
            rechecks_of(
                &ServerEvent::Role(RoleEvent::Delete { id: role }),
                &EventScope::Community(community)
            ),
            vec![Recheck::Community(community)]
        );
        let cleared = ServerEvent::ChannelOverride(ChannelOverrideEvent::Delete { channel, role });
        assert_eq!(
            rechecks_of(&cleared, &EventScope::Community(community)),
            vec![Recheck::Channel(channel)]
        );
        let block = ServerEvent::UserBlockChanged {
            user: blocked,
            blocked: true,
        };
        assert_eq!(
            rechecks_of(&block, &EventScope::User(blocker)),
            vec![Recheck::User(blocker), Recheck::User(blocked)]
        );
        let message = ServerEvent::VoiceSpeaking {
            channel,
            user: blocker,
            speaking: true,
        };
        assert!(rechecks_of(&message, &EventScope::Channel(channel)).is_empty());
        let signed_out = ServerEvent::SignInsEnded {
            ended: None,
            kept: Some("kept".to_string()),
            at: chrono::Utc::now(),
        };
        assert_eq!(
            rechecks_of(&signed_out, &EventScope::User(blocker)),
            vec![Recheck::SignIns {
                user: blocker,
                ended: None,
                kept: Some("kept".to_string()),
            }]
        );
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

    #[test]
    fn the_community_never_learns_a_members_list_position() {
        use aspen_wire::message_enum::UserCommunity;
        let (community, user) = (CommunityId::new(), UserId::new());
        let joined = ServerEvent::UserCommunity(UserCommunityEvent::Create(UserCommunity {
            community,
            user,
            sort_index: Some(3),
            roles: Vec::new(),
            nickname: None,
        }));
        let copy = serde_json::to_value(for_community(&joined).expect("a copy")).unwrap();
        assert_eq!(copy["sortIndex"], serde_json::Value::Null);
        // A reorder is the member's alone; the community is told nothing.
        let reordered = ServerEvent::UserCommunity(UserCommunityEvent::Update {
            community,
            user,
            sort_index: Some(Some(1)),
            roles: None,
            nickname: None,
        });
        assert!(for_community(&reordered).is_none());
        // A nickname is the community's to see.
        let renamed = ServerEvent::UserCommunity(UserCommunityEvent::Update {
            community,
            user,
            sort_index: Some(Some(1)),
            roles: None,
            nickname: Some(Some("Aster".to_string())),
        });
        let copy = serde_json::to_value(for_community(&renamed).expect("a copy")).unwrap();
        assert!(copy.get("sortIndex").is_none());
        assert_eq!(copy["nickname"], "Aster");
        let promoted = ServerEvent::UserCommunity(UserCommunityEvent::Update {
            community,
            user,
            sort_index: Some(Some(1)),
            roles: Some(Vec::new()),
            nickname: None,
        });
        let copy = serde_json::to_value(for_community(&promoted).expect("a copy")).unwrap();
        assert!(copy.get("sortIndex").is_none());
        assert_eq!(copy["roles"], serde_json::json!([]));
    }
}
