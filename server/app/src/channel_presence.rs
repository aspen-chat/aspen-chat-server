//! How many people are online in a channel: those whose status is online (`app::user_status`),
//! not away, among the members of its community who may view it, or among a DM's recipients.
//!
//! A community channel's count starts from its community's set of members who may be online,
//! confirms each by their presence keys, and keeps those who may view the channel, so its cost
//! follows how many are online rather than how many belong. Everyone who may view a channel
//! sees the same count, so each server reuses a recent one (`app::recent`). A thread counts as
//! its parent channel, whose viewers are its viewers.
//!
//! Nobody counts anyone whose presence they may not learn (`app::user_status::presence_visible`):
//! a DM's count keeps only those, and a community channel's shared count has the caller's
//! blockers who are online there now taken off it. The shared count may be up to ten seconds
//! old while that correction is current, so for those seconds after a blocker comes online or
//! leaves, the count the blocked caller reads is one off, and then right again.

use crate::context::GlobalServerContext;
use crate::events::{ChannelHome, channel_home};
use crate::{ChannelId, CommunityId, UserId};
use aspen_schema::{community_user, dm_recipient, user_block};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use std::collections::HashSet;
use std::sync::Arc;

/// How many people are online in `channel`, for someone who may view it.
pub async fn online_in_channel(
    state: &GlobalServerContext,
    caller: UserId,
    channel: ChannelId,
) -> crate::Result<u32> {
    let mut conn = state.connection_pool.get().await?;
    crate::permissions::channel_access(state, conn.as_mut(), caller, channel).await?;
    match channel_home(state, conn.as_mut(), channel).await? {
        ChannelHome::Direct(dm) => {
            let recipients: Vec<UserId> = dm_recipient::table
                .select(dm_recipient::user)
                .filter(dm_recipient::channel.eq(dm))
                .load(conn.as_mut())
                .await?;
            let visible =
                crate::user_status::presence_visible(conn.as_mut(), caller, &recipients).await?;
            drop(conn);
            Ok(
                crate::user_status::online_among(state, visible.into_iter().collect())
                    .await?
                    .len() as u32,
            )
        }
        ChannelHome::Community {
            community,
            governing,
        } => {
            let blockers: Vec<UserId> = user_block::table
                .inner_join(
                    community_user::table.on(community_user::user
                        .eq(user_block::blocker)
                        .and(community_user::community.eq(community))),
                )
                .select(user_block::blocker)
                .filter(user_block::blocked.eq(caller))
                .load(conn.as_mut())
                .await?;
            drop(conn);
            let shared = state
                .channel_presence
                .get_or_work(governing, || count_viewers(state, community, governing))
                .await?;
            if blockers.is_empty() {
                return Ok(shared);
            }
            let hidden = online_viewers(state, community, governing, blockers).await?;
            Ok(shared.saturating_sub(hidden.len() as u32))
        }
    }
}

/// How many members of `community` are online and may view `channel`: from who of it is online,
/// by the roles they hold, worked out once per community for all its channels' counts
/// (`community_online`, a `Recent`), so a community's channels cost one read of its members
/// and roles between them rather than one each.
async fn count_viewers(
    state: &GlobalServerContext,
    community: CommunityId,
    channel: ChannelId,
) -> crate::Result<u32> {
    let groups = state
        .community_online
        .get_or_work(community, || async {
            let candidates = crate::user_status::online_candidates(state, community).await?;
            let online = crate::user_status::online_among(state, candidates).await?;
            let mut conn = state.connection_pool.get().await?;
            Ok(
                crate::visibility::online_groups(conn.as_mut(), community, &online)
                    .await?
                    .map(Arc::new),
            )
        })
        .await?;
    Ok(groups.map_or(0, |groups| groups.viewers_of(channel)))
}

/// Those of `candidates`, members of `community`, who are online and may view `channel`.
async fn online_viewers(
    state: &GlobalServerContext,
    community: CommunityId,
    channel: ChannelId,
    candidates: Vec<UserId>,
) -> crate::Result<HashSet<UserId>> {
    let online = crate::user_status::online_among(state, candidates).await?;
    if online.is_empty() {
        return Ok(online);
    }
    let mut conn = state.connection_pool.get().await?;
    crate::visibility::viewers(conn.as_mut(), community, &online, channel).await
}
