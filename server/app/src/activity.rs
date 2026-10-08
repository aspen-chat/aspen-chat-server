//! The activity feed: every message that tells a user of itself, from everywhere they read on
//! this deployment, newest first. Apps signed in to several deployments merge each one's feed.
//!
//! A message tells its reader by the same rule as their notifications (`app::push`, and the
//! apps' own): in a DM, a community, or a channel, every message where their level is `all`
//! (a DM's by default), only those that tag them where it is `tags` (a community's by default),
//! and nothing where it is `nothing`, a thread counting as its parent; and every reply in a thread
//! they follow (`app::thread_follow`), whatever their level. Never in a channel they muted (a
//! thread's parent included), never by them or anyone they blocked, never from before they
//! joined, and never a message that only records something (an echo, a poll's result, a call, a
//! command). What is read is decided as they may read it now, as search decides it.

use crate::channel::ChannelType;
use crate::context::GlobalServerContext;
use crate::message::{Message, MessageKind, MessageWithRelations};
use crate::notification_setting::{NotificationLevel, default_level};
use crate::visibility::Visibility;
use crate::{ChannelId, CommunityId, MessageId, RoleId, UserId};
use aspen_schema::{
    channel, community_member_role, community_user, dm_recipient, mention, message, read_state,
    thread_follow, user_block,
};
use diesel::dsl::{exists, not};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use std::collections::{HashMap, HashSet};

/// The most messages one page of the feed holds.
pub const MAX_PAGE: u32 = 50;

/// Which part of the feed to read.
#[derive(Debug, Clone, Default)]
pub struct ActivityQuery {
    /// Only these communities' messages, or every community's when `None`.
    pub communities: Option<Vec<CommunityId>>,
    /// Whether DMs' messages are read.
    pub dms: bool,
    /// Only messages the reader has not read.
    pub unread: bool,
    /// Only messages older than this one: the last of the previous page.
    pub before: Option<MessageId>,
    pub limit: u32,
}

/// Where messages tell the user of themselves, decided from their settings, mutes, follows, and
/// what they may read.
#[derive(Debug, Default, PartialEq, Eq)]
struct Places {
    /// Every message here tells them, and in these channels' threads.
    all: Vec<ChannelId>,
    /// Messages tagging them here tell them, and in these channels' threads.
    tags: Vec<ChannelId>,
    /// Threads every reply in which tells them.
    followed: Vec<ChannelId>,
}

/// One channel or DM the user may read, and their level and mute there.
struct Place {
    channel: ChannelId,
    level: NotificationLevel,
    muted: bool,
}

fn sort_places(places: &[Place], followed: &[(ChannelId, ChannelId)]) -> Places {
    let mut sorted = Places::default();
    let mut unmuted = HashSet::new();
    for place in places.iter().filter(|p| !p.muted) {
        unmuted.insert(place.channel);
        match place.level {
            NotificationLevel::All => sorted.all.push(place.channel),
            NotificationLevel::Tags => sorted.tags.push(place.channel),
            NotificationLevel::Nothing => {}
        }
    }
    sorted.followed = followed
        .iter()
        .filter(|(_, parent)| unmuted.contains(parent))
        .map(|(thread, _)| *thread)
        .collect();
    sorted
}

