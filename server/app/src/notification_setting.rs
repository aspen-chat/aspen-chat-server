//! What each user wants to be told of, for themself alone: for a whole community, or one of its
//! text channels, or a DM, every message (`all`), only messages that tag them (`tags`), or
//! nothing. A channel's own setting outranks its community's, and without either a DM tells of
//! every message and a community channel of tags ([`default_level`]); a thread follows its
//! parent. Phones are woken by these (`app::push`), and the apps notify by them. Every change is
//! published to the user's own subject as `notificationSettingChanged`, so their other devices
//! follow.

use crate::channel::ChannelType;
use crate::context::GlobalServerContext;
use crate::t;
use crate::{ChannelId, CommunityId, EventScope, UserId, publish_event};
use aspen_schema::{channel, community_user, notification_setting};
use aspen_wire::message_enum::server_event::ServerEvent;
pub use aspen_wire::notification_setting::NotificationLevel;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use uuid::Uuid;

/// What a setting is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationTarget {
    Community(CommunityId),
    Channel(ChannelId),
}

/// A user's setting for a community or a channel.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = notification_setting)]
pub struct NotificationSetting {
    pub community: Option<CommunityId>,
    pub channel: Option<ChannelId>,
    pub level: NotificationLevel,
}

/// The level of a channel neither the user nor its community has a setting for: every message
/// of a DM, which is written to its few people, and tags in a community, which may be large.
pub fn default_level(ty: ChannelType) -> NotificationLevel {
    match ty {
        ChannelType::Dm | ChannelType::GroupDm => NotificationLevel::All,
        _ => NotificationLevel::Tags,
    }
}

/// Sets the user's level for `target`, replacing any setting already there. Answers whether one
/// was.
pub async fn set(
    state: &GlobalServerContext,
    user: UserId,
    target: NotificationTarget,
    level: NotificationLevel,
) -> crate::Result<bool> {
    let mut conn = state.connection_pool.get().await?;
    check_target(state, conn.as_mut(), user, target).await?;
    let (community, channel_id) = columns(target);
    conn.transaction(|conn| {
        async move {
            let removed = delete_row(conn, user, target).await?;
            diesel::insert_into(notification_setting::table)
                .values((
                    notification_setting::id.eq(Uuid::now_v7()),
                    notification_setting::user.eq(user),
                    notification_setting::community.eq(community),
                    notification_setting::channel.eq(channel_id),
                    notification_setting::level.eq(level),
                ))
                .execute(conn)
                .await?;
            publish_event(
                state,
                conn,
                EventScope::User(user),
                &ServerEvent::NotificationSettingChanged {
                    community,
                    channel: channel_id,
                    level: Some(level),
                },
            )
            .await?;
            Ok::<_, crate::Error>(removed > 0)
        }
        .scope_boxed()
    })
    .await
}

/// Returns `target` to the level it would have without the user's setting.
pub async fn reset(
    state: &GlobalServerContext,
    user: UserId,
    target: NotificationTarget,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let (community, channel_id) = columns(target);
    conn.transaction(|conn| {
        async move {
            if delete_row(conn, user, target).await? > 0 {
                publish_event(
                    state,
                    conn,
                    EventScope::User(user),
                    &ServerEvent::NotificationSettingChanged {
                        community,
                        channel: channel_id,
                        level: None,
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

fn columns(target: NotificationTarget) -> (Option<CommunityId>, Option<ChannelId>) {
    match target {
        NotificationTarget::Community(community) => (Some(community), None),
        NotificationTarget::Channel(channel) => (None, Some(channel)),
    }
}

async fn delete_row(
    conn: &mut diesel_async::AsyncPgConnection,
    user: UserId,
    target: NotificationTarget,
) -> QueryResult<usize> {
    let mine = notification_setting::table.filter(notification_setting::user.eq(user));
    match target {
        NotificationTarget::Community(community) => {
            diesel::delete(mine.filter(notification_setting::community.eq(community)))
                .execute(conn)
                .await
        }
        NotificationTarget::Channel(channel_id) => {
            diesel::delete(mine.filter(notification_setting::channel.eq(channel_id)))
                .execute(conn)
                .await
        }
    }
}

/// A setting may be made for a community the user belongs to, or a text channel or DM they may
/// read; a community the user is not in, or a channel they may not read, is not found.
async fn check_target(
    state: &GlobalServerContext,
    conn: &mut diesel_async::AsyncPgConnection,
    user: UserId,
    target: NotificationTarget,
) -> crate::Result<()> {
    match target {
        NotificationTarget::Community(community) => {
            let member: bool = diesel::select(diesel::dsl::exists(
                community_user::table.filter(
                    community_user::community
                        .eq(community)
                        .and(community_user::user.eq(user)),
                ),
            ))
            .get_result(conn)
            .await?;
            if !member {
                return Err(diesel::result::Error::NotFound.into());
            }
        }
        NotificationTarget::Channel(channel_id) => {
            crate::permissions::channel_access(state, conn, user, channel_id).await?;
            let ty: ChannelType = channel::table
                .select(channel::ty)
                .filter(
                    channel::id
                        .eq(channel_id)
                        .and(channel::deleted_at.is_null()),
                )
                .first(conn)
                .await?;
            if !matches!(
                ty,
                ChannelType::Text | ChannelType::Dm | ChannelType::GroupDm | ChannelType::Plugin
            ) {
                return Err(crate::Error::Validation(t!("notificationSettingKind")));
            }
        }
    }
    Ok(())
}

/// The user's settings for the listed channels (DMs they are in).
pub async fn read_channel_settings(
    state: &GlobalServerContext,
    user: UserId,
    channels: &[ChannelId],
) -> crate::Result<Vec<NotificationSetting>> {
    read_settings(state, user, channels, &[]).await
}

/// The user's settings for every DM and group DM they are in, however many there are: rows they
/// made themselves, found from their own key, which the DM list sends whole beside its pages.
pub async fn read_dm_settings(
    state: &GlobalServerContext,
    user: UserId,
) -> crate::Result<Vec<NotificationSetting>> {
    use aspen_schema::dm_recipient;
    let mut conn = state.connection_pool.get().await?;
    Ok(notification_setting::table
        .select(NotificationSetting::as_select())
        .filter(notification_setting::user.eq(user))
        .filter(diesel::dsl::exists(
            dm_recipient::table.filter(
                dm_recipient::channel
                    .nullable()
                    .eq(notification_setting::channel)
                    .and(dm_recipient::user.eq(user)),
            ),
        ))
        .load(conn.as_mut())
        .await?)
}

/// The settings of `visible`'s user for its communities and for the channels they may view in
/// them.
pub async fn read_community_settings(
    state: &GlobalServerContext,
    visible: &crate::visibility::Visibility,
) -> crate::Result<Vec<NotificationSetting>> {
    read_settings(
        state,
        visible.user(),
        &visible.visible_channels(),
        visible.communities(),
    )
    .await
}

/// The user's settings for the listed communities and the listed channels, in one query, each
/// arm found through the user's own `(user, community)` or `(user, channel)` index.
async fn read_settings(
    state: &GlobalServerContext,
    user: UserId,
    channels: &[ChannelId],
    communities: &[CommunityId],
) -> crate::Result<Vec<NotificationSetting>> {
    let mut conn = state.connection_pool.get().await?;
    Ok(notification_setting::table
        .select(NotificationSetting::as_select())
        .filter(notification_setting::user.eq(user))
        .filter(
            notification_setting::community
                .eq_any(communities.to_vec())
                .or(notification_setting::channel.eq_any(channels.to_vec())),
        )
        .load(conn.as_mut())
        .await?)
}
