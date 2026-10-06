//! What file transfers between the people of a call need from this server: STUN, so that two
//! devices can find the addresses a direct connection would use, and a TURN relay for transfers
//! that go through the server, on one UDP port (`[transfer]`). The offers and transfers
//! themselves are the rooms' (`rooms`); this is the network side.
//!
//! The relay is for transfers between participants of calls here and nothing else:
//! - It authenticates a transfer's credentials only while that transfer is live, and deletes
//!   its allocations the moment it ends, so a cancelled transfer stops at the relay too.
//! - One side of one transfer holds at most `ADDRESSES_PER_CREDENTIAL` allocations: its
//!   credentials are accepted from that many client addresses and no more, and an allocation is
//!   bound to the address that asked for it, so no one can take the relay's ports with a
//!   credential handed out for one transfer.
//! - It forwards only between allocations of its own. Every transfer through it is relayed at
//!   both ends (a relay candidate pairs only with the other side's relay candidate), so it is
//!   never an open relay to the rest of the internet. Traffic between two allocations is looped
//!   back inside the server rather than sent to its own public address, which a server behind
//!   NAT may not reach.
//! - Everything it forwards shares one rate of `relay_mbps` for the whole server. It is shaped
//!   rather than policed: what arrives faster waits in one queue, in order, and a pacer sends it
//!   at the rate, so a transfer's congestion control sees delay and settles near the rate. Past
//!   half full, the queue drops the odd datagram, more often the fuller it is (random early
//!   drop), so a sender slows on one loss it recovers from at once, rather than filling the
//!   queue and losing a run of datagrams, which stalls it until a retransmission timeout. The queue is beside the
//!   TURN server's read loop, never in it, so waiting data holds up no one's control messages.
//!
//! The data channel between the two devices is encrypted end to end (DTLS), so the relay
//! carries only ciphertext.

use crate::config::TransferConfig;
use async_trait::async_trait;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hmac::{Hmac, Mac};
use rand::RngExt as _;
use sha2::Sha256;
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{info, warn};
use turn::auth::{AuthHandler, generate_auth_key};
use turn::relay::RelayAddressGenerator;
use turn::server::Server;
use turn::server::config::{ConnConfig, ServerConfig};
use uuid::Uuid;
use voice_protocol::signal::{IceServer, TransferMode, TransferPolicy, TransferRole};
use webrtc_util::Conn;

/// The realm TURN credentials are for.
const REALM: &str = "aspen";
/// The client addresses one side of one transfer may use the relay from, and so the most
/// allocations it holds (the relay listens on one socket, so each allocation is one client
/// address). A browser allocates from each network interface it gathers on; two leave room for
/// a second interface or one change of address on the way.
const ADDRESSES_PER_CREDENTIAL: usize = 2;
/// How much the relay holds waiting to be sent, in seconds of its rate.
const QUEUE_SECONDS: f64 = 0.2;
/// The least it holds, so that a slow rate still queues a window's worth.
const MIN_QUEUE_BYTES: usize = 256 * 1024;
/// The chance of dropping a datagram when the queue is all but full; it rises from nothing at
/// half full.
const MAX_EARLY_DROP: f64 = 0.02;
/// How far the pacer may get ahead of its rate after a pause, in seconds of the rate: enough
/// to fill the gaps a timer's resolution leaves, too little to let a burst through.
const PACER_BURST_SECONDS: f64 = 0.002;
/// The least it may get ahead, so that one full-sized datagram always fits.
const MIN_PACER_BURST_BYTES: f64 = 16.0 * 1024.0;

/// The server's STUN and TURN, and the transfers whose credentials it accepts.
pub struct Relay {
    policy: TransferPolicy,
    /// Where clients reach STUN and TURN, as ICE server URLs name it.
    host: String,
    port: u16,
    secret: [u8; 32],
    live: Arc<Mutex<LiveCredentials>>,
    server: Server,
}

