//! What becomes of a call as its voice server reports on it, the commands a moderator sends that
//! server, and the ending of calls that sat alone too long.

use super::ring::{end_ring, record_call, start_call};
use super::servers::{offer_silence, reporting};
use super::{OptionalNotFound, VoiceParticipant, VoiceServer, VoiceSession, participant_record};
use crate::context::GlobalServerContext;
use crate::events::Publishing;
use crate::permissions::{Permissions, channel_access, community_access, missing};
use crate::t;
use crate::{
    CategoryId, ChannelId, CommunityId, EventScope, UserId, VoiceServerId, VoiceSessionId,
    publish_event,
};
use aspen_schema::user as user_table;
use aspen_schema::{channel, voice_participant, voice_server, voice_session};
use aspen_wire::message_enum;
use aspen_wire::message_enum::server_event::{
    ServerEvent, VoiceParticipantEvent, VoiceSessionEvent,
};
use aspen_wire::voice::VoiceSessionEndReason;
use chrono::{DateTime, Duration, Utc};
use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use tracing::{info, warn};
use voice_protocol::control::{
    ParticipantSnapshot, REPORT_PARTITIONS, VoiceCommand, VoiceReport, command_subject,
};
use voice_protocol::signal::KickReason;

// ---------------------------------------------------------------------------------------------
// Moderation

/// Server-mutes or unmutes someone in a channel's call: the mute is the community's
/// (`super::mutes`), so it stands in every call of the community until lifted. The voice server
/// holding the call applies it and reports the new state, which becomes the participant's
/// `update` event; the record returned is the state as recorded before the command lands. Takes
/// being able to view the channel, and Manage calls in the community over someone ranking below
/// the caller, who may not be the owner.
pub async fn mute_participant(
    state: &GlobalServerContext,
    caller: UserId,
    channel: ChannelId,
    user: UserId,
    muted: bool,
) -> crate::Result<message_enum::VoiceParticipant> {
    let (community, participant) = {
        let mut conn = state.connection_pool.get().await?;
        let access = channel_access(state, conn.as_mut(), caller, channel).await?;
        // A DM's call has no moderators.
        let Some(community) = access.community.as_ref().map(|c| c.community) else {
            return Err(missing(Permissions::MANAGE_CALLS));
        };
        let session = session_on_channel(conn.as_mut(), channel)
            .await?
            .ok_or(crate::Error::Diesel(diesel::result::Error::NotFound))?;
        let participant: VoiceParticipant = voice_participant::table
            .select(VoiceParticipant::as_select())
            .filter(
                voice_participant::session
                    .eq(session.id)
                    .and(voice_participant::user.eq(user)),
            )
            .first(conn.as_mut())
            .await?;
        (community, participant)
    };
    let (_, changed) = super::mutes::set_muted(state, caller, community, user, muted).await?;
    // A change is announced, and the announcement rechecks their calls; one that changed
    // nothing still brings their calls in line with the mute as it stands.
    if !changed {
        recheck(state, Recheck::User(user));
    }
    Ok(participant_record(&participant, channel))
}

/// Removes someone from a channel's call. The voice server disconnects them, telling them
/// why, and reports their leaving, which deletes their participant row. Takes Manage calls and
/// ranking above them, who may not be the owner.
pub async fn kick_participant(
    state: &GlobalServerContext,
    caller: UserId,
    channel: ChannelId,
    user: UserId,
) -> crate::Result<()> {
    command_participant(state, caller, channel, user, |session| VoiceCommand::Kick {
        session: session.0,
        user: user.0,
        reason: None,
    })
    .await?;
    Ok(())
}

