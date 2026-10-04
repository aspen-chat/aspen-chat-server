//! A DM's call ringing the people in it, and the message it leaves when it ends.

use super::VoiceSession;
use super::sessions::session_on_channel;
use crate::api::message_enum;
use crate::api::message_enum::server_event::{MessageEvent, ServerEvent, VoiceRingEvent};
use crate::app;
use crate::app::channel::ChannelType;
use crate::app::context::GlobalServerContext;
use crate::app::message::MessageKind;
use crate::app::permissions::channel_access;
use crate::app::{ChannelId, EventScope, UserId, VoiceSessionId, publish_event};
use crate::app::{MaybeLoaded, MessageId};
use crate::database::schema::{channel, dm_recipient, message, voice_ring, voice_session};
use chrono::{DateTime, Duration, Utc};
use diesel::{ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable, SelectableHelper};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

/// How long a DM's call rings the people it rings, unless they join or decline sooner.
pub const RING_SECONDS: i64 = 15;

/// Someone a DM's call is ringing (`voice_ring`).
#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = voice_ring)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct VoiceRing {
    session: VoiceSessionId,
    user: UserId,
    caller: UserId,
    rung_at: DateTime<Utc>,
    until: DateTime<Utc>,
}

fn ring_record(ring: &VoiceRing, channel: ChannelId) -> message_enum::VoiceRing {
    message_enum::VoiceRing {
        session: ring.session,
        user: ring.user,
        channel,
        caller: ring.caller,
        until: ring.until,
    }
}

/// Notes who started a call, its first participant, and, in a DM or group DM, rings everyone
/// else in it for `RING_SECONDS`.
pub(super) async fn start_call(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    session: &VoiceSession,
    caller: UserId,
) -> app::Result<()> {
    diesel::update(voice_session::table)
        .filter(voice_session::id.eq(session.id))
        .set(voice_session::started_by.eq(Some(caller)))
        .execute(conn)
        .await?;
    let ty: ChannelType = channel::table
        .select(channel::ty)
        .filter(channel::id.eq(session.channel))
        .first(conn)
        .await?;
    if !matches!(ty, ChannelType::Dm | ChannelType::GroupDm) {
        return Ok(());
    }
    let others: Vec<UserId> = dm_recipient::table
        .select(dm_recipient::user)
        .filter(dm_recipient::channel.eq(session.channel))
        .filter(dm_recipient::user.ne(caller))
        .load(conn)
        .await?;
    let now = Utc::now();
    for user in others {
        let ring = VoiceRing {
            session: session.id,
            user,
            caller,
            rung_at: now,
            until: now + Duration::seconds(RING_SECONDS),
        };
        diesel::insert_into(voice_ring::table)
            .values(&ring)
            .on_conflict_do_nothing()
            .execute(conn)
            .await?;
        publish_event(
            state,
            conn,
            EventScope::Channel(session.channel),
            &ServerEvent::VoiceRing(VoiceRingEvent::Create(ring_record(&ring, session.channel))),
        )
        .await?;
    }
    Ok(())
}

/// Ends the ring of `user` for a call, if it rings them: they joined it or declined it.
pub(super) async fn end_ring(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    session: &VoiceSession,
    user: UserId,
) -> app::Result<()> {
    let deleted = diesel::delete(voice_ring::table)
        .filter(voice_ring::session.eq(session.id))
        .filter(voice_ring::user.eq(user))
        .execute(conn)
        .await?;
    if deleted > 0 {
        publish_event(
            state,
            conn,
            EventScope::Channel(session.channel),
            &ServerEvent::VoiceRing(VoiceRingEvent::Delete {
                session: session.id,
                user,
            }),
        )
        .await?;
    }
    Ok(())
}

/// Declines a DM's call that rings the caller: it stops ringing them on every device. A call
/// that does not ring them, or no call at all, is left as it is.
pub async fn decline_ring(
    state: &GlobalServerContext,
    user: UserId,
    channel_id: ChannelId,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    channel_access(state, conn.as_mut(), user, channel_id).await?;
    conn.transaction(|conn| {
        async move {
            if let Some(session) = session_on_channel(conn.as_mut(), channel_id).await? {
                end_ring(state, conn.as_mut(), &session, user).await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Who the calls in `channels` are ringing, as long as each ring lasts.
pub async fn read_channels_rings(
    state: &GlobalServerContext,
    channels: &[ChannelId],
) -> app::Result<Vec<message_enum::VoiceRing>> {
    if channels.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<(VoiceRing, ChannelId)> = voice_ring::table
        .inner_join(voice_session::table)
        .select((VoiceRing::as_select(), voice_session::channel))
        .filter(voice_session::channel.eq_any(channels))
        .filter(voice_ring::until.gt(Utc::now()))
        .load(conn.as_mut())
        .await?;
    Ok(rows
        .iter()
        .map(|(ring, channel)| ring_record(ring, *channel))
        .collect())
}

/// Records that a DM's call ended, in the name of whoever started it: as a message of kind
/// `Call` saying how long it lasted, or, if no one else ever joined, of kind `MissedCall`. A
/// call in a voice channel, or one no one was ever in, leaves nothing. The session is read
/// afresh, since whether it had company is decided as people join.
pub(super) async fn record_call(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    session: &VoiceSession,
) -> app::Result<()> {
    let Some(starter) = session.started_by else {
        return Ok(());
    };
    let had_company: bool = voice_session::table
        .select(voice_session::had_company)
        .filter(voice_session::id.eq(session.id))
        .first(conn)
        .await?;
    let ty: ChannelType = channel::table
        .select(channel::ty)
        .filter(channel::id.eq(session.channel))
        .first(conn)
        .await?;
    if !matches!(ty, ChannelType::Dm | ChannelType::GroupDm) {
        return Ok(());
    }
    let now = Utc::now();
    let seconds = (now - session.created_at).num_seconds().max(0);
    let row = app::message::Message {
        id: MessageId::new(),
        channel: MaybeLoaded::from_id(session.channel),
        content: String::new(),
        author: MaybeLoaded::from_id(starter),
        timestamp: now,
        deleted_at: None,
        edited_at: None,
        kind: if had_company {
            MessageKind::Call
        } else {
            MessageKind::MissedCall
        },
        poll: None,
        thread: None,
        echo_of: None,
        mentions: app::mention::Mentions::default(),
        call_seconds: had_company.then(|| i32::try_from(seconds).unwrap_or(i32::MAX)),
        command_bot: None,
        linked_messages: Default::default(),
        warning: None,
        altered_by: Vec::new(),
    };
    diesel::insert_into(message::table)
        .values(&row)
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        EventScope::Channel(session.channel),
        &ServerEvent::Message(MessageEvent::Create(app::message::record(
            &row,
            Vec::new(),
            Vec::new(),
        ))),
    )
    .await?;
    Ok(())
}

/// Deletes rings that ran out a while ago. Every client ends a ring at its `until` by its own
/// clock and reads never return a spent one, so this is housekeeping and announces nothing.
pub(super) async fn clear_spent_rings(state: &GlobalServerContext) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    diesel::delete(voice_ring::table)
        .filter(voice_ring::until.lt(Utc::now() - Duration::minutes(1)))
        .execute(conn.as_mut())
        .await?;
    Ok(())
}
