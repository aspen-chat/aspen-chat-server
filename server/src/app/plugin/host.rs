//! The sandbox a plugin's calls run in, and the host calls it may make (`spec/plugin.wit`).
//!
//! Each call runs in a fresh instance in a store of its own, so nothing survives from one call
//! to the next but what the plugin keeps through the host. A call first waits for a place among
//! the calls running (`[plugins] concurrency` and `concurrency_per_plugin`). A store has a
//! memory ceiling (`[plugins] memory_mib`, for all its memories together), bounded instances,
//! tables, and table elements (`CallLimits`), and a deadline: the engine's epoch ticks every
//! millisecond, and at each tick a call past its deadline traps, while one within it yields to
//! the runtime, so a plugin that spins neither overruns nor holds a worker thread. The WASI interfaces a
//! component's standard library imports are provided with nothing behind them: no files,
//! sockets, environment, or arguments.
//!
//! Every host call checks the plugin's granted permissions, and every read is made as whoever
//! the call serves (`Phase`): the caller of a route, and otherwise the plugin's principal, and
//! only where the plugin runs.

use super::registry::LoadedPlugin;
use super::{PluginPermission, PluginText, annotation, principal, storage};
use crate::app::context::GlobalServerContext;
use crate::app::events::{ChannelHome, channel_home};
use crate::app::message::Message as MessageRow;
use crate::app::permissions::channel_access;
use crate::app::user::UserPg;
use crate::app::{self, AttachmentId, ChannelId, CommunityId, MessageId, UserId};
use crate::database::schema::{attachment, channel, message, message_attachment, user};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use std::collections::HashSet;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use uuid::Uuid;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Engine, Store, UpdateDeadline};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

/// The bindings `spec/plugin.wit` generates, in a module of their own so that nothing imported
/// here (Diesel's query methods) is mistaken for what they call.
mod bindings {
    wasmtime::component::bindgen!({
        path: "../spec/plugin.wit",
        world: "plugin",
        imports: { default: async },
        exports: { default: async },
    });
}

pub(super) use bindings::aspen::plugin::types as wit;
pub(super) use bindings::{Plugin, PluginPre, aspen};

/// How often the engine's epoch ticks, which is how finely deadlines are kept.
const TICK: Duration = Duration::from_millis(1);
/// The most a plugin may send or receive in one call to another host.
const MAX_FETCH_BYTES: usize = 4 << 20;
/// The most a plugin event's payload may be, in bytes of JSON.
const MAX_EVENT_PAYLOAD: usize = 16 << 10;
/// The longest a counter's key may be.
const MAX_COUNTER_KEY: usize = 200;
/// The longest a counter's window may be, in seconds.
const MAX_COUNTER_WINDOW: u32 = 7 * 24 * 60 * 60;
/// The most core and component instances one call's component may make. The examples, built
/// for `wasm32-wasip2`, make three.
const MAX_INSTANCES: usize = 16;
/// The most tables one call may hold; the examples hold two.
const MAX_TABLES: usize = 8;
/// The most memories one call may hold, whose sizes together are what `[plugins] memory_mib`
/// bounds; the examples hold one.
const MAX_MEMORIES: usize = 4;
/// The most elements one table may grow to.
const MAX_TABLE_ELEMENTS: usize = 50_000;

/// What one call may take: its memories' bytes together at most `[plugins] memory_mib`, and a
/// bounded number of instances, tables, and table elements.
struct CallLimits {
    /// The bytes its memories may still grow by.
    memory_left: usize,
}

impl wasmtime::ResourceLimiter for CallLimits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let more = desired.saturating_sub(current);
        if more > self.memory_left {
            return Ok(false);
        }
        // Counted before the memory grows; a growth that then fails stays counted, which only
        // errs toward less.
        self.memory_left -= more;
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(desired <= MAX_TABLE_ELEMENTS)
    }

    fn instances(&self) -> usize {
        MAX_INSTANCES
    }

    fn tables(&self) -> usize {
        MAX_TABLES
    }

    fn memories(&self) -> usize {
        MAX_MEMORIES
    }
}

/// The engine every plugin of this server runs on.
pub(super) fn engine() -> wasmtime::Result<Engine> {
    let mut config = wasmtime::Config::new();
    config.epoch_interruption(true);
    config.wasm_component_model(true);
    Engine::new(&config)
}

/// What every plugin is linked against: the empty WASI interfaces, and the host.
pub(super) fn linker(engine: &Engine) -> wasmtime::Result<Linker<CallState>> {
    let mut linker = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    Plugin::add_to_linker::<CallState, HasSelf<CallState>>(&mut linker, |state| state)?;
    Ok(linker)
}

/// Compiles a component and checks that it imports nothing the host does not offer and exports
/// what a plugin must.
pub(super) fn prepare(
    engine: &Engine,
    linker: &Linker<CallState>,
    bytes: &[u8],
) -> wasmtime::Result<PluginPre<CallState>> {
    let component = Component::new(engine, bytes)?;
    PluginPre::new(linker.instantiate_pre(&component)?)
}

/// Ticks the engine's epoch for as long as the process runs. Started once.
pub(super) fn start_ticker(engine: Engine) {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("plugin-epoch".into())
            .spawn(move || {
                loop {
                    std::thread::sleep(TICK);
                    engine.increment_epoch();
                }
            })
            .expect("the plugin epoch thread starts");
    });
}

/// Whom a call serves.
#[derive(Debug, Clone, Copy)]
pub(super) enum Phase {
    /// Deciding a message about to be saved: no network, and actions wait until it answers.
    Intercept,
    /// Handling something that happened.
    Observe,
    /// Answering a request: reads are made as the caller.
    Route { caller: UserId },
}