/// Removes `user` from every call they are in, as when they are banned from the deployment
/// (`app::user_ban`) or their account ends. Each voice server disconnects them and reports
/// their leaving; a join not yet reported is caught as its report is applied (`recheck_seat`).
pub async fn kick_everywhere(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    user: UserId,
) -> crate::Result<()> {
    let calls: Vec<(VoiceSessionId, VoiceServerId)> = voice_participant::table
        .inner_join(voice_session::table)
        .select((voice_session::id, voice_session::voice_server))
        .filter(voice_participant::user.eq(user))
        .load(conn)
        .await?;
    for (session, server) in calls {
        let payload = serde_json::to_vec(&VoiceCommand::Kick {
            session: session.0,
            user: user.0,
            reason: Some(KickReason::AccessLost),
        })?;
        state
            .nats()
            .client()
            .publish(command_subject(server.0), payload.into())
            .await
            .map_err(crate::Error::VoiceCommand)?;
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
) -> crate::Result<message_enum::VoiceParticipant> {
    let mut conn = state.connection_pool.get().await?;
    let access = channel_access(state, conn.as_mut(), caller, channel).await?;
    // A DM's call has no moderators.
    let Some(community) = access
        .community
        .as_ref()
        .filter(|community| community.has(Permissions::MANAGE_CALLS))
    else {
        return Err(missing(Permissions::MANAGE_CALLS));
    };
    // As with removing or banning, a moderator acts only on members ranking below them, and
    // never on the owner; a participant who is no member (a deployment moderator) ranks as
    // nobody.
    if user != caller {
        match community_access(conn.as_mut(), user, community.community).await? {
            Some(theirs) if theirs.owner => {
                return Err(crate::Error::Forbidden(t!("permissionRank")));
            }
            Some(theirs) if theirs.member => community.require_above(theirs.role_rank())?,
            _ => {}
        }
    }
    let session = session_on_channel(conn.as_mut(), channel)
        .await?
        .ok_or(crate::Error::Diesel(diesel::result::Error::NotFound))?;
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
        .map_err(crate::Error::VoiceCommand)?;
    Ok(participant_record(&participant, channel))
}

// ---------------------------------------------------------------------------------------------
// Who may stay

/// Whose calls a recheck covers.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Recheck {
    /// The user's participants that joined on tokens of sign-ins that ended, as `signInsEnded`
    /// names them: `ended` alone, or without it every one but `kept`. They leave their calls;
    /// nothing else about the user's calls changes.
    SignIns {
        user: UserId,
        ended: Option<String>,
        kept: Option<String>,
    },
    /// Everyone in a call in one of the community's channels.
    Community(CommunityId),
    /// Everyone in the channel's call.
    Channel(ChannelId),
    /// Everyone in a call in one of the category's channels.
    Category(CategoryId),
    /// Every call the user is in.
    User(UserId),
    /// Every call on the deployment.
    Everyone,
}

/// Brings the calls `which` names in line with what their participants may now do, after a
/// change to it has committed: whoever may no longer view the channel or join voice there (or
/// is banned from the deployment, or gone) is removed, and everyone else's grants are sent to
/// their voice server, which stops whatever they may no longer send. Runs on its own task, so
/// the change that called for it does not wait; a failure is logged, and the next change or
/// join brings the call in line.
pub fn recheck(state: &GlobalServerContext, which: Recheck) {
    let state = state.clone();
    tokio::spawn(async move {
        let rechecked = async {
            let mut conn = state.connection_pool.get().await?;
            recheck_in(&state, conn.as_mut(), which.clone()).await
        };
        if let Err(e) = rechecked.await {
            warn!(?which, error = %e, "could not recheck who may stay in calls");
        }
    });
}

/// `recheck` on `conn`, waiting for it, for an operator command, which has no server context.
pub async fn recheck_in(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    which: Recheck,
) -> crate::Result<()> {
    // Read from the row rather than a server's copy, which an operator command does not have.
    let file_transfers = crate::deployment_settings::load(conn).await?.file_transfers;
    let seats = voice_participant::table
        .inner_join(voice_session::table)
        .select((
            voice_session::id,
            voice_session::voice_server,
            voice_session::channel,
            voice_participant::user,
        ))
        .into_boxed();
    let seats = match which {
        Recheck::Community(community) => seats.filter(
            voice_session::channel.eq_any(
                channel::table
                    .select(channel::id)
                    .filter(channel::community.eq(Some(community))),
            ),
        ),
        Recheck::Channel(channel) => seats.filter(voice_session::channel.eq(channel)),
        Recheck::Category(category) => seats.filter(
            voice_session::channel.eq_any(
                channel::table
                    .select(channel::id)
                    .filter(channel::parent_category.eq(Some(category))),
            ),
        ),
        Recheck::User(user) | Recheck::SignIns { user, .. } => {
            seats.filter(voice_participant::user.eq(user))
        }
        Recheck::Everyone => seats,
    };
    let seats: Vec<(VoiceSessionId, VoiceServerId, ChannelId, UserId)> = seats.load(conn).await?;
    if let Recheck::SignIns { ended, kept, .. } = which {
        // The voice server knows which sign-in each participant joined on; it decides.
        for (session, server, _, user) in seats {
            let command = VoiceCommand::EndSignIns {
                session: session.0,
                user: user.0,
                ended: ended.clone(),
                kept: kept.clone(),
            };
            send_command(state, server, &command).await?;
        }
        return Ok(());
    }
    for (session, server, channel, user) in seats {
        let seat = Seat {
            session,
            server,
            channel,
            user,
        };
        recheck_seat(state, conn, file_transfers, seat).await?;
    }
    Ok(())
}

