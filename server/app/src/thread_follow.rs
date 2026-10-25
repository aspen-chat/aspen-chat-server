//! Threads a user follows, each of whose replies tells them (`app::push`, the apps'
//! notifications, and the activity feed, `app::activity`) whatever their notification level for
//! the parent, though a mute of the parent still silences it. A user follows a thread by hand,
//! or by taking part: when a message of theirs starts it, when they post in it, and when a reply
//! tags them by name. Bots follow only by hand. Unfollowing ends it until they follow again or take part again.
//!
//! A user follows at most [`MAX_THREAD_FOLLOWS`] threads on a deployment; past that, those they
//! followed least recently are let go, though a follow made by taking part only ever lets go
//! another made by taking part, and is not made when every follow they have was made by hand.
//! A reply that tags someone who has blocked its author does not make them follow. Following a thread they may
//! no longer read changes nothing for them: whatever it would tell them is decided by whether
//! they may read it now. Every change is published to the user's own subject as
//! `threadFollowChanged`, so their other devices follow.

use crate::channel::{Channel, ChannelType};
use crate::context::GlobalServerContext;
use crate::mention::Mentions;
use crate::t;
use crate::{ChannelId, EventScope, UserId, publish_event};
use aspen_schema::{channel, thread_follow, user, user_block};
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::collections::HashSet;

/// The most threads one user follows on one deployment.
pub const MAX_THREAD_FOLLOWS: i64 = 1000;

/// A thread the user follows, and when they last followed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = thread_follow)]
pub struct ThreadFollow {
    pub thread: ChannelId,
    pub followed_at: DateTime<Utc>,
}

/// Follows `thread` for the user, by hand. Returns the follow and whether they followed it
/// already.
pub async fn follow(
    state: &GlobalServerContext,
    user: UserId,
    thread: ChannelId,
) -> crate::Result<(ThreadFollow, bool)> {
    let mut conn = state.connection_pool.get().await?;
    crate::permissions::channel_access(state, conn.as_mut(), user, thread).await?;
    let ty: ChannelType = channel::table
        .select(channel::ty)
        .filter(channel::id.eq(thread))
        .first(conn.as_mut())
        .await?;
    if ty != ChannelType::Thread {
        return Err(crate::Error::Validation(t!("threadFollowKind")));
    }
    conn.transaction(|conn| {
        async move {
            let started = take_part(state, conn.as_mut(), &[user], thread, How::ByHand).await?;
            Ok::<_, crate::Error>((
                ThreadFollow {
                    thread,
                    followed_at: started.at,
                },
                started.users.is_empty(),
            ))
        }
        .scope_boxed()
    })
    .await
}

/// Stops following `thread` for the user, if they follow it.
pub async fn unfollow(
    state: &GlobalServerContext,
    user: UserId,
    thread: ChannelId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let removed = diesel::delete(
                thread_follow::table
                    .filter(thread_follow::user.eq(user))
                    .filter(thread_follow::thread.eq(thread)),
            )
            .execute(conn.as_mut())
            .await?;
            if removed > 0 {
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::User(user),
                    &ServerEvent::ThreadFollowChanged {
                        thread,
                        following: false,
                    },
                )
                .await?;
            }
            Ok::<_, crate::Error>(())
        }
        .scope_boxed()
    })
    .await
}

/// Who began following in [`take_part`], and when.
pub struct Followed {
    pub users: Vec<UserId>,
    pub at: DateTime<Utc>,
}

/// How a follow is made: by hand, or by taking part in the thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum How {
    ByHand,
    TakingPart,
}

/// Has each of `users` follow `thread`, on `conn`, which is expected to be inside the caller's
/// transaction (the one posting the reply that makes them take part, or following by hand), and
/// marks it followed now for those who followed it already, so the threads they take part in are
/// the last let go; a follow by hand marks it made by hand. Someone at
/// [`MAX_THREAD_FOLLOWS`] makes room ([`make_room`]): by hand, by letting go the follow they
/// made least recently; by taking part, only a follow also made by taking part, so with none
/// but follows by hand they do not begin following. Returns those who began following.
pub async fn take_part(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    users: &[UserId],
    thread: ChannelId,
    how: How,
) -> crate::Result<Followed> {
    let at = Utc::now();
    if users.is_empty() {
        return Ok(Followed {
            users: Vec::new(),
            at,
        });
    }
    let already: Vec<UserId> = diesel::update(
        thread_follow::table
            .filter(thread_follow::thread.eq(thread))
            .filter(thread_follow::user.eq_any(users)),
    )
    .set((
        thread_follow::followed_at.eq(at),
        thread_follow::by_hand
            .eq(thread_follow::by_hand
                .or((how == How::ByHand).into_sql::<diesel::sql_types::Bool>())),
    ))
    .returning(thread_follow::user)
    .get_results(conn)
    .await?;
    let already: HashSet<UserId> = already.into_iter().collect();
    let mut began = Vec::new();
    for user in users.iter().copied().filter(|user| !already.contains(user)) {
        let Some(let_go) = make_room(conn, user, how).await? else {
            continue;
        };
        let made = diesel::insert_into(thread_follow::table)
            .values((
                thread_follow::user.eq(user),
                thread_follow::thread.eq(thread),
                thread_follow::followed_at.eq(at),
                thread_follow::by_hand.eq(how == How::ByHand),
            ))
            .on_conflict_do_nothing()
            .execute(conn)
            .await?;
        if made == 1 {
            began.push(user);
            publish_event(
                state,
                conn,
                EventScope::User(user),
                &ServerEvent::ThreadFollowChanged {
                    thread,
                    following: true,
                },
            )
            .await?;
        }
        for let_go in let_go {
            publish_event(
                state,
                conn,
                EventScope::User(user),
                &ServerEvent::ThreadFollowChanged {
                    thread: let_go,
                    following: false,
                },
            )
            .await?;
        }
    }
    Ok(Followed { users: began, at })
}

