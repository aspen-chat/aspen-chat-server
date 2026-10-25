//! Whether this server is up (`app::health`), for what stands before it or watches it: load
//! balancers, reverse proxies, and deployment tools. Anyone may ask, without a session, and
//! neither answer is rate limited: those that ask do so from one address, many times a minute.
//! A private worker, which takes no requests, answers both on a listener of their own
//! ([`serve_for_private_worker`]).

use crate::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::extract::NoContent;
use crate::t;
use crate::{API_PREFIX, TAG_DEPLOYMENT};
use aspen_app::context::GlobalServerContext;
use axum::extract::State;

/// Whether this server is running and taking requests. It asks nothing of the database or any
/// other service, so it answers while they are down: what sends a server its requests should
/// ask this, since a server that cannot reach its database still tells people so.
/// Unauthenticated.
#[utoipa::path(
    get,
    path = "/health",
    tag = TAG_DEPLOYMENT,
    responses(
        (status = NO_CONTENT, description = "The server is running."),
    )
)]
pub async fn health() -> NoContent {
    NoContent
}

/// Whether this server reaches everything it needs to do its work: its database, NATS, and
/// Valkey. For starting a server before the one it replaces is stopped. The services are asked
/// at most once a second however often this is. Which it cannot reach is in the server's log,
/// not the answer, since anyone may ask. Unauthenticated.
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = TAG_DEPLOYMENT,
    responses(
        (status = NO_CONTENT, description = "The server reaches every service it needs."),
        (status = SERVICE_UNAVAILABLE, description = "`notReady`: the server cannot reach a service it needs; its log says which", body = Problem),
    )
)]
pub async fn ready(State(state): State<GlobalServerContext>) -> ApiResult<NoContent> {
    let readiness = aspen_app::health::readiness(&state).await;
    if readiness.is_ready() {
        return Ok(NoContent);
    }
    Err(ApiError::new(ProblemCode::NotReady).with_detail(t!("notReadyPublicDetail")))
}

/// [`ready`] on a private worker's listener, which only the machine it runs on reaches, naming
/// the services it cannot reach.
async fn ready_naming_services(State(state): State<GlobalServerContext>) -> ApiResult<NoContent> {
    let readiness = aspen_app::health::readiness(&state).await;
    if readiness.is_ready() {
        return Ok(NoContent);
    }
    Err(ApiError::new(ProblemCode::NotReady).with_detail(t!(
        "notReadyDetail",
        services = readiness.unreachable().join(", ")
    )))
}

/// Opens a private worker's health listener (`[health] listen_addr`), serving the two endpoints
/// at the paths a server that takes requests serves them. Bound before this returns, so an
/// address that cannot be had stops the worker as it starts.
pub async fn serve_for_private_worker(state: GlobalServerContext) -> Result<(), aspen_app::Error> {
    let address = state.config.health.listen_addr;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| {
            aspen_app::Error::Config(config::ConfigError::Message(format!(
                "[health] listen_addr {address} could not be listened on: {error}"
            )))
        })?;
    let router = axum::Router::new()
        .route(&format!("{API_PREFIX}/health"), axum::routing::get(health))
        .route(
            &format!("{API_PREFIX}/health/ready"),
            axum::routing::get(ready_naming_services),
        )
        .with_state(state);
    tracing::info!(%address, "serving health checks");
    tokio::spawn(async move {
        if let Err(error) = axum::serve(listener, router).await {
            tracing::error!("the health listener stopped: {error}");
        }
    });
    Ok(())
}
