//! Calls this server makes to other hosts on someone else's say-so: fetching a page a message
//! links to, or the document of a deployment someone named. A host name can resolve to
//! anything, so these refuse addresses inside a network (loopback, private, link-local, and
//! the like), which would otherwise let anyone who can name a host make this server reach
//! services only it can see.

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::{IpAddr, SocketAddr};

/// Whether `ip` is on the public internet, as far as the address alone says.
pub fn is_public_address(ip: IpAddr) -> bool {
    let ip = ip.to_canonical();
    if ip.is_loopback() || ip.is_multicast() || ip.is_unspecified() {
        return false;
    }
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_link_local()
                || v4.is_private()
                // Shared address space, which carriers use behind their NAT (RFC 6598).
                || (a == 100 && (64..128).contains(&b))
                || a == 0)
        }
        IpAddr::V6(v6) => !(v6.is_unicast_link_local() || v6.is_unique_local()),
    }
}

/// Why `PublicResolver` found no address to connect to, which callers tell apart for the
/// people they report to.
#[derive(Debug, thiserror::Error)]
pub enum ResolveError {
    /// The name does not resolve at all.
    #[error("{0} was not found")]
    NotFound(String),
    /// It resolves, but only to addresses inside a network.
    #[error("{0} has no public address")]
    NoPublicAddress(String),
}

/// A resolver for `reqwest` that answers only with public addresses (`is_public_address`),
/// unless `allow_private` is set. It filters at the moment of connecting, so a name cannot
/// resolve to a public address when checked and a private one when used.
#[derive(Debug, Clone, Copy)]
pub struct PublicResolver {
    pub allow_private: bool,
}

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let allow_private = self.allow_private;
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let all: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await
                .map_err(|_| ResolveError::NotFound(host.clone()))?
                .collect();
            if all.is_empty() {
                return Err(ResolveError::NotFound(host).into());
            }
            let found: Vec<SocketAddr> = all
                .into_iter()
                .filter(|addr| allow_private || is_public_address(addr.ip()))
                .collect();
            if found.is_empty() {
                return Err(ResolveError::NoPublicAddress(host).into());
            }
            Ok(Box::new(found.into_iter()) as Addrs)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_addresses_are_not_public() {
        for inside in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public_address(inside.parse().unwrap()), "{inside}");
        }
        for outside in ["1.1.1.1", "100.128.0.1", "2606:4700::1111"] {
            assert!(is_public_address(outside.parse().unwrap()), "{outside}");
        }
    }
}