/// Lets go what `user` follows past [`MAX_THREAD_FOLLOWS`] less one, the least recently
/// followed first, so one more follow made `how` fits: by hand, any follow
/// (`thread_follow_by_age`); by taking part, only those also made by taking part
/// (`thread_follow_automatic_by_age`). Answers the threads let go, or `None` when there is no
/// room to make.
async fn make_room(
    conn: &mut AsyncPgConnection,
    user: UserId,
    how: How,
) -> crate::Result<Option<Vec<ChannelId>>> {
    let held: i64 = diesel::sql_query(
        "SELECT count(*) AS count FROM (SELECT 1 FROM thread_follow WHERE \"user\" = $1 \
         LIMIT $2) follows",
    )
    .bind::<diesel::sql_types::Uuid, _>(user)
    .bind::<diesel::sql_types::BigInt, _>(MAX_THREAD_FOLLOWS)
    .get_result::<Count>(conn)
    .await?
    .count;
    if held < MAX_THREAD_FOLLOWS {
        return Ok(Some(Vec::new()));
    }
    let mut oldest = thread_follow::table
        .select(thread_follow::thread)
        .filter(thread_follow::user.eq(user))
        .order_by(thread_follow::followed_at.asc())
        .limit(1)
        .into_boxed();
    if how == How::TakingPart {
        oldest = oldest.filter(thread_follow::by_hand.eq(false));
    }
    let Some(let_go) = oldest.first::<ChannelId>(conn).await.optional()? else {
        return Ok(None);
    };
    diesel::delete(
        thread_follow::table
            .filter(thread_follow::user.eq(user))
            .filter(thread_follow::thread.eq(let_go)),
    )
    .execute(conn)
    .await?;
    Ok(Some(vec![let_go]))
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

/// The threads the user follows that they may read now.
pub async fn read_follows(
    state: &GlobalServerContext,
    user: UserId,
) -> crate::Result<Vec<ThreadFollow>> {
    let readable = crate::search::readable_everywhere(state, user).await?;
    let mut conn = state.connection_pool.get().await?;
    Ok(thread_follow::table
        .inner_join(channel::table.on(channel::id.eq(thread_follow::thread)))
        .select(ThreadFollow::as_select())
        .filter(thread_follow::user.eq(user))
        .filter(channel::deleted_at.is_null())
        .filter(channel::parent_channel.eq_any(readable))
        .order_by(thread_follow::followed_at.desc())
        .load(conn.as_mut())
        .await?)
}

/// Has those a new reply in `thread` makes take part follow it: its author, and those it tags by
/// name who may read it and have not blocked its author, bots aside. On `conn`, inside the
/// transaction posting the reply.
pub async fn reply_posted(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    thread: &Channel,
    author: UserId,
    mentions: &Mentions,
) -> crate::Result<()> {
    let mut tagged: HashSet<UserId> = mentions.users.iter().copied().collect();
    tagged.remove(&author);
    // A DM's tags name only its people; a community's may name members who cannot see here.
    if let (Some(community), Some(parent)) = (
        thread.community.as_ref().map(|c| *c.id()),
        thread.parent_channel,
    ) && !tagged.is_empty()
    {
        tagged = crate::visibility::viewers(conn, community, &tagged, parent).await?;
    }
    if !tagged.is_empty() {
        let tagged_list: Vec<UserId> = tagged.iter().copied().collect();
        let blocking: Vec<UserId> = user_block::table
            .select(user_block::blocker)
            .filter(user_block::blocked.eq(author))
            .filter(user_block::blocker.eq_any(&tagged_list))
            .load(conn)
            .await?;
        for blocker in blocking {
            tagged.remove(&blocker);
        }
    }
    let users: Vec<UserId> = std::iter::once(author).chain(tagged).collect();
    took_part(state, conn, &users, thread.id).await
}

/// Has the people among `users` follow `thread` for taking part in it, as [`take_part`] does:
/// bots, which answer what they are sent rather than what they took part in, the system
/// account, and deleted accounts are passed over.
pub async fn took_part(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    users: &[UserId],
    thread: ChannelId,
) -> crate::Result<()> {
    let people: Vec<UserId> = user::table
        .select(user::id)
        .filter(user::id.eq_any(users))
        .filter(user::bot.eq(false))
        .filter(user::system.eq(false))
        .filter(user::deleted_at.is_null())
        .load(conn)
        .await?;
    take_part(state, conn, &people, thread, How::TakingPart).await?;
    Ok(())
}
