//! The plugins this server runs: each installed plugin that is on, compiled once, and which of
//! them run where. Installed plugins live in the database, so every API server runs the same;
//! a change is announced on `CHANGED_SUBJECT`, on which each server reloads, and a community's
//! use of plugins is cached for a minute at most besides.

use super::host::{self, CallState};
use super::manifest::Manifest;
use super::{CHANGED_SUBJECT, Mode, PluginPermission, settings};
use crate::app::context::GlobalServerContext;
use crate::app::events::ChannelHome;
use crate::app::{self, CommunityId, UserId};
use crate::database::schema::{community_plugin, plugin, user};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

/// How long a community's use of plugins is trusted without hearing of a change.
const COMMUNITY_TTL: Duration = Duration::from_secs(60);

/// An installed plugin this server runs.
pub struct LoadedPlugin {
    pub id: String,
    pub manifest: Manifest,
    /// The permissions the operator granted, which every host call is checked against.
    pub granted: BTreeSet<PluginPermission>,
    pub mode: Mode,
    /// The deployment's settings, with defaults for what is not set.
    pub settings: Map<String, Value>,
    /// Its account, when it has one.
    pub principal: Option<UserId>,
    /// When it was last changed, which tells a reload whether to compile it again.
    pub revision: DateTime<Utc>,
    /// The compiled component, ready to instantiate.
    pub(super) pre: host::PluginPre<CallState>,
}

impl LoadedPlugin {
    /// Whether the operator granted it `permission`.
    pub fn holds(&self, permission: PluginPermission) -> bool {
        self.granted.contains(&permission)
    }

    /// Its text of `text` in `locale`.
    pub fn render(&self, locale: &str, text: &super::PluginText) -> String {
        super::render(
            &self.manifest.messages,
            &self.manifest.default_language,
            locale,
            text,
        )
    }

    /// Its name in `locale`.
    pub fn name(&self, locale: &str) -> String {
        self.render(
            locale,
            &super::PluginText {
                key: self.manifest.name.clone(),
                args: Default::default(),
            },
        )
    }
}

/// A plugin running somewhere, with that community's settings for it.
#[derive(Clone)]
pub struct Running {
    pub plugin: Arc<LoadedPlugin>,
    /// The community's settings, with defaults for what is not set; `None` outside a community.
    pub community_settings: Option<Map<String, Value>>,
}

/// One community's rows of `community_plugin`: whether it turned each plugin on, and its
/// settings.
#[derive(Default)]
struct CommunityUse {
    by_plugin: HashMap<String, (bool, Map<String, Value>)>,
}

/// Every plugin this server runs, and what it knows of communities' use of them.
pub struct Plugins {
    pub(super) engine: wasmtime::Engine,
    pub(super) linker: wasmtime::component::Linker<CallState>,
    /// The plugins that are on, in the operator's order.
    loaded: RwLock<Arc<Vec<Arc<LoadedPlugin>>>>,
    communities: Mutex<HashMap<CommunityId, (Instant, Arc<CommunityUse>)>>,
    /// Each observing plugin's consumer task, by id, with the revision it runs.
    observers: Mutex<HashMap<String, (DateTime<Utc>, tokio::task::AbortHandle)>>,
    /// Serializes reloads, so two announcements close together load in order.
    reloading: tokio::sync::Mutex<()>,
}

/// What `CHANGED_SUBJECT` carries: a community whose use of plugins changed, or nothing when
/// installed plugins did.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Change {
    #[serde(default)]
    pub community: Option<CommunityId>,
}

impl Plugins {
    /// The engine and linker every plugin of this server runs with, and nothing loaded yet.
    pub fn new() -> app::Result<Self> {
        let engine = host::engine().map_err(|e| app::Error::Plugin(format!("{e:#}")))?;
        let linker = host::linker(&engine).map_err(|e| app::Error::Plugin(format!("{e:#}")))?;
        Ok(Plugins {
            engine,
            linker,
            loaded: RwLock::default(),
            communities: Mutex::default(),
            observers: Mutex::default(),
            reloading: tokio::sync::Mutex::new(()),
        })
    }

    /// Whether `bytes` is a component this host runs: one that compiles, imports nothing the
    /// host does not offer, and exports a plugin's hooks.
    pub fn check_component(&self, bytes: &[u8]) -> Result<(), String> {
        host::prepare(&self.engine, &self.linker, bytes)
            .map(|_| ())
            .map_err(|e| format!("{e:#}"))
    }

    /// The plugins that are on, in order.
    pub fn loaded(&self) -> Arc<Vec<Arc<LoadedPlugin>>> {
        self.loaded.read().expect("plugin registry").clone()
    }

    /// The plugin `id`, if it is on.
    pub fn get(&self, id: &str) -> Option<Arc<LoadedPlugin>> {
        self.loaded().iter().find(|p| p.id == id).cloned()
    }

