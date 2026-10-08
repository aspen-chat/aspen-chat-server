//! How far each user has read each channel, and so which channels hold something new for them.
//!
//! A read position is a message id rather than a reference to a message: ids are UUIDv7 and
//! ordered by time, so "read up to this id" stays exact after the message it names is deleted,
//! and deleting a message never writes here. A position only moves forward, whichever device
//! reports it, and each move is published to the user's own subject as `channelRead` so their
//! other devices follow. Posting in a channel moves the poster's position to their message.
//!
//! Nothing a member's channels held before they joined (the community, or the DM) is unread to
//! them: until they have read past it, their position is that moment. A channel is unread while
//! it holds a message by someone else after the position; the caller's own messages, and those
//! of anyone they have blocked (`app::block`), never make a channel unread. Threads keep no
//! position of their own.
//!
//! Each read state also counts the unread messages that tag the caller (`app::mention`):
//! directly, through a role they hold now, or as everyone, up to [`MAX_COUNTED_MENTIONS`]. The
//! same messages are left out as for being unread.

use crate::channel::ChannelType;
use crate::context::GlobalServerContext;
use crate::t;
use crate::{ChannelId, EventScope, MessageId, UserId, publish_event};
use aspen_schema::{channel, message};
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, Nullable, Timestamptz, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

/// Where a user has read a channel up to, and the newest message there by anyone else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadState {
    pub channel: ChannelId,
    /// A position among the channel's message ids: every message after it is unread. It may
    /// name a message that has since been deleted, or none at all when it is the moment the
    /// user joined.
    pub last_read: MessageId,
    /// The newest message after `last_read` written by neither the user nor anyone they have
    /// blocked, if there is one: the channel is unread exactly when there is.
    pub last_message: Option<MessageId>,
    /// How many of the unread messages tag the user, up to [`MAX_COUNTED_MENTIONS`].
    pub mentions: u32,
}

/// The position just before every message posted after `at`: the smallest UUIDv7 of its
/// millisecond.
fn position_at(at: DateTime<Utc>) -> MessageId {
    let millis = u64::try_from(at.timestamp_millis()).unwrap_or(0);
    MessageId(uuid::Builder::from_unix_timestamp_millis(millis, &[0; 10]).into_uuid())
}

#[derive(QueryableByName)]
struct Row {
    #[diesel(sql_type = PgUuid)]
    channel: ChannelId,
    #[diesel(sql_type = Nullable<PgUuid>)]
    last_read: Option<MessageId>,
    #[diesel(sql_type = Timestamptz)]
    joined_at: DateTime<Utc>,
    #[diesel(sql_type = Nullable<PgUuid>)]
    last_message: Option<MessageId>,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    mentions: i64,
}

impl From<Row> for ReadState {
    fn from(row: Row) -> Self {
        let joined = position_at(row.joined_at);
        ReadState {
            channel: row.channel,
            last_read: row.last_read.filter(|r| r.0 > joined.0).unwrap_or(joined),
            last_message: row.last_message,
            mentions: u32::try_from(row.mentions).unwrap_or(u32::MAX),
        }
    }
}

/// The most unread tags counted in one channel: a count this high reads as "this many or more",
/// which the apps show as "99+", and no read of a channel's tags goes further than it.
pub const MAX_COUNTED_MENTIONS: i64 = 100;