/// An action taken while intercepting, run once the hook has answered.
#[derive(Debug, Clone)]
pub(super) enum Deferred {
    Send {
        channel: ChannelId,
        content: String,
    },
    SendCard {
        channel: ChannelId,
        content: String,
        card: super::card::Card,
    },
    UpdateCard {
        message: MessageId,
        card: Option<super::card::Card>,
    },
    Delete(MessageId),
    React {
        message: MessageId,
        emoji: String,
    },
    Remove {
        community: CommunityId,
        user: UserId,
    },
    Ban {
        community: CommunityId,
        user: UserId,
        reason: Option<String>,
        seconds: Option<u64>,
    },
}

/// What one call's store holds: the empty WASI context, its limits, what host calls need, and
/// its places among the calls running (`Plugins::admit`).
pub(super) struct CallState {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: CallLimits,
    pub(super) call: Call,
    _permit: super::registry::CallPermit,
}

/// What a call's host calls need: whose call it is, whom it serves, and what it was shown.
pub(super) struct Call {
    server: GlobalServerContext,
    plugin: Arc<LoadedPlugin>,
    phase: Phase,
    /// The attachments of the records the call was shown, which it may read.
    shown: HashSet<AttachmentId>,
    /// What it asked to do while intercepting.
    pub(super) deferred: Vec<Deferred>,
}

impl WasiView for CallState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// Why a call did not answer.
#[derive(Debug)]
pub(super) struct CallFailed(pub String);

impl std::fmt::Display for CallFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A plugin instantiated for one call.
pub(super) struct Instance {
    pub(super) store: Store<CallState>,
    pub(super) bindings: Plugin,
    pub(super) deadline: Instant,
}

impl Instance {
    /// A fresh instance of `plugin` for one call, serving as `phase` says, with `budget` to
    /// answer in.
    pub(super) async fn new(
        state: &GlobalServerContext,
        plugin: Arc<LoadedPlugin>,
        phase: Phase,
        shown: HashSet<AttachmentId>,
        budget: Duration,
    ) -> Result<Self, CallFailed> {
        let memory = usize::try_from(state.config.plugins.memory_mib << 20).unwrap_or(usize::MAX);
        let deadline = Instant::now() + budget;
        // Waiting for a place counts against the call's time, so a crowd of calls fails as
        // each one's manifest says rather than queueing without end.
        let permit = state
            .plugins
            .admit(&plugin.id, deadline)
            .await
            .ok_or_else(|| CallFailed("too many plugin calls were running to start".into()))?;
        let wasi = wasmtime_wasi::WasiCtxBuilder::new()
            .allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false)
            .build();
        let pre = plugin.pre.clone();
        let mut store = Store::new(
            &state.plugins.engine,
            CallState {
                wasi,
                table: ResourceTable::new(),
                limits: CallLimits {
                    memory_left: memory,
                },
                call: Call {
                    server: state.clone(),
                    plugin,
                    phase,
                    shown,
                    deferred: Vec::new(),
                },
                _permit: permit,
            },
        );
        store.limiter(|call| &mut call.limits);
        store.set_epoch_deadline(1);
        store.epoch_deadline_callback(move |_| {
            if Instant::now() >= deadline {
                Err(wasmtime::Error::msg("ran out of time"))
            } else {
                Ok(UpdateDeadline::Yield(1))
            }
        });
        let bindings = within(deadline, pre.instantiate_async(&mut store)).await?;
        Ok(Instance {
            store,
            bindings,
            deadline,
        })
    }
}

/// `work`, cut off at `deadline` however long it waits on the host.
pub(super) async fn within<T>(
    deadline: Instant,
    work: impl Future<Output = wasmtime::Result<T>>,
) -> Result<T, CallFailed> {
    match tokio::time::timeout_at(deadline.into(), work).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(CallFailed(format!("{e:#}"))),
        Err(_) => Err(CallFailed("ran out of time".into())),
    }
}

// Converting what the host knows into what the interface says.

/// A person as a plugin is shown them, with their roles in `community`.
pub(super) async fn person(
    conn: &mut AsyncPgConnection,
    id: UserId,
    community: Option<CommunityId>,
) -> app::Result<wit::Person> {
    let row: UserPg = user::table
        .select(UserPg::as_select())
        .filter(user::id.eq(id))
        .first(conn)
        .await?;
    let roles = match community {
        Some(community) => app::role::roles_of_members(conn, &[(community, id)])
            .await?
            .remove(&(community, id))
            .unwrap_or_default(),
        None => Vec::new(),
    };
    Ok(wit::Person {
        id: id.0.to_string(),
        username: row.name,
        display_name: row.display_name,
        bot: row.bot,
        home_domain: row.home_domain.map(|d| d.to_string()),
        roles: roles.into_iter().map(|r| r.0.to_string()).collect(),
    })
}

/// Where `channel` is, as a plugin is told.
pub(super) async fn place(
    conn: &mut AsyncPgConnection,
    channel_id: ChannelId,
) -> app::Result<wit::Place> {
    let (community, parent): (Option<CommunityId>, Option<ChannelId>) = channel::table
        .select((channel::community, channel::parent_channel))
        .filter(channel::id.eq(channel_id))
        .first(conn)
        .await?;
    let community = match (community, parent) {
        (None, Some(parent)) => {
            channel::table
                .select(channel::community)
                .filter(channel::id.eq(parent))
                .first::<Option<CommunityId>>(conn)
                .await?
        }
        (community, _) => community,
    };
    Ok(wit::Place {
        channel: channel_id.0.to_string(),
        community: community.map(|c| c.0.to_string()),
        thread_of: parent.map(|p| p.0.to_string()),
        direct: community.is_none(),
    })
}

