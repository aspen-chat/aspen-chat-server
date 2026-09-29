//! What each user wants to be told of, for themself alone: for a whole community, or one of its
//! text channels, or a DM, every message (`all`), only messages that tag them (`tags`), or
//! nothing. A channel's own setting outranks its community's, and without either a DM tells of
//! every message and a community channel of tags ([`default_level`]); a thread follows its
//! parent. Phones are woken by these (`app::push`), and the apps notify by them. Every change is
//! published to the user's own subject as `notificationSettingChanged`, so their other devices
//! follow.

use crate::api::ChannelType;
use crate::api::message_enum::server_event::ServerEvent;
use crate::app::{
    self, ChannelId, CommunityId, EventScope, GlobalServerContext, UserId, publish_event,
};
use crate::database::schema::{channel, community_user, notification_setting};
use crate::t;
use diesel::prelude::*;
use diesel::{AsExpression, FromSqlRow};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// How much of a community or channel to be told of.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum NotificationLevel {
    /// Every message.
    All,
    /// Only messages that tag the user: by name, through a role they hold, or as everyone.
    Tags,
    /// Nothing, not even tags.
    Nothing,
}

app::wire_name_traits!(NotificationLevel);
app::text_sql_traits!(NotificationLevel);

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
) -> app::Result<bool> {
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
            Ok::<_, app::Error>(removed > 0)
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
) -> app::Result<()> {
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
            Ok::<_, app::Error>(())
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
) -> app::Result<()> {
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
            app::permissions::channel_access(state, conn, user, channel_id).await?;
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
                ChannelType::Text | ChannelType::Dm | ChannelType::GroupDm
            ) {
                return Err(app::Error::Validation(t!("notificationSettingKind")));
            }
        }
    }
    Ok(())
}

/// The user's settings for the listed communities and every channel in them, and for the listed
/// channels, in one query.
pub async fn read_settings(
    state: &GlobalServerContext,
    user: UserId,
    channels: &[ChannelId],
    communities: &[CommunityId],
) -> app::Result<Vec<NotificationSetting>> {
    let mut conn = state.connection_pool.get().await?;
    let in_communities = channel::table
        .select(channel::id.nullable())
        .filter(channel::community.eq_any(communities.to_vec()));
    Ok(notification_setting::table
        .select(NotificationSetting::as_select())
        .filter(notification_setting::user.eq(user))
        .filter(
            notification_setting::community
                .eq_any(communities.to_vec())
                .or(notification_setting::channel.eq_any(channels.to_vec()))
                .or(notification_setting::channel.eq_any(in_communities)),
        )
        .load(conn.as_mut())
        .await?)
}
