//! Voice calls, as the API server sees them.
//!
//! The media itself flows through voice servers, which are separate processes registered in
//! the `voice_server` table. This module hands out join offers (a short-lived token plus the
//! servers worth trying), keeps the registry, disables servers that keep failing, and turns
//! the voice servers' NATS reports into rows and client events. A session binds a channel to
//! one server while anyone is in the call; it is created by the first report of a participant
//! and ends when the last one leaves, so the channel can land anywhere the next time.

use crate::api::message_enum;
use crate::api::message_enum::server_event::{
    ServerEvent, VoiceParticipantEvent, VoiceSessionEvent,
};
use crate::api::voice::VoiceSessionEndReason;
use crate::api::{ChannelType, GlobalServerContext};
use crate::app;
use crate::app::permissions::{Permissions, channel_access, missing};
use crate::app::{
    ChannelId, CommunityId, EventScope, UserId, VoiceServerId, VoiceSessionId, publish_event,
};
use crate::database::schema::{
    channel, voice_participant, voice_server, voice_server_failure, voice_session,
};
use crate::t;
use chrono::{DateTime, Duration, Utc};
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable,
    Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use futures_util::StreamExt;
use rand::seq::SliceRandom;
use tracing::{error, info, warn};
use uuid::Uuid;
use voice_protocol::control::{
    REPORT_QUEUE_GROUP, REPORT_SUBJECT, VoiceCommand, VoiceReport, command_subject,
};
use voice_protocol::token::{JoinClaims, sign};

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

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = voice_server_failure)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct VoiceServerFailure {
    voice_server: VoiceServerId,
    user: UserId,
    reported_at: DateTime<Utc>,
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
// Registry

/// Installs the servers listed in `aspen.toml`, matched by name: a listed server is created or
/// has its address and capacity updated, and nothing is removed, so an operator can also add
/// servers through the API.
pub async fn seed_servers(state: &GlobalServerContext) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    for seed in &state.config.voice.servers {
        let row = VoiceServer {
            id: VoiceServerId::new(),
            name: seed.name.clone(),
            url: seed.url.clone(),
            capacity: i32::try_from(seed.capacity).unwrap_or(i32::MAX),
            enabled: true,
            created_at: Utc::now(),
            last_report_at: None,
            reported_participants: 0,
        };
        diesel::insert_into(voice_server::table)
            .values(&row)
            .on_conflict(voice_server::name)
            .do_update()
            .set((
                voice_server::url.eq(&row.url),
                voice_server::capacity.eq(row.capacity),
            ))
            .execute(conn.as_mut())
            .await?;
        info!(name = seed.name, url = seed.url, "voice server seeded");
    }
    Ok(())
}

pub async fn list_servers(state: &GlobalServerContext) -> app::Result<Vec<VoiceServer>> {
    let mut conn = state.connection_pool.get().await?;
    Ok(voice_server::table
        .select(VoiceServer::as_select())
        .order(voice_server::name)
        .load(conn.as_mut())
        .await?)
}

pub async fn create_server(
    state: &GlobalServerContext,
    name: String,
    url: String,
    capacity: i32,
) -> app::Result<VoiceServer> {
    let row = VoiceServer {
        id: VoiceServerId::new(),
        name,
        url,
        capacity,
        enabled: true,
        created_at: Utc::now(),
        last_report_at: None,
        reported_participants: 0,
    };
    let mut conn = state.connection_pool.get().await?;
    diesel::insert_into(voice_server::table)
        .values(&row)
        .execute(conn.as_mut())
        .await?;
    Ok(row)
}

pub async fn update_server(
    state: &GlobalServerContext,
    id: VoiceServerId,
    changes: VoiceServerChangeset,
) -> app::Result<VoiceServer> {
    let mut conn = state.connection_pool.get().await?;
    Ok(diesel::update(voice_server::table)
        .filter(voice_server::id.eq(id))
        .set(changes)
        .returning(VoiceServer::as_select())
        .get_result(conn.as_mut())
        .await?)
}

/// Removes a server. Its sessions go with it, so their participants are told to leave.
pub async fn delete_server(state: &GlobalServerContext, id: VoiceServerId) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let sessions: Vec<VoiceSession> = voice_session::table
                .select(VoiceSession::as_select())
                .filter(voice_session::voice_server.eq(id))
                .load(conn.as_mut())
                .await?;
            for session in sessions {
                end_session(
                    state,
                    conn.as_mut(),
                    &session,
                    VoiceSessionEndReason::ServerRemoved,
                )
                .await?;
            }
            let deleted = diesel::delete(voice_server::table)
                .filter(voice_server::id.eq(id))
                .execute(conn.as_mut())
                .await?;
            if deleted == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