/// The records of `ids`, in the order given.
pub(super) async fn attachments(
    conn: &mut AsyncPgConnection,
    ids: &[AttachmentId],
) -> app::Result<Vec<wit::Attachment>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    type Row = (
        AttachmentId,
        String,
        String,
        Option<i32>,
        Option<i32>,
        Option<String>,
    );
    let rows: Vec<Row> = attachment::table
        .select((
            attachment::id,
            attachment::file_name,
            attachment::mime_type,
            attachment::width,
            attachment::height,
            attachment::description,
        ))
        .filter(attachment::id.eq_any(ids))
        .load(conn)
        .await?;
    Ok(ids
        .iter()
        .filter_map(|id| rows.iter().find(|r| r.0 == *id))
        .map(
            |(id, file_name, mime_type, width, height, description)| wit::Attachment {
                id: id.0.to_string(),
                file_name: file_name.clone(),
                content_type: mime_type.clone(),
                width: width.and_then(|w| u32::try_from(w).ok()),
                height: height.and_then(|h| u32::try_from(h).ok()),
                description: description.clone(),
            },
        )
        .collect())
}

/// A saved message as a plugin is shown it, with its attachments' ids.
pub(super) async fn message_record(
    conn: &mut AsyncPgConnection,
    row: &MessageRow,
) -> app::Result<(wit::Message, Vec<AttachmentId>)> {
    let ids: Vec<AttachmentId> = message_attachment::table
        .select(message_attachment::attachment_id)
        .filter(message_attachment::message_id.eq(row.id))
        .load(conn)
        .await?;
    let place = place(conn, *row.channel.id()).await?;
    let community = place
        .community
        .as_deref()
        .and_then(|c| Uuid::parse_str(c).ok())
        .map(CommunityId);
    let author = person(conn, *row.author.id(), community).await?;
    let kind = serde_json::to_value(row.kind)?
        .as_str()
        .unwrap_or_default()
        .to_string();
    Ok((
        wit::Message {
            id: row.id.0.to_string(),
            author,
            place,
            content: row.content.clone(),
            attachments: attachments(conn, &ids).await?,
            kind,
            timestamp: row.timestamp.to_rfc3339(),
            edited_at: row.edited_at.map(|t| t.to_rfc3339()),
            altered_by: row.altered_by.iter().flatten().cloned().collect(),
        },
        ids,
    ))
}

/// The id a plugin names, or why it is not one.
fn parse_id(id: &str) -> Result<Uuid, wit::Error> {
    Uuid::parse_str(id).map_err(|_| wit::Error::Invalid(format!("{id:?} is not an id")))
}

/// What a plugin is told of an app failure: refusals and absences as they are, anything else
/// as unavailable, which is logged.
pub(super) fn from_app(plugin: &str, error: app::Error) -> wit::Error {
    match error {
        app::Error::Diesel(diesel::result::Error::NotFound) => wit::Error::NotFound,
        app::Error::Forbidden(reason) => wit::Error::Denied(reason.into_owned()),
        app::Error::Unauthorized | app::Error::Blocked => wit::Error::Denied("not allowed".into()),
        app::Error::Banned { .. } | app::Error::DeploymentBanned { .. } => {
            wit::Error::Denied("banned".into())
        }
        app::Error::Validation(reason) | app::Error::Conflict(reason) => {
            wit::Error::Invalid(reason.into_owned())
        }
        app::Error::PluginRefused(reason) => wit::Error::Denied(reason.into_owned()),
        other => {
            tracing::warn!(plugin, "a host call failed: {other}");
            wit::Error::Unavailable("the server could not do this just now".into())
        }
    }
}

impl Call {
    fn require(&self, permission: PluginPermission) -> Result<(), wit::Error> {
        if self.plugin.holds(permission) {
            Ok(())
        } else {
            Err(wit::Error::Denied(format!(
                "the plugin does not hold the permission {permission}"
            )))
        }
    }

    fn fail(&self, error: app::Error) -> wit::Error {
        from_app(&self.plugin.id, error)
    }

    async fn conn(
        &self,
    ) -> Result<diesel_async::pooled_connection::deadpool::Object<AsyncPgConnection>, wit::Error>
    {
        self.server
            .connection_pool
            .get()
            .await
            .map_err(|e| self.fail(e.into()))
    }

    /// Who reads are made as: the caller of a route, and otherwise the plugin's principal.
    fn reader(&self) -> Result<UserId, wit::Error> {
        match self.phase {
            Phase::Route { caller } => Ok(caller),
            Phase::Intercept | Phase::Observe => self
                .plugin
                .principal
                .ok_or_else(|| wit::Error::Denied("the plugin has no principal to read as".into())),
        }
    }

    /// Where `channel` is, when the plugin runs there; not found otherwise, as if it were not.
    async fn running_at(
        &self,
        conn: &mut AsyncPgConnection,
        channel_id: ChannelId,
    ) -> Result<ChannelHome, wit::Error> {
        let home = channel_home(&self.server, conn, channel_id)
            .await
            .map_err(|e| self.fail(e))?;
        match self
            .server
            .plugins
            .runs_at(conn, &self.plugin.id, home)
            .await
            .map_err(|e| self.fail(e))?
        {
            Some(_) => Ok(home),
            None => Err(wit::Error::NotFound),
        }
    }

    /// While answering a route, that the caller may view `channel`; not found otherwise, as if
    /// it were not. Outside a route there is no caller to ask about.
    async fn caller_views(
        &self,
        conn: &mut AsyncPgConnection,
        channel_id: ChannelId,
    ) -> Result<(), wit::Error> {
        if let Phase::Route { caller } = self.phase {
            channel_access(&self.server, conn, caller, channel_id)
                .await
                .map_err(|_| wit::Error::NotFound)?;
        }
        Ok(())
    }

    /// Whether the plugin runs in `community`.
    async fn running_in(
        &self,
        conn: &mut AsyncPgConnection,
        community: CommunityId,
    ) -> Result<(), wit::Error> {
        let running = self
            .server
            .plugins
            .running_in_community(conn, community)
            .await
            .map_err(|e| self.fail(e))?;
        if running.iter().any(|r| r.plugin.id == self.plugin.id) {
            Ok(())
        } else {
            Err(wit::Error::NotFound)
        }
    }

