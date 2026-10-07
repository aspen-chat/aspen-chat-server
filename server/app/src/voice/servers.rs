//! The registry of voice servers, the join offers that choose among them, the failure reports
//! that suspend one, and the reaping of calls on servers that stopped reporting.

use super::sessions::end_session;
use super::{OptionalNotFound, VoiceServer, VoiceServerChangeset, VoiceSession};
use crate::channel::ChannelType;
use crate::context::GlobalServerContext;
use crate::events::Publishing;
use crate::permissions::{ChannelAccess, Permissions, channel_access};
use crate::t;
use crate::user::UserPg;
use crate::{ChannelId, UserId, VoiceServerId};
use aspen_schema::{channel, voice_server, voice_server_failure, voice_session};
use aspen_wire::message_enum;
use aspen_wire::voice::VoiceSessionEndReason;
use chrono::{DateTime, Duration, Utc};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable,
    SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use fred::interfaces::KeysInterface;
use fred::types::Expiration;
use rand::seq::SliceRandom;
use tracing::warn;
use uuid::Uuid;
use voice_protocol::token::{Grants, JoinClaims, sign};

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = voice_server_failure)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct VoiceServerFailure {
    voice_server: VoiceServerId,
    user: UserId,
    reported_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------------------------
// Registry

pub async fn list_servers(state: &GlobalServerContext) -> crate::Result<Vec<VoiceServer>> {
    list_servers_in(state.connection_pool.get().await?.as_mut()).await
}

/// Every registered server, by name.
pub async fn list_servers_in(conn: &mut AsyncPgConnection) -> crate::Result<Vec<VoiceServer>> {
    Ok(voice_server::table
        .select(VoiceServer::as_select())
        .order(voice_server::name)
        .load(conn)
        .await?)
}

pub async fn create_server(
    state: &GlobalServerContext,
    name: String,
    url: String,
    capacity: i32,
) -> crate::Result<VoiceServer> {
    create_server_in(
        state.connection_pool.get().await?.as_mut(),
        name,
        url,
        capacity,
    )
    .await
}

/// Registers a server, enabled. A name already taken is a unique violation.
pub async fn create_server_in(
    conn: &mut AsyncPgConnection,
    name: String,
    url: String,
    capacity: i32,
) -> crate::Result<VoiceServer> {
    let row = VoiceServer {
        id: VoiceServerId::new(),
        name,
        url,
        capacity,
        enabled: true,
        created_at: Utc::now(),
        last_report_at: None,
        reported_participants: 0,
        suspended_until: None,
    };
    diesel::insert_into(voice_server::table)
        .values(&row)
        .execute(conn)
        .await?;
    Ok(row)
}

pub async fn update_server(
    state: &GlobalServerContext,
    id: VoiceServerId,
    changes: VoiceServerChangeset,
) -> crate::Result<VoiceServer> {
    update_server_in(state.connection_pool.get().await?.as_mut(), id, changes).await
}

/// Changes a server. Enabling or disabling it is an operator's decision, which lifts any
/// suspension for failures, and enabling it also forgets the failures that led to one.
pub async fn update_server_in(
    conn: &mut AsyncPgConnection,
    id: VoiceServerId,
    changes: VoiceServerChangeset,
) -> crate::Result<VoiceServer> {
    let enabled = changes.enabled;
    conn.transaction(|conn| {
        async move {
            let mut updated: VoiceServer = diesel::update(voice_server::table)
                .filter(voice_server::id.eq(id))
                .set(changes)
                .returning(VoiceServer::as_select())
                .get_result(conn)
                .await?;
            if let Some(enabled) = enabled {
                updated = diesel::update(voice_server::table)
                    .filter(voice_server::id.eq(id))
                    .set(voice_server::suspended_until.eq(None::<DateTime<Utc>>))
                    .returning(VoiceServer::as_select())
                    .get_result(conn)
                    .await?;
                if enabled {
                    diesel::delete(voice_server_failure::table)
                        .filter(voice_server_failure::voice_server.eq(id))
                        .execute(conn)
                        .await?;
                }
            }
            Ok(updated)
        }
        .scope_boxed()
    })
    .await
}

/// Removes a server. Its sessions go with it, so their participants are told to leave.
pub async fn delete_server(state: &GlobalServerContext, id: VoiceServerId) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let removed = conn
        .transaction(|conn| {
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
                    return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
                }
                Ok(())
            }
            .scope_boxed()
        })
        .await;
    if removed.is_ok() {
        super::reports::forget_server(state, id).await;
    }
    removed
}