// ---------------------------------------------------------------------------------------------
// Joining

/// What a client needs to join a channel's call.
pub struct JoinOffer {
    /// The call already in progress, if any; then `candidates` is its server alone.
    pub session: Option<message_enum::VoiceSession>,
    /// The servers to try, in no particular order; the client sorts them by its own latency.
    pub candidates: Vec<VoiceServer>,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    /// What the token lets them send, which the voice server enforces.
    pub speak: bool,
    pub share_screen: bool,
    pub transfer_files: bool,
}

/// Whether a server is offered to a joiner: enabled, with room, and heard from within the
/// silence limit. A server that has never reported is one that has not started, so it is not
/// offered; a voice server reports its load the moment it starts.
fn accepts_sessions(server: &VoiceServer, now: DateTime<Utc>, offer_silence: Duration) -> bool {
    server.enabled
        && server.reported_participants < server.capacity
        && reporting(server, now, offer_silence)
}

fn reporting(server: &VoiceServer, now: DateTime<Utc>, offer_silence: Duration) -> bool {
    server
        .last_report_at
        .is_some_and(|at| now - at <= offer_silence)
}

/// At most `limit` of `servers`, chosen at random. The choice is where a preference for
/// servers near the user would go once the deployment has one.
fn pick_candidates(mut servers: Vec<VoiceServer>, limit: usize) -> Vec<VoiceServer> {
    servers.shuffle(&mut rand::rng());
    servers.truncate(limit);
    servers
}

pub async fn join_offer(
    state: &GlobalServerContext,
    user: UserId,
    channel_id: ChannelId,
) -> app::Result<JoinOffer> {
    let mut conn = state.connection_pool.get().await?;
    let ty: ChannelType = channel::table
        .select(channel::ty)
        .filter(
            channel::id
                .eq(channel_id)
                .and(channel::deleted_at.is_null()),
        )
        .first(conn.as_mut())
        .await?;
    // A DM's or group DM's call is its recipients', and, since no one holds a community
    // permission there, no one moderates it.
    if !matches!(ty, ChannelType::Voice | ChannelType::Dm | ChannelType::GroupDm) {
        return Err(app::Error::Validation(t!("voiceChannelOnly")));
    }
    let access = channel_access(state, conn.as_mut(), user, channel_id).await?;
    access.require(Permissions::JOIN_VOICE)?;
    let voice = &state.config.voice;
    let now = Utc::now();
    let existing: Option<VoiceSession> = voice_session::table
        .select(VoiceSession::as_select())
        .filter(voice_session::channel.eq(channel_id))
        .first(conn.as_mut())
        .await
        .optional_not_found()?;
    let silence = Duration::seconds(i64::try_from(voice.offer_silence_seconds).unwrap_or(60));
    // A call on a server that has gone silent is not one the joiner can reach; they are
    // offered fresh servers instead, and the session the new server starts replaces it.
    let reachable = match existing {
        Some(session) => {
            let server: VoiceServer = voice_server::table
                .select(VoiceServer::as_select())
                .filter(voice_server::id.eq(session.voice_server))
                .first(conn.as_mut())
                .await?;
            if reporting(&server, now, silence) {
                Some((session, server))
            } else {
                None
            }
        }
        None => None,
    };
    let (session, candidates) = match reachable {
        Some((session, server)) => (
            Some(message_enum::VoiceSession::from(&session)),
            vec![server],
        ),
        None => {
            let servers: Vec<VoiceServer> = voice_server::table
                .select(VoiceServer::as_select())
                .filter(voice_server::enabled.eq(true))
                .load(conn.as_mut())
                .await?;
            let open: Vec<VoiceServer> = servers
                .into_iter()
                .filter(|server| accepts_sessions(server, now, silence))
                .collect();
            (None, pick_candidates(open, voice.candidate_limit))
        }
    };
    if candidates.is_empty() {
        return Err(app::Error::Validation(t!("voiceNoServers")));
    }
    let expires_at =
        now + Duration::seconds(i64::try_from(voice.join_token_ttl_seconds).unwrap_or(60));
    let (speak, share_screen, transfer_files) = (
        access.has(Permissions::SPEAK),
        access.has(Permissions::SHARE_SCREEN),
        voice.file_transfers && access.has(Permissions::TRANSFER_FILES),
    );
    let claims = JoinClaims {
        user: user.0,
        channel: channel_id.0,
        servers: candidates.iter().map(|server| server.id.0).collect(),
        expires_at: expires_at.timestamp(),
        nonce: Uuid::now_v7(),
        speak,
        share_screen,
        transfer_files,
    };
    Ok(JoinOffer {
        session,
        candidates,
        token: sign(&claims, voice.token_secret.as_bytes()),
        expires_at,
        speak,
        share_screen,
        transfer_files,
    })
}

