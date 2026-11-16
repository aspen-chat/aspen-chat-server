//! The ring watch (`ring`) as Swift and Kotlin reach it, through UniFFI, beside the engine
//! (`bindings`): the platform's `Http` and `SocketFactory` as declared there, wrapped here into
//! the engine's traits, a `RingObserver` of the platform's, and the `RingWatch` object the apps
//! hold while an incoming call rings. `Ring` and `RingEnd` cross as they are.
//!
//! Every observer method is called on the watch's thread; a foreign implementation must not
//! throw (UniFFI has nowhere to put the exception), nor release the watch from inside `ended`,
//! which runs on the thread releasing it waits for.

use crate::api::Credentials;
use crate::bindings;
use crate::ring::{self, Ring, RingEnd};
use crate::transport;
use async_trait::async_trait;
use std::sync::Arc;

/// Hears the ring's end, once (`ring::RingObserver`).
#[uniffi::export(with_foreign)]
pub trait RingObserver: Send + Sync {
    fn ended(&self, end: RingEnd);
}

struct ForeignRingObserver(Arc<dyn RingObserver>);

impl ring::RingObserver for ForeignRingObserver {
    fn ended(&self, end: RingEnd) {
        self.0.ended(end);
    }
}

/// `bindings::Http` as the engine's `transport::Http`.
struct RingHttp(Arc<dyn bindings::Http>);

#[async_trait]
impl transport::Http for RingHttp {
    async fn request(
        &self,
        request: transport::HttpRequest,
    ) -> Result<transport::HttpResponse, transport::TransportError> {
        Ok(self.0.request(request.into()).await?.into())
    }
}

/// `bindings::Socket` as the engine's `transport::Socket`.
struct RingSocket(Arc<dyn bindings::Socket>);

#[async_trait]
impl transport::Socket for RingSocket {
    async fn send_text(&self, text: String) -> Result<(), transport::TransportError> {
        Ok(self.0.send_text(text).await?)
    }

    async fn receive(&self) -> transport::Received {
        self.0.receive().await
    }

    fn close(&self) {
        self.0.close_socket();
    }
}

/// `bindings::SocketFactory` as the engine's `transport::SocketFactory`.
struct RingSockets(Arc<dyn bindings::SocketFactory>);

#[async_trait]
impl transport::SocketFactory for RingSockets {
    async fn connect(
        &self,
        url: String,
        bearer: Option<String>,
    ) -> Result<Arc<dyn transport::Socket>, transport::TransportError> {
        let connected = self.0.connect(url, bearer).await?;
        Ok(Arc::new(RingSocket(connected.socket)))
    }
}

/// The platform under a watch (`ring::Platform`): what the engine's `Platform` has besides peer
/// connections, which a ring needs none of.
#[derive(uniffi::Record)]
pub struct RingPlatform {
    pub http: Arc<dyn bindings::Http>,
    pub sockets: Arc<dyn bindings::SocketFactory>,
}

/// `ring::Options`.
#[derive(uniffi::Record)]
pub struct RingOptions {
    /// The deployment's `public_url`.
    pub api_url: String,
    pub credentials: Arc<dyn Credentials>,
    /// Whether a plain `http:` deployment at this machine's own addresses may be reached, as a
    /// development build allows.
    pub loopback_plain_text: bool,
}

/// `ring::RingWatch` as the apps hold it while an incoming call rings: started as the call is
/// reported to the system, stopped when this phone answers or declines, and its observer told
/// when the ring ends anywhere else. Releasing the last reference stops it and waits for its
/// thread.
#[derive(uniffi::Object)]
pub struct RingWatch(ring::RingWatch);

#[uniffi::export]
impl RingWatch {
    /// Starts watching `ring` on a thread of its own; `observer.ended` is called there, once at
    /// most.
    #[uniffi::constructor]
    pub fn start(
        platform: RingPlatform,
        options: RingOptions,
        ring: Ring,
        observer: Arc<dyn RingObserver>,
    ) -> Arc<Self> {
        let platform = ring::Platform {
            http: Arc::new(RingHttp(platform.http)),
            sockets: Arc::new(RingSockets(platform.sockets)),
        };
        let options = ring::Options {
            api_url: options.api_url,
            credentials: options.credentials,
            loopback_plain_text: options.loopback_plain_text,
        };
        Arc::new(Self(ring::RingWatch::start(
            platform,
            options,
            ring,
            Arc::new(ForeignRingObserver(observer)),
        )))
    }

    /// Ends the watch with no report: this phone answered or declined the ring itself.
    pub fn stop(&self) {
        self.0.stop();
    }
}
