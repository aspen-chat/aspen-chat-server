//! What each user wants to be told of, for themself alone: for a whole community, one of its
//! categories, or one of its text channels, or a DM, every message (`all`), only messages that
//! tag them (`tags`), or nothing. A channel's own setting outranks its category's, which
//! outranks its community's, and without any a DM, or a channel of a small community
//! (`app::large_community`), tells of every message, and a channel of a large one of tags
//! ([`level_of`], [`default_level`]); a thread follows its parent. Phones are woken by these
//! (`app::push`), and the apps notify by them. Every change is published to the user's own
//! subject as `notificationSettingChanged`, so their other devices follow.

use crate::channel::ChannelType;
use crate::context::GlobalServerContext;
use crate::t;
use crate::{CategoryId, ChannelId, CommunityId, EventScope, UserId, publish_event};
use aspen_schema::{category, community_user, notification_setting};
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
    Category(CategoryId),
    Channel(ChannelId),
}

/// A user's setting for a community, a category, or a channel.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = notification_setting)]
pub struct NotificationSetting {
    pub community: Option<CommunityId>,
    pub category: Option<CategoryId>,
    pub channel: Option<ChannelId>,
    pub level: NotificationLevel,
}

/// A channel's level for the user: its own setting, else its category's, else its community's,
/// else the default for its kind and, in a community, whether that community is `small`.
pub fn level_of(
    own: Option<NotificationLevel>,
    category: Option<NotificationLevel>,
    community: Option<NotificationLevel>,
    ty: ChannelType,
    small: bool,
) -> NotificationLevel {
    own.or(category)
        .or(community)
        .unwrap_or_else(|| default_level(ty, small))
}

/// The level of a channel neither the user nor its community has a setting for: every message
/// of a DM, which is written to its few people, and of a small community's channel; tags in a
/// large community (`app::large_community`), where every message would be too many. `small` is
/// whether the channel's community is small, and is ignored for a DM.
pub fn default_level(ty: ChannelType, small: bool) -> NotificationLevel {
    match ty {
        ChannelType::Dm | ChannelType::GroupDm => NotificationLevel::All,
        _ if small => NotificationLevel::All,
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
    let (community, category_id, channel_id) = columns(target);
    conn.transaction(|conn| {
        async move {
            let removed = delete_row(conn, user, target).await?;
            diesel::insert_into(notification_setting::table)
                .values((
                    notification_setting::id.eq(Uuid::now_v7()),
                    notification_setting::user.eq(user),
                    notification_setting::community.eq(community),
                    notification_setting::category.eq(category_id),
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
                    category: category_id,
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
    let (community, category_id, channel_id) = columns(target);
    conn.transaction(|conn| {
        async move {
            if delete_row(conn, user, target).await? > 0 {
                publish_event(
                    state,
                    conn,
                    EventScope::User(user),
                    &ServerEvent::NotificationSettingChanged {
                        community,
                        category: category_id,
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

fn columns(
    target: NotificationTarget,
) -> (Option<CommunityId>, Option<CategoryId>, Option<ChannelId>) {
    match target {
        NotificationTarget::Community(community) => (Some(community), None, None),
        NotificationTarget::Category(category) => (None, Some(category), None),
        NotificationTarget::Channel(channel) => (None, None, Some(channel)),
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
        NotificationTarget::Category(category_id) => {
            diesel::delete(mine.filter(notification_setting::category.eq(category_id)))
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

/// A setting may be made for a community the user belongs to, a category they may learn of, or a
/// text channel or DM they may read; any other is not found.
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
        NotificationTarget::Category(category_id) => {
            crate::category::viewed_category(conn, user, category_id).await?;
        }
        NotificationTarget::Channel(channel_id) => {
            let ty = crate::permissions::channel_access(state, conn, user, channel_id)
                .await?
                .ty;
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
    let mut conn = state.connection_pool.get().await?;
    Ok(notification_setting::table
        .select(NotificationSetting::as_select())
        .filter(notification_setting::user.eq(user))
        .filter(notification_setting::channel.eq_any(channels.to_vec()))
        .load(conn.as_mut())
        .await?)
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

/// The settings of `visible`'s user for its communities, for the categories of them they may
/// learn of, and for the channels they may view in them. Each kind is found through the user's
/// own `(user, community)`, `(user, category)`, or `(user, channel)` index.
pub async fn read_community_settings(
    state: &GlobalServerContext,
    visible: &crate::visibility::Visibility,
) -> crate::Result<Vec<NotificationSetting>> {
    let user = visible.user();
    let communities = visible.communities().to_vec();
    let mut conn = state.connection_pool.get().await?;
    let mut settings: Vec<NotificationSetting> = notification_setting::table
        .select(NotificationSetting::as_select())
        .filter(notification_setting::user.eq(user))
        .filter(notification_setting::community.eq_any(&communities))
        .load(conn.as_mut())
        .await?;
    let in_categories: Vec<(NotificationSetting, CommunityId)> = notification_setting::table
        .inner_join(category::table)
        .select((NotificationSetting::as_select(), category::community))
        .filter(notification_setting::user.eq(user))
        .filter(category::community.eq_any(&communities))
        .filter(category::deleted_at.is_null())
        .load(conn.as_mut())
        .await?;
    settings.extend(
        in_categories
            .into_iter()
            .filter(|(setting, community)| {
                setting
                    .category
                    .is_some_and(|c| visible.can_view_category(*community, c))
            })
            .map(|(setting, _)| setting),
    );
    settings.extend(
        notification_setting::table
            .select(NotificationSetting::as_select())
            .filter(notification_setting::user.eq(user))
            .filter(notification_setting::channel.eq_any(visible.visible_channels()))
            .load(conn.as_mut())
            .await?,
    );
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_channel_takes_its_own_level_then_its_categorys_then_its_communitys() {
        use NotificationLevel::{All, Nothing, Tags};
        assert_eq!(
            level_of(
                Some(Nothing),
                Some(All),
                Some(Tags),
                ChannelType::Text,
                true
            ),
            Nothing
        );
        assert_eq!(
            level_of(None, Some(All), Some(Nothing), ChannelType::Text, false),
            All
        );
        assert_eq!(
            level_of(None, None, Some(Nothing), ChannelType::Text, true),
            Nothing
        );
    }

    #[test]
    fn without_a_setting_a_small_community_and_a_dm_tell_of_everything() {
        use NotificationLevel::{All, Tags};
        assert_eq!(level_of(None, None, None, ChannelType::Text, true), All);
        assert_eq!(level_of(None, None, None, ChannelType::Plugin, true), All);
        assert_eq!(level_of(None, None, None, ChannelType::Text, false), Tags);
        assert_eq!(level_of(None, None, None, ChannelType::Dm, false), All);
        assert_eq!(level_of(None, None, None, ChannelType::GroupDm, false), All);
    }
}
