//! Request metrics (`aspen_metrics::api`), recorded by a route layer on every API route.

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
