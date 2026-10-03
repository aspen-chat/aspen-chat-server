//! What becomes of a call as its voice server reports on it, the commands a moderator sends that
//! server, and the ending of calls that sat alone too long.

use super::ring::{end_ring, record_call, start_call};
use super::{OptionalNotFound, VoiceParticipant, VoiceSession, participant_record};
use crate::api::message_enum;
use crate::api::message_enum::server_event::{
    ServerEvent, VoiceParticipantEvent, VoiceSessionEvent,
};
use crate::api::voice::VoiceSessionEndReason;
use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::permissions::{Permissions, channel_access, missing};
use crate::app::{ChannelId, EventScope, UserId, VoiceServerId, VoiceSessionId, publish_event};
use crate::database::schema::{voice_participant, voice_server, voice_session};
use chrono::{DateTime, Duration, Utc};
use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use tracing::{info, warn};
use voice_protocol::control::{
    ParticipantSnapshot, REPORT_PARTITIONS, VoiceCommand, VoiceReport, command_subject,
};

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

/// Removes `user` from every call they are in, as when they are banned from the deployment
/// (`app::user_ban`). Each voice server disconnects them and reports their leaving.
pub async fn kick_everywhere(state: &GlobalServerContext, user: UserId) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let calls: Vec<(VoiceSessionId, VoiceServerId)> = voice_participant::table
        .inner_join(voice_session::table)
        .select((voice_session::id, voice_session::voice_server))
        .filter(voice_participant::user.eq(user))
        .load(conn.as_mut())
        .await?;
    for (session, server) in calls {
        let payload = serde_json::to_vec(&VoiceCommand::Kick {
            session: session.0,
            user: user.0,
        })?;
        state
            .nats_context
            .client()
            .publish(command_subject(server.0), payload.into())
            .await
            .map_err(app::Error::VoiceCommand)?;
    }
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

