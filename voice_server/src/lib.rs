//! An Aspen voice server. See `voice_protocol` for what it agrees with the API server and the
//! clients, `rooms` for the calls it carries, and `voice_server.toml` for its settings.

pub mod capacity;
pub mod config;
mod limits;
mod metrics;
mod outbox;
mod reporter;
mod rooms;
mod signalling;
mod token_keys;
mod transfer;

pub use reporter::Link;

use anyhow::Context;
use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;

/// A voice server whose calls, media threads, file transfer relay, and reports are running. Its
/// `router` holds the health check and signalling, which whoever listens for it serves.
pub struct VoiceServer {
    pub router: Router,
    rooms: Arc<rooms::Rooms>,
    relay: Arc<transfer::Relay>,
    token_keys: Arc<token_keys::TokenKeys>,
    limits: Arc<limits::Limits>,
}

/// Why a voice server stopped on its own.
pub enum Failure {
    /// A media thread stopped, taking its calls' media with it.
    MediaThread,
    /// The API servers said this server's id is not registered.
    Unregistered,
}

impl VoiceServer {
    /// Starts the server `config` describes, reporting over `link`.
    pub async fn start(config: &config::VoiceServerConfig, link: Link) -> anyhow::Result<Self> {
        config.check_secret()?;
        if config.token_secret.is_some() {
            tracing::warn!(
                "token_secret is set, so this server also takes join tokens of the shared-secret \
                 form; leave it out once every API server signs join tokens with its key"
            );
        }
        config.transfer.check(&config.rtc)?;
        let limits = Arc::new(
            limits::Limits::new(&config.rate_limits)
                .map_err(|message| anyhow::anyhow!("rate limits: {message}"))?,
        );
        let reporter = reporter::Reporter::new(link, config.id);
        aspen_limits::suspension::watch(reporter.bus(), limits.suspension().clone(), "voice");
        let token_keys = token_keys::TokenKeys::start(reporter.bus(), config.id);
        let media_addresses = config.rtc.media_addresses()?;
        let ports = config.rtc.media_ports(config.media_threads)?;
        // A media thread that stops takes its calls' media with it and would leave their rooms
        // looking alive; the server stops instead, telling everyone, so their clients rejoin and
        // whatever supervises the process starts it again.
        let media = voice_media::Media::start(voice_media::MediaConfig {
            addresses: media_addresses
                .iter()
                .map(config::MediaAddress::resolved)
                .collect::<anyhow::Result<_>>()?,
            first_port: *ports.start(),
            threads: config.media_threads.max(1),
            audio_slots: rooms::AUDIO_SLOTS,
            max_watched_video: rooms::MAX_WATCHED_VIDEO,
        })
        .context("could not start the media threads")?;
        for address in &media_addresses {
            info!(
                ip = %address.ip,
                announced = address.host(),
                ports = format!("{}-{}", ports.start(), ports.end()),
                "media listening, over UDP and TCP"
            );
        }
        let relay = Arc::new(
            transfer::Relay::start(&config.transfer, &media_addresses)
                .await
                .context("could not start STUN and TURN for file transfers")?,
        );
        let rooms = rooms::Rooms::new(
            config.id,
            media,
            reporter.reports(),
            Arc::clone(&relay),
            &config.rate_limits,
            rooms::AUDIO_SLOTS,
        );
        {
            let rooms = Arc::clone(&rooms);
            reporter
                .clone()
                .spawn_load_reports(config.id, move || rooms.participant_count());
        }
        {
            let rooms = Arc::clone(&rooms);
            reporter.clone().spawn_snapshots(move || rooms.snapshot());
        }
        {
            let rooms = Arc::clone(&rooms);
            reporter
                .spawn_commands(config.id, move |command| {
                    let rooms = Arc::clone(&rooms);
                    tokio::spawn(async move { rooms.command(command).await });
                })
                .await?;
        }
        let state = signalling::AppState {
            server: config.id,
            token_secret: config.token_secret.as_deref().map(Arc::from),
            token_keys: Arc::clone(&token_keys),
            rooms: Arc::clone(&rooms),
            limits: Arc::clone(&limits),
            used_tokens: Arc::default(),
            // Sized for the messages compressed at one moment, about one per runtime thread.
            deflate: yawc::DeflatePool::new(
                std::thread::available_parallelism().map_or(4, std::num::NonZero::get),
            ),
        };
        // The health check is what clients measure latency against, from any origin. The limits
        // run inside the CORS layer, so a refusal still reaches the page that asked.
        let app = Router::new()
            .route(
                signalling::HEALTH_PATH,
                get(|| async { StatusCode::NO_CONTENT }),
            )
            .route(signalling::SIGNALLING_PATH, get(signalling::upgrade))
            .route_layer(axum::middleware::from_fn_with_state(
                state.clone(),
                signalling::limit_http,
            ))
            .with_state(state)
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .expose_headers([axum::http::header::RETRY_AFTER]),
            );
        Ok(Self {
            router: app,
            rooms,
            relay,
            token_keys,
            limits,
        })
    }

    /// Samples what the server holds into its metrics, for a process that serves them.
    pub fn spawn_metrics_samplers(&self) {
        metrics::spawn_samplers(Arc::clone(&self.rooms), Arc::clone(&self.limits));
    }

    /// Completes once the server can carry calls no longer.
    pub async fn failed(&self) -> Failure {
        tokio::select! {
            () = self.rooms.media_stopped() => Failure::MediaThread,
            () = self.token_keys.unregistered() => Failure::Unregistered,
        }
    }

    /// Ends every call, telling its people, and stops the relay.
    pub async fn shutdown(self) {
        self.rooms.shutdown().await;
        self.relay.shutdown().await;
    }
}