    /// The scope a plugin names, checked: where the plugin runs, and while answering a route,
    /// where the caller may look.
    async fn scope(&self, scope: wit::Scope) -> Result<storage::Scope, wit::Error> {
        let mut conn = self.conn().await?;
        let scope = match scope {
            wit::Scope::Deployment => storage::Scope::Deployment,
            wit::Scope::Community(id) => {
                let community = CommunityId(parse_id(&id)?);
                self.running_in(conn.as_mut(), community).await?;
                if let Phase::Route { caller } = self.phase {
                    app::permissions::require_member(conn.as_mut(), caller, community)
                        .await
                        .map_err(|_| wit::Error::NotFound)?;
                }
                storage::Scope::Community(community)
            }
            wit::Scope::Channel(id) => {
                let channel_id = ChannelId(parse_id(&id)?);
                self.running_at(conn.as_mut(), channel_id).await?;
                if let Phase::Route { caller } = self.phase {
                    channel_access(&self.server, conn.as_mut(), caller, channel_id)
                        .await
                        .map_err(|_| wit::Error::NotFound)?;
                }
                storage::Scope::Channel(channel_id)
            }
            wit::Scope::User(id) => {
                let user = UserId(parse_id(&id)?);
                if let Phase::Route { caller } = self.phase
                    && caller != user
                {
                    return Err(wit::Error::NotFound);
                }
                storage::Scope::User(user)
            }
        };
        Ok(scope)
    }

    /// The message `id`, when it is where the plugin runs and whoever reads may read it.
    async fn readable_message(&self, id: MessageId) -> Result<MessageRow, wit::Error> {
        let reader = self.reader()?;
        let mut conn = self.conn().await?;
        let row: MessageRow = message::table
            .select(MessageRow::as_select())
            .filter(message::id.eq(id).and(message::deleted_at.is_null()))
            .first(conn.as_mut())
            .await
            .map_err(|_| wit::Error::NotFound)?;
        self.running_at(conn.as_mut(), *row.channel.id()).await?;
        channel_access(&self.server, conn.as_mut(), reader, *row.channel.id())
            .await
            .map_err(|_| wit::Error::NotFound)?;
        Ok(row)
    }

    /// Runs `action` as the principal now, or after the hook answers while intercepting.
    async fn act(&mut self, action: Deferred) -> Result<Option<MessageId>, wit::Error> {
        self.require(PluginPermission::Act)?;
        if self.plugin.principal.is_none() {
            return Err(wit::Error::Denied("the plugin has no principal".into()));
        }
        if let Phase::Intercept = self.phase {
            self.deferred.push(action);
            return Ok(None);
        }
        principal::act(&self.server, &self.plugin, action)
            .await
            .map_err(|e| self.fail(e))
    }
}

impl aspen::plugin::types::Host for CallState {}

impl Call {
    async fn settings(&mut self) -> String {
        serde_json::to_string(&self.plugin.settings).unwrap_or_else(|_| "{}".into())
    }

    async fn log(&mut self, level: wit::Level, message: String) {
        let message: String = message.chars().take(2000).collect();
        let plugin = self.plugin.id.as_str();
        match level {
            wit::Level::Debug => tracing::debug!(plugin, "{message}"),
            wit::Level::Info => tracing::info!(plugin, "{message}"),
            wit::Level::Warn => tracing::warn!(plugin, "{message}"),
            wit::Level::Error => tracing::error!(plugin, "{message}"),
        }
    }

    async fn counter_add(
        &mut self,
        key: String,
        window_seconds: u32,
        amount: u32,
    ) -> Result<u64, wit::Error> {
        if key.is_empty() || key.len() > MAX_COUNTER_KEY {
            return Err(wit::Error::Invalid(format!(
                "a counter's key is 1 to {MAX_COUNTER_KEY} bytes"
            )));
        }
        if !(1..=MAX_COUNTER_WINDOW).contains(&window_seconds) {
            return Err(wit::Error::Invalid(format!(
                "a counter's window is 1 to {MAX_COUNTER_WINDOW} seconds"
            )));
        }
        storage::count(&self.server, &self.plugin.id, &key, window_seconds, amount)
            .await
            .map_err(|e| self.fail(e))
    }

    async fn annotate_message(
        &mut self,
        message_id: String,
        annotation: wit::Annotation,
    ) -> Result<(), wit::Error> {
        self.require(PluginPermission::MessagesAnnotate)?;
        let id = MessageId(parse_id(&message_id)?);
        let annotation = annotation::Annotation::try_from(annotation)?;
        let mut conn = self.conn().await?;
        let channel_id: ChannelId = message::table
            .select(message::channel)
            .filter(message::id.eq(id).and(message::deleted_at.is_null()))
            .first(conn.as_mut())
            .await
            .map_err(|_| wit::Error::NotFound)?;
        self.running_at(conn.as_mut(), channel_id).await?;
        if let Phase::Route { caller } = self.phase {
            channel_access(&self.server, conn.as_mut(), caller, channel_id)
                .await
                .map_err(|_| wit::Error::NotFound)?;
        }
        annotation::set_on_message(&self.server, conn.as_mut(), &self.plugin.id, id, annotation)
            .await
            .map_err(|e| self.fail(e))
    }

    async fn clear_message_annotation(
        &mut self,
        message_id: String,
        kind: String,
    ) -> Result<(), wit::Error> {
        self.require(PluginPermission::MessagesAnnotate)?;
        let id = MessageId(parse_id(&message_id)?);
        let mut conn = self.conn().await?;
        annotation::clear_on_message(&self.server, conn.as_mut(), &self.plugin.id, id, &kind)
            .await
            .map_err(|e| self.fail(e))
    }

    async fn annotate_user(
        &mut self,
        user_id: String,
        annotation: wit::Annotation,
    ) -> Result<(), wit::Error> {
        self.require(PluginPermission::UsersAnnotate)?;
        let id = UserId(parse_id(&user_id)?);
        let annotation = annotation::Annotation::try_from(annotation)?;
        let mut conn = self.conn().await?;
        annotation::set_on_user(&self.server, conn.as_mut(), &self.plugin.id, id, annotation)
            .await
            .map_err(|e| self.fail(e))
    }

