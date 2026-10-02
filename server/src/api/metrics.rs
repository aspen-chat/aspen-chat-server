//! Request metrics (`aspen_metrics::api`), recorded by a route layer on every API route.

use crate::app::context::GlobalServerContext;
use crate::app::rate_limit::route_key;
use axum::extract::{MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use std::time::Instant;

pub async fn observe(request: Request, next: Next) -> Response {
    let route = request.extensions().get::<MatchedPath>().map(|matched| {
        let path = matched.as_str();
        route_key(
            request.method().as_str(),
            path.strip_prefix(crate::api::API_PREFIX).unwrap_or(path),
        )
    });
    let started = Instant::now();
    let response = next.run(request).await;
    crate::app::fleet::note_request(response.status().as_u16());
    if let Some(route) = route {
        metrics::histogram!(aspen_metrics::api::HTTP_DURATION, "route" => route.clone())
            .record(started.elapsed().as_secs_f64());
        metrics::counter!(
            aspen_metrics::api::HTTP_REQUESTS,
            "route" => route,
            "status" => response.status().as_str().to_string()
        )
        .increment(1);
    }
    response
}

/// Refreshes the gauges that are sampled rather than kept current.
pub(crate) fn spawn_samplers(context: GlobalServerContext) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(aspen_metrics::SAMPLE_INTERVAL);
        loop {
            interval.tick().await;
            let status = context.connection_pool.status();
            for (state, value) in [
                ("size", status.size),
                ("available", status.available),
                ("waiting", status.waiting),
                ("max", status.max_size),
            ] {
                ::metrics::gauge!(aspen_metrics::api::DB_POOL, "state" => state).set(value as f64);
            }
            let suspended = context.rate_limiter.suspension().current().is_some();
            ::metrics::gauge!(aspen_metrics::api::RATE_LIMITS_SUSPENDED).set(if suspended {
                1.0
            } else {
                0.0
            });
        }
    });
}