/// The messages of the user's feed that `query` asks for, newest first.
pub async fn read_activity(
    state: &GlobalServerContext,
    user: UserId,
    query: ActivityQuery,
) -> crate::Result<Vec<MessageWithRelations>> {
    let mut conn = state.connection_pool.get().await?;
    let mut communities = crate::events::memberships(conn.as_mut(), user).await?;
    if let Some(asked) = &query.communities {
        communities.retain(|c| asked.contains(c));
    }
    let dms: Vec<ChannelId> = if query.dms {
        dm_recipient::table
            .select(dm_recipient::channel)
            .filter(dm_recipient::user.eq(user))
            .load(conn.as_mut())
            .await?
    } else {
        Vec::new()
    };
    let roles: Vec<RoleId> = community_member_role::table
        .select(community_member_role::role)
        .filter(community_member_role::user.eq(user))
        .filter(community_member_role::community.eq_any(&communities))
        .load(conn.as_mut())
        .await?;
    drop(conn);
    let visible = Visibility::load(state, user, &communities).await?;
    let (community_settings, dm_settings, community_mutes, dm_mutes) = tokio::try_join!(
        crate::notification_setting::read_community_settings(state, &visible),
        crate::notification_setting::read_channel_settings(state, user, &dms),
        crate::channel_mute::read_community_mutes(state, &visible),
        crate::channel_mute::read_channel_mutes(state, user, &dms),
    )?;
    let mut conn = state.connection_pool.get().await?;
    let channels = visible.visible_channels();
    let homes: HashMap<ChannelId, Option<CommunityId>> = channel::table
        .select((channel::id, channel::community))
        .filter(channel::id.eq_any(&channels))
        .load::<(ChannelId, Option<CommunityId>)>(conn.as_mut())
        .await?
        .into_iter()
        .collect();
    let mut for_channel = HashMap::new();
    let mut for_community = HashMap::new();
    for setting in community_settings.into_iter().chain(dm_settings) {
        match (setting.channel, setting.community) {
            (Some(channel), _) => for_channel.insert(channel, setting.level),
            (None, Some(community)) => for_community.insert(community, setting.level),
            (None, None) => None,
        };
    }
    let muted: HashSet<ChannelId> = community_mutes
        .iter()
        .chain(&dm_mutes)
        .map(|m| m.channel)
        .collect();
    let places: Vec<Place> = channels
        .iter()
        .map(|channel| (*channel, ChannelType::Text))
        .chain(dms.iter().map(|dm| (*dm, ChannelType::Dm)))
        .map(|(channel, ty)| Place {
            channel,
            level: for_channel
                .get(&channel)
                .or_else(|| {
                    homes
                        .get(&channel)
                        .copied()
                        .flatten()
                        .and_then(|community| for_community.get(&community))
                })
                .copied()
                .unwrap_or_else(|| default_level(ty)),
            muted: muted.contains(&channel),
        })
        .collect();
    let readable: Vec<ChannelId> = places.iter().map(|p| p.channel).collect();
    let followed: Vec<(ChannelId, ChannelId)> = thread_follow::table
        .inner_join(channel::table.on(channel::id.eq(thread_follow::thread)))
        .select((
            thread_follow::thread,
            channel::parent_channel.assume_not_null(),
        ))
        .filter(thread_follow::user.eq(user))
        .filter(channel::deleted_at.is_null())
        .filter(channel::parent_channel.eq_any(&readable))
        .load(conn.as_mut())
        .await?;
    let places = sort_places(&places, &followed);
    if places == Places::default() {
        return Ok(Vec::new());
    }
    let tagged = exists(
        mention::table.filter(
            mention::message.eq(message::id).and(
                mention::target_user
                    .eq(user)
                    .or(mention::everyone)
                    .or(mention::target_role.assume_not_null().eq_any(roles)),
            ),
        ),
    );
    let mut messages = message::table
        .inner_join(channel::table.on(channel::id.eq(message::channel)))
        .select(Message::as_select())
        .filter(message::deleted_at.is_null())
        .filter(channel::deleted_at.is_null())
        .filter(message::author.ne(user))
        .filter(
            message::channel
                .eq_any(places.all.clone())
                .or(channel::parent_channel.eq_any(places.all))
                .or(message::channel.eq_any(places.followed))
                .or(message::channel
                    .eq_any(places.tags.clone())
                    .or(channel::parent_channel.eq_any(places.tags))
                    .and(tagged)),
        )
        // What only records something says nothing of its own, as it wakes no phone.
        .filter(message::kind.ne_all([
            MessageKind::ThreadEcho,
            MessageKind::PollClosed,
            MessageKind::Call,
            MessageKind::MissedCall,
            MessageKind::Command,
        ]))
        .filter(not(exists(
            user_block::table.filter(
                user_block::blocker
                    .eq(user)
                    .and(user_block::blocked.eq(message::author)),
            ),
        )))
        // Nothing from before they joined the community, or the DM a thread's parent is.
        .filter(
            exists(
                community_user::table.filter(
                    community_user::community
                        .nullable()
                        .eq(channel::community)
                        .and(community_user::user.eq(user))
                        .and(community_user::joined_at.lt(message::timestamp)),
                ),
            )
            .or(exists(
                dm_recipient::table.filter(
                    dm_recipient::user
                        .eq(user)
                        .and(dm_recipient::joined_at.lt(message::timestamp))
                        .and(
                            dm_recipient::channel
                                .eq(message::channel)
                                .or(dm_recipient::channel.nullable().eq(channel::parent_channel)),
                        ),
                ),
            )),
        )
        .into_boxed();
    if query.unread {
        messages = messages.filter(not(exists(
            read_state::table.filter(
                read_state::user
                    .eq(user)
                    .and(read_state::channel.eq(message::channel))
                    .and(read_state::message.ge(message::id)),
            ),
        )));
    }
    if let Some(before) = query.before {
        messages = messages.filter(message::id.lt(before));
    }
    let messages: Vec<Message> = messages
        .order_by(message::id.desc())
        .limit(i64::from(query.limit.clamp(1, MAX_PAGE)))
        .load(conn.as_mut())
        .await?;
    crate::message::with_relations(state, conn.as_mut(), messages).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(level: NotificationLevel, muted: bool) -> Place {
        Place {
            channel: ChannelId::new(),
            level,
            muted,
        }
    }

    #[test]
    fn levels_sort_places_and_mutes_silence_them_and_their_threads() {
        let all = place(NotificationLevel::All, false);
        let tags = place(NotificationLevel::Tags, false);
        let nothing = place(NotificationLevel::Nothing, false);
        let muted = place(NotificationLevel::All, true);
        let (in_all, in_nothing, in_muted) = (ChannelId::new(), ChannelId::new(), ChannelId::new());
        let followed = [
            (in_all, all.channel),
            (in_nothing, nothing.channel),
            (in_muted, muted.channel),
        ];
        let expected = Places {
            all: vec![all.channel],
            tags: vec![tags.channel],
            // Following outranks a level of nothing; a mute outranks following.
            followed: vec![in_all, in_nothing],
        };
        assert_eq!(
            sort_places(&[all, tags, nothing, muted], &followed),
            expected
        );
    }
}