    async fn clear_user_annotation(
        &mut self,
        user_id: String,
        kind: String,
    ) -> Result<(), wit::Error> {
        self.require(PluginPermission::UsersAnnotate)?;
        let id = UserId(parse_id(&user_id)?);
        let mut conn = self.conn().await?;
        annotation::clear_on_user(&self.server, conn.as_mut(), &self.plugin.id, id, &kind)
            .await
            .map_err(|e| self.fail(e))
    }

    async fn read_attachment(&mut self, attachment_id: String) -> Result<Vec<u8>, wit::Error> {
        self.require(PluginPermission::AttachmentsRead)?;
        let id = AttachmentId(parse_id(&attachment_id)?);
        let mut conn = self.conn().await?;
        let allowed = match self.phase {
            Phase::Route { .. } => {
                // A route reads what the caller may: an attachment of a message they may read.
                let messages: Vec<MessageId> = message_attachment::table
                    .select(message_attachment::message_id)
                    .filter(message_attachment::attachment_id.eq(id))
                    .load(conn.as_mut())
                    .await
                    .map_err(|e| self.fail(e.into()))?;
                drop(conn);
                let mut found = false;
                for message_id in messages {
                    if self.readable_message(message_id).await.is_ok() {
                        found = true;
                        break;
                    }
                }
                conn = self.conn().await?;
                found
            }
            Phase::Intercept | Phase::Observe => self.shown.contains(&id),
        };
        if !allowed {
            return Err(wit::Error::NotFound);
        }
        let key: String = attachment::table
            .select(attachment::storage_key)
            .filter(
                attachment::id
                    .eq(id)
                    .and(attachment::ready_at.is_not_null()),
            )
            .first(conn.as_mut())
            .await
            .map_err(|_| wit::Error::NotFound)?;
        drop(conn);
        // Read no more than the plugin may take: an object storage says is larger is refused
        // before any of it is read, and one that runs past the limit as it is read is dropped.
        let limit = self.plugin.manifest.attachment_limit.unwrap_or(0);
        let mut bytes = Vec::new();
        let read = self
            .server
            .media_store
            .copy_object_to(&key, limit, &mut bytes)
            .await
            .map_err(|e| self.fail(e))?;
        if read.is_none() {
            return Err(wit::Error::Limit(format!(
                "the attachment is larger than the plugin's attachmentLimit, {limit} bytes"
            )));
        }
        Ok(bytes)
    }

    async fn read_message(&mut self, message_id: String) -> Result<wit::Message, wit::Error> {
        self.require(PluginPermission::MessagesRead)?;
        let id = MessageId(parse_id(&message_id)?);
        let row = self.readable_message(id).await?;
        let mut conn = self.conn().await?;
        let (record, ids) = message_record(conn.as_mut(), &row)
            .await
            .map_err(|e| self.fail(e))?;
        self.shown.extend(ids);
        Ok(record)
    }