/// The credentials of live transfers, by username, with the client addresses each has been
/// accepted from.
type LiveCredentials = HashMap<String, HashSet<SocketAddr>>;

impl Relay {
    /// Starts STUN and TURN on `config.port` of `bind`, announced as `announced` (the media
    /// address clients already reach), relaying at most `config.relay_mbps` in all.
    pub async fn start(
        config: &TransferConfig,
        bind: IpAddr,
        announced: &str,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            config.relay_min_port <= config.relay_max_port,
            "transfer.relay_min_port is above transfer.relay_max_port"
        );
        let announced_ip = tokio::net::lookup_host((announced, 0))
            .await?
            .map(|address| address.ip())
            .find(IpAddr::is_ipv4)
            .ok_or_else(|| anyhow::anyhow!("{announced} has no IPv4 address to relay on"))?;
        let listener = UdpSocket::bind(SocketAddr::new(bind, config.port)).await?;
        let live: Arc<Mutex<LiveCredentials>> = Arc::default();
        let secret = rand::rng().random::<[u8; 32]>();
        let relaying = config.relay_mbps > 0;
        let pacer = Pacer::start(config.relay_mbps);
        let server = Server::new(ServerConfig {
            conn_configs: vec![ConnConfig {
                conn: Arc::new(listener),
                relay_addr_generator: Box::new(Allocations {
                    bind,
                    announced: announced_ip,
                    min_port: config.relay_min_port,
                    max_port: config.relay_max_port,
                    next: AtomicU16::new(config.relay_min_port),
                    pacer,
                }),
            }],
            realm: REALM.to_string(),
            auth_handler: Arc::new(Credentials {
                relaying,
                secret,
                live: live.clone(),
            }),
            channel_bind_timeout: std::time::Duration::from_secs(0),
            alloc_close_notify: None,
        })
        .await?;
        info!(
            port = config.port,
            relay_mbps = config.relay_mbps,
            "STUN and TURN listening for file transfers"
        );
        Ok(Self {
            policy: TransferPolicy {
                relay_mbps: relaying.then_some(config.relay_mbps),
            },
            host: announced.to_string(),
            port: config.port,
            secret,
            live,
            server,
        })
    }

    /// Whether transfers may be relayed here, and how fast.
    pub fn policy(&self) -> TransferPolicy {
        self.policy.clone()
    }

    /// Opens one side (`role`) of the transfer of `offer` to `receiver`, and says which ICE
    /// servers it uses: STUN, unless the transfer is relayed only, and TURN with credentials for
    /// this side of this transfer alone, when the server relays.
    pub fn open(
        &self,
        offer: Uuid,
        receiver: Uuid,
        role: TransferRole,
        mode: TransferMode,
    ) -> Vec<IceServer> {
        let mut servers = Vec::new();
        if mode == TransferMode::DirectPreferred {
            servers.push(IceServer {
                urls: vec![format!("stun:{}:{}", self.host, self.port)],
                username: None,
                credential: None,
            });
        }
        if self.policy.relay_mbps.is_some() {
            let username = username(offer, receiver, role);
            let credential = password(&self.secret, &username);
            self.live
                .lock()
                .expect("live transfers")
                .entry(username.clone())
                .or_default();
            servers.push(IceServer {
                urls: vec![format!("turn:{}:{}?transport=udp", self.host, self.port)],
                username: Some(username),
                credential: Some(credential),
            });
        }
        servers
    }

    /// Ends one side of the transfer of `offer` to `receiver` at the relay: its credentials
    /// stop working and its allocations close.
    pub async fn close(&self, offer: Uuid, receiver: Uuid, role: TransferRole) {
        let username = username(offer, receiver, role);
        let was_live = self
            .live
            .lock()
            .expect("live transfers")
            .remove(&username)
            .is_some();
        if was_live && let Err(e) = self.server.delete_allocations_by_username(username).await {
            warn!(
                error = e.to_string(),
                "could not close a transfer's allocations"
            );
        }
    }

    pub async fn shutdown(&self) {
        if let Err(e) = self.server.close().await {
            warn!(error = e.to_string(), "closing STUN and TURN failed");
        }
    }
}

