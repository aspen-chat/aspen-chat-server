//! Installing, upgrading, configuring, ordering, turning on and off, removing, and purging
//! plugins: the terminal's `plugins` commands (`operator::plugins`), and the dashboard's under
//! the deployment permission Manage plugins (`api::plugin`), which may change everything here
//! but what runs: installing, upgrading, removing, and purging are the terminal's alone, as is
//! granting permissions, since running code is a power too strong for a web API.
//!
//! Every change touches `plugin.updated_at` and is announced on `CHANGED_SUBJECT`, so every API
//! server loads it without a restart.

use super::manifest::Manifest;
use super::{Mode, PluginPermission, settings};
use crate::api::message_enum::server_event::ServerEvent;
use crate::app::events::Publishing;
use crate::app::user::UserPg;
use crate::app::{self, CommunityId, EventScope, UserId, publish_event};
use crate::database::schema::{
    bot_command_list, community_user, message_annotation, plugin, user, user_annotation,
};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

/// An installed plugin as the operator and the dashboard see it.
#[derive(Debug, Clone)]
pub struct Installed {
    pub id: String,
    pub version: String,
    pub manifest: Manifest,
    pub granted: BTreeSet<PluginPermission>,
    pub mode: Mode,
    pub enabled: bool,
    pub position: i32,
    pub settings: Map<String, Value>,
    pub storage_bytes: i64,
    pub principal: Option<UserId>,
    pub installed_at: DateTime<Utc>,
    pub removed: bool,
}

type InstalledRow = (
    String,
    String,
    Value,
    Vec<Option<String>>,
    String,
    bool,
    i32,
    Value,
    i64,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

/// Every installed plugin, the removed ones too when `removed`, in the operator's order.
pub async fn list(conn: &mut AsyncPgConnection, removed: bool) -> app::Result<Vec<Installed>> {
    let mut query = plugin::table
        .select((
            plugin::id,
            plugin::version,
            plugin::manifest,
            plugin::granted,
            plugin::mode,
            plugin::enabled,
            plugin::position,
            plugin::settings,
            plugin::storage_bytes,
            plugin::installed_at,
            plugin::removed_at,
        ))
        .order((plugin::position.asc(), plugin::id.asc()))
        .into_boxed();
    if !removed {
        query = query.filter(plugin::removed_at.is_null());
    }
    let rows: Vec<InstalledRow> = query.load(conn).await?;
    let principals: Vec<(Option<String>, UserId)> = user::table
        .select((user::plugin, user::id))
        .filter(user::plugin.is_not_null())
        .load(conn)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, version, manifest, granted, mode, enabled, position, stored, bytes, at, removed_at) in
        rows
    {
        let Ok(manifest) = serde_json::from_value::<Manifest>(manifest) else {
            tracing::error!(plugin = id, "its manifest is not one this version reads");
            continue;
        };
        out.push(Installed {
            principal: principals
                .iter()
                .find(|(p, _)| p.as_deref() == Some(id.as_str()))
                .map(|(_, u)| *u),
            granted: granted
                .into_iter()
                .flatten()
                .filter_map(|p| p.parse().ok())
                .collect(),
            mode: mode.parse().unwrap_or(Mode::OptIn),
            settings: match stored {
                Value::Object(map) => map,
                _ => Map::new(),
            },
            id,
            version,
            manifest,
            enabled,
            position,
            storage_bytes: bytes,
            installed_at: at,
            removed: removed_at.is_some(),
        });
    }
    Ok(out)
}

/// The plugin `id`, removed or not.
pub async fn find(conn: &mut AsyncPgConnection, id: &str) -> app::Result<Installed> {
    list(conn, true)
        .await?
        .into_iter()
        .find(|p| p.id == id)
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))
}

/// What installing did.
pub enum Outcome {
    /// A plugin new to this deployment, off until it is turned on.
    New,
    /// A plugin removed before, installed again with the data it kept.
    Restored,
    /// An upgrade (or a reinstall) of one installed.
    Upgraded { from: String },
}