    async fn fetch(&mut self, request: wit::HttpRequest) -> Result<wit::HttpResponse, wit::Error> {
        self.require(PluginPermission::Network)?;
        if let Phase::Intercept = self.phase {
            return Err(wit::Error::Denied(
                "a plugin calls no other host while intercepting".into(),
            ));
        }
        let url = url::Url::parse(&request.url)
            .map_err(|_| wit::Error::Invalid(format!("{:?} is not a URL", request.url)))?;
        let host = url.host_str().unwrap_or_default();
        if url.scheme() != "https" || !self.plugin.manifest.hosts.iter().any(|h| h == host) {
            return Err(wit::Error::Denied(format!(
                "{host} is not among the plugin's hosts, or the URL is not https"
            )));
        }
        // An address is connected to without the client's resolver, which refuses the inside
        // of a network only for names.
        if crate::app::outbound::names_inside_address(&url) {
            return Err(wit::Error::Denied(format!(
                "{host} is an address inside a network, which plugins do not call"
            )));
        }
        if request.body.len() > MAX_FETCH_BYTES {
            return Err(wit::Error::Limit(format!(
                "a request body is at most {MAX_FETCH_BYTES} bytes"
            )));
        }
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|_| wit::Error::Invalid(format!("{:?} is not a method", request.method)))?;
        let mut outgoing = super::route::outbound_client()
            .request(method, url)
            .body(request.body);
        for (name, value) in request.headers {
            if !name.eq_ignore_ascii_case("host") {
                outgoing = outgoing.header(name, value);
            }
        }
        let response = outgoing
            .send()
            .await
            .map_err(|e| wit::Error::Unavailable(format!("{e}")))?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|v| (name.as_str().to_string(), v.to_string()))
            })
            .collect();
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| wit::Error::Unavailable(format!("{e}")))?;
            if body.len() + chunk.len() > MAX_FETCH_BYTES {
                return Err(wit::Error::Limit(format!(
                    "a response is at most {MAX_FETCH_BYTES} bytes"
                )));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(wit::HttpResponse {
            status,
            headers,
            body,
        })
    }

    async fn storage_get(
        &mut self,
        scope: wit::Scope,
        key: String,
    ) -> Result<Option<Vec<u8>>, wit::Error> {
        self.require(PluginPermission::Storage)?;
        let scope = self.scope(scope).await?;
        let mut conn = self.conn().await?;
        storage::get(conn.as_mut(), &self.plugin.id, &scope, &key)
            .await
            .map_err(|e| self.fail(e))
    }

    async fn storage_set(
        &mut self,
        scope: wit::Scope,
        key: String,
        value: Vec<u8>,
    ) -> Result<(), wit::Error> {
        self.require(PluginPermission::Storage)?;
        let scope = self.scope(scope).await?;
        let quota = self.plugin.manifest.storage_quota.unwrap_or(0);
        let mut conn = self.conn().await?;
        storage::set(conn.as_mut(), &self.plugin.id, quota, &scope, &key, &value).await
    }

    async fn storage_delete(&mut self, scope: wit::Scope, key: String) -> Result<(), wit::Error> {
        self.require(PluginPermission::Storage)?;
        let scope = self.scope(scope).await?;
        let mut conn = self.conn().await?;
        storage::delete(conn.as_mut(), &self.plugin.id, &scope, &key)
            .await
            .map_err(|e| self.fail(e))
    }

    async fn storage_list(
        &mut self,
        scope: wit::Scope,
        prefix: String,
        after: Option<String>,
        limit: u32,
    ) -> Result<Vec<(String, Vec<u8>)>, wit::Error> {
        self.require(PluginPermission::Storage)?;
        let scope = self.scope(scope).await?;
        let mut conn = self.conn().await?;
        storage::list(
            conn.as_mut(),
            &self.plugin.id,
            &scope,
            &prefix,
            after.as_deref(),
            limit.min(storage::MAX_LIST),
        )
        .await
        .map_err(|e| self.fail(e))
    }

    async fn publish(
        &mut self,
        audience: wit::Audience,
        kind: String,
        payload: String,
    ) -> Result<(), wit::Error> {
        self.require(PluginPermission::Events)?;
        if kind.is_empty()
            || kind.len() > 64
            || !kind
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        {
            return Err(wit::Error::Invalid(
                "an event's kind is 1 to 64 letters, digits, dots, hyphens, and underscores".into(),
            ));
        }
        if payload.len() > MAX_EVENT_PAYLOAD {
            return Err(wit::Error::Limit(format!(
                "an event's payload is at most {MAX_EVENT_PAYLOAD} bytes"
            )));
        }
        let payload: serde_json::Value = serde_json::from_str(&payload)
            .map_err(|_| wit::Error::Invalid("an event's payload is JSON".into()))?;
        let mut conn = self.conn().await?;
        let target = match audience {
            // While answering a route, only where the caller may look, as its reads are.
            wit::Audience::Channel(id) => {
                let channel_id = ChannelId(parse_id(&id)?);
                self.running_at(conn.as_mut(), channel_id).await?;
                self.caller_views(conn.as_mut(), channel_id).await?;
                super::Target::Channel(channel_id)
            }
            wit::Audience::Community(id) => {
                let community = CommunityId(parse_id(&id)?);
                self.running_in(conn.as_mut(), community).await?;
                if let Phase::Route { caller } = self.phase {
                    app::permissions::require_member(conn.as_mut(), caller, community)
                        .await
                        .map_err(|_| wit::Error::NotFound)?;
                }
                super::Target::Community(community)
            }
            wit::Audience::User(id) => {
                let user = UserId(parse_id(&id)?);
                if let Phase::Route { caller } = self.phase
                    && caller != user
                {
                    return Err(wit::Error::Denied(
                        "a route publishes to its caller alone among users".into(),
                    ));
                }
                super::Target::User(user)
            }
        };
        super::publish(
            &self.server,
            conn.as_mut(),
            &self.plugin.id,
            kind,
            target,
            payload,
        )
        .await
        .map_err(|e| self.fail(e))
    }

    async fn send_message(
        &mut self,
        channel_id: String,
        content: String,
    ) -> Result<Option<String>, wit::Error> {
        let channel = ChannelId(parse_id(&channel_id)?);
        {
            let mut conn = self.conn().await?;
            self.running_at(conn.as_mut(), channel).await?;
        }
        self.act(Deferred::Send { channel, content })
            .await
            .map(|id| id.map(|id| id.0.to_string()))
    }

    async fn delete_message(&mut self, message_id: String) -> Result<(), wit::Error> {
        let id = MessageId(parse_id(&message_id)?);
        self.act(Deferred::Delete(id)).await.map(|_| ())
    }

    async fn add_reaction(&mut self, message_id: String, emoji: String) -> Result<(), wit::Error> {
        let message = MessageId(parse_id(&message_id)?);
        self.act(Deferred::React { message, emoji })
            .await
            .map(|_| ())
    }

    async fn remove_member(&mut self, community: String, user: String) -> Result<(), wit::Error> {
        let community = CommunityId(parse_id(&community)?);
        let user = UserId(parse_id(&user)?);
        self.act(Deferred::Remove { community, user })
            .await
            .map(|_| ())
    }

    async fn ban_member(
        &mut self,
        community: String,
        user: String,
        reason: Option<String>,
        seconds: Option<u64>,
    ) -> Result<(), wit::Error> {
        let community = CommunityId(parse_id(&community)?);
        let user = UserId(parse_id(&user)?);
        self.act(Deferred::Ban {
            community,
            user,
            reason,
            seconds,
        })
        .await
        .map(|_| ())
    }

    async fn place_of(&mut self, channel_id: String) -> Result<wit::Place, wit::Error> {
        let channel_id = ChannelId(parse_id(&channel_id)?);
        let mut conn = self.conn().await?;
        self.running_at(conn.as_mut(), channel_id).await?;
        if let Phase::Route { caller } = self.phase {
            channel_access(&self.server, conn.as_mut(), caller, channel_id)
                .await
                .map_err(|_| wit::Error::NotFound)?;
        }
        place(conn.as_mut(), channel_id)
            .await
            .map_err(|e| self.fail(e))
    }

    async fn community_settings(&mut self, community: String) -> Result<String, wit::Error> {
        let community = CommunityId(parse_id(&community)?);
        let mut conn = self.conn().await?;
        if let Phase::Route { caller } = self.phase {
            app::permissions::require_member(conn.as_mut(), caller, community)
                .await
                .map_err(|_| wit::Error::NotFound)?;
        }
        let running = self
            .server
            .plugins
            .running_in_community(conn.as_mut(), community)
            .await
            .map_err(|e| self.fail(e))?;
        let found = running
            .into_iter()
            .find(|r| r.plugin.id == self.plugin.id)
            .ok_or(wit::Error::NotFound)?;
        Ok(
            serde_json::to_string(&found.community_settings.unwrap_or_default())
                .unwrap_or_else(|_| "{}".into()),
        )
    }

    async fn caller_may(
        &mut self,
        channel_id: String,
        permission: String,
    ) -> Result<bool, wit::Error> {
        let channel_id = ChannelId(parse_id(&channel_id)?);
        let permission: crate::app::permissions::Permission = permission
            .parse()
            .map_err(|_| wit::Error::Invalid(format!("{permission:?} is not a permission")))?;
        let reader = self.reader()?;
        let mut conn = self.conn().await?;
        self.running_at(conn.as_mut(), channel_id).await?;
        Ok(
            match channel_access(&self.server, conn.as_mut(), reader, channel_id).await {
                Ok(access) => access.has(permission.bits()),
                Err(_) => false,
            },
        )
    }

    async fn set_timer(
        &mut self,
        key: String,
        due: String,
        payload: String,
    ) -> Result<(), wit::Error> {
        self.require(PluginPermission::Timers)?;
        let mut conn = self.conn().await?;
        super::timer::set(conn.as_mut(), &self.plugin.id, &key, &due, &payload).await
    }

    async fn cancel_timer(&mut self, key: String) -> Result<(), wit::Error> {
        self.require(PluginPermission::Timers)?;
        let mut conn = self.conn().await?;
        super::timer::cancel(conn.as_mut(), &self.plugin.id, &key)
            .await
            .map_err(|e| self.fail(e))
    }

    async fn notify(
        &mut self,
        user: String,
        channel_id: String,
        text: wit::Text,
        message_id: Option<String>,
    ) -> Result<bool, wit::Error> {
        self.require(PluginPermission::Notify)?;
        let user = UserId(parse_id(&user)?);
        let channel_id = ChannelId(parse_id(&channel_id)?);
        let message_id = message_id
            .map(|m| parse_id(&m).map(MessageId))
            .transpose()?;
        let text = PluginText::from(text);
        if text.key.is_empty() || text.key.len() > 64 {
            return Err(wit::Error::Invalid("a text's key is 1 to 64 bytes".into()));
        }
        {
            let mut conn = self.conn().await?;
            self.running_at(conn.as_mut(), channel_id).await?;
            self.caller_views(conn.as_mut(), channel_id).await?;
        }
        super::notice::notify(
            &self.server,
            &self.plugin.id,
            user,
            channel_id,
            text,
            message_id,
        )
        .await
        .map_err(|e| self.fail(e))
    }

    async fn capability_path(&mut self, name: String) -> Result<String, wit::Error> {
        self.require(PluginPermission::Capabilities)?;
        let Phase::Route { caller } = self.phase else {
            return Err(wit::Error::Denied(
                "a capability is given to the caller of a route".into(),
            ));
        };
        let mut conn = self.conn().await?;
        super::capability::path(conn.as_mut(), &self.plugin.id, caller, &name)
            .await
            .map_err(|e| self.fail(e))
    }

    async fn revoke_capability(&mut self, name: String) -> Result<(), wit::Error> {
        self.require(PluginPermission::Capabilities)?;
        let Phase::Route { caller } = self.phase else {
            return Err(wit::Error::Denied(
                "a capability is revoked for the caller of a route".into(),
            ));
        };
        let mut conn = self.conn().await?;
        super::capability::revoke(conn.as_mut(), &self.plugin.id, caller, &name)
            .await
            .map_err(|e| self.fail(e))
    }

    async fn send_card(
        &mut self,
        channel_id: String,
        content: String,
        card: wit::Card,
    ) -> Result<Option<String>, wit::Error> {
        let channel = ChannelId(parse_id(&channel_id)?);
        let card = super::card::Card::from_wit(&self.plugin.id, card)?;
        {
            let mut conn = self.conn().await?;
            self.running_at(conn.as_mut(), channel).await?;
        }
        self.act(Deferred::SendCard {
            channel,
            content,
            card,
        })
        .await
        .map(|id| id.map(|id| id.0.to_string()))
    }

    async fn update_card(
        &mut self,
        message_id: String,
        card: Option<wit::Card>,
    ) -> Result<(), wit::Error> {
        let message = MessageId(parse_id(&message_id)?);
        let card = card
            .map(|card| super::card::Card::from_wit(&self.plugin.id, card))
            .transpose()?;
        self.act(Deferred::UpdateCard { message, card })
            .await
            .map(|_| ())
    }
}

