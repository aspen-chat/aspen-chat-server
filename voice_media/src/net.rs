//! A media thread's sockets: on each media address, one UDP socket and one TCP listener on the
//! thread's port, and the TCP streams accepted from clients UDP cannot reach. A stream carries
//! ICE, DTLS, and SRTP framed as RFC 4571 says, each packet led by its length in two bytes.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::time::Instant;
use str0m::Candidate;
use str0m::net::TcpType;

/// The receive and send buffers each UDP socket asks for. Every connection on the thread shares
/// the socket, so a burst (a thousand DTLS handshakes at once) or a moment the thread falls
/// behind must fit; the kernel caps it at `net.core.rmem_max`.
pub const UDP_BUFFER_BYTES: usize = 4 * 1024 * 1024;
/// What a stream may hold waiting to be written, past which its client is too slow and the
/// stream is dropped (ICE then tries UDP or another stream).
const TCP_OUTBOX_BYTES: usize = 1024 * 1024;
/// How long a stream may go without one of its packets being taken by a connection.
pub const TCP_UNIDENTIFIED_SECONDS: u64 = 10;
/// Streams a thread holds at once.
pub const MAX_TCP_STREAMS: usize = 4096;

pub struct Listener {
    pub udp: UdpSocket,
    pub tcp: TcpListener,
    /// Where str0m is told packets arrive over each: the announced address when there is one,
    /// since that is what its candidates name, on the port each is bound to.
    pub udp_local: SocketAddr,
    pub tcp_local: SocketAddr,
}

/// How many free UDP ports `Listener::bind` tries, given port 0, for one whose number TCP also
/// has free.
const EPHEMERAL_ATTEMPTS: u32 = 16;

impl Listener {
    /// Binds UDP and TCP on `port` of `ip`. Port 0, as tests and `estimate-capacity` give,
    /// takes a free UDP port and the same for TCP, trying other ports while something else
    /// holds that one over TCP (`EPHEMERAL_ATTEMPTS`).
    pub fn bind(
        ip: std::net::IpAddr,
        announced: Option<std::net::IpAddr>,
        port: u16,
    ) -> io::Result<Self> {
        let mut attempts = if port == 0 { EPHEMERAL_ATTEMPTS } else { 1 };
        let (udp, tcp, port, bound) = loop {
            let bound = SocketAddr::new(ip, port);
            let udp = aspen_sockets::std_udp_socket(bound, Some(UDP_BUFFER_BYTES))
                .map_err(|e| io::Error::new(e.kind(), format!("media UDP on {bound}: {e}")))?;
            let port = udp.local_addr()?.port();
            let bound = SocketAddr::new(ip, port);
            match aspen_sockets::std_tcp_listener(bound) {
                Ok(tcp) => break (udp, tcp, port, bound),
                Err(e) if e.kind() == io::ErrorKind::AddrInUse && attempts > 1 => attempts -= 1,
                Err(e) => {
                    return Err(io::Error::new(
                        e.kind(),
                        format!("media TCP on {bound}: {e}"),
                    ));
                }
            }
        };
        if let Ok((receive, _)) = aspen_sockets::buffer_sizes(&udp)
            && receive < UDP_BUFFER_BYTES
        {
            tracing::warn!(
                "media UDP on {bound} has a {receive}-byte receive buffer, less than the \
                 {UDP_BUFFER_BYTES} asked for; raise net.core.rmem_max (and wmem_max) or bursts of \
                 joins may drop packets"
            );
        }
        let shown = announced.unwrap_or(ip);
        Ok(Self {
            udp_local: SocketAddr::new(shown, port),
            tcp_local: SocketAddr::new(shown, tcp.local_addr()?.port()),
            udp,
            tcp,
        })
    }

    /// A host candidate over UDP and a passive one over TCP, as every connection is offered.
    pub fn candidates(&self) -> [Candidate; 2] {
        [
            Candidate::host(self.udp_local, "udp").expect("a host candidate for a bound address"),
            Candidate::builder()
                .tcp()
                .host(self.tcp_local)
                .tcptype(TcpType::Passive)
                .build()
                .expect("a passive TCP candidate for a bound address"),
        ]
    }
}