/// Installs `manifest` with `component`, or upgrades the plugin of its id, granting the
/// permissions it asks for (`dms` only with `grant_dms`). A new plugin is installed off, in
/// `mode`, after every other; an upgrade keeps its mode, order, whether it is on, and the
/// settings it still has. Its principal is made, or renamed, and its commands published.
pub async fn install(
    publisher: &impl Publishing,
    conn: &mut AsyncPgConnection,
    manifest: &Manifest,
    component: &[u8],
    mode: Mode,
    grant_dms: bool,
) -> app::Result<Outcome> {
    let mut granted = manifest.permissions.clone();
    if !grant_dms {
        granted.remove(&PluginPermission::Dms);
    }
    let granted: Vec<String> = granted.iter().map(|p| p.to_string()).collect();
    let stored_manifest = serde_json::to_value(manifest)?;
    let outcome = conn
        .transaction(|conn| {
            async move {
                let existing: Option<(String, Value, Option<DateTime<Utc>>)> = plugin::table
                    .select((plugin::version, plugin::settings, plugin::removed_at))
                    .filter(plugin::id.eq(&manifest.id))
                    .for_update()
                    .first(conn)
                    .await
                    .optional()?;
                let outcome = match existing {
                    None => {
                        let last: Option<i32> = plugin::table
                            .select(diesel::dsl::max(plugin::position))
                            .first(conn)
                            .await?;
                        diesel::insert_into(plugin::table)
                            .values((
                                plugin::id.eq(&manifest.id),
                                plugin::version.eq(&manifest.version),
                                plugin::manifest.eq(&stored_manifest),
                                plugin::component.eq(component),
                                plugin::granted.eq(&granted),
                                plugin::mode.eq(mode.to_string()),
                                plugin::enabled.eq(false),
                                plugin::position.eq(last.map_or(0, |p| p + 1)),
                            ))
                            .execute(conn)
                            .await?;
                        Outcome::New
                    }
                    Some((version, stored, removed_at)) => {
                        // Settings the plugin no longer has are dropped.
                        let mut kept = match stored {
                            Value::Object(map) => map,
                            _ => Map::new(),
                        };
                        kept.retain(|name, _| manifest.settings.iter().any(|f| &f.name == name));
                        diesel::update(plugin::table.filter(plugin::id.eq(&manifest.id)))
                            .set((
                                plugin::version.eq(&manifest.version),
                                plugin::manifest.eq(&stored_manifest),
                                plugin::component.eq(component),
                                plugin::granted.eq(&granted),
                                plugin::settings.eq(Value::Object(kept)),
                                plugin::removed_at.eq(None::<DateTime<Utc>>),
                                plugin::updated_at.eq(diesel::dsl::now),
                            ))
                            .execute(conn)
                            .await?;
                        if removed_at.is_some() {
                            Outcome::Restored
                        } else {
                            Outcome::Upgraded { from: version }
                        }
                    }
                };
                match &manifest.principal {
                    Some(principal) => {
                        ensure_principal(publisher, conn, manifest, principal).await?
                    }
                    None => retire_principal(publisher, conn, &manifest.id).await?,
                }
                Ok::<_, app::Error>(outcome)
            }
            .scope_boxed()
        })
        .await?;
    Ok(outcome)
}

