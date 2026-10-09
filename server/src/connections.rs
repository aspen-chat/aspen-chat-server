//! What the API server's listener admits (`[connections]`): at most so many connections at
//! once, so many from one address, and so many from one network (an IPv4 /24 or an IPv6 /48),
//! each counted for as long as its socket is open, an event stream's upgraded WebSocket
//! included, since the socket goes with it; and how long a connection may stay open with no
//! request in it (`Activity`).
//!
//! A connection over any limit is closed as soon as it is accepted. Reverse proxies named in
//! `[rate_limits] trusted_proxies` carry many clients' connections, so they count only toward
//! the total; an IPv6 address counts by its `[rate_limits] ipv6_prefix` network, as rate limits
//! count it.
//!
//! Once someone signs in on a connection (a request presents a session, or its event stream
//! identifies), it counts toward its user's share (`max_per_user`) instead of its address's and
//! network's, while the user has room. The address limits then bound only connections nobody
//! has signed in on, and the many people behind one carrier-grade NAT are each held to their own
//! share rather than all to one address's. A connection whose user has no room left stays on
//! its address's share, so signing in never closes one.

use aspen_api::rate_limit::CountedConnection;
use aspen_app::UserId;
use aspen_app::aspen_config::ConnectionsConfig;
use aspen_limits::ClientAddresses;
use hyper::body::{Body, Frame, SizeHint};
use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

/// How often a refusal is logged at most, in seconds, so a flood does not flood the log too.
const REFUSAL_LOG_EVERY: u64 = 60;

/// Admits connections within the limits.
pub struct Gate {
    open: Arc<Semaphore>,
    max_per_ip: usize,
    max_per_network: usize,
    max_per_user: usize,
    addresses: ClientAddresses,
    held: Mutex<Held>,
    last_refusal_logged: AtomicU64,
}

/// What untrusted addresses hold open, by address and by network, and what users who signed in
/// on their connections hold.
#[derive(Default)]
struct Held {
    per_ip: HashMap<String, usize>,
    per_network: HashMap<IpAddr, usize>,
    per_user: HashMap<UserId, usize>,
}

/// Counts one more against `key`, or none when it already holds `max`.
fn take<K: std::hash::Hash + Eq>(counts: &mut HashMap<K, usize>, key: K, max: usize) -> bool {
    let count = counts.entry(key).or_default();
    if *count >= max {
        return false;
    }
    *count += 1;
    true
}

/// Gives back one taken against `key`, forgetting it at none.
fn give_back<K: std::hash::Hash + Eq>(counts: &mut HashMap<K, usize>, key: &K) {
    if let Some(count) = counts.get_mut(key) {
        *count -= 1;
        if *count == 0 {
            counts.remove(key);
        }
    }
}

/// The network an address counts toward: its IPv4 /24, or its IPv6 /48, the block a single
/// site is usually given, so one holder of many addresses cannot take the server's share of
/// connections by spreading them over its addresses.
fn network(ip: IpAddr) -> IpAddr {
    match aspen_limits::canonical(ip) {
        IpAddr::V4(v4) => IpAddr::V4(Ipv4Addr::from_bits(v4.to_bits() & !0xff)),
        IpAddr::V6(v6) => IpAddr::V6(Ipv6Addr::from_bits(v6.to_bits() & !((1u128 << 80) - 1))),
    }
}

/// Whose share one connection counts toward, besides the total.
enum Share {
    /// A trusted proxy's, or one given back.
    None,
    /// Its address's and its network's.
    Address(String, IpAddr),
    /// The user who signed in on it.
    User(UserId),
}

/// One admitted connection's place within the limits, given back when it is dropped.
pub struct Admitted {
    place: Arc<Place>,
    _open: OwnedSemaphorePermit,
}

/// Where an admitted connection counts, which moves to its user when someone signs in on it.
pub struct Place {
    gate: Arc<Gate>,
    share: Mutex<Share>,
}

impl Gate {
    pub fn new(config: &ConnectionsConfig, addresses: ClientAddresses) -> Arc<Self> {
        Arc::new(Self {
            open: Arc::new(Semaphore::new(config.max.min(Semaphore::MAX_PERMITS))),
            max_per_ip: config.max_per_ip,
            max_per_network: config.max_per_network,
            max_per_user: config.max_per_user,
            addresses,
            held: Mutex::new(Held::default()),
            last_refusal_logged: AtomicU64::new(0),
        })
    }