/// Names one side of one transfer: a sender with several receivers holds one per transfer.
fn username(offer: Uuid, receiver: Uuid, role: TransferRole) -> String {
    let side = match role {
        TransferRole::Sender => "s",
        TransferRole::Receiver => "r",
    };
    format!("{offer}:{receiver}:{side}")
}

fn password(secret: &[u8], username: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).expect("any key length");
    mac.update(username.as_bytes());
    STANDARD.encode(mac.finalize().into_bytes())
}

/// Accepts the credentials of live transfers, only while the server relays, and from at most
/// `ADDRESSES_PER_CREDENTIAL` client addresses each.
struct Credentials {
    relaying: bool,
    secret: [u8; 32],
    live: Arc<Mutex<LiveCredentials>>,
}

impl AuthHandler for Credentials {
    fn auth_handle(
        &self,
        username: &str,
        realm: &str,
        src_addr: SocketAddr,
    ) -> Result<Vec<u8>, turn::Error> {
        if !self.relaying {
            return Err(turn::Error::Other("this server does not relay".to_string()));
        }
        {
            let mut live = self.live.lock().expect("live transfers");
            let Some(addresses) = live.get_mut(username) else {
                return Err(turn::Error::Other("no such transfer".to_string()));
            };
            // Every request of an allocation comes from the address that made it, so counting
            // addresses counts allocations, and an allocation already made keeps working.
            if !addresses.contains(&src_addr) {
                if addresses.len() >= ADDRESSES_PER_CREDENTIAL {
                    return Err(turn::Error::Other(
                        "this transfer already relays from as many addresses as it may".to_string(),
                    ));
                }
                addresses.insert(src_addr);
            }
        }
        Ok(generate_auth_key(
            username,
            realm,
            &password(&self.secret, username),
        ))
    }
}

/// One datagram waiting to be relayed.
struct Datagram {
    socket: Arc<UdpSocket>,
    to: SocketAddr,
    data: Vec<u8>,
}

/// The relay's shared rate: one queue for every allocation, sent from in order at `relay_mbps`.
struct Pacer {
    queue: mpsc::UnboundedSender<Datagram>,
    /// Bytes waiting in the queue.
    queued: Arc<AtomicUsize>,
    limit: usize,
}

impl Pacer {
    fn start(mbps: u32) -> Arc<Self> {
        let rate = f64::from(mbps) * 1_000_000.0 / 8.0;
        let limit = ((rate * QUEUE_SECONDS) as usize).max(MIN_QUEUE_BYTES);
        let burst = (rate * PACER_BURST_SECONDS).max(MIN_PACER_BURST_BYTES);
        let (queue, mut waiting) = mpsc::unbounded_channel::<Datagram>();
        let queued = Arc::new(AtomicUsize::new(0));
        let sent = queued.clone();
        tokio::spawn(async move {
            let mut tokens = burst;
            let mut then = Instant::now();
            while let Some(datagram) = waiting.recv().await {
                let size = datagram.data.len() as f64;
                loop {
                    let now = Instant::now();
                    tokens = (tokens + now.duration_since(then).as_secs_f64() * rate).min(burst);
                    then = now;
                    if tokens >= size || rate == 0.0 {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs_f64((size - tokens) / rate))
                        .await;
                }
                tokens -= size;
                sent.fetch_sub(datagram.data.len(), Ordering::Relaxed);
                if datagram
                    .socket
                    .send_to(&datagram.data, datagram.to)
                    .await
                    .is_ok()
                {
                    metrics::counter!(aspen_metrics::voice::RELAYED_BYTES)
                        .increment(datagram.data.len() as u64);
                }
            }
        });
        Arc::new(Self {
            queue,
            queued,
            limit,
        })
    }