/// The user's read states for the listed channels, in one query: only channels they belong to
/// (as a member of the community or a recipient of the DM), never threads or deleted channels.
/// Everything is read from the channel's position on: the newest message after it through the
/// `(channel, id)` index, and its tags through `mention`'s indexes on who is tagged (one branch
/// each for the user, everyone, and each role the user holds), so the work grows with what is
/// unread and is cut off at [`MAX_COUNTED_MENTIONS`].
async fn read(
    conn: &mut AsyncPgConnection,
    user: UserId,
    channels: &[ChannelId],
) -> crate::Result<Vec<ReadState>> {
    let rows: Vec<Row> = diesel::sql_query(
        r#"
        SELECT c.id AS channel,
               rs.message AS last_read,
               j.joined_at,
               m.id AS last_message,
               COALESCE(mc.mentions, 0) AS mentions
        FROM channel c
        LEFT JOIN community_user cu ON cu.community = c.community AND cu."user" = $1
        LEFT JOIN dm_recipient dr ON dr.channel = c.id AND dr."user" = $1
        LEFT JOIN read_state rs ON rs.channel = c.id AND rs."user" = $1
        CROSS JOIN LATERAL (
            SELECT COALESCE(cu.joined_at, dr.joined_at) AS joined_at
        ) j
        CROSS JOIN LATERAL (
            SELECT GREATEST(rs.message, aspen_uuid_floor(j.joined_at)) AS after
        ) p
        LEFT JOIN LATERAL (
            SELECT message.id FROM message
            WHERE message.channel = c.id
              AND message.id > p.after
              AND message.deleted_at IS NULL
              AND message.author <> $1
              AND NOT EXISTS (
                  SELECT 1 FROM user_block
                  WHERE user_block.blocker = $1 AND user_block.blocked = message.author
              )
            ORDER BY message.id DESC
            LIMIT 1
        ) m ON true
        LEFT JOIN LATERAL (
            SELECT count(*) AS mentions FROM (
                SELECT tagged.id
                FROM (
                    SELECT mn.message FROM mention mn
                    WHERE mn.target_user = $1 AND mn.channel = c.id AND mn.message > p.after
                    UNION
                    SELECT mn.message FROM mention mn
                    WHERE mn.everyone AND mn.channel = c.id AND mn.message > p.after
                    UNION
                    SELECT mn.message
                    FROM community_member_role r
                    JOIN mention mn ON mn.target_role = r.role
                    WHERE r."user" = $1 AND r.community = c.community
                      AND mn.channel = c.id AND mn.message > p.after
                ) t
                JOIN message tagged ON tagged.id = t.message
                WHERE tagged."timestamp" > j.joined_at
                  AND tagged.deleted_at IS NULL
                  AND tagged.author <> $1
                  AND NOT EXISTS (
                      SELECT 1 FROM user_block
                      WHERE user_block.blocker = $1 AND user_block.blocked = tagged.author
                  )
                LIMIT $3
            ) counted
        ) mc ON true
        WHERE c.id = ANY($2)
          AND c.deleted_at IS NULL
          AND c.parent_channel IS NULL
          AND (cu."user" IS NOT NULL OR dr."user" IS NOT NULL)
        "#,
    )
    .bind::<PgUuid, _>(user.0)
    .bind::<Array<PgUuid>, _>(channels.iter().map(|c| c.0).collect::<Vec<_>>())
    .bind::<diesel::sql_types::BigInt, _>(MAX_COUNTED_MENTIONS)
    .load(conn)
    .await?;
    Ok(rows.into_iter().map(ReadState::from).collect())
}

/// The read state of every channel of `visible`'s communities that its user may view.
pub async fn read_communities_read_states(
    state: &GlobalServerContext,
    visible: &crate::visibility::Visibility,
) -> crate::Result<Vec<ReadState>> {
    let mut conn = state.connection_pool.get().await?;
    read(conn.as_mut(), visible.user(), &visible.visible_channels()).await
}

/// The user's read state of each of `channels` they belong to.
pub async fn read_channels_read_states(
    state: &GlobalServerContext,
    user: UserId,
    channels: &[ChannelId],
) -> crate::Result<Vec<ReadState>> {
    let mut conn = state.connection_pool.get().await?;
    read(conn.as_mut(), user, channels).await
}

