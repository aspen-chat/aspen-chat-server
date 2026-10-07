//! How many people are online in a channel: those whose status is online (`app::user_status`),
//! not away, among the members of its community who may view it, or among a DM's recipients.
//!
//! A community channel's count starts from its community's set of members who may be online,
//! confirms each by their presence keys, and keeps those who may view the channel, so its cost
//! follows how many are online rather than how many belong. Everyone who may view a channel
//! sees the same count, so each server reuses a recent one (`app::recent`). A thread counts as
//! its parent channel, whose viewers are its viewers.

use crate::context::GlobalServerContext;
use crate::events::{ChannelHome, channel_home};
use crate::{ChannelId, CommunityId, UserId};
use aspen_schema::dm_recipient;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;

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
            drop(conn);
            Ok(crate::user_status::online_among(state, recipients)
                .await?
                .len() as u32)
        }
        ChannelHome::Community {
            community,
            governing,
        } => {
            drop(conn);
            state
                .channel_presence
                .get_or_work(governing, || count_viewers(state, community, governing))
                .await
        }
    }
}

/// How many members of `community` are online and may view `channel`.
async fn count_viewers(
    state: &GlobalServerContext,
    community: CommunityId,
    channel: ChannelId,
) -> crate::Result<u32> {
    let candidates = crate::user_status::online_candidates(state, community).await?;
    let online = crate::user_status::online_among(state, candidates).await?;
    if online.is_empty() {
        return Ok(0);
    }
    let mut conn = state.connection_pool.get().await?;
    let viewers = crate::visibility::viewers(conn.as_mut(), community, &online, channel).await?;
    Ok(viewers.len() as u32)
}
