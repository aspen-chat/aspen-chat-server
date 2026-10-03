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
use std::collections::HashMap;
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
    /// The WebAuthn relying party, when `[auth.passkeys]` is configured.
    pub webauthn: Option<Arc<webauthn_rs::Webauthn>>,
    /// Where each channel belongs (`app::events::channel_home`), filled as it is asked; a
    /// channel never moves.
    pub channel_homes: Arc<Mutex<HashMap<app::ChannelId, app::events::ChannelHome>>>,
    /// Each channel's recent count of who is online in it (`app::channel_presence`).
    pub channel_presence: Arc<app::channel_presence::PresenceCounts>,
    /// The server's one reading of the event stream, which every event stream connection
    /// registers with.
    pub event_feed: app::event_feed::EventFeed,
    /// What every call to another deployment is made with (`app::federation::fetch`).
    pub federation_client: reqwest::Client,
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
        let webauthn = app::passkey::relying_party(&config.auth)?;
        let federation_client = app::federation::fetch::client(&config.federation)?;

        Ok(Self {
            channel_homes: Arc::new(Mutex::new(HashMap::new())),
            channel_presence: Arc::default(),
            connection_pool: {
                let conn_manager =
                    AsyncDieselConnectionManager::<AsyncPgConnection>::new(&config.database_url);
                let pool = Pool::builder(conn_manager);
                match config.database_pool_size {
                    Some(size) => pool.max_size(size),
                    None => pool,
                }
                .build()?
            },
            event_feed: app::event_feed::EventFeed::start(
                context.clone(),
                config.event_queue_size,
                config.event_feed_shards,
            ),
            nats_context: Arc::new(context),
            valkey,
            media_store,
            rate_limiter: Arc::new(rate_limiter),
            webauthn,
            federation_client,
            config: config.into(),
        })
    }
}

/// Starts the app's background tasks: the poll closer, the voice report listener and reaper, the
/// fleet heartbeat, the federation standing confirmer, and the push dispatcher, seeding the voice
/// servers and making the federation and push keys where they are missing.
pub async fn start_background_tasks(context: &GlobalServerContext) -> Result<(), app::Error> {
    app::poll::spawn_closer(context.clone());
    app::voice::seed_servers(context).await?;
    app::voice::spawn_report_listener(context.clone()).await?;
    app::voice::spawn_reaper(context.clone());
    app::fleet::spawn_heartbeat(context.clone());
    if context.config.federation.domain.is_some() {
        app::federation::ensure_key(context.connection_pool.get().await?.as_mut()).await?;
    }
    app::federation::standing::spawn_confirmer(context.clone());
    app::push::ensure_key(context.connection_pool.get().await?.as_mut()).await?;
    app::push::spawn_dispatcher(context.clone());
    Ok(())
}