/// The user's read state of one channel; not found for a channel they do not belong to or may
/// not view, or a thread.
pub async fn read_read_state(
    state: &GlobalServerContext,
    user: UserId,
    channel: ChannelId,
) -> crate::Result<ReadState> {
    {
        let mut conn = state.connection_pool.get().await?;
        crate::permissions::channel_access(state, conn.as_mut(), user, channel).await?;
    }
    read_channels_read_states(state, user, &[channel])
        .await?
        .into_iter()
        .next()
        .ok_or(crate::Error::Diesel(diesel::result::Error::NotFound))
}

/// Moves the user's position in `channel` forward to `message`, publishing `channelRead` when
/// it moved, on `conn`, which is expected to be inside the caller's transaction. A position
/// already at or past `message` is left alone. Returns whether it moved.
pub async fn advance(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user: UserId,
    channel: ChannelId,
    message: MessageId,
) -> crate::Result<bool> {
    // Inserts the position, or moves an existing one, but only ever forward.
    let written = diesel::sql_query(
        r#"
        INSERT INTO read_state ("user", channel, message) VALUES ($1, $2, $3)
        ON CONFLICT ("user", channel) DO UPDATE SET message = excluded.message
        WHERE read_state.message < excluded.message
        "#,
    )
    .bind::<PgUuid, _>(user.0)
    .bind::<PgUuid, _>(channel.0)
    .bind::<PgUuid, _>(message.0)
    .execute(conn)
    .await?;
    if written == 0 {
        return Ok(false);
    }
    publish_event(
        state,
        conn,
        EventScope::User(user),
        &ServerEvent::ChannelRead {
            channel,
            last_read: message,
        },
    )
    .await?;
    Ok(true)
}

/// Records that the user has read `channel` up to `message`, a message of that channel,
/// deleted or not. A position already past it stays where it is.
pub async fn mark_read(
    state: &GlobalServerContext,
    user: UserId,
    channel_id: ChannelId,
    message_id: MessageId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    crate::permissions::channel_access(state, conn.as_mut(), user, channel_id).await?;
    let ty: ChannelType = channel::table
        .select(channel::ty)
        .filter(
            channel::id
                .eq(channel_id)
                .and(channel::deleted_at.is_null()),
        )
        .first(conn.as_mut())
        .await?;
    if ty == ChannelType::Thread {
        return Err(crate::Error::Validation(t!("readStateThread")));
    }
    let in_channel: bool = diesel::select(diesel::dsl::exists(
        message::table.filter(
            message::id
                .eq(message_id)
                .and(message::channel.eq(channel_id)),
        ),
    ))
    .get_result(conn.as_mut())
    .await?;
    if !in_channel {
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    }
    conn.transaction(|conn| {
        async move {
            advance(state, conn.as_mut(), user, channel_id, message_id).await?;
            Ok::<_, crate::Error>(())
        }
        .scope_boxed()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn a_join_is_read_before_everything_posted_after_it() {
        let joined = Utc.with_ymd_and_hms(2026, 9, 28, 12, 0, 0).unwrap();
        let position = position_at(joined);
        let later = uuid::Uuid::new_v7(uuid::Timestamp::from_unix(
            uuid::NoContext,
            u64::try_from(joined.timestamp()).unwrap(),
            1_000_000,
        ));
        let earlier = uuid::Uuid::new_v7(uuid::Timestamp::from_unix(
            uuid::NoContext,
            u64::try_from(joined.timestamp()).unwrap() - 1,
            0,
        ));
        assert!(later > position.0);
        assert!(earlier < position.0);
    }

    #[test]
    fn a_position_from_before_joining_counts_from_the_join() {
        let old = MessageId::new();
        let joined_at = Utc::now() + chrono::Duration::seconds(5);
        let state = ReadState::from(Row {
            channel: ChannelId::new(),
            last_read: Some(old),
            joined_at,
            last_message: None,
            mentions: 0,
        });
        assert_eq!(state.last_read, position_at(joined_at));
    }
}