    /// A place for a connection from `peer`, or `None` when it would be over a limit.
    pub fn admit(self: &Arc<Self>, peer: IpAddr) -> Option<Admitted> {
        let Ok(open) = self.open.clone().try_acquire_owned() else {
            self.log_refusal("the server holds [connections] max connections");
            return None;
        };
        let share = if self.addresses.is_trusted(peer) {
            Share::None
        } else {
            let key = self.addresses.key(peer);
            let network = network(peer);
            let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
            if !take(&mut held.per_ip, key.clone(), self.max_per_ip) {
                drop(held);
                self.log_refusal("one address holds [connections] max_per_ip connections");
                return None;
            }
            if !take(&mut held.per_network, network, self.max_per_network) {
                give_back(&mut held.per_ip, &key);
                drop(held);
                self.log_refusal("one network holds [connections] max_per_network connections");
                return None;
            }
            Share::Address(key, network)
        };
        Some(Admitted {
            place: Arc::new(Place {
                gate: self.clone(),
                share: Mutex::new(share),
            }),
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

impl Admitted {
    /// Where the connection counts, for its requests to say who signed in on it.
    pub fn place(&self) -> Arc<Place> {
        self.place.clone()
    }
}

impl Drop for Admitted {
    fn drop(&mut self) {
        self.place.give_back();
    }
}

impl Place {
    fn give_back(&self) {
        let mut share = self.share.lock().unwrap_or_else(|e| e.into_inner());
        let mut held = self.gate.held.lock().unwrap_or_else(|e| e.into_inner());
        match std::mem::replace(&mut *share, Share::None) {
            Share::None => {}
            Share::Address(key, network) => {
                give_back(&mut held.per_ip, &key);
                give_back(&mut held.per_network, &network);
            }
            Share::User(user) => give_back(&mut held.per_user, &user),
        }
    }
}

impl CountedConnection for Place {
    fn signed_in(&self, user: UserId) {
        let mut share = self.share.lock().unwrap_or_else(|e| e.into_inner());
        let Share::Address(key, network) = &*share else {
            return;
        };
        let mut held = self.gate.held.lock().unwrap_or_else(|e| e.into_inner());
        if !take(&mut held.per_user, user, self.gate.max_per_user) {
            return;
        }
        give_back(&mut held.per_ip, key);
        give_back(&mut held.per_network, network);
        drop(held);
        *share = Share::User(user);
    }
}

/// Whether one connection has a request open, and since when it has had none, so a connection
/// that stays open with nothing in it is closed after `[connections] idle_seconds`. HTTP/2
/// keeps a connection open between requests for as long as the client likes, answering pings,
/// and each one takes a place within the limits. A request counts as open from when it reaches
/// the service until its response's body is dropped, sent or abandoned. An HTTP/1.1 connection
/// upgraded to a WebSocket is no longer the HTTP connection's, so it is not closed by this.
pub struct Activity {
    open: AtomicUsize,
    /// Milliseconds after `since` when a request last opened or closed.
    last_ms: AtomicU64,
    since: Instant,
}

/// One open request's place in its connection's `Activity`.
pub struct Busy(Arc<Activity>);

impl Activity {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            open: AtomicUsize::new(0),
            last_ms: AtomicU64::new(0),
            since: Instant::now(),
        })
    }

    fn touch(&self) {
        let elapsed = u64::try_from(self.since.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_ms.store(elapsed, Ordering::SeqCst);
    }

    /// Marks a request open until the returned `Busy` is dropped.
    pub fn begin(self: &Arc<Self>) -> Busy {
        self.open.fetch_add(1, Ordering::SeqCst);
        self.touch();
        Busy(self.clone())
    }

    /// Finishes once the connection has had no request open for `after`.
    pub async fn idle(&self, after: Duration) {
        loop {
            let last = self.since + Duration::from_millis(self.last_ms.load(Ordering::SeqCst));
            let deadline = last + after;
            if Instant::now() < deadline {
                tokio::time::sleep_until(deadline).await;
            } else if self.open.load(Ordering::SeqCst) == 0 {
                return;
            } else {
                // A request is open; when it closes it moves the deadline on.
                tokio::time::sleep(after).await;
            }
        }
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.touch();
        self.0.open.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A response body that keeps its request counted as open (`Busy`) until it is dropped.
pub struct Tracked<B> {
    inner: B,
    _busy: Busy,
}

impl<B> Tracked<B> {
    pub fn new(inner: B, busy: Busy) -> Self {
        Self { inner, _busy: busy }
    }
}

impl<B: Body + Unpin> Body for Tracked<B> {
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.inner).poll_frame(cx)
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
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
        gate_with_networks(max, max_per_ip, usize::MAX)
    }

    fn gate_with_networks(max: usize, max_per_ip: usize, max_per_network: usize) -> Arc<Gate> {
        let config = ConnectionsConfig {
            max,
            max_per_ip,
            max_per_network,
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
        assert!(gate.held.lock().unwrap().per_ip.is_empty());
    }

    #[test]
    fn one_network_holds_at_most_its_share() {
        let gate = gate_with_networks(100, 2, 3);
        let held: Vec<_> = ["203.0.113.1", "203.0.113.2", "203.0.113.3"]
            .iter()
            .map(|a| gate.admit(a.parse().unwrap()).unwrap())
            .collect();
        // The same /24, though another address.
        assert!(gate.admit("203.0.113.200".parse().unwrap()).is_none());
        // The refusal gave back the address's place: it holds none.
        assert!(
            !gate
                .held
                .lock()
                .unwrap()
                .per_ip
                .contains_key("203.0.113.200")
        );
        assert!(gate.admit("203.0.114.1".parse().unwrap()).is_some());
        // An IPv6 /48 is one network, whatever /64 within it.
        let _a = gate.admit("2001:db8:1:1::1".parse().unwrap()).unwrap();
        let _b = gate.admit("2001:db8:1:2::1".parse().unwrap()).unwrap();
        let _c = gate.admit("2001:db8:1:3::1".parse().unwrap()).unwrap();
        assert!(gate.admit("2001:db8:1:4::1".parse().unwrap()).is_none());
        assert!(gate.admit("2001:db8:2::1".parse().unwrap()).is_some());
        drop(held);
        assert!(gate.admit("203.0.113.200".parse().unwrap()).is_some());
    }

    #[test]
    fn signing_in_moves_a_connection_to_its_users_share() {
        let config = ConnectionsConfig {
            max: 100,
            max_per_ip: 2,
            max_per_network: usize::MAX,
            max_per_user: 2,
            ..ConnectionsConfig::default()
        };
        let gate = Gate::new(&config, ClientAddresses::new(&[], 64).unwrap());
        let nat: IpAddr = "203.0.113.1".parse().unwrap();
        let (alice, bob) = (UserId::new(), UserId::new());
        let first = gate.admit(nat).unwrap();
        let second = gate.admit(nat).unwrap();
        assert!(gate.admit(nat).is_none());
        // Signed in, they leave the address's share for their users'.
        first.place().signed_in(alice);
        second.place().signed_in(bob);
        // Signing in again on the same connection changes nothing.
        first.place().signed_in(bob);
        let third = gate.admit(nat).unwrap();
        let fourth = gate.admit(nat).unwrap();
        assert!(gate.admit(nat).is_none());
        // Alice has room for one more; past it, a connection stays on the address's share.
        third.place().signed_in(alice);
        fourth.place().signed_in(alice);
        let fifth = gate.admit(nat).unwrap();
        assert!(gate.admit(nat).is_none());
        drop((first, second, third, fourth, fifth));
        let held = gate.held.lock().unwrap();
        assert!(held.per_user.is_empty());
        assert!(held.per_ip.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_connection_is_idle_only_with_no_request_open() {
        let activity = Activity::new();
        let after = Duration::from_secs(120);
        let busy = activity.begin();
        // A request open for longer than the idle time keeps the connection.
        let idle = tokio::time::timeout(Duration::from_secs(300), activity.idle(after)).await;
        assert!(idle.is_err());
        drop(busy);
        let start = Instant::now();
        activity.idle(after).await;
        assert!(start.elapsed() >= after);
    }
}