/// Applies one report from a voice server, `published` being when the report stream took it.
/// The reports about one channel arrive here one at a time in the order they were sent
/// (`super::reports`), and applying one twice changes nothing more, since a report whose
/// acknowledgement was lost is delivered again.
pub(super) async fn apply_report(
    state: &GlobalServerContext,
    report: VoiceReport,
    published: DateTime<Utc>,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    // A speaking change is only passed on: it writes nothing, so it needs no transaction, and
    // it names its channel, so it needs no lookup either.
    if let VoiceReport::Speaking {
        channel,
        user,
        speaking,
        ..
    } = report
    {
        let channel = ChannelId::from(channel);
        return publish_event(
            state,
            conn.as_mut(),
            EventScope::Channel(channel),
            &ServerEvent::VoiceSpeaking {
                channel,
                user: UserId::from(user),
                speaking,
            },
        )
        .await;
    }
    conn.transaction(|conn| {
        async move {
            match report {
                VoiceReport::Load {
                    server,
                    participants,
                } => {
                    // When the server sent it, not when it was applied: a report that waited in
                    // the stream says nothing about whether the server is up now.
                    diesel::update(voice_server::table)
                        .filter(voice_server::id.eq(VoiceServerId::from(server)))
                        .set((
                            voice_server::last_report_at.eq(published),
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
                    record_session(state, conn.as_mut(), server, session, channel).await?;
                }
                VoiceReport::ParticipantJoined { session, user, .. } => {
                    let session_id = VoiceSessionId::from(session);
                    let Some(mut existing) = find_session(conn.as_mut(), session_id).await? else {
                        warn!(
                            session = session.to_string(),
                            "participant reported for an unknown session"
                        );
                        return Ok(());
                    };
                    record_participant(
                        state,
                        conn.as_mut(),
                        &mut existing,
                        &ParticipantSnapshot {
                            user,
                            muted: false,
                            deafened: false,
                            sharing_screen: false,
                        },
                    )
                    .await?;
                }
                VoiceReport::ParticipantLeft { session, user, .. } => {
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
                // Applied above, outside any transaction.
                VoiceReport::Speaking { .. } => {}
                VoiceReport::ParticipantState {
                    session,
                    user,
                    muted,
                    deafened,
                    sharing_screen,
                    ..
                } => {
                    record_state(
                        state,
                        conn.as_mut(),
                        VoiceSessionId::from(session),
                        &ParticipantSnapshot {
                            user,
                            muted,
                            deafened,
                            sharing_screen,
                        },
                    )
                    .await?;
                }
                VoiceReport::SessionEnded { session, .. } => {
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
                    ..
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
                VoiceReport::SessionSnapshot {
                    server,
                    session,
                    channel,
                    participants,
                } => {
                    apply_snapshot(
                        state,
                        conn.as_mut(),
                        server,
                        session,
                        channel,
                        &participants,
                    )
                    .await?;
                }
                VoiceReport::SessionsHeld {
                    server,
                    partition,
                    sessions,
                } => {
                    end_sessions_not_held(state, conn.as_mut(), server, partition, &sessions)
                        .await?;
                }
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Records the session a voice server started, or returns the one already recorded under its
/// id. `None` when the channel's call could not be recorded as this one.
async fn record_session(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    server: uuid::Uuid,
    session: uuid::Uuid,
    channel: uuid::Uuid,
) -> app::Result<Option<VoiceSession>> {
    let now = Utc::now();
    let mut row = VoiceSession {
        id: VoiceSessionId::from(session),
        channel: ChannelId::from(channel),
        voice_server: VoiceServerId::from(server),
        created_at: now,
        alone_since: Some(now),
        started_by: None,
        had_company: false,
    };
    // A voice server reports a session only once it holds the room, and it holds the room
    // because a client with a valid join token arrived, so the call it announces is the one the
    // channel now has. A session already recorded for the channel is therefore a room that no
    // longer exists: the same server's, lost to a restart, or another server's that the joiner
    // could not reach. It is ended as lost, which sends its participants to rejoin, and their
    // offers now name this server.
    if let Some(stale) = session_on_channel(conn, row.channel).await? {
        if stale.id == row.id {
            return Ok(Some(stale));
        }
        // The call goes on on this server: its people rejoin, so it keeps its start and its
        // starter, and rings no one again.
        row.created_at = stale.created_at;
        row.started_by = stale.started_by;
        row.had_company = stale.had_company;
        end_session(state, conn, &stale, VoiceSessionEndReason::ServerLost).await?;
    }
    match diesel::insert_into(voice_session::table)
        .values(&row)
        .execute(conn)
        .await
    {
        Ok(_) => {
            publish_event(
                state,
                conn,
                EventScope::Channel(row.channel),
                &ServerEvent::VoiceSession(VoiceSessionEvent::Create(
                    message_enum::VoiceSession::from(&row),
                )),
            )
            .await?;
            Ok(Some(row))
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
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}

/// Records someone joining `session` in the state `joined` gives, unless they are recorded in
/// it already. The first to join starts the call, which in a DM rings everyone else.
async fn record_participant(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    session: &mut VoiceSession,
    joined: &ParticipantSnapshot,
) -> app::Result<()> {
    let row = VoiceParticipant {
        session: session.id,
        user: UserId::from(joined.user),
        joined_at: Utc::now(),
        muted: joined.muted,
        deafened: joined.deafened,
        sharing_screen: joined.sharing_screen,
    };
    let inserted = diesel::insert_into(voice_participant::table)
        .values(&row)
        .on_conflict_do_nothing()
        .execute(conn)
        .await?;
    if inserted == 0 {
        return Ok(());
    }
    publish_event(
        state,
        conn,
        EventScope::Channel(session.channel),
        &ServerEvent::VoiceParticipant(VoiceParticipantEvent::Create(participant_record(
            &row,
            session.channel,
        ))),
    )
    .await?;
    note_company(conn, session).await?;
    end_ring(state, conn, session, row.user).await?;
    if session.started_by.is_none() {
        start_call(state, conn, session, row.user).await?;
        session.started_by = Some(row.user);
    }
    Ok(())
}

/// Records a participant's mute, deafen, and sharing state as `now` gives it.
async fn record_state(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    session: VoiceSessionId,
    now: &ParticipantSnapshot,
) -> app::Result<()> {
    let user = UserId::from(now.user);
    let changed = diesel::update(voice_participant::table)
        .filter(
            voice_participant::session
                .eq(session)
                .and(voice_participant::user.eq(user)),
        )
        .set((
            voice_participant::muted.eq(now.muted),
            voice_participant::deafened.eq(now.deafened),
            voice_participant::sharing_screen.eq(now.sharing_screen),
        ))
        .execute(conn)
        .await?;
    if changed > 0 {
        publish_event(
            state,
            conn,
            EventScope::Session(session),
            &ServerEvent::VoiceParticipant(VoiceParticipantEvent::Update {
                session,
                user,
                muted: Some(now.muted),
                deafened: Some(now.deafened),
                sharing_screen: Some(now.sharing_screen),
            }),
        )
        .await?;
    }
    Ok(())
}

/// Makes the record of one call match what its voice server says of it: the session recorded,
/// everyone in it recorded in the state they are in, and no one else. Nothing changes when the
/// record was already right; a change means a report was lost, and is logged.
async fn apply_snapshot(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    server: uuid::Uuid,
    session: uuid::Uuid,
    channel: uuid::Uuid,
    participants: &[ParticipantSnapshot],
) -> app::Result<()> {
    let known = find_session(conn, VoiceSessionId::from(session))
        .await?
        .is_some();
    let Some(mut existing) = record_session(state, conn, server, session, channel).await? else {
        return Ok(());
    };
    let recorded: Vec<VoiceParticipant> = voice_participant::table
        .select(VoiceParticipant::as_select())
        .filter(voice_participant::session.eq(existing.id))
        .load(conn)
        .await?;
    let mut repairs = usize::from(!known);
    for participant in participants {
        match recorded.iter().find(|row| row.user.0 == participant.user) {
            None => {
                record_participant(state, conn, &mut existing, participant).await?;
                repairs += 1;
            }
            Some(row)
                if (row.muted, row.deafened, row.sharing_screen)
                    != (
                        participant.muted,
                        participant.deafened,
                        participant.sharing_screen,
                    ) =>
            {
                record_state(state, conn, existing.id, participant).await?;
                repairs += 1;
            }
            Some(_) => {}
        }
    }
    let departed: Vec<UserId> = recorded
        .iter()
        .map(|row| row.user)
        .filter(|user| !participants.iter().any(|p| p.user == user.0))
        .collect();
    for user in &departed {
        remove_participant(state, conn, &existing, *user).await?;
    }
    if !departed.is_empty() {
        note_company(conn, &existing).await?;
        repairs += departed.len();
    }
    if repairs > 0 {
        warn!(
            session = session.to_string(),
            server = server.to_string(),
            repairs,
            "a voice server's snapshot repaired the record of a call, so reports about it were lost"
        );
    }
    Ok(())
}

/// Ends every session recorded on `server`, in a channel of lane `partition`, that it no
/// longer holds.
async fn end_sessions_not_held(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    server: uuid::Uuid,
    partition: u8,
    held: &[uuid::Uuid],
) -> app::Result<()> {
    let held: Vec<VoiceSessionId> = held.iter().copied().map(VoiceSessionId::from).collect();
    // `voice_protocol::control::partition`, computed by the database.
    let in_lane = diesel::dsl::sql::<diesel::sql_types::Bool>(&format!(
        "get_byte(uuid_send(channel), 15) % {REPORT_PARTITIONS} = "
    ))
    .bind::<diesel::sql_types::Integer, _>(i32::from(partition));
    let gone: Vec<VoiceSession> = voice_session::table
        .select(VoiceSession::as_select())
        .filter(voice_session::voice_server.eq(VoiceServerId::from(server)))
        .filter(voice_session::id.ne_all(held))
        .filter(in_lane)
        .load(conn)
        .await?;
    for session in gone {
        warn!(
            session = session.id.0.to_string(),
            server = server.to_string(),
            "a voice server no longer holds a call recorded on it, so reports about it were lost"
        );
        // Its people may still believe they are in it, so they are sent to rejoin.
        end_session(state, conn, &session, VoiceSessionEndReason::ServerLost).await?;
    }
    Ok(())
}

/// The call on `channel` whose server has not reported within `silence`, if that is the
/// call there.
pub(super) async fn session_on_channel(
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
    if count >= 2 && !session.had_company {
        diesel::update(voice_session::table)
            .filter(voice_session::id.eq(session.id))
            .set(voice_session::had_company.eq(true))
            .execute(conn)
            .await?;
    }
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
pub(super) async fn end_session(
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
    // A call lost with its voice server goes on on another, so only one that is over is
    // recorded.
    if !matches!(reason, VoiceSessionEndReason::ServerLost) {
        record_call(state, conn, session).await?;
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

/// Ends every call that has gone `idle_session_seconds` without holding two people at once.
pub(super) async fn reap_idle_sessions(state: &GlobalServerContext) -> app::Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