/// Makes the plugin's principal, or renames it, and publishes its commands.
async fn ensure_principal(
    publisher: &impl Publishing,
    conn: &mut AsyncPgConnection,
    manifest: &Manifest,
    principal: &super::manifest::Principal,
) -> app::Result<()> {
    let display_name = super::render(
        &manifest.messages,
        &manifest.default_language,
        &manifest.default_language,
        &super::PluginText {
            key: principal.display_name.clone(),
            args: Default::default(),
        },
    );
    let existing: Option<UserId> = user::table
        .select(user::id)
        .filter(
            user::plugin
                .eq(&manifest.id)
                .and(user::deleted_at.is_null()),
        )
        .first(conn)
        .await
        .optional()?;
    let id = match existing {
        Some(id) => {
            diesel::update(user::table.filter(user::id.eq(id)))
                .set((
                    user::name.eq(&principal.username),
                    user::display_name.eq(&display_name),
                ))
                .execute(conn)
                .await?;
            publish_event(
                publisher,
                conn,
                EventScope::UserEverywhere(id),
                &ServerEvent::User(crate::api::message_enum::server_event::UserEvent::Update {
                    id,
                    name: Some(principal.username.clone()),
                    icon: None,
                    display_name: Some(Some(display_name)),
                    pronouns: None,
                    bio: None,
                    status: None,
                    bot_owner: None,
                    bot_public: None,
                }),
            )
            .await?;
            id
        }
        None => {
            let id = UserId::new();
            let now = Utc::now();
            diesel::insert_into(user::table)
                .values(UserPg {
                    id,
                    name: principal.username.clone(),
                    icon: None,
                    // No password verifies against an empty hash, and no token is issued.
                    password_hash: String::new(),
                    created_at: now,
                    last_seen_at: now,
                    deleted_at: None,
                    display_name: Some(display_name),
                    pronouns: None,
                    bio: None,
                    status_text: None,
                    status_emoji: None,
                    bot: true,
                    system: false,
                    bot_owner: None,
                    bot_public: false,
                    home_domain: None,
                    home_id: None,
                    home_icon: None,
                    plugin: Some(manifest.id.clone()),
                })
                .execute(conn)
                .await
                .map_err(|e| match e {
                    diesel::result::Error::DatabaseError(
                        diesel::result::DatabaseErrorKind::UniqueViolation,
                        _,
                    ) => app::Error::Validation(
                        format!(
                            "the username {} is taken; the plugin's principal needs it",
                            principal.username
                        )
                        .into(),
                    ),
                    other => other.into(),
                })?;
            id
        }
    };
    let commands = serde_json::to_value(crate::app::bot_command::CommandList {
        commands: principal.commands.clone(),
    })?;
    diesel::insert_into(bot_command_list::table)
        .values((
            bot_command_list::bot.eq(id),
            bot_command_list::commands.eq(&commands),
        ))
        .on_conflict(bot_command_list::bot)
        .do_update()
        .set((
            bot_command_list::commands.eq(&commands),
            bot_command_list::updated_at.eq(diesel::dsl::now),
        ))
        .execute(conn)
        .await?;
    publish_event(
        publisher,
        conn,
        EventScope::UserEverywhere(id),
        &ServerEvent::BotCommandsChanged { bot: id },
    )
    .await
}

/// Takes the plugin's principal out of every community it is in, as the plugin goes or stops
/// acting. The account stays, as does what it posted.
async fn retire_principal(
    publisher: &impl Publishing,
    conn: &mut AsyncPgConnection,
    id: &str,
) -> app::Result<()> {
    let principal: Option<UserId> = user::table
        .select(user::id)
        .filter(user::plugin.eq(id))
        .first(conn)
        .await
        .optional()?;
    let Some(principal) = principal else {
        return Ok(());
    };
    let communities: Vec<CommunityId> = community_user::table
        .select(community_user::community)
        .filter(community_user::user.eq(principal))
        .load(conn)
        .await?;
    for community in communities {
        super::principal::leave(publisher, conn, principal, community).await?;
    }
    Ok(())
}

/// Changes what `id` is: whether it is on, its mode, or its settings (`patch` laid over them).
/// Turning it on needs every required setting.
pub async fn update(
    conn: &mut AsyncPgConnection,
    id: &str,
    enabled: Option<bool>,
    mode: Option<Mode>,
    patch: Option<&Map<String, Value>>,
) -> app::Result<Installed> {
    let installed = find(conn, id).await?;
    if installed.removed {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    }
    let fields = &installed.manifest.settings;
    let label = |name: &str| {
        fields
            .iter()
            .find(|f| f.name == name)
            .map(|f| {
                super::render(
                    &installed.manifest.messages,
                    &installed.manifest.default_language,
                    app::locale::current(),
                    &super::PluginText {
                        key: f.label.clone(),
                        args: Default::default(),
                    },
                )
            })
            .unwrap_or_else(|| name.to_string())
    };
    let next = match patch {
        Some(patch) => settings::apply(fields, &installed.settings, patch)
            .map_err(|p| settings::refusal(&p, &label(&p.field)))?,
        None => installed.settings.clone(),
    };
    if enabled == Some(true) {
        settings::apply(fields, &next, &Map::new())
            .map_err(|p| settings::refusal(&p, &label(&p.field)))?;
    }
    diesel::update(plugin::table.filter(plugin::id.eq(id)))
        .set((
            plugin::enabled.eq(enabled.unwrap_or(installed.enabled)),
            plugin::mode.eq(mode.unwrap_or(installed.mode).to_string()),
            plugin::settings.eq(Value::Object(next)),
            plugin::updated_at.eq(diesel::dsl::now),
        ))
        .execute(conn)
        .await?;
    find(conn, id).await
}

