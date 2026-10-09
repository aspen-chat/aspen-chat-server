//! Plugins telling people of something (`notify`), as Aspen tells them of a message that tags
//! them: only while they may view the channel, and only where their settings would tell them of
//! such a message (it is not muted, and their level there is not "nothing"; a thread counts as
//! its parent). A notice reaches their apps as `pluginNotice` and wakes their phones with the
//! push pointer `notice`, whose code reads it back (`read`); it is kept a week for that, and goes
//! with its channel.

use super::PluginText;
use crate::context::GlobalServerContext;
use crate::notification_setting::{NotificationLevel, default_level};
use crate::permissions::channel_access;
use crate::{ChannelId, CommunityId, EventScope, MessageId, PluginNoticeId, UserId, publish_event};
use aspen_schema::{channel, channel_mute, message, notification_setting, plugin_notice};
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::Serialize;
use utoipa::ToSchema;

/// How long a notice is kept for phones to read.
pub const KEPT: Duration = Duration::days(7);

/// The channel whose settings and mute decide for `channel_id`: a thread's parent, or itself,
/// with its kind and community.
async fn deciding(
    conn: &mut AsyncPgConnection,
    channel_id: ChannelId,
) -> crate::Result<(ChannelId, crate::channel::ChannelType, Option<CommunityId>)> {
    let (parent, ty, community): (
        Option<ChannelId>,
        crate::channel::ChannelType,
        Option<CommunityId>,
    ) = channel::table
        .select((channel::parent_channel, channel::ty, channel::community))
        .filter(channel::id.eq(channel_id))
        .first(conn)
        .await?;
    match parent {
        Some(parent) => {
            let (ty, community) = channel::table
                .select((channel::ty, channel::community))
                .filter(channel::id.eq(parent))
                .first(conn)
                .await?;
            Ok((parent, ty, community))
        }
        None => Ok((channel_id, ty, community)),
    }
}

/// Whether `user`'s settings would tell them of a message that tags them in `place`.
async fn would_tell(
    conn: &mut AsyncPgConnection,
    user: UserId,
    place: ChannelId,
    ty: crate::channel::ChannelType,
    community: Option<CommunityId>,
) -> crate::Result<bool> {
    let muted: bool = diesel::select(diesel::dsl::exists(
        channel_mute::table.filter(
            channel_mute::user
                .eq(user)
                .and(channel_mute::channel.eq(place))
                .and(
                    channel_mute::until
                        .is_null()
                        .or(channel_mute::until.gt(diesel::dsl::now)),
                ),
        ),
    ))
    .get_result(conn)
    .await?;
    if muted {
        return Ok(false);
    }
    let settings: Vec<(Option<ChannelId>, NotificationLevel)> = notification_setting::table
        .select((notification_setting::channel, notification_setting::level))
        .filter(notification_setting::user.eq(user))
        .filter(
            notification_setting::channel
                .eq(place)
                .or(notification_setting::community.nullable().eq(community)),
        )
        .load(conn)
        .await?;
    let level = settings
        .iter()
        .find(|(channel, _)| channel.is_some())
        .or_else(|| <[_]>::first(&settings))
        .map(|(_, level)| *level)
        .unwrap_or_else(|| default_level(ty));
    Ok(level != NotificationLevel::Nothing)
}

/// Tells `user` of `text` in `channel_id`, about `message_id` if given, for the plugin
/// `plugin`. Answers whether they were told.
pub async fn notify(
    state: &GlobalServerContext,
    plugin: &str,
    user: UserId,
    channel_id: ChannelId,
    text: PluginText,
    message_id: Option<MessageId>,
) -> crate::Result<bool> {
    let mut conn = state.connection_pool.get().await?;
    if channel_access(state, conn.as_mut(), user, channel_id)
        .await
        .is_err()
    {
        return Ok(false);
    }
    let (place, ty, community) = deciding(conn.as_mut(), channel_id).await?;
    if !would_tell(conn.as_mut(), user, place, ty, community).await? {
        return Ok(false);
    }
    if let Some(message_id) = message_id {
        // What it is about is in the channel, and there.
        message::table
            .select(message::id)
            .filter(
                message::id
                    .eq(message_id)
                    .and(message::channel.eq(channel_id))
                    .and(message::deleted_at.is_null()),
            )
            .first::<MessageId>(conn.as_mut())
            .await?;
    }
    let id = PluginNoticeId::new();
    let parent_channel = (place != channel_id).then_some(place);
    conn.transaction(|conn| {
        let text = text.clone();
        async move {
            diesel::insert_into(plugin_notice::table)
                .values((
                    plugin_notice::id.eq(id),
                    plugin_notice::plugin.eq(plugin),
                    plugin_notice::user.eq(user),
                    plugin_notice::channel.eq(channel_id),
                    plugin_notice::message.eq(message_id),
                    plugin_notice::text.eq(&text),
                ))
                .execute(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::User(user),
                &ServerEvent::PluginNotice {
                    id,
                    plugin: plugin.to_string(),
                    channel: channel_id,
                    community,
                    parent_channel,
                    text,
                    message: message_id,
                },
            )
            .await
        }
        .scope_boxed()
    })
    .await?;
    drop(conn);
    crate::push::notice(state, user, channel_id, id).await;
    Ok(true)
}