/// One participant as the record has them.
struct Seat {
    session: VoiceSessionId,
    server: VoiceServerId,
    channel: ChannelId,
    user: UserId,
}

/// Tells the seat's voice server what its participant may do in the call, or to remove them
/// when they may no longer be there. A voice server leaves grants that did not change as they
/// are.
async fn recheck_seat(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    file_transfers: bool,
    seat: Seat,
) -> crate::Result<()> {
    let Seat {
        session,
        server,
        channel,
        user,
    } = seat;
    // Gone, banned, or a bot whose owner is banned.
    let present: bool = diesel::select(diesel::dsl::exists(
        user_table::table.filter(
            user_table::id
                .eq(user)
                .and(user_table::deleted_at.is_null())
                .and(diesel::dsl::not(crate::user_ban::shut_out())),
        ),
    ))
    .get_result(conn)
    .await?;
    let removed = VoiceCommand::Kick {
        session: session.0,
        user: user.0,
        reason: Some(KickReason::AccessLost),
    };
    let access = if present {
        match channel_access(state, conn, user, channel).await {
            Ok(access) if access.has(Permissions::JOIN_VOICE) => Some(access),
            Ok(_) | Err(crate::Error::Diesel(diesel::result::Error::NotFound)) => None,
            Err(e) => return Err(e),
        }
    } else {
        None
    };
    let Some(access) = access else {
        return send_command(state, server, &removed).await;
    };
    let grant = VoiceCommand::Grant {
        session: session.0,
        user: user.0,
        grants: super::servers::grants_of(file_transfers, &access),
    };
    send_command(state, server, &grant).await?;
    // A moderator's mute as it stands in the community (`super::mutes`); a DM's call has none.
    // The voice server changes nothing when it already stands as said.
    let muted = match access.community.as_ref() {
        Some(community) => super::mutes::is_muted(conn, community.community, user).await?,
        None => false,
    };
    let mute = VoiceCommand::Mute {
        session: session.0,
        user: user.0,
        muted,
    };
    send_command(state, server, &mute).await
}

/// Sends `command` to the voice server `server`.
async fn send_command(
    state: &impl Publishing,
    server: VoiceServerId,
    command: &VoiceCommand,
) -> crate::Result<()> {
    let payload = serde_json::to_vec(command)?;
    state
        .nats()
        .client()
        .publish(command_subject(server.0), payload.into())
        .await
        .map_err(crate::Error::VoiceCommand)?;
    Ok(())
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
    from: VoiceServerId,
    published: DateTime<Utc>,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    if !reported_by(conn.as_mut(), &report, from).await? {
        warn!(
            server = %from.0,
            report = <&'static str>::from(&report),
            "a voice report about what is not that server's was dropped"
        );
        return Ok(());
    }
    // A speaking change is only passed on: it writes nothing, so it needs no transaction, and
    // the lookup of its session (`reported_by`) is all it reads.
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
    // Someone who joined on a token issued before a change to what they may do, or before
    // the sign-in it was issued to ended, is brought in line with it once their joining is
    // recorded.
    let joined = match &report {
        VoiceReport::ParticipantJoined {
            session,
            channel,
            user,
            sign_in,
        } => Some((
            VoiceSessionId::from(*session),
            ChannelId::from(*channel),
            UserId::from(*user),
            sign_in.clone(),
        )),
        _ => None,
    };
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
                    crate::file_transfer::record_offer(
                        conn.as_mut(),
                        crate::file_transfer::NewOffer {
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
                    crate::file_transfer::record_start(conn.as_mut(), record, receiver, mode)
                        .await?;
                }
                VoiceReport::TransferEnded {
                    record,
                    receiver,
                    ended_by,
                    reason,
                    ..
                } => {
                    crate::file_transfer::record_end(
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
            Ok::<_, crate::Error>(())
        }
        .scope_boxed()
    })
    .await?;
    if let Some((session, channel, user, sign_in)) = joined {
        let server: Option<VoiceServerId> = voice_session::table
            .select(voice_session::voice_server)
            .filter(voice_session::id.eq(session))
            .first(conn.as_mut())
            .await
            .optional_not_found()?;
        if let Some(server) = server {
            super::servers::note_joined(state, server, user).await;
        }
        let file_transfers = state.settings().file_transfers;
        if let Some(server) = server
            && let Err(e) = recheck_seat(
                state,
                conn.as_mut(),
                file_transfers,
                Seat {
                    session,
                    server,
                    channel,
                    user,
                },
            )
            .await
        {
            // The join is recorded; the next change to what they may do brings them in line.
            warn!(session = %session.0, error = %e, "could not recheck a joiner");
        }
        if let (Some(server), Some(sign_in)) = (server, sign_in)
            && let Err(e) =
                end_if_signed_out(state, conn.as_mut(), session, server, user, sign_in).await
        {
            warn!(session = %session.0, error = %e, "could not check a joiner's sign-in");
        }
    }
    Ok(())
}

