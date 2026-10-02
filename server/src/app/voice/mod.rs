//! Voice calls, as the API server sees them.
//!
//! The media itself flows through voice servers, which are separate processes registered in
//! the `voice_server` table. This module hands out join offers (a short-lived token plus the
//! servers worth trying), keeps the registry, disables servers that keep failing, and turns
//! the voice servers' NATS reports into rows and client events. A session binds a channel to
//! one server while anyone is in the call; it is created by the first report of a participant
//! and ends when the last one leaves, so the channel can land anywhere the next time.

mod ring;
mod servers;
mod sessions;

pub use ring::{decline_ring, read_channels_rings};
pub use servers::{
    create_server, delete_server, join_offer, list_servers, report_failure, seed_servers,
    update_server,
};
pub use sessions::{kick_participant, mute_participant, spawn_report_listener};

use ring::clear_spent_rings;
use servers::reap_silent_servers;
use sessions::reap_idle_sessions;

use crate::api::message_enum;
use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::permissions::channel_access;
use crate::app::{ChannelId, CommunityId, UserId, VoiceServerId, VoiceSessionId};
use crate::database::schema::{channel, voice_participant, voice_server, voice_session};
use chrono::{DateTime, Utc};
use diesel::{
    AsChangeset, ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable, SelectableHelper,
};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use tracing::error;

/// How often sessions whose server went silent are ended.
const REAPER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = voice_server)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct VoiceServer {
    pub id: VoiceServerId,
    pub name: String,
    pub url: String,
    pub capacity: i32,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub last_report_at: Option<DateTime<Utc>>,
    pub reported_participants: i32,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = voice_server)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct VoiceServerChangeset {
    pub name: Option<String>,
    pub url: Option<String>,
    pub capacity: Option<i32>,
    pub enabled: Option<bool>,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = voice_session)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct VoiceSession {
    id: VoiceSessionId,
    channel: ChannelId,
    voice_server: VoiceServerId,
    created_at: DateTime<Utc>,
    /// When the call last had one participant or fewer; `None` while two or more are in it.
    alone_since: Option<DateTime<Utc>>,
    /// Who started the call: its first participant, `None` until they are in it.
    started_by: Option<UserId>,
    /// Whether the call has ever held two people at once.
    had_company: bool,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = voice_participant)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct VoiceParticipant {
    session: VoiceSessionId,
    user: UserId,
    joined_at: DateTime<Utc>,
    muted: bool,
    deafened: bool,
    sharing_screen: bool,
}

impl From<&VoiceSession> for message_enum::VoiceSession {
    fn from(row: &VoiceSession) -> Self {
        message_enum::VoiceSession {
            id: row.id,
            channel: row.channel,
            voice_server: row.voice_server,
            created_at: row.created_at,
        }
    }
}

fn participant_record(
    row: &VoiceParticipant,
    channel: ChannelId,
) -> message_enum::VoiceParticipant {
    message_enum::VoiceParticipant {
        session: row.session,
        user: row.user,
        channel,
        joined_at: row.joined_at,
        muted: row.muted,
        deafened: row.deafened,
        sharing_screen: row.sharing_screen,
    }
}

// ---------------------------------------------------------------------------------------------
// Reads

/// The calls on `channel`, if any, with who is in them.
pub async fn read_channel_voice(
    state: &GlobalServerContext,
    caller: UserId,
    channel_id: ChannelId,
) -> app::Result<(
    Option<message_enum::VoiceSession>,
    Vec<message_enum::VoiceParticipant>,
)> {
    let mut conn = state.connection_pool.get().await?;
    channel_access(state, conn.as_mut(), caller, channel_id).await?;
    let session: Option<VoiceSession> = voice_session::table
        .select(VoiceSession::as_select())
        .filter(voice_session::channel.eq(channel_id))
        .first(conn.as_mut())
        .await
        .optional_not_found()?;
    let Some(session) = session else {
        return Ok((None, Vec::new()));
    };
    let participants: Vec<VoiceParticipant> = voice_participant::table
        .select(VoiceParticipant::as_select())
        .filter(voice_participant::session.eq(session.id))
        .order(voice_participant::joined_at)
        .load(conn.as_mut())
        .await?;
    Ok((
        Some(message_enum::VoiceSession::from(&session)),
        participants
            .iter()
            .map(|row| participant_record(row, channel_id))
            .collect(),
    ))
}

