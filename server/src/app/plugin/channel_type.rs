//! Channels of the kinds plugins add: `ty` `plugin`, with `plugin_type` naming the plugin and the
//! kind (`org.example.forums:board`). Someone who may manage channels makes one where a plugin
//! that declares the kind runs; its permissions are the ordinary channel permissions, and its
//! contents are the plugin's, kept in storage scoped to the channel. It holds no messages.

use super::PluginPermission;
use super::manifest::ChannelTypeDecl;
use super::registry::LoadedPlugin;
use crate::app::{self, CommunityId};
use crate::t;
use diesel_async::AsyncPgConnection;
use std::sync::Arc;

/// The plugin and kind `plugin_type` names: `{plugin}:{kind}`.
pub fn split(plugin_type: &str) -> Option<(&str, &str)> {
    plugin_type.rsplit_once(':')
}

/// The plugin running in `community` that declares `plugin_type`, and the kind, or a refusal
/// saying no plugin running there adds it.
pub async fn check(
    state: &app::context::GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    plugin_type: &str,
) -> app::Result<(Arc<LoadedPlugin>, ChannelTypeDecl)> {
    let unknown = || app::Error::Validation(t!("pluginTypeUnknown", kind = plugin_type));
    let (plugin_id, kind) = split(plugin_type).ok_or_else(unknown)?;
    let running = state.plugins.running_in_community(conn, community).await?;
    let plugin = running
        .into_iter()
        .map(|r| r.plugin)
        .find(|p| p.id == plugin_id && p.holds(PluginPermission::ChannelTypes))
        .ok_or_else(unknown)?;
    let declared = plugin
        .manifest
        .channel_types
        .get(kind)
        .cloned()
        .ok_or_else(unknown)?;
    Ok((plugin, declared))
}
