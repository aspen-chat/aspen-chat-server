//! An Aspen voice server. See `voice_protocol` for what it agrees with the API server and the
//! clients, `rooms` for the calls it carries, and `voice_server.toml` for its settings.

mod capacity;
mod config;
mod limits;
mod media;
mod metrics;
mod outbox;
mod reporter;
mod rooms;
mod signalling;
mod token_keys;
mod transfer;

use anyhow::Context;
use axum::Router;
use axum::http::StatusCode;
use axum::routing::get;
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
    // Listening on loopback means a proxy on this machine passes every client on, and without
    // it trusted every client has the proxy's address: the per-address limits then count
    // everyone together, and a few clients lock out the rest.
    if config.listen_addr.ip().is_loopback() && config.rate_limits.trusted_proxies.is_empty() {
        tracing::warn!(
            "listen_addr is a loopback address but [rate_limits] trusted_proxies is empty: every              client behind the proxy counts as the proxy's address, so the per-address limits              apply to everyone at once; list the proxy (trusted_proxies = [\"127.0.0.1\"])"
        );
    }

    let manager = WorkerManager::new();
    let mut workers = Vec::new();
    // A worker that dies takes its calls' media with it and would leave their rooms looking
    // alive; the server stops instead, telling everyone, so their clients rejoin and whatever
    // supervises the process starts it again.
    let worker_died = Arc::new(tokio::sync::Notify::new());
    for _ in 0..config.workers.max(1) {
        let mut settings = WorkerSettings::default();
        settings.log_level = WorkerLogLevel::Warn;
        settings.log_tags = vec![WorkerLogTag::Info];
        settings.rtc_port_range = config.rtc.min_port..=config.rtc.max_port;
        let worker = manager.create_worker(settings).await?;
        let died = Arc::clone(&worker_died);
        worker
            .on_dead(move |exit| {
                tracing::error!(?exit, "a mediasoup worker died; stopping the voice server");
                died.notify_one();
            })
            .detach();
        workers.push(worker);
    }
    info!(workers = workers.len(), "mediasoup workers started");

    if matches!(config.nats_auth()?, config::NatsAuth::Token(_)) {
        tracing::warn!(
            "signing in to NATS with the deployment's token, which lets this server do anything \
             the API servers can; give it a NATS user of its own ([nats_user])"
        );
    }
    let reporter = reporter::Reporter::connect(&config, config.nats_auth()?)
        .await
        .context("failed to connect to NATS")?;
    aspen_limits::suspension::watch(reporter.client(), limits.suspension().clone(), "voice");
    let token_keys = token_keys::TokenKeys::start(reporter.client(), config.id);
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
        &config.rate_limits,
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
        token_secret: config.token_secret.as_deref().map(Arc::from),
        token_keys: Arc::clone(&token_keys),
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
    let listener = tokio::net::TcpListener::bind(config.listen_addr).await?;
    info!(
        addr = config.listen_addr.to_string(),
        server = config.id.to_string(),
        "voice server listening"
    );
    let stopped = serve(listener, app, config.rate_limits.max_connections, async {
        tokio::select! {
            () = worker_died.notified() => Stopped::WorkerDied,
            () = token_keys.unregistered() => Stopped::Unregistered,
        }
    })
    .await;
    rooms.shutdown().await;
    relay.shutdown().await;
    match stopped {
        Stopped::Asked => Ok(()),
        Stopped::WorkerDied => anyhow::bail!("a mediasoup worker died"),
        Stopped::Unregistered => anyhow::bail!(
            "this voice server's id ({}) is not registered with the deployment; set `id` in \
             voice_server.toml to the id `aspen-chat-server voice-servers list` shows for it",
            config.id
        ),
    }
}

/// Why `serve` returned.
enum Stopped {
    /// The process was told to stop.
    Asked,
    /// A mediasoup worker died.
    WorkerDied,
    /// The API servers said this server's id is not registered.
    Unregistered,
}

/// How long a connection has to send a request's headers, from when it opens or its last
/// response was sent, before it is closed: a client that trickles them in, or opens connections
/// and sends nothing, cannot hold them.
const HEADER_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Serves `app` on `listener` until the process is told to stop or `failed` completes:
/// at most `max_connections` at once (past that, new ones wait in the listen backlog), each
/// given `HEADER_READ_TIMEOUT` for every request's headers, with WebSocket upgrades. Each
/// request carries its peer's address as axum's `ConnectInfo`.
async fn serve(
    listener: tokio::net::TcpListener,
    app: Router,
    max_connections: usize,
    failed: impl std::future::Future<Output = Stopped>,
) -> Stopped {
    use hyper_util::rt::{TokioIo, TokioTimer};
    use tower::ServiceExt as _;
    let permits = Arc::new(tokio::sync::Semaphore::new(max_connections.max(1)));
    let mut stopping = std::pin::pin!(async {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => Stopped::Asked,
            stopped = failed => stopped,
        }
    });
    let stopped = loop {
        let permit = tokio::select! {
            permit = Arc::clone(&permits).acquire_owned() => permit.expect("never closed"),
            stopped = &mut stopping => break stopped,
        };
        let (stream, peer) = tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok(accepted) => accepted,
                Err(e) => {
                    // Out of file descriptors, most likely; accepting again at once would spin.
                    tracing::warn!("could not accept a connection: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
            },
            stopped = &mut stopping => break stopped,
        };
        // Signalling frames are small and each is sent as it is written; with Nagle's algorithm
        // on, one written while the previous is unacknowledged waits for the client's delayed
        // ACK.
        if let Err(e) = stream.set_nodelay(true) {
            tracing::warn!("could not turn off Nagle's algorithm: {e}");
        }
        let app = app.clone();
        tokio::spawn(async move {
            let service = hyper::service::service_fn(move |mut request: hyper::Request<_>| {
                request
                    .extensions_mut()
                    .insert(axum::extract::ConnectInfo(peer));
                app.clone().oneshot(request)
            });
            let served = hyper::server::conn::http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_READ_TIMEOUT)
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
            if let Err(e) = served {
                tracing::debug!("a connection ended with an error: {e}");
            }
            drop(permit);
        });
    };
    info!("shutting down");
    stopped
}
