//! The state every request handler and background task shares, and the background tasks
//! started with it.

use crate::ASPEN_NATS_STREAM_NAME;
use crate::aspen_config::{AspenConfig, load_config};
use async_nats::jetstream::stream::{ConsumerLimits, DiscardPolicy, StorageType};
use diesel_async::{AsyncPgConnection, pooled_connection::deadpool::Pool};
use fred::prelude::ClientLike;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// What an API server is started as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// Serves the deployment at `public_url`: the API, its event streams, the web client, and
    /// the pages beside them, and does its share of the background work.
    Public,
    /// Opens no listening socket and does only the background work every API server shares (mail
    /// and digests, the voice report listener, standing checks, plugins' observers and timers,
    /// push), so it needs no web client, and its event feed reads nothing.
    PrivateWorker,
}

#[derive(Clone)]
pub struct GlobalServerContext {
    pub connection_pool: Pool<AsyncPgConnection>,
    pub nats_context: Arc<async_nats::jetstream::Context>,
    pub valkey: fred::clients::Client,
    pub media_store: Arc<crate::media_store::MediaStore>,
    pub config: Arc<AspenConfig>,
    pub rate_limiter: Arc<crate::rate_limit::RateLimiter>,
    /// Where each channel belongs (`app::events::channel_home`), filled as it is asked; a
    /// channel never moves.
    pub channel_homes: Arc<Mutex<HashMap<crate::ChannelId, crate::events::ChannelHome>>>,
    /// Each channel's recent count of who is online in it (`app::channel_presence`).
    pub channel_presence: Arc<crate::recent::Recent<crate::ChannelId, u32>>,
    /// Each community's recent set of members with a connection (`app::user_status`).
    pub connected_members:
        Arc<crate::recent::Recent<crate::CommunityId, Arc<HashSet<crate::UserId>>>>,
    /// The server's one reading of the event stream, which every event stream connection
    /// registers with.
    pub event_feed: crate::event_feed::EventFeed,
    /// What every call to another deployment is made with (`app::federation::fetch`).
    pub federation_client: reqwest::Client,
    /// What every push to a phone's push service is made with (`app::push::client`).
    pub push_client: reqwest::Client,
    /// What mailed codes are kept as digests under (`app::server_secret`).
    pub code_key: crate::server_secret::CodeKey,
    /// What join tokens are signed with (`app::server_secret`).
    pub join_token_key: crate::server_secret::JoinTokenKey,
    /// This server's copy of the deployment's settings (`app::deployment_settings`).
    pub settings: crate::deployment_settings::SettingsCache,
    /// The plugins this server runs (`app::plugin`).
    pub plugins: Arc<crate::plugin::Plugins>,
    /// What this server sends mail with, when `[email]` is configured (`app::email`).
    pub mailer: Option<Arc<crate::email::Mailer>>,
}

impl GlobalServerContext {
    /// The deployment's settings as this server last read them.
    pub fn settings(&self) -> std::sync::Arc<crate::deployment_settings::DeploymentSettings> {
        self.settings.current()
    }
}

