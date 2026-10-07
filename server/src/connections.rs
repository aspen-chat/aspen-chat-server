//! What the API server's listener admits (`[connections]`): at most so many connections at
//! once, and so many from one address, each counted for as long as its socket is open, an
//! event stream's upgraded WebSocket included, since the socket goes with it.
//!
//! A connection over either limit is closed as soon as it is accepted. Reverse proxies named in
//! `[rate_limits] trusted_proxies` carry many clients' connections, so they count only toward
//! the total; an IPv6 address counts by its `[rate_limits] ipv6_prefix` network, as rate limits
//! count it.

use aspen_app::aspen_config::ConnectionsConfig;
use aspen_limits::ClientAddresses;
use std::collections::HashMap;
use std::io;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// How often a refusal is logged at most, in seconds, so a flood does not flood the log too.
const REFUSAL_LOG_EVERY: u64 = 60;

/// Admits connections within the limits.
pub struct Gate {
    open: Arc<Semaphore>,
    max_per_ip: usize,
    addresses: ClientAddresses,
    per_ip: Mutex<HashMap<String, usize>>,
    last_refusal_logged: AtomicU64,
}

/// One admitted connection's place within the limits, given back when it is dropped.
pub struct Admitted {
    gate: Arc<Gate>,
    key: Option<String>,
    _open: OwnedSemaphorePermit,
}

impl Gate {
    pub fn new(config: &ConnectionsConfig, addresses: ClientAddresses) -> Arc<Self> {
        Arc::new(Self {
            open: Arc::new(Semaphore::new(config.max.min(Semaphore::MAX_PERMITS))),
            max_per_ip: config.max_per_ip,
            addresses,
            per_ip: Mutex::new(HashMap::new()),
            last_refusal_logged: AtomicU64::new(0),
        })
    }

    /// A place for a connection from `peer`, or `None` when it would be over a limit.
    pub fn admit(self: &Arc<Self>, peer: IpAddr) -> Option<Admitted> {
        let Ok(open) = self.open.clone().try_acquire_owned() else {
            self.log_refusal("the server holds [connections] max connections");
            return None;
        };
        let key = if self.addresses.is_trusted(peer) {
            None
        } else {
            let key = self.addresses.key(peer);
            let mut per_ip = self.per_ip.lock().unwrap_or_else(|e| e.into_inner());
            let count = per_ip.entry(key.clone()).or_default();
            if *count >= self.max_per_ip {
                drop(per_ip);
                self.log_refusal("one address holds [connections] max_per_ip connections");
                return None;
            }
            *count += 1;
            Some(key)
        };
        Some(Admitted {
            gate: self.clone(),
            key,
            _open: open,
        })
    }

    fn log_refusal(&self, why: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_secs());
        let last = self.last_refusal_logged.load(Ordering::Relaxed);
        if now >= last + REFUSAL_LOG_EVERY
            && self
                .last_refusal_logged
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            tracing::warn!("closing new connections: {why}");
        }
    }
}

impl Drop for Admitted {
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        let mut per_ip = self.gate.per_ip.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(count) = per_ip.get_mut(&key) {
            *count -= 1;
            if *count == 0 {
                per_ip.remove(&key);
            }
        }
    }
}

/// A socket that holds its connection's place for as long as it is open.
pub struct Counted<S> {
    inner: S,
    _admitted: Admitted,
}

impl<S> Counted<S> {
    pub fn new(inner: S, admitted: Admitted) -> Self {
        Self {
            inner,
            _admitted: admitted,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Counted<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Counted<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write_vectored(cx, bufs)
    }

    fn is_write_vectored(&self) -> bool {
        self.inner.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate(max: usize, max_per_ip: usize) -> Arc<Gate> {
        let config = ConnectionsConfig {
            max,
            max_per_ip,
            ..ConnectionsConfig::default()
        };
        Gate::new(
            &config,
            ClientAddresses::new(&["10.0.0.1".to_string()], 64).unwrap(),
        )
    }

    #[test]
    fn one_address_holds_at_most_its_share() {
        let gate = gate(10, 2);
        let a: IpAddr = "203.0.113.1".parse().unwrap();
        let first = gate.admit(a).unwrap();
        let _second = gate.admit(a).unwrap();
        assert!(gate.admit(a).is_none());
        // Another address has its own share, and an IPv6 network counts as one address.
        assert!(gate.admit("203.0.113.2".parse().unwrap()).is_some());
        let _v6 = gate.admit("2001:db8::1".parse().unwrap()).unwrap();
        let _v6b = gate.admit("2001:db8::2".parse().unwrap()).unwrap();
        assert!(gate.admit("2001:db8::3".parse().unwrap()).is_none());
        drop(first);
        assert!(gate.admit(a).is_some());
    }

    #[test]
    fn everyone_together_holds_at_most_the_total() {
        let gate = gate(3, 2);
        let proxy: IpAddr = "10.0.0.1".parse().unwrap();
        // A trusted proxy is limited only by the total.
        let held: Vec<_> = (0..3).map(|_| gate.admit(proxy).unwrap()).collect();
        assert!(gate.admit("203.0.113.1".parse().unwrap()).is_none());
        drop(held);
        assert!(gate.admit("203.0.113.1".parse().unwrap()).is_some());
        assert!(gate.per_ip.lock().unwrap().is_empty());
    }
}
