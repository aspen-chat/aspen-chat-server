//! Communities turning plugins on and configuring them, under the community permission Manage
//! plugins. An `optIn` plugin runs in a community once it is turned on there; an `everywhere`
//! plugin runs in every community, which may configure it but not turn it off. Turning a plugin
//! on brings in its principal, if it has one, with the permissions the person granted; turning
//! it off takes the principal out.
//!
//! Each change is published as a `communityPlugin` event, which only holders of Manage plugins
//! receive (`Aspen-Requires`), since settings may hold what the community keeps to its
//! moderators (a list of watched words, say), and every server forgets what it cached of the
//! community's use of plugins.

use super::registry::{self, LoadedPlugin};
use super::{Mode, settings};
use crate::api::message_enum::CommunityPlugin;
use crate::api::message_enum::server_event::{CommunityPluginEvent, ServerEvent};
use crate::app::context::GlobalServerContext;
use crate::app::permissions::{Permission, Permissions, require_member};
use crate::app::{self, CommunityId, EventScope, UserId, publish_event};
use crate::database::schema::community_plugin;
use crate::t;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde_json::{Map, Value};
use std::sync::Arc;

/// A community's row for a plugin, as stored.
struct Row {
    enabled: bool,
    settings: Map<String, Value>,
}

async fn row(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    plugin: &str,
) -> app::Result<Option<Row>> {
    let found: Option<(bool, Value)> = community_plugin::table
        .select((community_plugin::enabled, community_plugin::settings))
        .filter(
            community_plugin::community
                .eq(community)
                .and(community_plugin::plugin.eq(plugin)),
        )
        .for_update()
        .first(conn)
        .await
        .optional()?;
    Ok(found.map(|(enabled, settings)| Row {
        enabled,
        settings: match settings {
            Value::Object(map) => map,
            _ => Map::new(),
        },
    }))
}

/// The record holders of Manage plugins read: whether it runs there and its settings, without
/// their secrets.
fn record(plugin: &LoadedPlugin, community: CommunityId, row: Option<&Row>) -> CommunityPlugin {
    let stored = row.map(|r| r.settings.clone()).unwrap_or_default();
    let (shown, secrets_set) = settings::readable(&plugin.manifest.community_settings, &stored);
    CommunityPlugin {
        community,
        plugin: plugin.id.clone(),
        enabled: match plugin.mode {
            Mode::Everywhere => true,
            Mode::OptIn => row.is_some_and(|r| r.enabled),
        },
        settings: Value::Object(shown),
        secrets_set,
    }
}

/// The plugin `id`, if it is on.
fn loaded(state: &GlobalServerContext, id: &str) -> app::Result<Arc<LoadedPlugin>> {
    state
        .plugins
        .get(id)
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))
}

/// Every plugin that is on, with `community`'s use of it, for a holder of Manage plugins.
pub async fn list(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
) -> app::Result<Vec<CommunityPlugin>> {
    let mut conn = state.connection_pool.get().await?;
    require_member(conn.as_mut(), caller, community)
        .await?
        .require(Permissions::MANAGE_PLUGINS)?;
    let mut out = Vec::new();
    for plugin in state.plugins.loaded().iter() {
        let found = row(conn.as_mut(), community, &plugin.id).await?;
        out.push(record(plugin, community, found.as_ref()));
    }
    Ok(out)
}

/// `settings` checked against `plugin`'s community settings and laid over `current`, naming the
/// field by its label in the reader's language when it is wrong.
async fn checked(
    conn: &mut AsyncPgConnection,
    plugin: &LoadedPlugin,
    community: CommunityId,
    current: &Map<String, Value>,
    patch: &Map<String, Value>,
) -> app::Result<Map<String, Value>> {
    let fields = &plugin.manifest.community_settings;
    let label = |name: &str| {
        fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| {
                plugin.render(
                    app::locale::current(),
                    &super::PluginText {
                        key: f.label.clone(),
                        args: Default::default(),
                    },
                )
            })
            .unwrap_or_else(|| name.to_string())
    };
    let next = settings::apply(fields, current, patch)
        .map_err(|p| settings::refusal(&p, &label(&p.field)))?;
    settings::check_community(conn, fields, community, &next)
        .await
        .map_err(|p| settings::refusal(&p, &label(&p.field)))?;
    Ok(next)
}