// ---------------------------------------------------------------------------------------------
// Failure reports

/// The outcome of a failure report.
pub struct FailureOutcome {
    /// Distinct users who reported this server within the window, this one included.
    pub failures: u32,
    /// Whether the server is now disabled.
    pub disabled: bool,
}

/// Whether `failures` distinct users within the window is enough to take a server out.
fn should_disable(failures: u32, threshold: u32) -> bool {
    threshold > 0 && failures >= threshold
}

/// Records that `user` could not start a session on the server. The report counts once per
/// user within the window, at their latest attempt; once the configured number of distinct
/// users have reported, the server is disabled until an operator enables it again.
pub async fn report_failure(
    state: &GlobalServerContext,
    user: UserId,
    server: VoiceServerId,
) -> app::Result<FailureOutcome> {
    let voice = &state.config.voice;
    let now = Utc::now();
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            // The row references the server; an unknown server is a foreign key violation,
            // which the caller sees as "no such server".
            diesel::insert_into(voice_server_failure::table)
                .values(&VoiceServerFailure {
                    voice_server: server,
                    user,
                    reported_at: now,
                })
                .on_conflict((
                    voice_server_failure::voice_server,
                    voice_server_failure::user,
                ))
                .do_update()
                .set(voice_server_failure::reported_at.eq(now))
                .execute(conn.as_mut())
                .await
                .map_err(|e| match e {
                    diesel::result::Error::DatabaseError(
                        diesel::result::DatabaseErrorKind::ForeignKeyViolation,
                        _,
                    ) => diesel::result::Error::NotFound,
                    other => other,
                })?;
            let window_start = now
                - Duration::seconds(i64::try_from(voice.failure_window_seconds).unwrap_or(3600));
            let failures: i64 = voice_server_failure::table
                .filter(
                    voice_server_failure::voice_server
                        .eq(server)
                        .and(voice_server_failure::reported_at.gt(window_start)),
                )
                .count()
                .get_result(conn.as_mut())
                .await?;
            let failures = u32::try_from(failures).unwrap_or(u32::MAX);
            let mut disabled = false;
            if should_disable(failures, voice.failure_threshold) {
                let changed = diesel::update(voice_server::table)
                    .filter(
                        voice_server::id
                            .eq(server)
                            .and(voice_server::enabled.eq(true)),
                    )
                    .set(voice_server::enabled.eq(false))
                    .execute(conn.as_mut())
                    .await?;
                if changed > 0 {
                    warn!(
                        server = server.0.to_string(),
                        failures, "voice server disabled after failures from distinct users"
                    );
                }
                disabled = true;
            }
            Ok(FailureOutcome { failures, disabled })
        }
        .scope_boxed()
    })
    .await
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

// ---------------------------------------------------------------------------------------------
// Moderation

/// Server-mutes or unmutes someone in a channel's call. The voice server holding the call
/// applies it and reports the new state, which becomes the participant's `update` event; the
/// record returned is the state as recorded before the command lands. Takes Manage calls.
pub async fn mute_participant(
    state: &GlobalServerContext,
    caller: UserId,
    channel: ChannelId,
    user: UserId,
    muted: bool,
) -> app::Result<message_enum::VoiceParticipant> {
    command_participant(state, caller, channel, user, |session| VoiceCommand::Mute {
        session: session.0,
        user: user.0,
        muted,
    })
    .await
}

/// Removes someone from a channel's call. The voice server disconnects them, telling them
/// why, and reports their leaving, which deletes their participant row. Takes Manage calls.
pub async fn kick_participant(
    state: &GlobalServerContext,
    caller: UserId,
    channel: ChannelId,
    user: UserId,
) -> app::Result<()> {
    command_participant(state, caller, channel, user, |session| VoiceCommand::Kick {
        session: session.0,
        user: user.0,
    })
    .await?;
    Ok(())
}

