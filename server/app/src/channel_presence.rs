//! How many people are online in a channel: those whose status is online (`app::user_status`),
//! not away, among the members of its community who may view it, or among a DM's recipients,
//! and always the reader, who is using Aspen to read it whatever their own status says.
//!
//! A community channel's count starts from its community's set of members who may be online,
//! confirms each by their presence keys, and keeps those who may view the channel, so its cost
//! follows how many are online rather than how many belong. Each server works out who of a
//! community is online once for all its channels and readers ([`CommunityOnline`], kept in a
//! `Recent`), and again whenever a member's presence changes while a connection here views one
//! of its channels (`live`). A thread counts as its parent channel, whose viewers are its
//! viewers.
//!
//! Nobody counts anyone whose presence they may not learn (`app::user_status::presence_visible`):
//! a DM's count keeps only those, and a community channel's has the reader's blockers who are
//! online there taken off it, both from the same reading of who is online.

pub mod live;

use crate::context::GlobalServerContext;
use crate::events::{ChannelHome, channel_home, dm_recipients};
use crate::visibility::OnlineGroups;
use crate::{ChannelId, CommunityId, UserId};
use aspen_schema::{community_user, user_block};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use std::collections::HashSet;
use std::sync::Arc;

/// Who of a community is online, not away, and what they may view, as one server last worked
/// it out.
pub struct CommunityOnline {
    /// Those listed in the community whose keys say they are online or in do not disturb.
    online: HashSet<UserId>,
    /// `None` once the community is gone.
    groups: Option<OnlineGroups>,
}

impl CommunityOnline {
    /// How many of them may view `governing`, as `reader`, who may view it too, is told: with
    /// the reader whatever their status, and without `hidden` of them, whose presence the
    /// reader may not learn.
    fn count_for(&self, reader: UserId, governing: ChannelId, hidden: u32) -> u32 {
        let shared = self
            .groups
            .as_ref()
            .map_or(0, |groups| groups.viewers_of(governing));
        let reader_among = u32::from(self.online.contains(&reader));
        shared.saturating_sub(reader_among + hidden) + 1
    }

    /// Those of `users` it holds as online.
    fn online_of(&self, users: impl IntoIterator<Item = UserId>) -> HashSet<UserId> {
        users
            .into_iter()
            .filter(|user| self.online.contains(user))
            .collect()
    }
}

/// How many are online in `channel`, for `reader`, who may view it.
pub async fn online_in_channel(
    state: &GlobalServerContext,
    reader: UserId,
    channel: ChannelId,
) -> crate::Result<u32> {
    let mut conn = state.connection_pool.get().await?;
    crate::permissions::channel_access(state, conn.as_mut(), reader, channel).await?;
    match channel_home(state, conn.as_mut(), channel).await? {
        ChannelHome::Direct(dm) => {
            let recipients = dm_recipients(conn.as_mut(), dm).await?;
            let visible =
                crate::user_status::presence_visible(conn.as_mut(), reader, &recipients).await?;
            drop(conn);
            online_in_direct(state, reader, visible).await
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
                .filter(user_block::blocked.eq(reader))
                .load(conn.as_mut())
                .await?;
            drop(conn);
            let online = community_online(state, community).await?;
            let hidden = hidden(state, &online, community, governing, reader, blockers).await?;
            Ok(online.count_for(reader, governing, hidden))
        }
    }
}

/// How many of `visible`, a DM's recipients whose presence `reader` may learn, are online, with
/// the reader whatever their status.
async fn online_in_direct(
    state: &GlobalServerContext,
    reader: UserId,
    visible: HashSet<UserId>,
) -> crate::Result<u32> {
    let others = visible.into_iter().filter(|user| *user != reader).collect();
    let online = crate::user_status::online_among(state, others).await?;
    Ok(online.len() as u32 + 1)
}

/// Who of `community` is online: a recent reading this server made, or a new one.
async fn community_online(
    state: &GlobalServerContext,
    community: CommunityId,
) -> crate::Result<Arc<CommunityOnline>> {
    state
        .community_online
        .get_or_work(community, || read_community_online(state, community))
        .await
}

/// Who of `community` is online, read anew and kept for the requests that follow.
async fn renew_community_online(
    state: &GlobalServerContext,
    community: CommunityId,
) -> crate::Result<Arc<CommunityOnline>> {
    state
        .community_online
        .renew(community, || read_community_online(state, community))
        .await
}

/// From who of the community may be online, by the roles they hold, so that its channels cost
/// one read of its members and roles between them rather than one each.
async fn read_community_online(
    state: &GlobalServerContext,
    community: CommunityId,
) -> crate::Result<Arc<CommunityOnline>> {
    let candidates = crate::user_status::online_candidates(state, community).await?;
    let online = crate::user_status::online_among(state, candidates).await?;
    let mut conn = state.connection_pool.get().await?;
    let groups = crate::visibility::online_groups(conn.as_mut(), community, &online).await?;
    Ok(Arc::new(CommunityOnline { online, groups }))
}

/// How many of `blockers`, members of `community` who block `reader`, `online` holds as online
/// and may view `governing`.
async fn hidden(
    state: &GlobalServerContext,
    online: &CommunityOnline,
    community: CommunityId,
    governing: ChannelId,
    reader: UserId,
    blockers: Vec<UserId>,
) -> crate::Result<u32> {
    let mut blockers = online.online_of(blockers);
    blockers.remove(&reader);
    if blockers.is_empty() {
        return Ok(0);
    }
    let mut conn = state.connection_pool.get().await?;
    hidden_with(conn.as_mut(), community, governing, &blockers).await
}

/// How many of `blockers`, held as online, may view `governing`.
async fn hidden_with(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    governing: ChannelId,
    blockers: &HashSet<UserId>,
) -> crate::Result<u32> {
    Ok(
        crate::visibility::viewers(conn, community, blockers, governing)
            .await?
            .len() as u32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn user(n: u128) -> UserId {
        UserId(Uuid::from_u128(n))
    }

    fn nobody_online() -> CommunityOnline {
        CommunityOnline {
            online: HashSet::new(),
            groups: None,
        }
    }

    #[test]
    fn a_reader_counts_themself_whatever_their_status() {
        let channel = ChannelId(Uuid::from_u128(9));
        assert_eq!(nobody_online().count_for(user(1), channel, 0), 1);
    }

    #[test]
    fn a_reader_held_as_online_is_counted_once() {
        let channel = ChannelId(Uuid::from_u128(9));
        let online = CommunityOnline {
            online: HashSet::from([user(1)]),
            groups: None,
        };
        assert_eq!(online.count_for(user(1), channel, 0), 1);
        assert_eq!(
            online.online_of([user(1), user(2)]),
            HashSet::from([user(1)])
        );
    }
}