/// Turns `plugin` on in `community` (for an `everywhere` plugin, sets its settings there), with
/// `settings` laid over any it had and its principal brought in holding `grant`. Returns the
/// record and whether it was off before.
pub async fn enable(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    plugin_id: &str,
    patch: Map<String, Value>,
    grant: Vec<Permission>,
) -> app::Result<(CommunityPlugin, bool)> {
    let plugin = loaded(state, plugin_id)?;
    let mut conn = state.connection_pool.get().await?;
    let result = conn
        .transaction(|conn| {
            let plugin = plugin.clone();
            async move {
                let access = require_member(conn.as_mut(), caller, community).await?;
                access.require(Permissions::MANAGE_PLUGINS)?;
                let before = row(conn.as_mut(), community, &plugin.id).await?;
                let was_on = match plugin.mode {
                    Mode::Everywhere => true,
                    Mode::OptIn => before.as_ref().is_some_and(|r| r.enabled),
                };
                let current = before
                    .as_ref()
                    .map(|r| r.settings.clone())
                    .unwrap_or_default();
                let next = checked(conn.as_mut(), &plugin, community, &current, &patch).await?;
                diesel::insert_into(community_plugin::table)
                    .values((
                        community_plugin::community.eq(community),
                        community_plugin::plugin.eq(&plugin.id),
                        community_plugin::enabled.eq(true),
                        community_plugin::settings.eq(Value::Object(next.clone())),
                        community_plugin::updated_by.eq(caller),
                    ))
                    .on_conflict((community_plugin::community, community_plugin::plugin))
                    .do_update()
                    .set((
                        community_plugin::enabled.eq(true),
                        community_plugin::settings.eq(Value::Object(next.clone())),
                        community_plugin::updated_by.eq(caller),
                        community_plugin::updated_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn.as_mut())
                    .await?;
                if let Some(principal) = plugin.principal {
                    let granted = app::permissions::from_names(&grant);
                    let asked = app::permissions::from_names(
                        plugin
                            .manifest
                            .principal
                            .as_ref()
                            .map(|p| p.permissions.as_slice())
                            .unwrap_or_default(),
                    );
                    if !asked.contains(granted) {
                        return Err(app::Error::Validation(t!("pluginGrantNotAsked")));
                    }
                    super::principal::join(
                        state,
                        conn.as_mut(),
                        &access,
                        principal,
                        plugin.name(app::locale::current()),
                        granted,
                    )
                    .await?;
                }
                let after = Row {
                    enabled: true,
                    settings: next,
                };
                let record = record(&plugin, community, Some(&after));
                let event = match &before {
                    None => CommunityPluginEvent::Create(record.clone()),
                    Some(_) => CommunityPluginEvent::Update {
                        community,
                        plugin: plugin.id.clone(),
                        enabled: Some(record.enabled),
                        settings: Some(record.settings.clone()),
                        secrets_set: Some(record.secrets_set.clone()),
                    },
                };
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::Community(community),
                    &ServerEvent::CommunityPlugin(event),
                )
                .await?;
                Ok::<_, app::Error>((record, !was_on))
            }
            .scope_boxed()
        })
        .await?;
    registry::announce(&state.nats_context.client(), Some(community)).await?;
    Ok(result)
}

/// Changes `community`'s settings for `plugin`, laying `patch` over them.
pub async fn configure(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    plugin_id: &str,
    patch: Map<String, Value>,
) -> app::Result<CommunityPlugin> {
    let plugin = loaded(state, plugin_id)?;
    let mut conn = state.connection_pool.get().await?;
    let record = conn
        .transaction(|conn| {
            let plugin = plugin.clone();
            async move {
                require_member(conn.as_mut(), caller, community)
                    .await?
                    .require(Permissions::MANAGE_PLUGINS)?;
                let before = row(conn.as_mut(), community, &plugin.id).await?;
                let current = before
                    .as_ref()
                    .map(|r| r.settings.clone())
                    .unwrap_or_default();
                let next = checked(conn.as_mut(), &plugin, community, &current, &patch).await?;
                let enabled = before.as_ref().is_some_and(|r| r.enabled);
                diesel::insert_into(community_plugin::table)
                    .values((
                        community_plugin::community.eq(community),
                        community_plugin::plugin.eq(&plugin.id),
                        community_plugin::enabled.eq(enabled),
                        community_plugin::settings.eq(Value::Object(next.clone())),
                        community_plugin::updated_by.eq(caller),
                    ))
                    .on_conflict((community_plugin::community, community_plugin::plugin))
                    .do_update()
                    .set((
                        community_plugin::settings.eq(Value::Object(next.clone())),
                        community_plugin::updated_by.eq(caller),
                        community_plugin::updated_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn.as_mut())
                    .await?;
                let after = Row {
                    enabled,
                    settings: next,
                };
                let record = record(&plugin, community, Some(&after));
                let event = match &before {
                    None => CommunityPluginEvent::Create(record.clone()),
                    Some(_) => CommunityPluginEvent::Update {
                        community,
                        plugin: plugin.id.clone(),
                        enabled: None,
                        settings: Some(record.settings.clone()),
                        secrets_set: Some(record.secrets_set.clone()),
                    },
                };
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::Community(community),
                    &ServerEvent::CommunityPlugin(event),
                )
                .await?;
                Ok::<_, app::Error>(record)
            }
            .scope_boxed()
        })
        .await?;
    registry::announce(&state.nats_context.client(), Some(community)).await?;
    Ok(record)
}

/// Turns an `optIn` plugin off in `community`, taking its principal out. Its settings there are
/// kept for when it is turned on again.
pub async fn disable(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    plugin_id: &str,
) -> app::Result<()> {
    let plugin = loaded(state, plugin_id)?;
    if plugin.mode == Mode::Everywhere {
        return Err(app::Error::Validation(t!("pluginEverywhere")));
    }
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        let plugin = plugin.clone();
        async move {
            require_member(conn.as_mut(), caller, community)
                .await?
                .require(Permissions::MANAGE_PLUGINS)?;
            let changed = diesel::update(
                community_plugin::table.filter(
                    community_plugin::community
                        .eq(community)
                        .and(community_plugin::plugin.eq(&plugin.id))
                        .and(community_plugin::enabled),
                ),
            )
            .set((
                community_plugin::enabled.eq(false),
                community_plugin::updated_by.eq(caller),
                community_plugin::updated_at.eq(diesel::dsl::now),
            ))
            .execute(conn.as_mut())
            .await?;
            if changed == 0 {
                return Ok(());
            }
            if let Some(principal) = plugin.principal {
                super::principal::leave(state, conn.as_mut(), principal, community).await?;
            }
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Community(community),
                &ServerEvent::CommunityPlugin(CommunityPluginEvent::Update {
                    community,
                    plugin: plugin.id.clone(),
                    enabled: Some(false),
                    settings: None,
                    secrets_set: None,
                }),
            )
            .await?;
            Ok::<_, app::Error>(())
        }
        .scope_boxed()
    })
    .await?;
    registry::announce(&state.nats_context.client(), Some(community)).await?;
    Ok(())
}
