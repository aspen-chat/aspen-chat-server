//! The Aspen voice server's process: its allocator, command line, logging, NATS connection,
//! metrics, and listeners. What it serves is `voice_server::VoiceServer`.

use anyhow::Context;
use axum::Router;
use clap::Parser;
use schemars::schema_for;
use std::sync::Arc;
use tracing::info;
use voice_protocol::signal::VoiceSignalProtocol;
use voice_server::{Failure, Link, VoiceServer, capacity, config};

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
    /// short self-test, and prints the capacity to register it with
    /// (`aspen-chat-server voice-servers add --capacity`, or the dashboard). The self-test runs a media thread of its own on loopback, beside a running server if there
    /// is one, whose load then skews the measurement.
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
                    .unwrap_or_else(|_| "warn".into()),
            )
            .init();
        let media = config::load_media_config()
            .context("failed to read the media settings of voice_server.toml or the environment")?;
        return capacity::run(args, media).await;
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ASPEN_LOG")
                .unwrap_or_else(|_| "info,str0m=warn".into()),
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
    // Listening on loopback means a proxy on this machine passes every client on, and without
    // it trusted every client has the proxy's address: the per-address limits then count
    // everyone together, and a few clients lock out the rest.
    let (listen_addrs, listen_required) = config.listen_addrs();
    if listen_addrs.iter().any(|addr| addr.ip().is_loopback())
        && config.rate_limits.trusted_proxies.is_empty()
    {
        tracing::warn!(
            "listen_addr is a loopback address but [rate_limits] trusted_proxies is empty: every \
             client behind the proxy counts as the proxy's address, so the per-address limits \
             apply to everyone at once; list the proxy (trusted_proxies = [\"127.0.0.1\"])"
        );
    }

    if matches!(config.nats_auth()?, config::NatsAuth::Token(_)) {
        tracing::warn!(
            "signing in to NATS with the deployment's token, which lets this server do anything \
             the API servers can; give it a NATS user of its own ([nats_user])"
        );
    }
    let link = Link::connect(&config, config.nats_auth()?)
        .await
        .context("failed to connect to NATS")?;
    let server = VoiceServer::start(&config, link).await?;
    if config.metrics.enabled {
        aspen_metrics::install(config.metrics.listen_addr).map_err(|e| anyhow::anyhow!(e))?;
        server.spawn_metrics_samplers();
    }
    let listeners = aspen_sockets::tcp_listeners(&listen_addrs, listen_required)?;
    for listener in &listeners {
        info!(
            addr = listener.local_addr()?.to_string(),
            server = config.id.to_string(),
            "voice server listening"
        );
    }
    let stopped = serve(
        listeners,
        server.router.clone(),
        config.rate_limits.max_connections,
        async {
            match server.failed().await {
                Failure::MediaThread => Stopped::MediaThread,
                Failure::Unregistered => Stopped::Unregistered,
            }
        },
    )
    .await;
    server.shutdown().await;
    match stopped {
        Stopped::Asked => Ok(()),
        Stopped::MediaThread => anyhow::bail!("a media thread stopped"),
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
    /// A media thread stopped.
    MediaThread,
    /// The API servers said this server's id is not registered.
    Unregistered,
}

/// How long a connection has to send a request's headers, from when it opens or its last
/// response was sent, before it is closed: a client that trickles them in, or opens connections
/// and sends nothing, cannot hold them.
const HEADER_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// The next connection on any of `listeners`. `turn` moves the search's start on by one each
/// time, so a listener that always has connections waiting cannot keep the others from being
/// accepted.
async fn accept_any(
    listeners: &[tokio::net::TcpListener],
    turn: &mut usize,
) -> std::io::Result<(tokio::net::TcpStream, std::net::SocketAddr)> {
    let first = *turn;
    *turn = turn.wrapping_add(1);
    std::future::poll_fn(|cx| {
        for offset in 0..listeners.len() {
            let listener = &listeners[first.wrapping_add(offset) % listeners.len()];
            if let std::task::Poll::Ready(accepted) = listener.poll_accept(cx) {
                return std::task::Poll::Ready(accepted);
            }
        }
        std::task::Poll::Pending
    })
    .await
}

/// Serves `app` on `listeners` until the process is told to stop or `failed` completes:
/// at most `max_connections` at once (past that, new ones wait in the listen backlog), each
/// given `HEADER_READ_TIMEOUT` for every request's headers, with WebSocket upgrades. Each
/// request carries its peer's address as axum's `ConnectInfo`.
async fn serve(
    listeners: Vec<tokio::net::TcpListener>,
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
    let mut turn = 0;
    let stopped = loop {
        let permit = tokio::select! {
            permit = Arc::clone(&permits).acquire_owned() => permit.expect("never closed"),
            stopped = &mut stopping => break stopped,
        };
        let (stream, peer) = tokio::select! {
            accepted = accept_any(&listeners, &mut turn) => match accepted {
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