impl aspen::plugin::host::Host for CallState {
    async fn settings(&mut self) -> String {
        self.call.settings().await
    }

    async fn log(&mut self, level: wit::Level, message: String) {
        self.call.log(level, message).await
    }

    async fn counter_add(
        &mut self,
        key: String,
        window_seconds: u32,
        amount: u32,
    ) -> Result<u64, wit::Error> {
        self.call.counter_add(key, window_seconds, amount).await
    }

    async fn annotate_message(
        &mut self,
        message_id: String,
        annotation: wit::Annotation,
    ) -> Result<(), wit::Error> {
        self.call.annotate_message(message_id, annotation).await
    }

    async fn clear_message_annotation(
        &mut self,
        message_id: String,
        kind: String,
    ) -> Result<(), wit::Error> {
        self.call.clear_message_annotation(message_id, kind).await
    }

    async fn annotate_user(
        &mut self,
        user_id: String,
        annotation: wit::Annotation,
    ) -> Result<(), wit::Error> {
        self.call.annotate_user(user_id, annotation).await
    }

    async fn clear_user_annotation(
        &mut self,
        user_id: String,
        kind: String,
    ) -> Result<(), wit::Error> {
        self.call.clear_user_annotation(user_id, kind).await
    }

    async fn read_attachment(&mut self, attachment_id: String) -> Result<Vec<u8>, wit::Error> {
        self.call.read_attachment(attachment_id).await
    }

    async fn read_message(&mut self, message_id: String) -> Result<wit::Message, wit::Error> {
        self.call.read_message(message_id).await
    }

