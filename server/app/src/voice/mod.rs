//! Voice calls, as the API server sees them.
//!
//! The media itself flows through voice servers, which are separate processes registered in
//! the `voice_server` table. This module hands out join offers (a short-lived token plus the
//! servers worth trying), keeps the registry, suspends servers that keep failing, and turns
//! the voice servers' reports, read from the report stream (`reports`), into rows and client
//! events. A session binds a channel to
//! one server while anyone is in the call; it is created by the first report of a participant
//! and ends when the last one leaves, so the channel can land anywhere the next time.

pub mod mutes;
mod reports;
mod ring;
mod servers;
mod sessions;

pub use reports::spawn_report_listener;
use ring::clear_spent_rings;
pub use ring::{decline_ring, read_channels_rings};
use servers::reap_silent_servers;
pub use servers::{
    create_server, create_server_in, delete_idle_server, delete_server, join_offer, list_servers,
    list_servers_in, report_failure, update_server, update_server_in,
};
use sessions::reap_idle_sessions;
pub use sessions::{
    RECHECKS_AT_ONCE, Recheck, kick_everywhere, kick_participant, mute_participant, recheck,
    recheck_all_step, recheck_in,
};

use crate::context::GlobalServerContext;
use crate::permissions::channel_access;
use crate::{ChannelId, UserId, VoiceServerId, VoiceSessionId};
use aspen_schema::{channel, voice_participant, voice_server, voice_session};
use aspen_wire::message_enum;
use chrono::{DateTime, Utc};
use diesel::{
    AsChangeset, ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable, SelectableHelper,
};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use tracing::{error, warn};

/// Answers voice servers asking for the public half of the join token key
/// (`voice_protocol::control::TOKEN_KEY_SUBJECT`), for as long as the server runs. Every API
/// server answers, in one queue group, so any one of them up is enough. A voice server's NATS
/// user may receive replies only on its own inbox, and nothing but the API servers may publish
/// there, so the key it is given is the API servers'. NATS does not check the reply subject a
/// requester names against its permissions, so an answer goes only to a voice server's inbox
/// (`is_voice_inbox`), the asking server's when the request names it: anywhere else this
/// server, with its wider permissions, would be publishing where a voice server chose.
///
/// A request naming the voice server's id is also told whether that id is registered
/// (`servers::is_registered`), so a voice server given the wrong id says so at startup instead
/// of reporting to no one. At most `REGISTRATION_LOOKUPS` of those look-ups run at once, so a
/// voice server asking over and over holds that many database connections at most; a request
/// arriving while they are all running is answered with the key alone, which the voice server
/// takes as no answer to the question (the dashboard still lists an id it reports under that no
/// server has; see Administration).
pub async fn spawn_token_key_answerer(state: GlobalServerContext) -> crate::Result<()> {
    use futures_util::StreamExt;
    use voice_protocol::control::{TokenKeyRequest, is_voice_inbox};
    let client = state.nats_context.client();
    let mut requests = client
        .queue_subscribe(
            voice_protocol::control::TOKEN_KEY_SUBJECT,
            "aspen_api".to_string(),
        )
        .await?;
    let key = state.join_token_key.public();
    let lookups = std::sync::Arc::new(tokio::sync::Semaphore::new(REGISTRATION_LOOKUPS));
    tokio::spawn(async move {
        while let Some(request) = requests.next().await {
            let Some(reply) = request.reply else {
                continue;
            };
            let asking = serde_json::from_slice::<TokenKeyRequest>(&request.payload).ok();
            let asking_server = asking.as_ref().map(|asking| asking.server);
            if !is_voice_inbox(&reply, asking_server) {
                warn!(
                    reply = %reply,
                    server = ?asking_server,
                    "refused to answer a request for the join token key whose reply subject is not the voice server's inbox; a voice server may be compromised"
                );
                continue;
            }
            let (state, client, mut answer) = (state.clone(), client.clone(), key.clone());
            let lookup = asking_server.zip(lookups.clone().try_acquire_owned().ok());
            // Each answer is its own task, so one waiting for a database connection holds up
            // no other voice server.
            tokio::spawn(async move {
                if let Some((server, _lookup)) = lookup {
                    answer.registered = match servers::is_registered(
                        &state,
                        VoiceServerId::from(server),
                    )
                    .await
                    {
                        Ok(registered) => Some(registered),
                        Err(e) => {
                            warn!(error = %e, "could not tell a voice server whether it is registered");
                            None
                        }
                    };
                }
                let answer = match serde_json::to_vec(&answer) {
                    Ok(answer) => answer,
                    Err(e) => {
                        error!(error = %e, "could not encode the join token key");
                        return;
                    }
                };
                if let Err(e) = client.publish(reply, answer.into()).await {
                    error!(error = %e, "could not answer a voice server asking for the join token key");
                }
            });
        }
        error!("stopped answering voice servers asking for the join token key");
    });
    Ok(())
}

