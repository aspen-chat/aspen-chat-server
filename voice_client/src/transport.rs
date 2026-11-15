//! The network as the engine reaches it: one HTTP request at a time, and WebSockets. Each
//! platform implements these over its own stack (URLSession on iOS, OkHttp on Android), so TLS
//! trust, proxies, and the move between Wi-Fi and cellular are the operating system's, and the
//! engine runs no TLS of its own. The engine opens nothing itself; a test stands in for both.
//! Swift and Kotlin implement them through `bindings`, which carries headers as records rather
//! than pairs; `Received` crosses as it is.

use async_trait::async_trait;
use std::sync::Arc;

/// What the platform's network stack could not do, in its own words: no route, a refused
/// connection, a certificate it does not trust. An HTTP answer with an error status is not one
/// of these; it comes back as a `HttpResponse`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TransportError(pub String);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRequest {
    /// `GET` or `POST`, as the engine's calls need.
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    /// Sent as the request body, UTF-8; `None` sends none.
    pub body: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    /// The body as text; a body that is not UTF-8 may come back lossy, since every answer the
    /// engine reads is JSON.
    pub body: String,
}

/// Makes HTTP requests. The platform follows no redirects for the engine and adds no
/// credentials of its own: every header a request needs is in it.
#[async_trait]
pub trait Http: Send + Sync {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, TransportError>;
}

/// What a socket gave the engine next.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum Received {
    Text(String),
    /// The socket is closed, by either side, and gives nothing more.
    Closed,
}

/// One WebSocket. The platform answers the server's pings itself (URLSession and OkHttp do):
/// a voice server closes a socket it has heard nothing from, pongs included, for a minute.
#[async_trait]
pub trait Socket: Send + Sync {
    /// Sends one text frame; an error means the socket is closed or closing.
    async fn send_text(&self, text: String) -> Result<(), TransportError>;
    /// The next text frame, or `Closed` once there will be no more. Binary frames are skipped.
    /// Called by one reader at a time.
    async fn receive(&self) -> Received;
    /// Closes the socket; `receive` then answers `Closed`. Closing a closed socket does nothing.
    fn close(&self);
}

/// Opens WebSockets.
#[async_trait]
pub trait SocketFactory: Send + Sync {
    /// Connects to `url` (`ws:` or `wss:`), sending `Authorization: Bearer <bearer>` with the
    /// upgrade when one is given, and returns once the socket is open.
    async fn connect(
        &self,
        url: String,
        bearer: Option<String>,
    ) -> Result<Arc<dyn Socket>, TransportError>;
}