    /// Whether any plugin is on, so work only plugins need can be skipped.
    pub fn any(&self) -> bool {
        !self.loaded().is_empty()
    }

    /// The plugin whose principal `user` is, if it is on.
    pub fn of_principal(&self, user: UserId) -> Option<Arc<LoadedPlugin>> {
        self.loaded()
            .iter()
            .find(|p| p.principal == Some(user))
            .cloned()
    }

    /// Forgets what is cached of `community`, or of every community.
    fn forget(&self, community: Option<CommunityId>) {
        let mut communities = self.communities.lock().expect("plugin communities");
        match community {
            Some(community) => {
                communities.remove(&community);
            }
            None => communities.clear(),
        }
    }

    async fn community_use(
        &self,
        conn: &mut AsyncPgConnection,
        community: CommunityId,
    ) -> app::Result<Arc<CommunityUse>> {
        if let Some((at, found)) = self
            .communities
            .lock()
            .expect("plugin communities")
            .get(&community)
            && at.elapsed() < COMMUNITY_TTL
        {
            return Ok(found.clone());
        }
        let rows: Vec<(String, bool, Value)> = community_plugin::table
            .select((
                community_plugin::plugin,
                community_plugin::enabled,
                community_plugin::settings,
            ))
            .filter(community_plugin::community.eq(community))
            .load(conn)
            .await?;
        let found = Arc::new(CommunityUse {
            by_plugin: rows
                .into_iter()
                .map(|(plugin, enabled, settings)| {
                    let settings = match settings {
                        Value::Object(map) => map,
                        _ => Map::new(),
                    };
                    (plugin, (enabled, settings))
                })
                .collect(),
        });
        self.communities
            .lock()
            .expect("plugin communities")
            .insert(community, (Instant::now(), found.clone()));
        Ok(found)
    }

    /// The plugins that run in `community`, in order, each with the community's settings.
    pub async fn running_in_community(
        &self,
        conn: &mut AsyncPgConnection,
        community: CommunityId,
    ) -> app::Result<Vec<Running>> {
        let loaded = self.loaded();
        if loaded.is_empty() {
            return Ok(Vec::new());
        }
        let used = self.community_use(conn, community).await?;
        Ok(loaded
            .iter()
            .filter_map(|plugin| {
                let row = used.by_plugin.get(&plugin.id);
                let on = match plugin.mode {
                    Mode::Everywhere => true,
                    Mode::OptIn => row.is_some_and(|(enabled, _)| *enabled),
                };
                on.then(|| Running {
                    plugin: plugin.clone(),
                    community_settings: Some(settings::effective(
                        &plugin.manifest.community_settings,
                        &row.map(|(_, s)| s.clone()).unwrap_or_default(),
                    )),
                })
            })
            .collect())
    }

    /// The plugins that run in DMs: those granted `dms`.
    pub fn running_in_dms(&self) -> Vec<Running> {
        self.loaded()
            .iter()
            .filter(|p| p.holds(PluginPermission::Dms))
            .map(|plugin| Running {
                plugin: plugin.clone(),
                community_settings: None,
            })
            .collect()
    }

    /// The plugins that run where `home` is.
    pub async fn running_at(
        &self,
        conn: &mut AsyncPgConnection,
        home: ChannelHome,
    ) -> app::Result<Vec<Running>> {
        match home {
            ChannelHome::Community { community, .. } => {
                self.running_in_community(conn, community).await
            }
            ChannelHome::Direct(_) => Ok(self.running_in_dms()),
        }
    }

    /// Whether `plugin` runs where `home` is, and with which community settings.
    pub async fn runs_at(
        &self,
        conn: &mut AsyncPgConnection,
        plugin: &str,
        home: ChannelHome,
    ) -> app::Result<Option<Running>> {
        Ok(self
            .running_at(conn, home)
            .await?
            .into_iter()
            .find(|r| r.plugin.id == plugin))
    }