    /// Queues a datagram to be sent at the rate; false when it is dropped, early or because the
    /// queue is full.
    fn offer(&self, datagram: Datagram) -> bool {
        let size = datagram.data.len();
        let half = self.limit / 2;
        let waiting = self.queued.load(Ordering::Relaxed);
        if waiting > half {
            let fullness = (waiting - half) as f64 / half as f64;
            if rand::rng().random::<f64>() < fullness * MAX_EARLY_DROP {
                return false;
            }
        }
        if self.queued.fetch_add(size, Ordering::Relaxed) + size > self.limit {
            self.queued.fetch_sub(size, Ordering::Relaxed);
            return false;
        }
        if self.queue.send(datagram).is_err() {
            self.queued.fetch_sub(size, Ordering::Relaxed);
            return false;
        }
        true
    }
}

/// Makes each allocation's relay socket: a port from the relay range, announced at the media
/// address, wrapped so it forwards only to other allocations, at the shared rate.
struct Allocations {
    bind: IpAddr,
    announced: IpAddr,
    min_port: u16,
    max_port: u16,
    next: AtomicU16,
    pacer: Arc<Pacer>,
}

#[async_trait]
impl RelayAddressGenerator for Allocations {
    fn validate(&self) -> Result<(), turn::Error> {
        Ok(())
    }

    async fn allocate_conn(
        &self,
        use_ipv4: bool,
        _requested_port: u16,
    ) -> Result<(Arc<dyn Conn + Send + Sync>, SocketAddr), turn::Error> {
        if !use_ipv4 {
            return Err(turn::Error::Other(
                "transfers are relayed over IPv4".to_string(),
            ));
        }
        let span = u32::from(self.max_port - self.min_port) + 1;
        for _ in 0..span {
            let offset = u32::from(self.next.fetch_add(1, Ordering::Relaxed)) % span;
            let port = self.min_port + offset as u16;
            if let Ok(socket) = UdpSocket::bind(SocketAddr::new(self.bind, port)).await {
                let conn = RelayConn {
                    socket: Arc::new(socket),
                    announced: self.announced,
                    loopback: if self.bind.is_unspecified() {
                        IpAddr::V4(Ipv4Addr::LOCALHOST)
                    } else {
                        self.bind
                    },
                    min_port: self.min_port,
                    max_port: self.max_port,
                    pacer: self.pacer.clone(),
                };
                return Ok((Arc::new(conn), SocketAddr::new(self.announced, port)));
            }
        }
        Err(turn::Error::Other("every relay port is taken".to_string()))
    }
}

/// One allocation's relay socket.
struct RelayConn {
    socket: Arc<UdpSocket>,
    /// The address allocations are announced at.
    announced: IpAddr,
    /// Where another allocation is reached from inside the server.
    loopback: IpAddr,
    min_port: u16,
    max_port: u16,
    pacer: Arc<Pacer>,
}

impl RelayConn {
    fn is_relay_port(&self, port: u16) -> bool {
        (self.min_port..=self.max_port).contains(&port)
    }
}

#[async_trait]
impl Conn for RelayConn {
    async fn connect(&self, addr: SocketAddr) -> Result<(), webrtc_util::Error> {
        self.socket.connect(addr).await?;
        Ok(())
    }

    async fn recv(&self, buf: &mut [u8]) -> Result<usize, webrtc_util::Error> {
        Ok(self.socket.recv(buf).await?)
    }

    /// What another allocation sent, named by its announced address as the peer knows it.
    async fn recv_from(&self, buf: &mut [u8]) -> Result<(usize, SocketAddr), webrtc_util::Error> {
        loop {
            let (n, from) = self.socket.recv_from(buf).await?;
            if from.ip() == self.loopback && self.is_relay_port(from.port()) {
                return Ok((n, SocketAddr::new(self.announced, from.port())));
            }
            // Nothing but another allocation may send through the relay.
        }
    }