/// Sends the voice server holding `channel`'s call a command about one of its participants.
/// No call, or no such participant in it, is not found.
async fn command_participant(
    state: &GlobalServerContext,
    caller: UserId,
    channel: ChannelId,
    user: UserId,
    command: impl FnOnce(VoiceSessionId) -> VoiceCommand,
) -> app::Result<message_enum::VoiceParticipant> {
    let mut conn = state.connection_pool.get().await?;
    let access = channel_access(state, conn.as_mut(), caller, channel).await?;
    // A DM's call has no moderators.
    if !access.community_has(Permissions::MANAGE_CALLS) {
        return Err(missing(Permissions::MANAGE_CALLS));
    }
    let session = session_on_channel(conn.as_mut(), channel)
        .await?
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))?;
    let participant: VoiceParticipant = voice_participant::table
        .select(VoiceParticipant::as_select())
        .filter(
            voice_participant::session
                .eq(session.id)
                .and(voice_participant::user.eq(user)),
        )
        .first(conn.as_mut())
        .await?;
    let payload = serde_json::to_vec(&command(session.id))?;
    state
        .nats_context
        .client()
        .publish(command_subject(session.voice_server.0), payload.into())
        .await
        .map_err(app::Error::VoiceCommand)?;
    Ok(participant_record(&participant, channel))
}

// ---------------------------------------------------------------------------------------------
// Reports from voice servers

/// Subscribes to the voice servers' reports and applies each one. The subscription is in a
/// queue group, so with several API servers running each report is handled by one of them.
pub async fn spawn_report_listener(state: GlobalServerContext) -> app::Result<()> {
    let client = state.nats_context.client();
    let mut reports = client
        .queue_subscribe(REPORT_SUBJECT, REPORT_QUEUE_GROUP.to_string())
        .await?;
    tokio::spawn(async move {
        while let Some(message) = reports.next().await {
            let report: VoiceReport = match serde_json::from_slice(&message.payload) {
                Ok(report) => report,
                Err(e) => {
                    warn!(error = e.to_string(), "unreadable voice report ignored");
                    continue;
                }
            };
            if let Err(e) = apply_report(&state, report).await {
                error!(error = e.to_string(), "applying a voice report failed");
            }
        }
        error!("the voice report subscription ended");
    });
    Ok(())
}