    /// Reads the installed plugins again, compiling those that changed, and starts and stops
    /// observers to match.
    pub async fn reload(&self, state: &GlobalServerContext) -> app::Result<()> {
        let _reloading = self.reloading.lock().await;
        let mut conn = state.connection_pool.get().await?;
        type Row = (
            String,
            Value,
            Vec<Option<String>>,
            String,
            Value,
            DateTime<Utc>,
        );
        let rows: Vec<Row> = plugin::table
            .select((
                plugin::id,
                plugin::manifest,
                plugin::granted,
                plugin::mode,
                plugin::settings,
                plugin::updated_at,
            ))
            .filter(plugin::enabled.and(plugin::removed_at.is_null()))
            .order((plugin::position.asc(), plugin::id.asc()))
            .load(conn.as_mut())
            .await?;
        let principals: HashMap<String, UserId> = user::table
            .select((user::plugin, user::id))
            .filter(user::plugin.is_not_null().and(user::deleted_at.is_null()))
            .load::<(Option<String>, UserId)>(conn.as_mut())
            .await?
            .into_iter()
            .filter_map(|(plugin, id)| plugin.map(|p| (p, id)))
            .collect();
        let previous = self.loaded();
        let mut next = Vec::with_capacity(rows.len());
        for (id, manifest, granted, mode, stored, revision) in rows {
            if let Some(same) = previous
                .iter()
                .find(|p| p.id == id && p.revision == revision)
            {
                next.push(same.clone());
                continue;
            }
            let manifest: Manifest = match serde_json::from_value(manifest) {
                Ok(manifest) => manifest,
                Err(e) => {
                    tracing::error!(
                        plugin = id,
                        "its manifest is not one this server reads: {e}"
                    );
                    continue;
                }
            };
            let bytes: Option<Vec<u8>> = plugin::table
                .select(plugin::component)
                .filter(plugin::id.eq(&id))
                .first(conn.as_mut())
                .await?;
            let Some(bytes) = bytes else {
                continue;
            };
            let engine = self.engine.clone();
            let linker = self.linker.clone();
            let compiled =
                tokio::task::spawn_blocking(move || host::prepare(&engine, &linker, &bytes))
                    .await
                    .map_err(|e| app::Error::Plugin(e.to_string()))?;
            let pre = match compiled {
                Ok(pre) => pre,
                Err(e) => {
                    tracing::error!(plugin = id, "could not compile its component: {e:#}");
                    continue;
                }
            };
            let granted = granted
                .into_iter()
                .flatten()
                .filter_map(|p| p.parse().ok())
                .collect();
            let stored = match stored {
                Value::Object(map) => map,
                _ => Map::new(),
            };
            next.push(Arc::new(LoadedPlugin {
                settings: settings::effective(&manifest.settings, &stored),
                principal: principals.get(&id).copied(),
                mode: mode.parse().unwrap_or(Mode::OptIn),
                id,
                manifest,
                granted,
                revision,
                pre,
            }));
        }
        drop(conn);
        tracing::info!(
            plugins = ?next.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
            "plugins loaded"
        );
        *self.loaded.write().expect("plugin registry") = Arc::new(next);
        self.forget(None);
        self.sync_observers(state);
        Ok(())
    }

    /// Starts a consumer for each observing plugin that lacks one at its revision, and stops
    /// those of plugins no longer on.
    fn sync_observers(&self, state: &GlobalServerContext) {
        let loaded = self.loaded();
        let mut observers = self.observers.lock().expect("plugin observers");
        observers.retain(|id, (revision, task)| {
            let keep = loaded
                .iter()
                .any(|p| &p.id == id && &p.revision == revision);
            if !keep {
                task.abort();
            }
            keep
        });
        for plugin in loaded.iter() {
            if plugin.manifest.hooks.observe.is_empty() || observers.contains_key(&plugin.id) {
                continue;
            }
            let task = tokio::spawn(super::observe::run(state.clone(), plugin.clone()));
            observers.insert(plugin.id.clone(), (plugin.revision, task.abort_handle()));
        }
    }
}

/// Loads the installed plugins, then keeps them current: on every announcement of a change,
/// and with the engine's clock ticking for the deadlines calls run against.
pub async fn start(state: &GlobalServerContext) -> app::Result<()> {
    host::start_ticker(state.plugins.engine.clone());
    state.plugins.reload(state).await?;
    let client = state.nats_context.client();
    let state = state.clone();
    tokio::spawn(async move {
        loop {
            match client.subscribe(CHANGED_SUBJECT).await {
                Ok(mut changes) => {
                    while let Some(message) = changes.next().await {
                        let change: Change =
                            serde_json::from_slice(&message.payload).unwrap_or_default();
                        match change.community {
                            Some(community) => state.plugins.forget(Some(community)),
                            None => {
                                if let Err(e) = state.plugins.reload(&state).await {
                                    tracing::error!("could not reload plugins: {e}");
                                }
                            }
                        }
                    }
                }
                Err(e) => tracing::error!("could not hear of changes to plugins: {e}"),
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
    Ok(())
}

/// Tells every API server that installed plugins changed, or with `community`, that the
/// community's use of them did.
pub async fn announce(
    client: &async_nats::Client,
    community: Option<CommunityId>,
) -> app::Result<()> {
    let payload = serde_json::to_vec(&Change { community })?;
    client
        .publish(CHANGED_SUBJECT, payload.into())
        .await
        .map_err(|e| app::Error::Plugin(e.to_string()))?;
    client
        .flush()
        .await
        .map_err(|e| app::Error::Plugin(e.to_string()))?;
    Ok(())
}