    async fn fetch(&mut self, request: wit::HttpRequest) -> Result<wit::HttpResponse, wit::Error> {
        self.call.fetch(request).await
    }

    async fn storage_get(
        &mut self,
        scope: wit::Scope,
        key: String,
    ) -> Result<Option<Vec<u8>>, wit::Error> {
        self.call.storage_get(scope, key).await
    }

    async fn storage_set(
        &mut self,
        scope: wit::Scope,
        key: String,
        value: Vec<u8>,
    ) -> Result<(), wit::Error> {
        self.call.storage_set(scope, key, value).await
    }

    async fn storage_delete(&mut self, scope: wit::Scope, key: String) -> Result<(), wit::Error> {
        self.call.storage_delete(scope, key).await
    }

    async fn storage_list(
        &mut self,
        scope: wit::Scope,
        prefix: String,
        after: Option<String>,
        limit: u32,
    ) -> Result<Vec<(String, Vec<u8>)>, wit::Error> {
        self.call.storage_list(scope, prefix, after, limit).await
    }

    async fn publish(
        &mut self,
        audience: wit::Audience,
        kind: String,
        payload: String,
    ) -> Result<(), wit::Error> {
        self.call.publish(audience, kind, payload).await
    }

    async fn send_message(
        &mut self,
        channel_id: String,
        content: String,
    ) -> Result<Option<String>, wit::Error> {
        self.call.send_message(channel_id, content).await
    }

    async fn delete_message(&mut self, message_id: String) -> Result<(), wit::Error> {
        self.call.delete_message(message_id).await
    }

    async fn add_reaction(&mut self, message_id: String, emoji: String) -> Result<(), wit::Error> {
        self.call.add_reaction(message_id, emoji).await
    }

    async fn remove_member(&mut self, community: String, user: String) -> Result<(), wit::Error> {
        self.call.remove_member(community, user).await
    }

    async fn ban_member(
        &mut self,
        community: String,
        user: String,
        reason: Option<String>,
        seconds: Option<u64>,
    ) -> Result<(), wit::Error> {
        self.call.ban_member(community, user, reason, seconds).await
    }

    async fn caller_may(
        &mut self,
        channel: String,
        permission: String,
    ) -> Result<bool, wit::Error> {
        self.call.caller_may(channel, permission).await
    }

    async fn place_of(&mut self, channel: String) -> Result<wit::Place, wit::Error> {
        self.call.place_of(channel).await
    }

    async fn community_settings(&mut self, community: String) -> Result<String, wit::Error> {
        self.call.community_settings(community).await
    }

    async fn set_timer(
        &mut self,
        key: String,
        due: String,
        payload: String,
    ) -> Result<(), wit::Error> {
        self.call.set_timer(key, due, payload).await
    }

    async fn cancel_timer(&mut self, key: String) -> Result<(), wit::Error> {
        self.call.cancel_timer(key).await
    }

    async fn notify(
        &mut self,
        user: String,
        channel: String,
        text: wit::Text,
        message: Option<String>,
    ) -> Result<bool, wit::Error> {
        self.call.notify(user, channel, text, message).await
    }

    async fn capability_path(&mut self, name: String) -> Result<String, wit::Error> {
        self.call.capability_path(name).await
    }

    async fn revoke_capability(&mut self, name: String) -> Result<(), wit::Error> {
        self.call.revoke_capability(name).await
    }

    async fn send_card(
        &mut self,
        channel: String,
        content: String,
        card: wit::Card,
    ) -> Result<Option<String>, wit::Error> {
        self.call.send_card(channel, content, card).await
    }

    async fn update_card(
        &mut self,
        message: String,
        card: Option<wit::Card>,
    ) -> Result<(), wit::Error> {
        self.call.update_card(message, card).await
    }
}

impl From<&PluginText> for wit::Text {
    fn from(text: &PluginText) -> Self {
        wit::Text {
            key: text.key.clone(),
            args: text
                .args
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        }
    }
}

impl From<wit::Text> for PluginText {
    fn from(text: wit::Text) -> Self {
        PluginText {
            key: text.key,
            args: text.args.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime::ResourceLimiter;

    #[test]
    fn a_calls_memories_share_one_ceiling() {
        let mut limits = CallLimits { memory_left: 100 };
        assert!(limits.memory_growing(0, 60, None).unwrap());
        // A second memory draws on what the first left.
        assert!(!limits.memory_growing(0, 60, None).unwrap());
        assert!(limits.memory_growing(0, 40, None).unwrap());
        assert!(!limits.memory_growing(40, 41, None).unwrap());
        assert!(
            !limits
                .table_growing(0, MAX_TABLE_ELEMENTS + 1, None)
                .unwrap()
        );
    }

    /// The example plugins, once built for `wasm32-wasip2` (as CI builds them), instantiate
    /// within a call's limits; a plugin not built is passed over.
    #[tokio::test]
    async fn the_example_plugins_fit_a_calls_limits() {
        let engine = engine().unwrap();
        for name in ["word_filter", "forum", "calendar"] {
            let path = format!(
                "{}/../plugins/{name}/target/wasm32-wasip2/release/aspen_{name}.wasm",
                env!("CARGO_MANIFEST_DIR")
            );
            let Ok(bytes) = std::fs::read(&path) else {
                eprintln!("{name} is not built; passing it over");
                continue;
            };
            let component = Component::new(&engine, &bytes).unwrap();
            let mut linker: Linker<CallLimits> = Linker::new(&engine);
            linker.define_unknown_imports_as_traps(&component).unwrap();
            let mut store = Store::new(
                &engine,
                CallLimits {
                    memory_left: 64 << 20,
                },
            );
            store.limiter(|limits| limits);
            store.set_epoch_deadline(u64::MAX / 2);
            linker
                .instantiate_async(&mut store, &component)
                .await
                .unwrap_or_else(|e| panic!("{name} does not fit a call's limits: {e:#}"));
        }
    }
}
