//! An Aspen voice server. See `voice_protocol` for what it agrees with the API server and the
//! clients, `rooms` for the calls it carries, and `voice_server.toml` for its settings.

mod config;
mod limits;
mod reporter;
mod rooms;
mod signalling;

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

#[derive(Parser, Debug)]
#[command(about = "An Aspen voice server")]
struct Opt {
    /// Write `voice_signal_schema.json` to the working directory and exit.
    #[arg(long)]
    gen_signal_schema: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ASPEN_LOG")
                .unwrap_or_else(|_| "info,mediasoup=warn".into()),
        )
        .init();
    let opt = Opt::parse();
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

    let reporter = reporter::Reporter::connect(&config.nats_url, &config.nats_auth_token)
        .await
        .context("failed to connect to NATS")?;
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
    let rooms = rooms::Rooms::new(
        config.id,
        workers,
        config.rtc.ip,
        announced_address,
        reporter.clone(),
    );
    {
        let rooms = Arc::clone(&rooms);
        reporter
            .clone()
            .spawn_load_reports(config.id, move || rooms.participant_count());
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
        token_secret: config.token_secret.clone().into(),
        rooms: Arc::clone(&rooms),
        limits,
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
    Ok(())
}