async fn apply_report(state: &GlobalServerContext, report: VoiceReport) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            match report {
                VoiceReport::Load {
                    server,
                    participants,
                } => {
                    diesel::update(voice_server::table)
                        .filter(voice_server::id.eq(VoiceServerId::from(server)))
                        .set((
                            voice_server::last_report_at.eq(Utc::now()),
                            voice_server::reported_participants
                                .eq(i32::try_from(participants).unwrap_or(i32::MAX)),
                        ))
                        .execute(conn.as_mut())
                        .await?;
                }
                VoiceReport::SessionStarted {
                    server,
                    session,
                    channel,
                } => {
                    let now = Utc::now();
                    let row = VoiceSession {
                        id: VoiceSessionId::from(session),
                        channel: ChannelId::from(channel),
                        voice_server: VoiceServerId::from(server),
                        created_at: now,
                        alone_since: Some(now),
                    };
                    // A voice server reports a session only once it holds the room, and it
                    // holds the room because a client with a valid join token arrived, so the
                    // call it announces is the one the channel now has. A session already
                    // recorded for the channel is therefore a room that no longer exists: the
                    // same server's, lost to a restart, or another server's that the joiner
                    // could not reach. It is ended as lost, which sends its participants to
                    // rejoin, and their offers now name this server.
                    if let Some(stale) = session_on_channel(conn.as_mut(), row.channel).await? {
                        if stale.id == row.id {
                            return Ok(());
                        }
                        end_session(
                            state,
                            conn.as_mut(),
                            &stale,
                            VoiceSessionEndReason::ServerLost,
                        )
                        .await?;
                    }
                    match diesel::insert_into(voice_session::table)
                        .values(&row)
                        .execute(conn.as_mut())
                        .await
                    {
                        Ok(_) => {
                            publish_event(
                                state,
                                conn.as_mut(),
                                EventScope::Channel(row.channel),
                                &ServerEvent::VoiceSession(VoiceSessionEvent::Create(
                                    message_enum::VoiceSession::from(&row),
                                )),
                            )
                            .await?;
                        }
                        Err(diesel::result::Error::DatabaseError(
                            diesel::result::DatabaseErrorKind::UniqueViolation,
                            _,
                        )) => {
                            warn!(
                                channel = channel.to_string(),
                                server = server.to_string(),
                                "voice server reported a session for a channel already in a call"
                            );
                        }
                        Err(e) => return Err(e.into()),
                    }
                }
                VoiceReport::ParticipantJoined { session, user } => {
                    let session_id = VoiceSessionId::from(session);
                    let Some(existing) = find_session(conn.as_mut(), session_id).await? else {
                        warn!(
                            session = session.to_string(),
                            "participant reported for an unknown session"
                        );
                        return Ok(());
                    };
                    let row = VoiceParticipant {
                        session: session_id,
                        user: UserId::from(user),
                        joined_at: Utc::now(),
                        muted: false,
                        deafened: false,
                        sharing_screen: false,
                    };
                    let inserted = diesel::insert_into(voice_participant::table)
                        .values(&row)
                        .on_conflict_do_nothing()
                        .execute(conn.as_mut())
                        .await?;
                    if inserted > 0 {
                        publish_event(
                            state,
                            conn.as_mut(),
                            EventScope::Channel(existing.channel),
                            &ServerEvent::VoiceParticipant(VoiceParticipantEvent::Create(
                                participant_record(&row, existing.channel),
                            )),
                        )
                        .await?;
                        note_company(conn.as_mut(), &existing).await?;
                    }
                }
                VoiceReport::ParticipantLeft { session, user } => {
                    let session_id = VoiceSessionId::from(session);
                    let Some(existing) = find_session(conn.as_mut(), session_id).await? else {
                        return Ok(());
                    };
                    remove_participant(state, conn.as_mut(), &existing, UserId::from(user)).await?;
                    let remaining = note_company(conn.as_mut(), &existing).await?;
                    if remaining == 0 {
                        end_session(
                            state,
                            conn.as_mut(),
                            &existing,
                            VoiceSessionEndReason::Empty,
                        )
                        .await?;
                    }
                }
                VoiceReport::Speaking {
                    session,
                    user,
                    speaking,
                } => {
                    let Some(existing) =
                        find_session(conn.as_mut(), VoiceSessionId::from(session)).await?
                    else {
                        return Ok(());
                    };
                    publish_event(
                        state,
                        conn.as_mut(),
                        EventScope::Channel(existing.channel),
                        &ServerEvent::VoiceSpeaking {
                            channel: existing.channel,
                            user: UserId::from(user),
                            speaking,
                        },
                    )
                    .await?;
                }
                VoiceReport::ParticipantState {
                    session,
                    user,
                    muted,
                    deafened,
                    sharing_screen,
                } => {
                    let session_id = VoiceSessionId::from(session);
                    let changed = diesel::update(voice_participant::table)
                        .filter(
                            voice_participant::session
                                .eq(session_id)
                                .and(voice_participant::user.eq(UserId::from(user))),
                        )
                        .set((
                            voice_participant::muted.eq(muted),
                            voice_participant::deafened.eq(deafened),
                            voice_participant::sharing_screen.eq(sharing_screen),
                        ))
                        .execute(conn.as_mut())
                        .await?;
                    if changed > 0 {
                        publish_event(
                            state,
                            conn.as_mut(),
                            EventScope::Session(session_id),
                            &ServerEvent::VoiceParticipant(VoiceParticipantEvent::Update {
                                session: session_id,
                                user: UserId::from(user),
                                muted: Some(muted),
                                deafened: Some(deafened),
                                sharing_screen: Some(sharing_screen),
                            }),
                        )
                        .await?;
                    }
                }
                VoiceReport::SessionEnded { session } => {
                    if let Some(existing) =
                        find_session(conn.as_mut(), VoiceSessionId::from(session)).await?
                    {
                        end_session(
                            state,
                            conn.as_mut(),
                            &existing,
                            VoiceSessionEndReason::Empty,
                        )
                        .await?;
                    }
                }
                VoiceReport::FileOffered {
                    channel,
                    record,
                    sender,
                    name,
                    size,
                    allow_direct,
                    valid_for_seconds,
                } => {
                    app::file_transfer::record_offer(
                        conn.as_mut(),
                        app::file_transfer::NewOffer {
                            channel,
                            record,
                            sender,
                            name,
                            size,
                            allow_direct,
                            valid_for_seconds,
                        },
                    )
                    .await?;
                }
                VoiceReport::TransferStarted {
                    record,
                    receiver,
                    mode,
                    ..
                } => {
                    app::file_transfer::record_start(conn.as_mut(), record, receiver, mode).await?;
                }
                VoiceReport::TransferEnded {
                    record,
                    receiver,
                    ended_by,
                    reason,
                } => {
                    app::file_transfer::record_end(
                        conn.as_mut(),
                        record,
                        receiver,
                        ended_by,
                        reason,
                    )
                    .await?;
                }
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// The call on `channel` whose server has not reported within `silence`, if that is the
/// call there.
async fn session_on_channel(
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
) -> app::Result<Option<VoiceSession>> {
    voice_session::table
        .select(VoiceSession::as_select())
        .filter(voice_session::channel.eq(channel))
        .first(conn)
        .await
        .optional_not_found()
}

async fn find_session(
    conn: &mut AsyncPgConnection,
    id: VoiceSessionId,
) -> app::Result<Option<VoiceSession>> {
    voice_session::table
        .select(VoiceSession::as_select())
        .filter(voice_session::id.eq(id))
        .first(conn)
        .await
        .optional_not_found()
}

async fn remove_participant(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    session: &VoiceSession,
    user: UserId,
) -> app::Result<()> {
    let deleted = diesel::delete(voice_participant::table)
        .filter(
            voice_participant::session
                .eq(session.id)
                .and(voice_participant::user.eq(user)),
        )
        .execute(conn)
        .await?;
    if deleted > 0 {
        publish_event(
            state,
            conn,
            EventScope::Channel(session.channel),
            &ServerEvent::VoiceParticipant(VoiceParticipantEvent::Delete {
                session: session.id,
                user,
            }),
        )
        .await?;
    }
    Ok(())
}

/// The value `alone_since` takes after the participant count settles at `count`: cleared
/// while the call has company, the earlier of now and the standing value while it does not.
fn alone_after(
    count: i64,
    current: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if count >= 2 {
        None
    } else {
        current.or(Some(now))
    }
}

/// Re-evaluates whether the call has company after a join or leave and returns how many are
/// in it.
async fn note_company(conn: &mut AsyncPgConnection, session: &VoiceSession) -> app::Result<i64> {
    let count: i64 = voice_participant::table
        .filter(voice_participant::session.eq(session.id))
        .count()
        .get_result(conn)
        .await?;
    let current: Option<DateTime<Utc>> = voice_session::table
        .select(voice_session::alone_since)
        .filter(voice_session::id.eq(session.id))
        .first(conn)
        .await?;
    let next = alone_after(count, current, Utc::now());
    if next != current {
        diesel::update(voice_session::table)
            .filter(voice_session::id.eq(session.id))
            .set(voice_session::alone_since.eq(next))
            .execute(conn)
            .await?;
    }
    Ok(count)
}

/// Ends a call: every participant is told to leave, the reason is announced, then the
/// session goes.
async fn end_session(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    session: &VoiceSession,
    reason: VoiceSessionEndReason,
) -> app::Result<()> {
    let participants: Vec<VoiceParticipant> = voice_participant::table
        .select(VoiceParticipant::as_select())
        .filter(voice_participant::session.eq(session.id))
        .load(conn)
        .await?;
    for participant in participants {
        remove_participant(state, conn, session, participant.user).await?;
    }
    let deleted = diesel::delete(voice_session::table)
        .filter(voice_session::id.eq(session.id))
        .execute(conn)
        .await?;
    if deleted > 0 {
        publish_event(
            state,
            conn,
            EventScope::Channel(session.channel),
            &ServerEvent::VoiceSessionEnded {
                id: session.id,
                channel: session.channel,
                reason,
            },
        )
        .await?;
        publish_event(
            state,
            conn,
            EventScope::Channel(session.channel),
            &ServerEvent::VoiceSession(VoiceSessionEvent::Delete { id: session.id }),
        )
        .await?;
    }
    Ok(())
}

/// Starts the task that ends the sessions of voice servers that stopped reporting and the
/// calls that have sat with one person for too long.
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
        }
    });
}

