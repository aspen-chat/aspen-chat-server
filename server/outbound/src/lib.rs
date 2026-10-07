//! Calls this server makes to other hosts on someone else's say-so: fetching a page a message
//! links to and its picture, or the document of a deployment someone named. A host name can
//! resolve to anything, and a URL or a redirect can name an address outright, so these refuse
//! addresses inside a network (loopback, private, link-local, and the like), which would
//! otherwise let anyone who can name a host make this server reach services only it can see:
//! names as they are resolved ([`PublicResolver`]), and addresses in URLs and redirects
//! ([`may_fetch`], [`checked_redirects`]).

use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use url::{Host, Url};

/// The IPv4 networks that are not on the public internet (IANA's special-purpose registry).
const INSIDE_V4: [(Ipv4Addr, u32); 15] = [
    // "This network", which reaches this host.
    (Ipv4Addr::new(0, 0, 0, 0), 8),
    (Ipv4Addr::new(10, 0, 0, 0), 8),
    // Shared address space, which carriers use behind their NAT (RFC 6598).
    (Ipv4Addr::new(100, 64, 0, 0), 10),
    (Ipv4Addr::new(127, 0, 0, 0), 8),
    (Ipv4Addr::new(169, 254, 0, 0), 16),
    (Ipv4Addr::new(172, 16, 0, 0), 12),
    // IETF protocol assignments.
    (Ipv4Addr::new(192, 0, 0, 0), 24),
    (Ipv4Addr::new(192, 0, 2, 0), 24),
    // The 6to4 relays' anycast.
    (Ipv4Addr::new(192, 88, 99, 0), 24),
    (Ipv4Addr::new(192, 168, 0, 0), 16),
    // Benchmarking.
    (Ipv4Addr::new(198, 18, 0, 0), 15),
    (Ipv4Addr::new(198, 51, 100, 0), 24),
    (Ipv4Addr::new(203, 0, 113, 0), 24),
    // Multicast.
    (Ipv4Addr::new(224, 0, 0, 0), 4),
    // Reserved, and the broadcast address.
    (Ipv4Addr::new(240, 0, 0, 0), 4),
];

/// The IPv6 global unicast space, outside which nothing is on the public internet.
const GLOBAL_V6: (Ipv6Addr, u32) = (Ipv6Addr::new(0x2000, 0, 0, 0, 0, 0, 0, 0), 3);

/// The networks within the global unicast space that are not on the public internet. The forms
/// that carry an IPv4 address (6to4 here; mapped, compatible, and NAT64 outside it) are judged
/// by that address instead.
const INSIDE_V6: [(Ipv6Addr, u32); 3] = [
    // IETF protocol assignments: Teredo (2001::/32), benchmarking, ORCHID, and the like.
    (Ipv6Addr::new(0x2001, 0, 0, 0, 0, 0, 0, 0), 23),
    (Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 0), 32),
    (Ipv6Addr::new(0x3fff, 0, 0, 0, 0, 0, 0, 0), 20),
];

fn within_v4(ip: Ipv4Addr, (network, length): (Ipv4Addr, u32)) -> bool {
    let mask = u32::MAX.checked_shl(32 - length).unwrap_or(0);
    ip.to_bits() & mask == network.to_bits() & mask
}

fn within_v6(ip: Ipv6Addr, (network, length): (Ipv6Addr, u32)) -> bool {
    let mask = u128::MAX.checked_shl(128 - length).unwrap_or(0);
    ip.to_bits() & mask == network.to_bits() & mask
}

/// The IPv4 address an IPv6 address carries, in the forms that reach it: IPv4-mapped
/// (`::ffff:0:0/96`), IPv4-compatible (`::/96`, which holds `::` and `::1` too), NAT64
/// (`64:ff9b::/96`), and 6to4 (`2002::/16`).
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    let [.., a, b, c, d] = ip.octets();
    match ip.segments() {
        [0, 0, 0, 0, 0, 0xffff | 0, _, _] | [0x64, 0xff9b, 0, 0, 0, 0, _, _] => {
            Some(Ipv4Addr::new(a, b, c, d))
        }
        [0x2002, high, low, ..] => Some(Ipv4Addr::from_bits(
            (u32::from(high) << 16) | u32::from(low),
        )),
        _ => None,
    }
}

/// Whether `ip` is on the public internet, as far as the address alone says. An IPv6 address
/// that carries an IPv4 one is judged by that.
pub fn is_public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => !INSIDE_V4.iter().any(|&network| within_v4(v4, network)),
        IpAddr::V6(v6) => match embedded_v4(v6) {
            Some(v4) => is_public_address(IpAddr::V4(v4)),
            None => {
                within_v6(v6, GLOBAL_V6) && !INSIDE_V6.iter().any(|&network| within_v6(v6, network))
            }
        },
    }
}

/// Whether `url` names its host by an address that is not public. A client connects to an
/// address it is given without asking its resolver, so `PublicResolver` never sees one; what
/// takes a URL from someone else checks it here.
pub fn names_inside_address(url: &Url) -> bool {
    match url.host() {
        Some(Host::Ipv4(v4)) => !is_public_address(IpAddr::V4(v4)),
        Some(Host::Ipv6(v6)) => !is_public_address(IpAddr::V6(v6)),
        Some(Host::Domain(_)) | None => false,
    }
}

/// Whether this server may fetch `url` on someone else's say-so: `http` or `https`, at a host
/// that is a name, which [`PublicResolver`] checks as it connects, or a public address. A URL
/// naming an address is connected to without resolving anything, so the address is checked
/// here.
pub fn may_fetch(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https") && url.has_host() && !names_inside_address(url)
}