/// Puts the plugins in `order`, the order intercepting plugins run in; plugins it leaves out
/// follow, in the order they were in.
pub async fn order(conn: &mut AsyncPgConnection, order: &[String]) -> app::Result<()> {
    let current = list(conn, true).await?;
    let mut ids: Vec<&str> = order
        .iter()
        .map(String::as_str)
        .filter(|id| current.iter().any(|p| p.id == *id))
        .collect();
    for plugin in &current {
        if !ids.contains(&plugin.id.as_str()) {
            ids.push(&plugin.id);
        }
    }
    conn.transaction(|conn| {
        async move {
            for (position, id) in ids.iter().enumerate() {
                diesel::update(plugin::table.filter(plugin::id.eq(*id)))
                    .set((
                        plugin::position.eq(i32::try_from(position).unwrap_or(i32::MAX)),
                        plugin::updated_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)
                    .await?;
            }
            Ok::<_, app::Error>(())
        }
        .scope_boxed()
    })
    .await
}

/// Removes `id`: it stops running everywhere, its component goes, its annotations go, and its
/// principal leaves every community. What it kept stays until it is purged.
pub async fn remove(
    publisher: &impl Publishing,
    conn: &mut AsyncPgConnection,
    id: &str,
) -> app::Result<()> {
    conn.transaction(|conn| {
        async move {
            let removed = diesel::update(
                plugin::table.filter(plugin::id.eq(id).and(plugin::removed_at.is_null())),
            )
            .set((
                plugin::enabled.eq(false),
                plugin::component.eq(None::<Vec<u8>>),
                plugin::removed_at.eq(diesel::dsl::now),
                plugin::updated_at.eq(diesel::dsl::now),
            ))
            .execute(conn)
            .await?;
            if removed == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            // Clients draw no annotation of a plugin the deployment no longer runs, so these go
            // without an event each.
            diesel::delete(message_annotation::table.filter(message_annotation::plugin.eq(id)))
                .execute(conn)
                .await?;
            diesel::delete(user_annotation::table.filter(user_annotation::plugin.eq(id)))
                .execute(conn)
                .await?;
            retire_principal(publisher, conn, id).await
        }
        .scope_boxed()
    })
    .await
}

/// Deletes everything a removed plugin kept: its storage, its communities' settings for it,
/// and its deployment settings. Its row stays, as the record that it was installed, and so does
/// its account, both of which a later install of the same plugin takes up again.
pub async fn purge(conn: &mut AsyncPgConnection, id: &str) -> app::Result<()> {
    use crate::database::schema::{community_plugin, plugin_storage};
    conn.transaction(|conn| {
        async move {
            let removed = diesel::update(
                plugin::table.filter(plugin::id.eq(id).and(plugin::removed_at.is_not_null())),
            )
            .set((
                plugin::settings.eq(Value::Object(Map::new())),
                plugin::storage_bytes.eq(0),
                plugin::updated_at.eq(diesel::dsl::now),
            ))
            .execute(conn)
            .await?;
            if removed == 0 {
                return Err(app::Error::Validation(
                    format!("{id} is not a removed plugin; remove it first").into(),
                ));
            }
            diesel::delete(plugin_storage::table.filter(plugin_storage::plugin.eq(id)))
                .execute(conn)
                .await?;
            diesel::delete(community_plugin::table.filter(community_plugin::plugin.eq(id)))
                .execute(conn)
                .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}