/// Ends every call that has gone `idle_session_seconds` without holding two people at once.
async fn reap_idle_sessions(state: &GlobalServerContext) -> app::Result<()> {
    let idle = Duration::seconds(
        i64::try_from(state.config.voice.idle_session_seconds).unwrap_or(24 * 60 * 60),
    );
    let cutoff = Utc::now() - idle;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let idle: Vec<VoiceSession> = voice_session::table
                .select(VoiceSession::as_select())
                .filter(voice_session::alone_since.lt(cutoff))
                .load(conn.as_mut())
                .await?;
            for session in idle {
                info!(
                    session = session.id.0.to_string(),
                    "ending a call that has been alone for the idle limit"
                );
                end_session(state, conn.as_mut(), &session, VoiceSessionEndReason::Idle).await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Ends every session on a server whose last report is older than `session_silence_seconds`.
/// A server that never reported is given that long from the session's start instead.
async fn reap_silent_servers(state: &GlobalServerContext) -> app::Result<()> {
    let silence = Duration::seconds(
        i64::try_from(state.config.voice.session_silence_seconds).unwrap_or(24 * 60 * 60),
    );
    let cutoff = Utc::now() - silence;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let stale: Vec<VoiceSession> = voice_session::table
                .inner_join(voice_server::table)
                .select(VoiceSession::as_select())
                .filter(
                    voice_server::last_report_at
                        .lt(cutoff)
                        .or(voice_server::last_report_at
                            .is_null()
                            .and(voice_session::created_at.lt(cutoff))),
                )
                .load(conn.as_mut())
                .await?;
            for session in stale {
                warn!(
                    session = session.id.0.to_string(),
                    server = session.voice_server.0.to_string(),
                    "ending a call whose voice server stopped reporting"
                );
                end_session(
                    state,
                    conn.as_mut(),
                    &session,
                    VoiceSessionEndReason::ServerLost,
                )
                .await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
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

#[cfg(test)]
mod tests {
    use super::*;

    fn server(
        enabled: bool,
        load: i32,
        capacity: i32,
        reported: Option<DateTime<Utc>>,
    ) -> VoiceServer {
        VoiceServer {
            id: VoiceServerId::new(),
            name: "s".to_string(),
            url: "https://voice.example.org".to_string(),
            capacity,
            enabled,
            created_at: Utc::now(),
            last_report_at: reported,
            reported_participants: load,
        }
    }

    #[test]
    fn only_enabled_servers_with_room_that_still_report_take_sessions() {
        let now = Utc::now();
        let timeout = Duration::seconds(60);
        assert!(accepts_sessions(
            &server(true, 3, 10, Some(now)),
            now,
            timeout
        ));
        // never reported: not started yet
        assert!(!accepts_sessions(&server(true, 0, 10, None), now, timeout));
        assert!(!accepts_sessions(
            &server(false, 0, 10, Some(now)),
            now,
            timeout
        ));
        assert!(!accepts_sessions(
            &server(true, 10, 10, Some(now)),
            now,
            timeout
        ));
        assert!(!accepts_sessions(
            &server(true, 0, 10, Some(now - Duration::seconds(61))),
            now,
            timeout
        ));
    }

    #[test]
    fn candidates_are_capped_and_the_threshold_counts_distinct_users() {
        let servers: Vec<VoiceServer> = (0..25).map(|_| server(true, 0, 10, None)).collect();
        let picked = pick_candidates(servers.clone(), 10);
        assert_eq!(picked.len(), 10);
        assert!(picked.iter().all(|p| servers.iter().any(|s| s.id == p.id)));
        assert_eq!(pick_candidates(servers[..3].to_vec(), 10).len(), 3);
        assert!(!should_disable(4, 5));
        assert!(should_disable(5, 5));
        assert!(!should_disable(100, 0));
    }

    #[test]
    fn a_call_is_alone_from_its_first_lonely_moment_until_someone_else_arrives() {
        let now = Utc::now();
        let earlier = now - Duration::hours(3);
        assert_eq!(alone_after(0, None, now), Some(now));
        assert_eq!(alone_after(1, None, now), Some(now));
        assert_eq!(alone_after(1, Some(earlier), now), Some(earlier));
        assert_eq!(alone_after(2, Some(earlier), now), None);
        assert_eq!(alone_after(3, None, now), None);
    }
}