    async fn send(&self, buf: &[u8]) -> Result<usize, webrtc_util::Error> {
        Ok(self.socket.send(buf).await?)
    }

    /// Queues for another allocation at the shared rate; anything else, and what overflows the
    /// queue, is dropped as though sent, which is what a relay does with what it will not carry.
    async fn send_to(&self, buf: &[u8], target: SocketAddr) -> Result<usize, webrtc_util::Error> {
        if target.ip() != self.announced || !self.is_relay_port(target.port()) {
            return Ok(buf.len());
        }
        let queued = self.pacer.offer(Datagram {
            socket: self.socket.clone(),
            to: SocketAddr::new(self.loopback, target.port()),
            data: buf.to_vec(),
        });
        if !queued {
            metrics::counter!(aspen_metrics::voice::RELAY_DROPPED_BYTES)
                .increment(buf.len() as u64);
        }
        Ok(buf.len())
    }

    fn local_addr(&self) -> Result<SocketAddr, webrtc_util::Error> {
        Ok(self.socket.local_addr()?)
    }

    fn remote_addr(&self) -> Option<SocketAddr> {
        self.socket.peer_addr().ok()
    }

    async fn close(&self) -> Result<(), webrtc_util::Error> {
        Ok(())
    }

    fn as_any(&self) -> &(dyn std::any::Any + Send + Sync) {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_pacer_sends_at_its_rate_and_drops_only_what_overflows() {
        let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let to = receiver.local_addr().unwrap();
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let pacer = Pacer::start(8); // a megabyte a second; its queue holds 256 KiB
        let datagram = || Datagram {
            socket: socket.clone(),
            to,
            data: vec![0; 1000],
        };
        let started = Instant::now();
        let accepted = (0..400).filter(|_| pacer.offer(datagram())).count();
        // The queue takes 256 KiB of the 400 KB offered at once, and drops the rest.
        assert!((250..=265).contains(&accepted), "{accepted}");
        let mut buf = [0u8; 2000];
        for _ in 0..accepted {
            receiver.recv_from(&mut buf).await.unwrap();
        }
        // About a quarter of a second at a megabyte a second.
        let took = started.elapsed().as_secs_f64();
        assert!((0.2..0.4).contains(&took), "{took}");
    }

    #[test]
    fn credentials_name_one_side_of_one_transfer() {
        let secret = [7; 32];
        let (offer, a, b) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let sender_side = username(offer, a, TransferRole::Sender);
        assert_ne!(sender_side, username(offer, a, TransferRole::Receiver));
        assert_ne!(sender_side, username(offer, b, TransferRole::Sender));
        assert_ne!(
            password(&secret, &sender_side),
            password(&secret, &username(offer, b, TransferRole::Sender))
        );
        let credentials = Credentials {
            relaying: true,
            secret,
            live: Arc::default(),
        };
        let from = SocketAddr::from(([192, 0, 2, 1], 5000));
        assert!(credentials.auth_handle(&sender_side, REALM, from).is_err());
        credentials
            .live
            .lock()
            .unwrap()
            .insert(sender_side.clone(), HashSet::new());
        assert_eq!(
            credentials.auth_handle(&sender_side, REALM, from).unwrap(),
            generate_auth_key(&sender_side, REALM, &password(&secret, &sender_side))
        );
        // A second address is taken, a third refused, and the first still works.
        let second = SocketAddr::from(([192, 0, 2, 1], 5001));
        let third = SocketAddr::from(([192, 0, 2, 2], 5000));
        assert!(credentials.auth_handle(&sender_side, REALM, second).is_ok());
        assert!(credentials.auth_handle(&sender_side, REALM, third).is_err());
        assert!(credentials.auth_handle(&sender_side, REALM, from).is_ok());
        let off = Credentials {
            relaying: false,
            ..credentials
        };
        assert!(off.auth_handle(&sender_side, REALM, from).is_err());
    }
}