/// Follows at most `hops` redirects, each only to a URL `allowed` allows (at least as strict as
/// [`may_fetch`]), so a public page cannot send the request on to an address inside a network.
pub fn checked_redirects(hops: usize, allowed: fn(&Url) -> bool) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= hops {
            attempt.error(format!("more than {hops} redirects"))
        } else if allowed(attempt.url()) {
            attempt.follow()
        } else {
            let refused = format!("a redirect to {}, which may not be fetched", attempt.url());
            attempt.error(refused)
        }
    })
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
/// and loopback ones too when `allow_loopback` is set, and any address when `allow_private` is.
/// It filters at the moment of connecting, so a name cannot resolve to a public address when
/// checked and a private one when used.
#[derive(Debug, Clone, Copy)]
pub struct PublicResolver {
    pub allow_private: bool,
    pub allow_loopback: bool,
}

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let PublicResolver {
            allow_private,
            allow_loopback,
        } = *self;
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
                .filter(|addr| {
                    allow_private
                        || is_public_address(addr.ip())
                        || (allow_loopback && addr.ip().is_loopback())
                })
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
    fn urls_naming_inside_addresses_are_told_apart() {
        for (url, inside) in [
            ("https://127.0.0.1/", true),
            ("https://[::1]:8443/push", true),
            ("https://[::ffff:169.254.169.254]/", true),
            ("https://10.0.0.1:443/", true),
            // Forms a URL parser reads as an IPv4 address.
            ("https://2130706433/", true),
            ("https://0x7f.1/", true),
            ("https://127.0x1/", true),
            ("https://1.1.1.1/", false),
            ("https://[2606:4700::1111]/", false),
            ("https://push.example.org/v1/push/abc", false),
            ("https://localhost/", false),
        ] {
            assert_eq!(names_inside_address(&url.parse().unwrap()), inside, "{url}");
        }
    }

    #[test]
    fn only_public_addresses_are_public() {
        for (address, public) in [
            // IPv4
            ("1.1.1.1", true),
            ("8.8.8.8", true),
            ("100.63.255.255", true),
            ("100.128.0.1", true),
            ("172.32.0.1", true),
            ("192.0.1.1", true),
            ("198.17.255.255", true),
            ("198.20.0.1", true),
            ("223.255.255.255", true),
            ("0.0.0.0", false),
            ("0.1.2.3", false),
            ("10.1.2.3", false),
            ("100.64.0.1", false),
            ("100.127.255.255", false),
            ("127.0.0.1", false),
            ("127.255.255.254", false),
            ("169.254.169.254", false),
            ("172.16.0.1", false),
            ("172.31.255.255", false),
            ("192.0.0.1", false),
            ("192.0.0.170", false),
            ("192.0.2.1", false),
            ("192.88.99.1", false),
            ("192.168.1.1", false),
            ("198.18.0.1", false),
            ("198.19.255.255", false),
            ("198.51.100.1", false),
            ("203.0.113.1", false),
            ("224.0.0.1", false),
            ("239.255.255.250", false),
            ("240.0.0.1", false),
            ("255.255.255.255", false),
            // IPv6
            ("2606:4700::1111", true),
            ("2a00:1450:4001::200e", true),
            ("2001:4860:4860::8888", true),
            ("::", false),
            ("::1", false),
            ("fd00::1", false),
            ("fc00::1", false),
            ("fe80::1", false),
            ("fec0::1", false),
            ("ff02::1", false),
            ("ff0e::1", false),
            ("100::1", false),
            ("5f00::1", false),
            ("2001::1", false),
            ("2001:0:4136:e378:8000:63bf:3fff:fdd2", false),
            ("2001:2::1", false),
            ("2001:db8::1", false),
            ("3fff::1", false),
            ("64:ff9b:1::1", false),
            // IPv6 carrying IPv4, judged by the IPv4 address
            ("::ffff:127.0.0.1", false),
            ("::ffff:10.0.0.1", false),
            ("::ffff:1.1.1.1", true),
            ("::127.0.0.1", false),
            ("::169.254.169.254", false),
            ("::8.8.8.8", true),
            ("64:ff9b::127.0.0.1", false),
            ("64:ff9b::a9fe:a9fe", false),
            ("64:ff9b::1.1.1.1", true),
            ("2002:7f00:1::1", false),
            ("2002:a9fe:a9fe::1", false),
            ("2002:c0a8:101::1", false),
            ("2002:0101:0101::1", true),
        ] {
            assert_eq!(
                is_public_address(address.parse().unwrap()),
                public,
                "{address}"
            );
        }
    }

    #[test]
    fn only_web_urls_at_names_or_public_addresses_may_be_fetched() {
        for refused in [
            "http://127.0.0.1/",
            "https://10.0.0.1/a.png",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]:8080/",
            "http://[::ffff:192.168.0.1]/",
            // Hosts written as one number are addresses too.
            "http://2130706433/",
            "http://0x7f000001/",
            "ftp://example.com/",
            "file:///etc/passwd",
            "data:text/html,hi",
        ] {
            assert!(!may_fetch(&Url::parse(refused).unwrap()), "{refused}");
        }
        for allowed in [
            "https://example.com/",
            "http://localhost.example/",
            "https://1.1.1.1/",
            "https://[2606:4700::1111]/",
        ] {
            assert!(may_fetch(&Url::parse(allowed).unwrap()), "{allowed}");
        }
    }
}
