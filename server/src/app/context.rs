//! The state every request handler and background task shares, and the background tasks
//! started with it.

use crate::app;
use crate::app::ASPEN_NATS_STREAM_NAME;
use crate::aspen_config::{AspenConfig, load_config};
use async_nats::ConnectOptions;
use async_nats::jetstream::stream::{ConsumerLimits, DiscardPolicy, StorageType};
use diesel_async::{
    AsyncPgConnection,
    pooled_connection::{AsyncDieselConnectionManager, deadpool::Pool},
};
use fred::prelude::ClientLike;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct GlobalServerContext {
    pub connection_pool: Pool<AsyncPgConnection>,
    pub nats_context: Arc<async_nats::jetstream::Context>,
    pub valkey: fred::clients::Client,
    pub media_store: Arc<app::media_store::MediaStore>,
    pub config: Arc<AspenConfig>,
    pub rate_limiter: Arc<app::rate_limit::RateLimiter>,
    /// Where each channel belongs (`app::events::channel_home`), filled as it is asked; a
    /// channel never moves.
    pub channel_homes: Arc<Mutex<HashMap<app::ChannelId, app::events::ChannelHome>>>,
    /// Each channel's recent count of who is online in it (`app::channel_presence`).
    pub channel_presence: Arc<app::recent::Recent<app::ChannelId, u32>>,
    /// Each community's recent set of members with a connection (`app::user_status`).
    pub connected_members: Arc<app::recent::Recent<app::CommunityId, Arc<HashSet<app::UserId>>>>,
    /// The server's one reading of the event stream, which every event stream connection
    /// registers with.
    pub event_feed: app::event_feed::EventFeed,
    /// What every call to another deployment is made with (`app::federation::fetch`).
    pub federation_client: reqwest::Client,
    /// This server's copy of the deployment's settings (`app::deployment_settings`).
    pub settings: app::deployment_settings::SettingsCache,
    /// The plugins this server runs (`app::plugin`).
    pub plugins: Arc<app::plugin::Plugins>,
    /// What this server sends mail with, when `[email]` is configured (`app::email`).
    pub mailer: Option<Arc<app::email::Mailer>>,
}

impl GlobalServerContext {
    /// The deployment's settings as this server last read them.
    pub fn settings(&self) -> std::sync::Arc<app::deployment_settings::DeploymentSettings> {
        self.settings.current()
    }
}

impl GlobalServerContext {
    /// Connects to everything the server needs. `routes` are the API's routes, which the rate
    /// limits are checked against.
    pub async fn new(routes: &[app::rate_limit::Route]) -> Result<Self, app::Error> {
        let config = load_config()?;
        app::login::configure_password_work(
            config.auth.password_hashing_threads,
            Duration::from_secs(config.auth.password_hashing_wait_seconds),
        );
        let rate_limiter = app::rate_limit::RateLimiter::compile(&config.rate_limits, routes)
            .map_err(|message| app::Error::Config(config::ConfigError::Message(message)))?;
        let client = async_nats::connect_with_options(
            &config.nats_url,
            ConnectOptions::new().token(config.nats_auth_token.clone()),
        )
        .await?;
        aspen_limits::suspension::watch(client.clone(), rate_limiter.suspension().clone(), "api");
        let context = async_nats::jetstream::new(client);
        context
            .create_or_update_stream(async_nats::jetstream::stream::Config {
                name: ASPEN_NATS_STREAM_NAME.to_string(),
                subjects: vec![format!("{}.>", app::events::SUBJECT_ROOT)],
                discard: DiscardPolicy::Old,
                max_messages: 1_000_000_000,
                max_bytes: 8 * 1024 * 1024 * 1024,
                max_age: app::event_feed::MAX_EVENT_AGE,
                storage: StorageType::Memory,
                consumer_limits: Some(ConsumerLimits {
                    max_ack_pending: 1000,
                    inactive_threshold: Duration::from_secs(60),
                }),
                ..Default::default()
            })
            .await?;
        let valkey_config = fred::prelude::Config::from_url(&config.valkey_url)?;
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

        let media_store = Arc::new(app::media_store::MediaStore::new(&config).await?);
        // Checked now so that a mistake in `[auth.passkeys]` stops the server as it starts.
        app::passkey::relying_party(&config.auth, app::deployment_settings::DEFAULT_NAME)?;
        let federation_client = app::federation::fetch::client(&config.federation)?;
        let mailer = config
            .email
            .as_ref()
            .map(app::email::Mailer::new)
            .transpose()?
            .map(Arc::new);
        let connection_pool = {
            let conn_manager =
                AsyncDieselConnectionManager::<AsyncPgConnection>::new(&config.database_url);
            let pool = Pool::builder(conn_manager);
            match config.database_pool_size {
                Some(size) => pool.max_size(size),
                None => pool,
            }
            .build()?
        };
        let settings = {
            let mut conn = connection_pool.get().await?;
            app::deployment_settings::pin_domain(
                conn.as_mut(),
                app::federation::own_domain(&config.federation).as_ref(),
            )
            .await?;
            app::deployment_settings::load(conn.as_mut()).await?
        };

        Ok(Self {
            channel_homes: Arc::new(Mutex::new(HashMap::new())),
            channel_presence: Arc::default(),
            connected_members: Arc::default(),
            connection_pool,
            event_feed: app::event_feed::EventFeed::start(
                context.clone(),
                config.event_queue_size,
                config.event_feed_shards,
            ),
            nats_context: Arc::new(context),
            valkey,
            media_store,
            rate_limiter: Arc::new(rate_limiter),
            federation_client,
            settings: app::deployment_settings::SettingsCache::new(settings),
            plugins: Arc::new(app::plugin::Plugins::new()?),
            mailer,
            config: config.into(),
        })
    }
}

/// Starts the app's background tasks: the settings watcher, the poll closer, the voice report
/// listener and reaper, the fleet heartbeat, the federation standing confirmer, the push
/// dispatcher, the mail sender and digest scheduler, the attachment preview maker and held message releaser, the sweeper of staging uploads, and the plugins with their observers, making the federation and push keys where
/// they are missing.
pub async fn start_background_tasks(context: &GlobalServerContext) -> Result<(), app::Error> {
    app::deployment_settings::spawn_watcher(context.clone());
    app::poll::spawn_closer(context.clone());
    app::voice::spawn_report_listener(context.clone()).await?;
    app::voice::spawn_reaper(context.clone());
    app::fleet::spawn_heartbeat(context.clone());
    if context.config.federation.domain.is_some() {
        app::federation::ensure_key(context.connection_pool.get().await?.as_mut()).await?;
    }
    app::federation::standing::spawn_confirmer(context.clone());
    app::push::ensure_key(context.connection_pool.get().await?.as_mut()).await?;
    app::push::spawn_dispatcher(context.clone());
    app::email::outbox::spawn_sender(context.clone());
    app::email::digest::spawn_scheduler(context.clone());
    app::attachment::preview::spawn_maker(context.clone());
    app::message::held::spawn_releaser(context.clone());
    app::media_store::spawn_upload_sweeper(context.clone());
    app::plugin::registry::start(context).await?;
    Ok(())
}
