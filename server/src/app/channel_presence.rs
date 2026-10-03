//! How many people are online in a channel: those whose status is online (`app::user_status`),
//! not away, among the members of its community who may view it, or among a DM's recipients.
//!
//! A community channel's count starts from its community's set of members who may be online,
//! confirms each by their presence keys, and keeps those who may view the channel, so its cost
//! follows how many are online rather than how many belong. Everyone who may view a channel
//! sees the same count, so each server works it out at most once per `FRESH_FOR` per channel,
//! and requests that arrive while it is being worked out wait for that one answer. A thread
//! counts as its parent channel, whose viewers are its viewers.

use crate::api::user::UserOnlineStatus;
use crate::app::context::GlobalServerContext;
use crate::app::events::{ChannelHome, channel_home};
use crate::app::{self, ChannelId, CommunityId, UserId};
use crate::database::schema::dm_recipient;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use lru::LruCache;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

/// How long one channel's count is reused before it is worked out again.
const FRESH_FOR: Duration = Duration::from_secs(10);
/// How many channels' counts one server keeps.
const REMEMBERED: NonZeroUsize = NonZeroUsize::new(10_000).unwrap();
/// How many people's presence keys one read asks for.
const STATUS_BATCH: usize = 1_000;

/// One channel's count: when it was started, and the count once worked out.
type Count = (Instant, Arc<OnceCell<u32>>);

/// Each channel's latest count, shared by every request on this server.
pub struct PresenceCounts {
    counts: Mutex<LruCache<ChannelId, Count>>,
}

impl Default for PresenceCounts {
    fn default() -> Self {
        Self {
            counts: Mutex::new(LruCache::new(REMEMBERED)),
        }
    }
}

impl PresenceCounts {
    /// The count for `channel`: one started within `FRESH_FOR`, or else a new one from `count`.
    /// A count that fails is not kept, so the next request tries again.
    async fn get_or_count<F, Fut>(&self, channel: ChannelId, count: F) -> app::Result<u32>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = app::Result<u32>>,
    {
        let cell = {
            let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
            match counts.get(&channel) {
                Some((started, cell)) if started.elapsed() < FRESH_FOR => cell.clone(),
                _ => {
                    let cell = Arc::new(OnceCell::new());
                    counts.put(channel, (Instant::now(), cell.clone()));
                    cell
                }
            }
        };
        cell.get_or_try_init(count).await.copied()
    }
}

/// How many people are online in `channel`, for someone who may view it.
pub async fn online_in_channel(
    state: &GlobalServerContext,
    caller: UserId,
    channel: ChannelId,
) -> app::Result<u32> {
    let mut conn = state.connection_pool.get().await?;
    app::permissions::channel_access(state, conn.as_mut(), caller, channel).await?;
    match channel_home(state, conn.as_mut(), channel).await? {
        ChannelHome::Direct(dm) => {
            let recipients: Vec<UserId> = dm_recipient::table
                .select(dm_recipient::user)
                .filter(dm_recipient::channel.eq(dm))
                .load(conn.as_mut())
                .await?;
            drop(conn);
            Ok(online_among(state, recipients).await?.len() as u32)
        }
        ChannelHome::Community {
            community,
            governing,
        } => {
            drop(conn);
            state
                .channel_presence
                .get_or_count(governing, || count_viewers(state, community, governing))
                .await
        }
    }
}

/// How many members of `community` are online and may view `channel`.
async fn count_viewers(
    state: &GlobalServerContext,
    community: CommunityId,
    channel: ChannelId,
) -> app::Result<u32> {
    let candidates = app::user_status::online_candidates(state, community).await?;
    let online = online_among(state, candidates).await?;
    if online.is_empty() {
        return Ok(0);
    }
    let mut conn = state.connection_pool.get().await?;
    let viewers = app::visibility::viewers(conn.as_mut(), community, &online, channel).await?;
    Ok(viewers.len() as u32)
}

/// Which of `users` are online, by their presence keys, a batch at a time.
async fn online_among(
    state: &GlobalServerContext,
    users: Vec<UserId>,
) -> app::Result<HashSet<UserId>> {
    let mut online = HashSet::new();
    for batch in users.chunks(STATUS_BATCH) {
        online.extend(
            app::user_status::users_online_status(state, batch.to_vec())
                .await?
                .into_iter()
                .filter(|(_, status)| matches!(status, UserOnlineStatus::Online))
                .map(|(user, _)| user),
        );
    }
    Ok(online)
}
