//! How long a request may take (`[connections] request_seconds`): from when it reaches the
//! router until its response's headers are ready, its body read included. A request still
//! unanswered then is answered `503` `requestTimedOut` and leaves its connection; its handler
//! goes on in its own task as it does when a client goes away (`settle_after_request`), so a
//! write it was making finishes or rolls back whole. A body still unread then fails to read,
//! so a client trickling one in cannot hold its handler open either.
//!
//! The longest request a client may make on purpose is a search reading through history, which
//! answers within `[search] round_seconds`; the configuration refuses a deadline that does not
//! leave it room (`ConnectionsConfig::check`). An event stream's upgrade is answered at once and
//! its WebSocket is not a request's, so the deadline does not reach it. What a response's body
//! does once it is under way is the listener's to bound (`connections::Tracked`).

use crate::error::{ApiError, ProblemCode};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use hyper::body::{Frame, SizeHint};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::time::{Instant, Sleep};

/// Answers a request that takes longer than `limit` with `requestTimedOut`, and cuts off its
/// body at the same moment.
pub async fn layer(State(limit): State<Duration>, request: Request, next: Next) -> Response {
    let deadline = Instant::now() + limit;
    let request = request.map(|body| Body::new(Deadlined::new(body, deadline)));
    match tokio::time::timeout_at(deadline, next.run(request)).await {
        Ok(response) => response,
        Err(_) => ApiError::new(ProblemCode::RequestTimedOut).into_response(),
    }
}

/// A request body that fails once its deadline passes, however steadily it was arriving.
struct Deadlined {
    inner: Body,
    deadline: Pin<Box<Sleep>>,
}

impl Deadlined {
    fn new(inner: Body, deadline: Instant) -> Self {
        Self {
            inner,
            deadline: Box::pin(tokio::time::sleep_until(deadline)),
        }
    }
}

/// What reading a body past its deadline fails with.
#[derive(Debug)]
struct BodyTimedOut;

impl std::fmt::Display for BodyTimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the request's body did not arrive within [connections] request_seconds")
    }
}

impl std::error::Error for BodyTimedOut {}

impl hyper::body::Body for Deadlined {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if let Poll::Ready(frame) = Pin::new(&mut self.inner).poll_frame(cx) {
            return Poll::Ready(frame);
        }
        match self.deadline.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Some(Err(axum::Error::new(BodyTimedOut)))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::{get, post};
    use futures_util::StreamExt as _;
    use tower::ServiceExt as _;

    fn router(limit: Duration) -> axum::Router {
        axum::Router::new()
            .route("/quick", get(|| async { "done" }))
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    "done"
                }),
            )
            .route("/echo", post(|body: Bytes| async move { body }))
            .layer(axum::middleware::from_fn_with_state(limit, layer))
    }

    #[tokio::test(start_paused = true)]
    async fn answers_in_time() {
        let response = router(Duration::from_secs(5))
            .oneshot(Request::get("/quick").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test(start_paused = true)]
    async fn times_out_a_slow_handler() {
        let response = router(Duration::from_secs(5))
            .oneshot(Request::get("/slow").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cuts_off_a_body_that_never_finishes() {
        // A body that sends one byte and then nothing more.
        let trickle =
            futures_util::stream::once(async { Ok::<_, std::io::Error>(Bytes::from("x")) })
                .chain(futures_util::stream::pending());
        let response = router(Duration::from_secs(5))
            .oneshot(
                Request::post("/echo")
                    .body(Body::from_stream(trickle))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(!response.status().is_success());
    }
}