/// Every call in progress on the voice channels of `communities`, with their participants.
/// Two queries however many communities there are.
pub async fn read_communities_voice(
    state: &GlobalServerContext,
    communities: &[CommunityId],
) -> app::Result<(
    Vec<message_enum::VoiceSession>,
    Vec<message_enum::VoiceParticipant>,
)> {
    if communities.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut conn = state.connection_pool.get().await?;
    let sessions: Vec<VoiceSession> = voice_session::table
        .inner_join(channel::table)
        .select(VoiceSession::as_select())
        .filter(channel::community.eq_any(communities.iter().map(|c| Some(*c))))
        .load(conn.as_mut())
        .await?;
    records_of_sessions(conn.as_mut(), sessions).await
}

/// The calls under way in each of `channels` (DMs, say), with who is in each.
pub async fn read_channels_voice(
    state: &GlobalServerContext,
    channels: &[ChannelId],
) -> app::Result<(
    Vec<message_enum::VoiceSession>,
    Vec<message_enum::VoiceParticipant>,
)> {
    if channels.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut conn = state.connection_pool.get().await?;
    let sessions: Vec<VoiceSession> = voice_session::table
        .select(VoiceSession::as_select())
        .filter(voice_session::channel.eq_any(channels))
        .load(conn.as_mut())
        .await?;
    records_of_sessions(conn.as_mut(), sessions).await
}

/// Sessions as records, with their participants in the order they joined, read in one query.
async fn records_of_sessions(
    conn: &mut AsyncPgConnection,
    sessions: Vec<VoiceSession>,
) -> app::Result<(
    Vec<message_enum::VoiceSession>,
    Vec<message_enum::VoiceParticipant>,
)> {
    let ids: Vec<VoiceSessionId> = sessions.iter().map(|s| s.id).collect();
    let participants: Vec<VoiceParticipant> = voice_participant::table
        .select(VoiceParticipant::as_select())
        .filter(voice_participant::session.eq_any(&ids))
        .order(voice_participant::joined_at)
        .load(conn)
        .await?;
    let channel_of =
        |session: VoiceSessionId| sessions.iter().find(|s| s.id == session).map(|s| s.channel);
    let participants = participants
        .iter()
        .filter_map(|row| channel_of(row.session).map(|channel| participant_record(row, channel)))
        .collect();
    Ok((
        sessions
            .iter()
            .map(message_enum::VoiceSession::from)
            .collect(),
        participants,
    ))
}

/// Starts the task that ends the sessions of voice servers that stopped reporting and the
/// calls that have sat with one person for too long, and clears rings that have run out.
pub fn spawn_reaper(state: GlobalServerContext) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(REAPER_INTERVAL);
        loop {
            interval.tick().await;
            if let Err(e) = reap_silent_servers(&state).await {
                error!(
                    error = e.to_string(),
                    "ending sessions of silent voice servers failed"
                );
            }
            if let Err(e) = reap_idle_sessions(&state).await {
                error!(error = e.to_string(), "ending idle voice sessions failed");
            }
            if let Err(e) = clear_spent_rings(&state).await {
                error!(error = e.to_string(), "clearing spent call rings failed");
            }
        }
    });
}

/// `first()` yields `NotFound` for an empty result; reads that expect that turn it into `None`.
trait OptionalNotFound<T> {
    fn optional_not_found(self) -> app::Result<Option<T>>;
}

impl<T> OptionalNotFound<T> for Result<T, diesel::result::Error> {
    fn optional_not_found(self) -> app::Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(diesel::result::Error::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
