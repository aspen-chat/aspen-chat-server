//! Listening sockets of one address family each. An IPv6 socket made the usual way also takes
//! IPv4 on its port (clients then appear at IPv4-mapped addresses, and whether it does follows
//! the machine's `bindv6only`), so `[::]` and `0.0.0.0` could not hold the same port. These
//! take IPv6 alone (`IPV6_V6ONLY`), which leaves the port's IPv4 side to a socket of its own
//! and gives every client the address it has.

use socket2::{Domain, Protocol, SockRef, Socket, Type};
use std::io;
use std::net::SocketAddr;
use tokio::net::{TcpListener, TcpSocket, UdpSocket};

/// Connections waiting to be accepted, past which the kernel refuses more (it clamps this to
/// `net.core.somaxconn`).
const TCP_BACKLOG: u32 = 1024;

/// Whether binding failed because this machine has no address of the family: IPv6 turned off
/// or not built into the kernel, say. Any other failure (the port taken, no permission) is a
/// fault whatever asked for the address.
pub fn family_missing(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::AddrNotAvailable
        || error.raw_os_error() == Some(libc::EAFNOSUPPORT)
}

pub fn tcp_listener(addr: SocketAddr) -> io::Result<TcpListener> {
    let socket = if addr.is_ipv6() {
        let socket = TcpSocket::new_v6()?;
        SockRef::from(&socket).set_only_v6(true)?;
        socket
    } else {
        TcpSocket::new_v4()?
    };
    // A restarted server binds its port while connections of the one before linger.
    socket.set_reuseaddr(true)?;
    socket.bind(addr)?;
    socket.listen(TCP_BACKLOG)
}

pub fn udp_socket(addr: SocketAddr) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    if addr.is_ipv6() {
        socket.set_only_v6(true)?;
    }
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    UdpSocket::from_std(socket.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    /// Both families hold one port at once, which is what listening on `0.0.0.0` and `::`
    /// together needs. Skipped on a machine without IPv6.
    #[tokio::test]
    async fn both_families_share_a_port() {
        let Ok(v6) = tcp_listener((Ipv6Addr::UNSPECIFIED, 0).into()) else {
            return;
        };
        let port = v6.local_addr().expect("address").port();
        tcp_listener((Ipv4Addr::UNSPECIFIED, port).into()).expect("the IPv4 side of the port");

        let v6 = udp_socket((Ipv6Addr::UNSPECIFIED, 0).into()).expect("IPv6 UDP");
        let port = v6.local_addr().expect("address").port();
        udp_socket((Ipv4Addr::UNSPECIFIED, port).into()).expect("the IPv4 side of the port");
    }
}