/// The most look-ups of whether a voice server is registered that one API server runs at once
/// (`spawn_token_key_answerer`). Each voice server asks once at startup, so a few cover a whole
/// fleet starting together.
const REGISTRATION_LOOKUPS: usize = 2;

/// How often the reaper runs (`reap`).
pub const REAPER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

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
    /// Until when it is left out of join offers because people kept failing to reach it
    /// (`report_failure`); it then takes calls again on its own.
    pub suspended_until: Option<DateTime<Utc>>,
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
) -> crate::Result<(
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

/// Every call in progress on the voice channels of `visible`'s communities that its user may
/// view, with their participants. Two queries however many communities there are.
pub async fn read_communities_voice(
    state: &GlobalServerContext,
    visible: &crate::visibility::Visibility,
) -> crate::Result<(
    Vec<message_enum::VoiceSession>,
    Vec<message_enum::VoiceParticipant>,
)> {
    let communities = visible.communities();
    if communities.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut conn = state.connection_pool.get().await?;
    let sessions: Vec<VoiceSession> = voice_session::table
        .inner_join(channel::table)
        .select(VoiceSession::as_select())
        .filter(channel::community.eq_any(communities.iter().map(|c| Some(*c))))
        .load::<VoiceSession>(conn.as_mut())
        .await?
        .into_iter()
        .filter(|s| visible.can_view(s.channel))
        .collect();
    records_of_sessions(conn.as_mut(), sessions).await
}

/// The calls under way in each of `channels` (DMs, say), with who is in each.
pub async fn read_channels_voice(
    state: &GlobalServerContext,
    channels: &[ChannelId],
) -> crate::Result<(
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
) -> crate::Result<(
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

/// How many calls one step of the reaper ends.
const REAP_BATCH: i64 = 50;

/// One step of the reaper (`jobs::JobKind::ReapVoice`, every [`REAPER_INTERVAL`]): ends the calls
/// of voice servers silent for `session_silence_seconds` and calls alone for
/// `idle_session_seconds`, [`REAP_BATCH`] of each at a time, each call in a transaction of its
/// own, and clears the rings that have run out. Carries on while batches come back full.
pub async fn reap(
    state: &GlobalServerContext,
    _job: &crate::jobs::Claimed,
) -> crate::Result<crate::jobs::Outcome> {
    let silent = reap_silent_servers(state, REAP_BATCH).await?;
    let idle = reap_idle_sessions(state, REAP_BATCH).await?;
    clear_spent_rings(state).await?;
    Ok(
        if silent as i64 >= REAP_BATCH || idle as i64 >= REAP_BATCH {
            crate::jobs::Outcome::Progress(serde_json::Value::Null)
        } else {
            crate::jobs::Outcome::Done
        },
    )
}

/// `first()` yields `NotFound` for an empty result; reads that expect that turn it into `None`.
trait OptionalNotFound<T> {
    fn optional_not_found(self) -> crate::Result<Option<T>>;
}

impl<T> OptionalNotFound<T> for Result<T, diesel::result::Error> {
    fn optional_not_found(self) -> crate::Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(diesel::result::Error::NotFound) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}