/// Removes a server that holds no calls, for the terminal, which cannot end calls since that
/// records them as each server does. One that holds calls is a conflict: disabling it lets them
/// end without new ones starting there, and the dashboard removes it at once, ending them.
pub async fn delete_idle_server(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    id: VoiceServerId,
) -> crate::Result<()> {
    conn.transaction(|conn| {
        async move {
            let calls: i64 = voice_session::table
                .filter(voice_session::voice_server.eq(id))
                .count()
                .get_result(conn)
                .await?;
            if calls > 0 {
                return Err(crate::Error::Conflict(t!(
                    "voiceServerHoldsCalls",
                    calls = calls
                )));
            }
            let deleted = diesel::delete(voice_server::table)
                .filter(voice_server::id.eq(id))
                .execute(conn)
                .await?;
            if deleted == 0 {
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await?;
    super::reports::forget_server(state, id).await;
    Ok(())
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
    pub camera: bool,
}

/// Whether a server is offered to a joiner: enabled, not suspended, with room, and heard from
/// within the silence limit. A server that has never reported is one that has not started, so
/// it is not offered; a voice server reports its load the moment it starts.
fn accepts_sessions(server: &VoiceServer, now: DateTime<Utc>, offer_silence: Duration) -> bool {
    server.enabled
        && !suspended(server, now)
        && server.reported_participants < server.capacity
        && reporting(server, now, offer_silence)
}

/// Whether a server is suspended for failures at `now`.
fn suspended(server: &VoiceServer, now: DateTime<Utc>) -> bool {
    server.suspended_until.is_some_and(|until| until > now)
}

/// Whether a server has reported within `offer_silence`: one that has not is probably down.
pub(super) fn reporting(server: &VoiceServer, now: DateTime<Utc>, offer_silence: Duration) -> bool {
    server
        .last_report_at
        .is_some_and(|at| now - at <= offer_silence)
}

/// How long a server may go without reporting before it counts as down (`offer_silence_seconds`).
pub(super) fn offer_silence(state: &GlobalServerContext) -> Duration {
    Duration::seconds(i64::try_from(state.config.voice.offer_silence_seconds).unwrap_or(60))
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
    caller: &crate::two_factor::Caller,
    channel_id: ChannelId,
) -> crate::Result<JoinOffer> {
    let user = caller.user;
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
    if !matches!(
        ty,
        ChannelType::Voice | ChannelType::Dm | ChannelType::GroupDm
    ) {
        return Err(crate::Error::Validation(t!("voiceChannelOnly")));
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
    let silence = offer_silence(state);
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
        return Err(crate::Error::Validation(t!("voiceNoServers")));
    }
    let expires_at =
        now + Duration::seconds(i64::try_from(voice.join_token_ttl_seconds).unwrap_or(60));
    note_offered(state, user, &candidates).await;
    let Grants {
        speak,
        share_screen,
        camera,
        transfer_files,
    } = grants_of(state.settings().file_transfers, &access);
    let claims = JoinClaims {
        user: user.0,
        channel: channel_id.0,
        servers: candidates.iter().map(|server| server.id.0).collect(),
        expires_at: expires_at.timestamp(),
        nonce: Uuid::now_v7(),
        speak,
        share_screen,
        transfer_files,
        camera,
        sign_in: caller.sign_in_held(),
    };
    Ok(JoinOffer {
        session,
        candidates,
        token: sign(&claims, voice.token_secret.as_bytes()),
        expires_at,
        speak,
        share_screen,
        transfer_files,
        camera,
    })
}

/// What someone with `access` to a channel may do in its call besides listen and watch, where
/// the deployment setting `file_transfers` is `file_transfers`.
pub(super) fn grants_of(file_transfers: bool, access: &ChannelAccess) -> Grants {
    Grants {
        speak: access.has(Permissions::SPEAK),
        share_screen: access.has(Permissions::SHARE_SCREEN),
        camera: access.has(Permissions::USE_CAMERA),
        transfer_files: file_transfers && access.has(Permissions::TRANSFER_FILES),
    }
}

// ---------------------------------------------------------------------------------------------
// Failure reports

/// How long after its join token expires a failure report from someone offered a server still
/// counts against it: the client reports each candidate as it gives up on it, within the
/// token's lifetime, and the margin covers the report's own trip.
const OFFER_REPORT_GRACE_SECONDS: u64 = 60;

/// The Valkey key saying `user` was recently offered `server`.
fn offered_key(server: VoiceServerId, user: UserId) -> String {
    format!("voice_offered:{}:{}", server.0, user.0)
}

/// Notes that `user` was offered `servers`, so their failure reports about them count. A
/// Valkey outage is logged and the offer goes ahead; reports from it then count for nothing,
/// which leaves servers enabled rather than letting anyone disable them.
async fn note_offered(state: &GlobalServerContext, user: UserId, servers: &[VoiceServer]) {
    let ttl = state
        .config
        .voice
        .join_token_ttl_seconds
        .saturating_add(OFFER_REPORT_GRACE_SECONDS);
    let ttl = i64::try_from(ttl).unwrap_or(i64::MAX);
    let pipeline = state.valkey.pipeline();
    let noted: Result<Vec<fred::types::Value>, fred::error::Error> = async {
        for server in servers {
            let () = pipeline
                .set(
                    offered_key(server.id, user),
                    1,
                    Some(Expiration::EX(ttl)),
                    None,
                    false,
                )
                .await?;
        }
        pipeline.all().await
    }
    .await;
    if let Err(e) = noted {
        warn!(error = %e, "could not note the voice servers offered to a joiner");
    }
}

/// Whether `user` was offered `server` recently enough for their report about it to count. A
/// Valkey outage counts as not offered.
async fn was_offered(state: &GlobalServerContext, user: UserId, server: VoiceServerId) -> bool {
    match state
        .valkey
        .exists::<i64, _>(offered_key(server, user))
        .await
    {
        Ok(found) => found > 0,
        Err(e) => {
            warn!(error = %e, "could not read whether a voice server was offered to a reporter");
            false
        }
    }
}

/// The Valkey key saying `user` joined a call on `server` recently.
fn joined_key(server: VoiceServerId, user: UserId) -> String {
    format!("voice_joined:{}:{}", server.0, user.0)
}

/// Notes that `user` joined a call on `server`, as its report of the join is applied, so that for
/// `failure_window_seconds` their failure reports about it count for nothing: they reached it.
/// A Valkey outage is logged and leaves it unnoted.
pub(super) async fn note_joined(state: &GlobalServerContext, server: VoiceServerId, user: UserId) {
    let ttl = i64::try_from(state.config.voice.failure_window_seconds).unwrap_or(i64::MAX);
    let noted: Result<(), fred::error::Error> = state
        .valkey
        .set(
            joined_key(server, user),
            1,
            Some(Expiration::EX(ttl)),
            None,
            false,
        )
        .await;
    if let Err(e) = noted {
        warn!(error = %e, "could not note that someone joined a call on a voice server");
    }
}

/// Whether `user` joined a call on `server` within the failure window. A Valkey outage counts
/// as joined, so that reports then count for nothing rather than for anyone.
async fn joined_there(state: &GlobalServerContext, user: UserId, server: VoiceServerId) -> bool {
    match state
        .valkey
        .exists::<i64, _>(joined_key(server, user))
        .await
    {
        Ok(found) => found > 0,
        Err(e) => {
            warn!(error = %e, "could not read whether a reporter joined a call on a voice server");
            true
        }
    }
}

/// The outcome of a failure report.
pub struct FailureOutcome {
    /// Distinct users whose reports about this server count within the window, this one
    /// included when it counted.
    pub failures: u32,
    /// Whether the server is now out of join offers: disabled by an operator, or suspended for
    /// failures.
    pub disabled: bool,
    /// Whether this report counted against the server.
    pub counted: bool,
}

/// Whether `failures` distinct users within the window is enough to suspend a server.
fn should_suspend(failures: u32, threshold: u32) -> bool {
    threshold > 0 && failures >= threshold
}

/// Records that `reporter` could not start a session on the server. A report counts only from
/// one of this deployment's own people (not a bot, whose owner may have many, nor a foreign
/// user, whom another deployment vouches for) who was offered the server in a join offer within
/// the token's lifetime and `OFFER_REPORT_GRACE_SECONDS` and has not joined a call on it within
/// the failure window, so no one can take out a server they were never sent to or reached; any
/// other report is answered with the server's standing and changes nothing. A report counts
/// once per user within the window, at their latest attempt. Once the configured number of
/// distinct users have reported, the server is suspended for `failure_window_seconds`, after
/// which it takes calls again with its failures forgotten (they are all older than the window
/// by then), unless that would leave no other server taking calls: throwaway accounts can then
/// suspend every server but one, never the last.
pub async fn report_failure(
    state: &GlobalServerContext,
    reporter: &UserPg,
    server: VoiceServerId,
) -> crate::Result<FailureOutcome> {
    let voice = &state.config.voice;
    let user = reporter.id;
    let now = Utc::now();
    let counted = !reporter.bot
        && !reporter.system
        && reporter.home_domain.is_none()
        && was_offered(state, user, server).await
        && !joined_there(state, user, server).await;
    let window = Duration::seconds(i64::try_from(voice.failure_window_seconds).unwrap_or(3600));
    let window_start = now - window;
    let silence = offer_silence(state);
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            // Every server, locked, so two suspensions at once cannot leave none taking calls.
            let servers: Vec<VoiceServer> = voice_server::table
                .select(VoiceServer::as_select())
                .order(voice_server::id)
                .for_update()
                .load(conn.as_mut())
                .await?;
            let target = servers
                .iter()
                .find(|candidate| candidate.id == server)
                .ok_or(crate::Error::Diesel(diesel::result::Error::NotFound))?;
            if counted {
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
                    .await?;
            }
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
            let mut disabled = !target.enabled || suspended(target, now);
            if counted && !disabled && should_suspend(failures, voice.failure_threshold) {
                let others = servers
                    .iter()
                    .filter(|other| other.id != server && accepts_sessions(other, now, silence))
                    .count();
                if others == 0 {
                    warn!(
                        server = server.0.to_string(),
                        failures,
                        "voice server left taking calls despite failures from distinct users: no other server takes calls"
                    );
                } else {
                    diesel::update(voice_server::table)
                        .filter(voice_server::id.eq(server))
                        .set(voice_server::suspended_until.eq(now + window))
                        .execute(conn.as_mut())
                        .await?;
                    warn!(
                        server = server.0.to_string(),
                        failures, "voice server suspended after failures from distinct users"
                    );
                    disabled = true;
                }
            }
            Ok(FailureOutcome {
                failures,
                disabled,
                counted,
            })
        }
        .scope_boxed()
    })
    .await
}

/// Ends every session on a server whose last report is older than `session_silence_seconds`.
/// A server that never reported is given that long from the session's start instead.
pub(super) async fn reap_silent_servers(state: &GlobalServerContext) -> crate::Result<()> {
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
            suspended_until: None,
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
        // suspended for failures until a minute from now, and then taking calls again
        let mut held = server(true, 0, 10, Some(now));
        held.suspended_until = Some(now + Duration::seconds(60));
        assert!(!accepts_sessions(&held, now, timeout));
        assert!(accepts_sessions(
            &held,
            now + Duration::seconds(60),
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
        assert!(!should_suspend(4, 5));
        assert!(should_suspend(5, 5));
        assert!(!should_suspend(100, 0));
    }
}
