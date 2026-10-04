//! An Aspen voice server. See `voice_protocol` for what it agrees with the API server and the
//! clients, `rooms` for the calls it carries, and `voice_server.toml` for its settings.

mod capacity;
mod config;
mod limits;
mod media;
mod metrics;
mod reporter;
mod rooms;
mod signalling;
mod transfer;

use anyhow::Context;
use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
use axum::serve::ListenerExt as _;
use clap::Parser;
use mediasoup::prelude::*;
use mediasoup::worker::{WorkerLogLevel, WorkerLogTag};
use schemars::schema_for;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;
use voice_protocol::signal::VoiceSignalProtocol;

/// jemalloc for the whole process (it also replaces `malloc`), which keeps memory from
/// fragmenting across threads and reports what it holds (`aspen_metrics::memory`).
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// jemalloc's options, read when it starts. Freed memory goes back to the system on its own
/// schedule through a background thread; without it, pages are returned only while the program
/// allocates, and a server gone idle after a busy hour keeps its peak resident size.
#[unsafe(export_name = "malloc_conf")]
pub static MALLOC_CONF: &[u8; 23] = b"background_thread:true\0";

#[derive(Parser, Debug)]
#[command(about = "An Aspen voice server")]
struct Opt {
    /// Write `voice_signal_schema.json` to the working directory and exit.
    #[arg(long)]
    gen_signal_schema: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Estimate, conservatively, how many people this machine can hold in calls: reads the
    /// hardware and the media settings, measures what a forwarded stream costs here with a
    /// short self-test, and prints the `capacity` for its `[[voice.servers]]` entry. The
    /// self-test runs its own mediasoup worker on loopback, beside a running server if there is
    /// one, whose load then skews the measurement.
    EstimateCapacity(capacity::EstimateArgs),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let opt = Opt::parse();
    if let Some(Command::EstimateCapacity(args)) = opt.command {
        // Its output is the estimate, on stdout; anything logged goes to stderr.
        tracing_subscriber::fmt()
            .with_writer(std::io::stderr)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_env("ASPEN_LOG")
                    .unwrap_or_else(|_| "warn,mediasoup=error".into()),
            )
            .init();
        let media = config::load_media_config()
            .context("failed to read the media settings of voice_server.toml or the environment")?;
        return capacity::run(args, media).await;
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ASPEN_LOG")
                .unwrap_or_else(|_| "info,mediasoup=warn".into()),
        )
        .init();
    if opt.gen_signal_schema {
        let schema = schema_for!(VoiceSignalProtocol);
        std::fs::write(
            "voice_signal_schema.json",
            serde_json::to_string_pretty(&schema)?,
        )?;
        return Ok(());
    }
    let config =
        config::load_config().context("failed to load voice_server.toml or environment")?;
    let limits = Arc::new(
        limits::Limits::new(&config.rate_limits)
            .map_err(|message| anyhow::anyhow!("rate limits: {message}"))?,
    );

    let manager = WorkerManager::new();
    let mut workers = Vec::new();
    for _ in 0..config.workers.max(1) {
        let mut settings = WorkerSettings::default();
        settings.log_level = WorkerLogLevel::Warn;
        settings.log_tags = vec![WorkerLogTag::Info];
        settings.rtc_port_range = config.rtc.min_port..=config.rtc.max_port;
        workers.push(manager.create_worker(settings).await?);
    }
    info!(workers = workers.len(), "mediasoup workers started");

    let reporter =
        reporter::Reporter::connect(&config.nats_url, &config.nats_auth_token, config.id)
            .await
            .context("failed to connect to NATS")?;
    aspen_limits::suspension::watch(reporter.client(), limits.suspension().clone(), "voice");
    let announced_address = config.rtc.resolved_announced_address()?;
    if let Some(address) = &announced_address {
        if address
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
        {
            anyhow::bail!(
                "rtc.announced_address is a loopback address, which browsers cannot send media to"
            );
        }
    } else if config.rtc.ip.is_loopback() {
        anyhow::bail!(
            "rtc.ip is a loopback address, which browsers cannot send media to; bind an interface or leave it unspecified"
        );
    }
    info!(
        ip = %config.rtc.ip,
        announced = announced_address.as_deref().unwrap_or("(same as ip)"),
        ports = format!("{}-{}", config.rtc.min_port, config.rtc.max_port),
        "media listening"
    );
    let relay = Arc::new(
        transfer::Relay::start(
            &config.transfer,
            config.rtc.ip,
            announced_address
                .clone()
                .unwrap_or_else(|| config.rtc.ip.to_string())
                .as_str(),
        )
        .await
        .context("could not start STUN and TURN for file transfers")?,
    );
    let rooms = rooms::Rooms::new(
        config.id,
        workers,
        config.rtc.ip,
        announced_address,
        reporter.clone(),
        Arc::clone(&relay),
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

    if config.metrics.enabled {
        aspen_metrics::install(config.metrics.listen_addr).map_err(|e| anyhow::anyhow!(e))?;
        metrics::spawn_samplers(Arc::clone(&rooms), Arc::clone(&limits));
    }
    let state = signalling::AppState {
        server: config.id,
        token_secret: config.token_secret.clone().into(),
        rooms: Arc::clone(&rooms),
        limits,
        used_tokens: Arc::default(),
    };
    // The health check is what clients measure latency against, from any origin. The limits
    // run inside the CORS layer, so a refusal still reaches the page that asked.
    let app = Router::new()
        .route("/health", get(|| async { StatusCode::NO_CONTENT }))
        .route("/ws", get(signalling::upgrade))
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
    // Signalling frames are small and each is sent as it is written; with Nagle's algorithm on,
    // one written while the previous is unacknowledged waits for the client's delayed ACK.
    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await?
        .tap_io(|stream| {
            if let Err(e) = stream.set_nodelay(true) {
                tracing::warn!("could not turn off Nagle's algorithm: {e}");
            }
        });
    info!(
        addr = config.listen_addr.to_string(),
        server = config.id.to_string(),
        "voice server listening"
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
        info!("shutting down");
    })
    .await?;
    rooms.shutdown().await;
    relay.shutdown().await;
    Ok(())
}