/// An accepted TCP stream.
pub struct Stream {
    pub stream: TcpStream,
    pub peer: SocketAddr,
    /// The listener (media address) it was accepted on.
    pub listener: usize,
    read: Vec<u8>,
    outbox: VecDeque<u8>,
    pub accepted_at: Instant,
    /// Whether a connection has taken one of its packets.
    pub identified: bool,
}

impl Stream {
    pub fn new(stream: TcpStream, peer: SocketAddr, listener: usize) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            peer,
            listener,
            read: Vec::new(),
            outbox: VecDeque::new(),
            accepted_at: Instant::now(),
            identified: false,
        })
    }

    /// Reads what has arrived and gives each whole packet to `packet`; `Err` when the stream has
    /// ended or broke.
    pub fn receive(&mut self, packet: &mut dyn FnMut(&[u8])) -> io::Result<()> {
        let mut buf = [0u8; 16 * 1024];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => self.read.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        let mut at = 0;
        while self.read.len() - at >= 2 {
            let size = u16::from_be_bytes([self.read[at], self.read[at + 1]]) as usize;
            if self.read.len() - at - 2 < size {
                break;
            }
            packet(&self.read[at + 2..at + 2 + size]);
            at += 2 + size;
        }
        self.read.drain(..at);
        Ok(())
    }

    /// Queues one packet, framed, and writes what the socket takes; `Err` when the client has
    /// fallen too far behind.
    pub fn send(&mut self, packet: &[u8]) -> io::Result<()> {
        let size = u16::try_from(packet.len()).map_err(|_| io::ErrorKind::InvalidInput)?;
        if self.outbox.len() + 2 + packet.len() > TCP_OUTBOX_BYTES {
            return Err(io::Error::other("TCP stream outbox full"));
        }
        self.outbox.extend(size.to_be_bytes());
        self.outbox.extend(packet);
        self.flush()
    }

    pub fn flush(&mut self) -> io::Result<()> {
        while !self.outbox.is_empty() {
            let (front, _) = self.outbox.as_slices();
            match self.stream.write(front) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.outbox.drain(..n);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn wants_write(&self) -> bool {
        !self.outbox.is_empty()
    }
}

/// The ICE username fragment a STUN binding request addresses, `local:remote` in its USERNAME,
/// so a first packet finds its connection without asking every connection.
pub fn stun_local_ufrag(packet: &[u8]) -> Option<&str> {
    // A STUN message: type (a binding request is 0x0001), length, the magic cookie.
    if packet.len() < 20 || packet[0] & 0xc0 != 0 || packet[4..8] != [0x21, 0x12, 0xa4, 0x42] {
        return None;
    }
    let length = u16::from_be_bytes([packet[2], packet[3]]) as usize;
    let body = packet.get(20..20 + length)?;
    let mut at = 0;
    while at + 4 <= body.len() {
        let kind = u16::from_be_bytes([body[at], body[at + 1]]);
        let size = u16::from_be_bytes([body[at + 2], body[at + 3]]) as usize;
        let value = body.get(at + 4..at + 4 + size)?;
        if kind == 0x0006 {
            let username = std::str::from_utf8(value).ok()?;
            return username.split(':').next();
        }
        // Attributes are padded to four bytes.
        at += 4 + size.div_ceil(4) * 4;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_binding_requests_username_names_the_connection() {
        let username = b"abcd:wxyz";
        let mut packet = vec![0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xa4, 0x42];
        packet.extend([0u8; 12]);
        // USERNAME, nine bytes, padded to twelve.
        let mut attribute = vec![0x00, 0x06, 0x00, username.len() as u8];
        attribute.extend(username);
        attribute.extend([0u8; 3]);
        let length = attribute.len() as u16;
        packet[2..4].copy_from_slice(&length.to_be_bytes());
        packet.extend(attribute);
        assert_eq!(stun_local_ufrag(&packet), Some("abcd"));
        // Not STUN: an RTP packet's first byte has its version bits set.
        assert_eq!(stun_local_ufrag(&[0x80; 40]), None);
    }
}
