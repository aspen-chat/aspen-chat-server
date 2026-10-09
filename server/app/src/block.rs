//! Users blocking one another. A block is its blocker's alone: the blocked user is never told,
//! and each change is published to the blocker's own subject as `userBlockChanged`, so their
//! other devices follow.
//!
//! What a block does is mostly the blocker's client's to do: it collapses the blocked user's
//! messages, leaves their reactions out, and silences and hides them in calls. The server
//! enforces what the blocker's client cannot:
//! - No one-to-one DM is started or written in while a block stands either way. Its history
//!   stays readable. `app::permissions::channel_access` leaves both people only View channel
//!   there, and refuses with `Blocked`.
//! - No group DM is started with, or joined by, two people with a block between them. A group
//!   DM that already holds both stays open to both.
//! - Their messages never make a channel unread for the blocker (`app::read_state`).
//! - Their reactions are left out of the blocker's reaction summaries and lists
//!   (`app::react`).

use crate::context::GlobalServerContext;
use crate::t;
use crate::{EventScope, UserId, publish_event};
use aspen_schema::{user, user_block};
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

/// The most people one person may block.
pub const MAX_BLOCKS: i64 = 10_000;

/// Someone the user has blocked, and since when.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = user_block)]
pub struct UserBlock {
    pub blocked: UserId,
    pub created_at: DateTime<Utc>,
}

/// Blocks `blocked` for `blocker`. Returns the block and whether it stood already.
pub async fn block(
    state: &GlobalServerContext,
    blocker: UserId,
    blocked: UserId,
) -> crate::Result<(UserBlock, bool)> {
    if blocker == blocked {
        return Err(crate::Error::Validation(t!("blockSelf")));
    }
    let mut conn = state.connection_pool.get().await?;
    // Not found for an account that does not exist or has been deleted.
    let system: bool = user::table
        .select(user::system)
        .filter(user::id.eq(blocked).and(user::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;
    // The system account's notices are the deployment's to send; muting its DM quiets them.
    if system {
        return Err(crate::Error::Validation(t!("systemAccountNoBlock")));
    }
    conn.transaction(|conn| {
        async move {
            // The blocker's row is held so two blocks at once cannot both pass the count.
            user::table
                .select(user::id)
                .filter(user::id.eq(blocker))
                .for_no_key_update()
                .first::<UserId>(conn.as_mut())
                .await?;
            let inserted: Option<UserBlock> = diesel::insert_into(user_block::table)
                .values((
                    user_block::blocker.eq(blocker),
                    user_block::blocked.eq(blocked),
                ))
                .on_conflict_do_nothing()
                .returning(UserBlock::as_returning())
                .get_result(conn.as_mut())
                .await
                .optional()?;
            let Some(block) = inserted else {
                let existing = user_block::table
                    .select(UserBlock::as_select())
                    .filter(
                        user_block::blocker
                            .eq(blocker)
                            .and(user_block::blocked.eq(blocked)),
                    )
                    .first(conn.as_mut())
                    .await?;
                return Ok((existing, true));
            };
            let held: i64 = user_block::table
                .filter(user_block::blocker.eq(blocker))
                .count()
                .get_result(conn.as_mut())
                .await?;
            if held > MAX_BLOCKS {
                return Err(crate::Error::Validation(t!("blockLimit", max = MAX_BLOCKS)));
            }
            publish_event(
                state,
                conn.as_mut(),
                EventScope::User(blocker),
                &ServerEvent::UserBlockChanged {
                    user: blocked,
                    blocked: true,
                },
            )
            .await?;
            Ok::<_, crate::Error>((block, false))
        }
        .scope_boxed()
    })
    .await
    .inspect(|_| told_of_block(state, blocker))
}

/// Tells those watching `blocker`'s presence of what a block made or lifted changes of it:
/// whoever it is between now learns it as offline, or learns it again (`app::presence_feed`).
fn told_of_block(state: &GlobalServerContext, blocker: UserId) {
    state.presence_feed.changed(blocker);
}

/// Lifts `blocker`'s block of `blocked`, if there is one.
pub async fn unblock(
    state: &GlobalServerContext,
    blocker: UserId,
    blocked: UserId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let removed = diesel::delete(
                user_block::table.filter(
                    user_block::blocker
                        .eq(blocker)
                        .and(user_block::blocked.eq(blocked)),
                ),
            )
            .execute(conn.as_mut())
            .await?;
            if removed > 0 {
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::User(blocker),
                    &ServerEvent::UserBlockChanged {
                        user: blocked,
                        blocked: false,
                    },
                )
                .await?;
            }
            Ok::<_, crate::Error>(())
        }
        .scope_boxed()
    })
    .await
    .inspect(|()| told_of_block(state, blocker))
}

/// Everyone the user has blocked, the most recent first.
pub async fn read_blocks(
    state: &GlobalServerContext,
    blocker: UserId,
) -> crate::Result<Vec<UserBlock>> {
    let mut conn = state.connection_pool.get().await?;
    Ok(user_block::table
        .select(UserBlock::as_select())
        .filter(user_block::blocker.eq(blocker))
        .order_by((user_block::created_at.desc(), user_block::blocked))
        .load(conn.as_mut())
        .await?)
}

/// Whether a block stands between any two of `users`, whichever of them made it.
pub async fn any_between(conn: &mut AsyncPgConnection, users: &[UserId]) -> crate::Result<bool> {
    if users.len() < 2 {
        return Ok(false);
    }
    Ok(diesel::select(diesel::dsl::exists(
        user_block::table.filter(
            user_block::blocker
                .eq_any(users)
                .and(user_block::blocked.eq_any(users)),
        ),
    ))
    .get_result(conn)
    .await?)
}