/// Takes `user`'s participant in `session` out of the call if the sign-in its join token was
/// issued to has ended since: its `signInsEnded` may have reached the voice server before
/// the participant joined.
async fn end_if_signed_out(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    session: VoiceSessionId,
    server: VoiceServerId,
    user: UserId,
    sign_in: String,
) -> crate::Result<()> {
    if crate::login::sign_in_live(conn, user, &sign_in).await? {
        return Ok(());
    }
    let command = VoiceCommand::EndSignIns {
        session: session.0,
        user: user.0,
        ended: Some(sign_in),
        kept: None,
    };
    send_command(state, server, &command).await
}

/// Whether `report`, which came from voice server `from`, is about what that server holds: a
/// report naming a server must name `from`; one about a recorded session must be about one on
/// `from`, in the channel it names, a speaking change needing the session recorded; and one
/// about a file must be about a channel whose call, if one is recorded, is on `from`, an offer
/// needing one there. So a voice server, limited by its NATS user to its own subjects, cannot
/// report on another's calls or channels.
async fn reported_by(
    conn: &mut AsyncPgConnection,
    report: &VoiceReport,
    from: VoiceServerId,
) -> crate::Result<bool> {
    let (session, channel) = match report {
        VoiceReport::Load { server, .. }
        | VoiceReport::SessionStarted { server, .. }
        | VoiceReport::SessionSnapshot { server, .. }
        | VoiceReport::SessionsHeld { server, .. } => {
            return Ok(VoiceServerId::from(*server) == from);
        }
        VoiceReport::ParticipantJoined {
            session, channel, ..
        }
        | VoiceReport::ParticipantLeft {
            session, channel, ..
        }
        | VoiceReport::ParticipantState {
            session, channel, ..
        }
        | VoiceReport::Speaking {
            session, channel, ..
        }
        | VoiceReport::SessionEnded { session, channel } => (Some(*session), *channel),
        VoiceReport::FileOffered { channel, .. }
        | VoiceReport::TransferStarted { channel, .. }
        | VoiceReport::TransferEnded { channel, .. } => (None, *channel),
    };
    let channel = ChannelId::from(channel);
    match session {
        // A session not recorded (ended, or never recorded) is ignored by the report's own
        // handling, except a speaking change, which is passed on without a lookup of its own.
        Some(session) => Ok(
            match find_session(conn, VoiceSessionId::from(session)).await? {
                Some(recorded) => recorded.voice_server == from && recorded.channel == channel,
                None => !matches!(report, VoiceReport::Speaking { .. }),
            },
        ),
        None => Ok(match session_on_channel(conn, channel).await? {
            Some(recorded) => recorded.voice_server == from,
            None => !matches!(report, VoiceReport::FileOffered { .. }),
        }),
    }
}

/// Whether a session a voice server reports replaces the one recorded for its channel, on
/// `recorded_server`: it does when that is the reporting server itself (whose room for it is
/// gone, lost to a restart, since a server holds one room per channel) or a server that has
/// stopped reporting (which the joiner could not reach, and which offers no longer name). A
/// recorded call on another server that still reports goes on, and the reported room is a
/// second call in the channel, made on a join token that named several servers, by two people
/// starting the call at once or by one using the token twice.
fn replaces(
    recorded_server: VoiceServerId,
    reporting_server: VoiceServerId,
    recorded_server_reporting: bool,
) -> bool {
    recorded_server == reporting_server || !recorded_server_reporting
}