/// A notice as a phone reads it, in the reader's language.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoticeRead {
    pub id: PluginNoticeId,
    /// The plugin's name.
    pub title: String,
    /// What it says.
    pub body: String,
    pub channel: ChannelId,
    pub community: Option<CommunityId>,
    /// For a notice in a thread, the channel the thread is in.
    pub parent_channel: Option<ChannelId>,
    pub message: Option<MessageId>,
    pub created_at: DateTime<Utc>,
}

/// `caller`'s notice `id`, while they may still view its channel and its plugin runs.
pub async fn read(
    state: &GlobalServerContext,
    caller: UserId,
    id: PluginNoticeId,
) -> crate::Result<NoticeRead> {
    let not_found = || crate::Error::Diesel(diesel::result::Error::NotFound);
    let mut conn = state.connection_pool.get().await?;
    let (plugin, channel_id, message_id, text, created_at): (
        String,
        ChannelId,
        Option<MessageId>,
        PluginText,
        DateTime<Utc>,
    ) = plugin_notice::table
        .select((
            plugin_notice::plugin,
            plugin_notice::channel,
            plugin_notice::message,
            plugin_notice::text,
            plugin_notice::created_at,
        ))
        .filter(plugin_notice::id.eq(id).and(plugin_notice::user.eq(caller)))
        // One past its time is read as gone, though the sweep may not have taken it yet.
        .filter(plugin_notice::created_at.gt(Utc::now() - KEPT))
        .first(conn.as_mut())
        .await?;
    channel_access(state, conn.as_mut(), caller, channel_id).await?;
    let (place, _, community) = deciding(conn.as_mut(), channel_id).await?;
    let loaded = state.plugins.get(&plugin).ok_or_else(not_found)?;
    let locale = crate::locale::current();
    Ok(NoticeRead {
        id,
        title: loaded.name(locale),
        body: loaded.render(locale, &text),
        channel: channel_id,
        community,
        parent_channel: (place != channel_id).then_some(place),
        message: message_id,
        created_at,
    })
}

/// Takes one of `plugin`'s turns at telling `user` of something: at most `[plugins]
/// notify_per_minute` in a minute and `notify_per_day` in a day, counted in Valkey across every
/// server. Past either, the notice is refused. While Valkey cannot be reached, notices are let
/// through, as other limits are.
pub async fn take_turn(
    state: &GlobalServerContext,
    plugin: &str,
    user: UserId,
) -> Result<(), super::host::wit::Error> {
    let config = &state.config.plugins;
    for (per, requests, seconds) in [
        ("minute", config.notify_per_minute, 60.0),
        ("day", config.notify_per_day, 86_400.0),
    ] {
        if requests == 0 {
            continue;
        }
        let rate = aspen_limits::Limit {
            requests,
            per_seconds: seconds,
            burst: None,
            bucket: None,
        }
        .rate();
        let key = format!("aspen:plugin_notify:{per}:{plugin}:{}", user.0);
        match crate::rate_limit::take(&state.valkey, &key, rate).await {
            Ok(None) => {}
            Ok(Some(_)) => {
                return Err(super::host::wit::Error::Limit(format!(
                    "a plugin may tell one person of at most {requests} things a {per}"
                )));
            }
            Err(e) => {
                tracing::warn!(error = %e, "could not count a plugin's notices; letting it through");
                return Ok(());
            }
        }
    }
    Ok(())
}