impl GlobalServerContext {
    /// Connects to everything the server needs. `routes` are the API's routes, which the rate
    /// limits are checked against.
    pub async fn new(
        routes: &[crate::rate_limit::Route],
        role: Role,
    ) -> Result<Self, crate::Error> {
        let config = load_config()?;
        crate::login::configure_password_work(
            config.auth.password_hashing_threads,
            Duration::from_secs(config.auth.password_hashing_wait_seconds),
        );
        let rate_limiter = crate::rate_limit::RateLimiter::compile(&config.rate_limits, routes)
            .map_err(|message| crate::Error::Config(config::ConfigError::Message(message)))?;
        let client = config.connect_nats().await?;
        aspen_tls::warn_if_nats_unencrypted(&config.nats_url, config.nats.tls.as_ref(), &client);
        aspen_limits::suspension::watch(client.clone(), rate_limiter.suspension().clone(), "api");
        let context = async_nats::jetstream::new(client);
        context
            .create_or_update_stream(async_nats::jetstream::stream::Config {
                name: ASPEN_NATS_STREAM_NAME.to_string(),
                subjects: vec![format!("{}.>", crate::events::SUBJECT_ROOT)],
                discard: DiscardPolicy::Old,
                max_messages: 1_000_000_000,
                max_bytes: 8 * 1024 * 1024 * 1024,
                max_age: crate::event_feed::MAX_EVENT_AGE,
                storage: StorageType::Memory,
                consumer_limits: Some(ConsumerLimits {
                    max_ack_pending: 1000,
                    inactive_threshold: Duration::from_secs(60),
                }),
                ..Default::default()
            })
            .await?;
        let mut valkey_config = fred::prelude::Config::from_url(&config.valkey_url)?;
        // A `rediss://` URL alone trusts the system's authorities; `check_valkey` refused
        // `[valkey.tls]` without one.
        if let Some(tls) = &config.valkey.tls {
            let client = tls
                .rustls_client_config("valkey.tls")
                .map_err(|message| crate::Error::Config(config::ConfigError::Message(message)))?;
            valkey_config.tls = Some(client.into());
        }
        // Commands are small and many are in flight at once; with Nagle's algorithm on, one sent
        // while another is unacknowledged waits for Valkey's delayed ACK.
        let valkey_connection = fred::types::config::ConnectionConfig {
            tcp: fred::types::config::TcpConfig {
                nodelay: Some(true),
                ..Default::default()
            },
            ..Default::default()
        };
        let valkey =
            fred::prelude::Client::new(valkey_config.clone(), None, Some(valkey_connection), None);
        valkey.init().await?;

        let media_store = Arc::new(crate::media_store::MediaStore::new(&config).await?);
        // Built now so that a `public_url` no relying party can be made for stops the server as
        // it starts.
        crate::passkey::relying_party(&config, crate::deployment_settings::DEFAULT_NAME)?;
        let federation_client = crate::federation::fetch::client(&config.federation)?;
        let push_client = crate::push::client(&config.federation)?;
        let mailer = config
            .email
            .as_ref()
            .map(|email| crate::email::Mailer::new(email, &config.public_url))
            .transpose()?
            .map(Arc::new);
        let connection_pool = {
            let database: crate::database::Database = config
                .database_url
                .parse()
                .map_err(|e: crate::database::Error| config::ConfigError::Message(e.to_string()))?;
            let pool = Pool::builder(database.manager())
                .runtime(::deadpool::Runtime::Tokio1)
                .wait_timeout(Some(Duration::from_secs(config.database_pool_wait_seconds)));
            match config.database_pool_size {
                Some(size) => pool.max_size(size),
                None => pool,
            }
            .build()?
        };
        let (settings, code_key, join_token_key) = {
            let mut conn = connection_pool.get().await?;
            crate::deployment_settings::pin_domain(
                conn.as_mut(),
                crate::federation::own_domain(&config.federation).as_ref(),
            )
            .await?;
            (
                crate::deployment_settings::load(conn.as_mut()).await?,
                crate::server_secret::CodeKey::load(conn.as_mut()).await?,
                crate::server_secret::JoinTokenKey::load(conn.as_mut()).await?,
            )
        };

        Ok(Self {
            channel_homes: Arc::new(Mutex::new(HashMap::new())),
            channel_presence: Arc::default(),
            connected_members: Arc::default(),
            connection_pool,
            event_feed: match role {
                Role::Public => crate::event_feed::EventFeed::start(
                    context.clone(),
                    config.event_queue_size,
                    config.event_feed_shards,
                    crate::event_feed::StreamCaps::new(&config.limits),
                ),
                Role::PrivateWorker => crate::event_feed::EventFeed::idle(),
            },
            nats_context: Arc::new(context),
            valkey,
            media_store,
            rate_limiter: Arc::new(rate_limiter),
            federation_client,
            push_client,
            code_key,
            join_token_key,
            settings: crate::deployment_settings::SettingsCache::new(settings),
            plugins: Arc::new(crate::plugin::Plugins::new(&config.plugins)?),
            mailer,
            config: config.into(),
        })
    }
}

/// Starts the app's background tasks: the settings watcher, the poll closer, the voice report
/// listener and reaper, the fleet heartbeat, the federation standing confirmer, the push
/// dispatcher, the mail sender and digest scheduler, the attachment preview maker and held
/// message releaser, the sweeper of staging uploads, the mover of evidence off the public read
/// path, and the plugins with their observers,
/// making the federation and push keys where they are missing.
pub async fn start_background_tasks(context: &GlobalServerContext) -> Result<(), crate::Error> {
    crate::deployment_settings::spawn_watcher(context.clone());
    crate::poll::spawn_closer(context.clone());
    crate::voice::spawn_report_listener(context.clone()).await?;
    crate::voice::spawn_token_key_answerer(context.clone()).await?;
    crate::voice::spawn_reaper(context.clone());
    crate::fleet::spawn_heartbeat(context.clone());
    if context.config.federation.domain.is_some() {
        crate::federation::ensure_key(context.connection_pool.get().await?.as_mut()).await?;
    }
    crate::federation::standing::spawn_confirmer(context.clone());
    crate::push::ensure_key(context.connection_pool.get().await?.as_mut()).await?;
    crate::push::spawn_dispatcher(context.clone());
    crate::email::outbox::spawn_sender(context.clone());
    crate::email::digest::spawn_scheduler(context.clone());
    crate::attachment::preview::spawn_maker(context.clone());
    crate::message::held::spawn_releaser(context.clone());
    crate::media_store::spawn_upload_sweeper(context.clone());
    crate::attachment::evidence::spawn_mover(context.clone());
    crate::plugin::registry::start(context).await?;
    Ok(())
}
