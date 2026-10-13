//! Whether this server is up (`app::health`), for what stands before it or watches it: load
//! balancers, reverse proxies, and deployment tools. Anyone may ask, without a session, and
//! neither answer is rate limited: those that ask do so from one address, many times a minute.

use crate::TAG_DEPLOYMENT;
use crate::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::extract::NoContent;
use crate::t;
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
/// Valkey. For starting a server before the one it replaces is stopped, and for finding what
/// is wrong with one. The services are asked at most once a second however often this is.
/// Unauthenticated.
#[utoipa::path(
    get,
    path = "/health/ready",
    tag = TAG_DEPLOYMENT,
    responses(
        (status = NO_CONTENT, description = "The server reaches every service it needs."),
        (status = SERVICE_UNAVAILABLE, description = "`notReady`: `detail` names the services it cannot reach", body = Problem),
    )
)]
pub async fn ready(State(state): State<GlobalServerContext>) -> ApiResult<NoContent> {
    let readiness = aspen_app::health::readiness(&state).await;
    if readiness.is_ready() {
        return Ok(NoContent);
    }
    Err(ApiError::new(ProblemCode::NotReady).with_detail(t!(
        "notReadyDetail",
        services = readiness.unreachable().join(", ")
    )))
}