/// Tells `server` to close the room of `session`, a second call in a channel whose call goes
/// on elsewhere. Its people are told the call is closing and rejoin where it is recorded.
async fn close_room(
    state: &GlobalServerContext,
    server: VoiceServerId,
    session: VoiceSessionId,
) -> crate::Result<()> {
    let payload = serde_json::to_vec(&VoiceCommand::Close { session: session.0 })?;
    state
        .nats_context
        .client()
        .publish(command_subject(server.0), payload.into())
        .await
        .map_err(crate::Error::VoiceCommand)
}

/// Records the session a voice server started, or returns the one already recorded under its
/// id. `None` when the channel's call could not be recorded as this one: then the room is a
/// second call in the channel, and its server is told to close it.
async fn record_session(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    server: uuid::Uuid,
    session: uuid::Uuid,
    channel: uuid::Uuid,
) -> crate::Result<Option<VoiceSession>> {
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
    // because a client with a valid join token arrived. A session already recorded for the
    // channel on the same server, or on one that has stopped reporting, is a room that no
    // longer exists or cannot be reached: it is ended as lost, which sends its participants to
    // rejoin, and their offers now name this server. One recorded on another server that still
    // reports is the channel's call, and stays so: this room is closed, sending its people to
    // rejoin there, and every later report of it (a snapshot, a join) finds it unrecorded and
    // comes here or is ignored, so it never displaces the call it duplicates.
    if let Some(stale) = session_on_channel(conn, row.channel).await? {
        if stale.id == row.id {
            return Ok(Some(stale));
        }
        let recorded_server: VoiceServer = voice_server::table
            .select(VoiceServer::as_select())
            .filter(voice_server::id.eq(stale.voice_server))
            .first(conn)
            .await?;
        let recorded_reporting = reporting(&recorded_server, now, offer_silence(state));
        if !replaces(stale.voice_server, row.voice_server, recorded_reporting) {
            warn!(
                channel = channel.to_string(),
                server = server.to_string(),
                session = session.to_string(),
                recorded = stale.id.0.to_string(),
                "a voice server reported a second call in a channel whose call goes on on another \
                 server; closing it"
            );
            close_room(state, row.voice_server, row.id).await?;
            return Ok(None);
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
) -> crate::Result<()> {
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
) -> crate::Result<()> {
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
) -> crate::Result<()> {
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
) -> crate::Result<()> {
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

/// The call recorded on `channel`, if any.
pub(super) async fn session_on_channel(
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
) -> crate::Result<Option<VoiceSession>> {
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
) -> crate::Result<Option<VoiceSession>> {
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
) -> crate::Result<()> {
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
async fn note_company(conn: &mut AsyncPgConnection, session: &VoiceSession) -> crate::Result<i64> {
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
) -> crate::Result<()> {
    let participants: Vec<VoiceParticipant> = voice_participant::table
        .select(VoiceParticipant::as_select())
        .filter(voice_participant::session.eq(session.id))
        .load(conn)
        .await?;
    for participant in participants {
        remove_participant(state, conn, session, participant.user).await?;
    }
    // Two may end a call at once (every API server's reapers, a voice server's report): the
    // first to lock its row ends it, and the other finds it gone and leaves it be, so the call
    // is recorded once. The row is locked after the participants' rows, as a join locks them.
    let current = voice_session::table
        .select(voice_session::id)
        .filter(voice_session::id.eq(session.id))
        .for_update()
        .first::<VoiceSessionId>(conn)
        .await
        .optional_not_found()?;
    if current.is_none() {
        return Ok(());
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
pub(super) async fn reap_idle_sessions(state: &GlobalServerContext) -> crate::Result<()> {
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
    fn a_reported_call_replaces_a_recorded_one_only_where_that_one_is_lost() {
        let (a, b) = (VoiceServerId::new(), VoiceServerId::new());
        // The same server's room, lost to a restart, whether or not it reports.
        assert!(replaces(a, a, true));
        assert!(replaces(a, a, false));
        // Another server that stopped reporting.
        assert!(replaces(a, b, false));
        // Another server that still reports: the reported room is the second call.
        assert!(!replaces(a, b, true));
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
