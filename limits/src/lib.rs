//! Rate limiting pieces the API server and the voice servers share: how a limit is written in
//! their configuration files, the GCRA arithmetic it becomes, where a client's address comes
//! from behind trusted reverse proxies, and an in-process limiter for a server whose clients'
//! state lives in that one process.
//!
//! Every limit is GCRA (the generic cell rate algorithm), a token bucket kept as one timestamp:
//! the theoretical arrival time `tat`, the moment the bucket would be empty again. A request at
//! `now` is allowed when `tat - tolerance <= now`, and moves `tat` to `max(tat, now) + emission`.
//! `emission` is the spacing of requests at the sustained rate and `tolerance` the credit that
//! lets `burst` of them come back to back.

use ipnet::IpNet;
use serde::Deserialize;
use std::collections::HashMap;
use std::hash::Hash;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// How a limit is written: `{ requests, per_seconds, burst?, bucket? }`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Limit {
    /// How many requests `per_seconds` allows, spread evenly.
    pub requests: u32,
    pub per_seconds: f64,
    /// How many may come back to back; `requests` when omitted.
    pub burst: Option<u32>,
    /// Requests counted in the same bucket share one budget; without one each place a limit is
    /// written counts on its own.
    pub bucket: Option<String>,
}

/// A limit, or `false` to remove one set elsewhere.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum LimitSetting {
    Off(bool),
    Limit(Limit),
}

/// Limits keyed by dimension (`ip`, `user`, `per_channel`, ...).
pub type RuleTable = HashMap<String, LimitSetting>;

/// Lays `over` onto `base` table by table: each limit in `over` replaces the one for the same
/// name and dimension whole, and everything else keeps its value in `base`.
pub fn overlay_tables(base: &mut HashMap<String, RuleTable>, over: HashMap<String, RuleTable>) {
    for (name, limits) in over {
        base.entry(name).or_default().extend(limits);
    }
}

/// A limit as GCRA uses it, in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rate {
    pub emission_ms: u64,
    pub tolerance_ms: u64,
}

impl LimitSetting {
    /// The limit, `None` for `false`, or why the setting is not a limit. `place` names it in
    /// the message, `rate_limits.endpoints."POST /users".ip` for instance.
    pub fn limit(&self, place: &str) -> Result<Option<&Limit>, String> {
        match self {
            LimitSetting::Off(false) => Ok(None),
            LimitSetting::Off(true) => Err(format!(
                "{place}: `true` is not a limit; give {{ requests, per_seconds }} or `false` to remove it"
            )),
            LimitSetting::Limit(limit) => {
                if limit.requests == 0
                    || !(limit.per_seconds > 0.0 && limit.per_seconds.is_finite())
                {
                    return Err(format!(
                        "{place}: requests and per_seconds must be positive"
                    ));
                }
                if limit.burst == Some(0) {
                    return Err(format!("{place}: burst must be positive"));
                }
                Ok(Some(limit))
            }
        }
    }
}

impl Limit {
    pub fn rate(&self) -> Rate {
        let emission_ms = ((self.per_seconds * 1000.0) / f64::from(self.requests))
            .ceil()
            .max(1.0) as u64;
        let burst = u64::from(self.burst.unwrap_or(self.requests));
        Rate {
            emission_ms,
            tolerance_ms: emission_ms * (burst - 1),
        }
    }
}

impl Rate {
    /// One GCRA step: the new `tat` when a request at `now_ms` is allowed, or how long it must
    /// wait. `tat_ms` is `None` for a bucket nobody has used.
    pub fn step(self, tat_ms: Option<u64>, now_ms: u64) -> Result<u64, Duration> {
        let tat = tat_ms.unwrap_or(now_ms).max(now_ms);
        let allow_at = tat.saturating_sub(self.tolerance_ms);
        if now_ms < allow_at {
            Err(Duration::from_millis(allow_at - now_ms))
        } else {
            Ok(tat + self.emission_ms)
        }
    }
}

/// Where a request comes from, given the reverse proxies that are trusted to say.
#[derive(Clone, Debug)]
pub struct ClientAddresses {
    trusted: Vec<IpNet>,
    ipv6_prefix: u8,
}

impl ClientAddresses {
    /// `trusted_proxies` are addresses or CIDR networks; an IPv6 client is counted by its
    /// network of `ipv6_prefix` leading bits, since one subscriber usually holds a whole /64.
    pub fn new(trusted_proxies: &[String], ipv6_prefix: u8) -> Result<Self, String> {
        if ipv6_prefix > 128 {
            return Err("rate_limits.ipv6_prefix must be at most 128".into());
        }
        let trusted = trusted_proxies
            .iter()
            .map(|proxy| {
                proxy
                    .parse::<IpNet>()
                    .or_else(|_| proxy.parse::<IpAddr>().map(IpNet::from))
                    .map_err(|_| {
                        format!(
                            "rate_limits.trusted_proxies: {proxy:?} is not an address or network"
                        )
                    })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            trusted,
            ipv6_prefix,
        })
    }

    pub fn is_trusted(&self, ip: IpAddr) -> bool {
        let ip = canonical(ip);
        self.trusted.iter().any(|net| net.contains(&ip))
    }

    /// The client's address: the peer's, unless the peer is a trusted proxy, in which case the
    /// right-most `X-Forwarded-For` address that is not itself a trusted proxy. Addresses
    /// further left are the client's own claims and are ignored. `forwarded` is every
    /// `X-Forwarded-For` header value, in order.
    pub fn client<'a>(&self, peer: IpAddr, forwarded: impl IntoIterator<Item = &'a str>) -> IpAddr {
        if !self.is_trusted(peer) {
            return canonical(peer);
        }
        let chain: Vec<IpAddr> = forwarded
            .into_iter()
            .flat_map(|value| value.split(','))
            .filter_map(|entry| entry.trim().parse().ok())
            .collect();
        chain
            .iter()
            .rev()
            .find(|ip| !self.is_trusted(**ip))
            .or(chain.first())
            .map_or(canonical(peer), |ip| canonical(*ip))
    }

    /// An address as limits count it: IPv4 as is, IPv6 by its `/ipv6_prefix` network.
    pub fn key(&self, ip: IpAddr) -> String {
        match canonical(ip) {
            IpAddr::V4(v4) => v4.to_string(),
            IpAddr::V6(v6) => IpNet::new(IpAddr::V6(v6), self.ipv6_prefix)
                .map(|net| net.trunc().to_string())
                .unwrap_or_else(|_| v6.to_string()),
        }
    }
}

/// An IPv4 address carried in IPv6 (`::ffff:a.b.c.d`) is the IPv4 address.
pub fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(IpAddr::V6(v6), IpAddr::V4),
        v4 => v4,
    }
}

/// GCRA buckets in this process's memory, for a server that alone sees its clients. Buckets
/// that have drained are forgotten as the map grows, so memory follows the number of clients
/// active within their limits' windows.
pub struct LocalLimiter<K> {
    epoch: Instant,
    buckets: Mutex<Buckets<K>>,
}

struct Buckets<K> {
    tats: HashMap<K, u64>,
    /// When the map reaches this size, drained buckets are swept and the mark set to twice
    /// what is left.
    sweep_at: usize,
}

const FIRST_SWEEP: usize = 1024;

impl<K: Hash + Eq> Default for LocalLimiter<K> {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
            buckets: Mutex::new(Buckets {
                tats: HashMap::new(),
                sweep_at: FIRST_SWEEP,
            }),
        }
    }
}

impl<K: Hash + Eq> LocalLimiter<K> {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    /// Counts a request against every `(bucket, rate)` given, or, if any refuses, against none
    /// and says how long to wait.
    pub fn check(&self, requests: Vec<(K, Rate)>) -> Result<(), Duration> {
        let now = self.now_ms();
        let mut buckets = self.buckets.lock().expect("limiter lock");
        let mut next = Vec::with_capacity(requests.len());
        let mut wait = Duration::ZERO;
        for (key, rate) in requests {
            match rate.step(buckets.tats.get(&key).copied(), now) {
                Ok(tat) => next.push((key, tat)),
                Err(needed) => wait = wait.max(needed),
            }
        }
        if !wait.is_zero() {
            return Err(wait);
        }
        for (key, tat) in next {
            buckets.tats.insert(key, tat);
        }
        if buckets.tats.len() >= buckets.sweep_at {
            buckets.tats.retain(|_, tat| *tat > now);
            buckets.sweep_at = (buckets.tats.len() * 2).max(FIRST_SWEEP);
        }
        Ok(())
    }

    /// How many buckets are held.
    pub fn len(&self) -> usize {
        self.buckets.lock().expect("limiter lock").tats.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limit(requests: u32, per_seconds: f64, burst: Option<u32>) -> Limit {
        Limit {
            requests,
            per_seconds,
            burst,
            bucket: None,
        }
    }

    #[test]
    fn a_limit_allows_its_burst_then_its_rate() {
        let rate = limit(3, 60.0, None).rate();
        assert_eq!(
            rate,
            Rate {
                emission_ms: 20_000,
                tolerance_ms: 40_000
            }
        );
        let mut tat = None;
        for _ in 0..3 {
            tat = Some(rate.step(tat, 0).unwrap());
        }
        assert_eq!(rate.step(tat, 0), Err(Duration::from_secs(20)));
        assert!(rate.step(tat, 20_000).is_ok());
    }

    #[test]
    fn settings_are_checked() {
        assert_eq!(LimitSetting::Off(false).limit("x"), Ok(None));
        assert!(
            LimitSetting::Off(true)
                .limit("x")
                .unwrap_err()
                .contains("not a limit")
        );
        assert!(LimitSetting::Limit(limit(0, 1.0, None)).limit("x").is_err());
        assert!(
            LimitSetting::Limit(limit(1, f64::NAN, None))
                .limit("x")
                .is_err()
        );
        assert!(
            LimitSetting::Limit(limit(1, 1.0, Some(0)))
                .limit("x")
                .is_err()
        );
    }

    #[test]
    fn overlays_replace_whole_limits() {
        let mut base = HashMap::from([(
            "health".to_string(),
            RuleTable::from([
                (
                    "ip".to_string(),
                    LimitSetting::Limit(limit(10, 1.0, Some(5))),
                ),
                (
                    "global".to_string(),
                    LimitSetting::Limit(limit(100, 1.0, None)),
                ),
            ]),
        )]);
        overlay_tables(
            &mut base,
            HashMap::from([(
                "health".to_string(),
                RuleTable::from([("ip".to_string(), LimitSetting::Limit(limit(3, 1.0, None)))]),
            )]),
        );
        assert_eq!(
            base["health"]["ip"],
            LimitSetting::Limit(limit(3, 1.0, None))
        );
        assert!(base["health"].contains_key("global"));
    }

    #[test]
    fn the_client_is_the_right_most_address_no_trusted_proxy_added() {
        let addresses = ClientAddresses::new(&["10.0.0.0/8".into(), "::1".into()], 64).unwrap();
        let ip = |text: &str| text.parse::<IpAddr>().unwrap();
        let proxy = ip("10.0.0.2");
        assert_eq!(
            addresses.client(ip("192.0.2.1"), ["198.51.100.9"]),
            ip("192.0.2.1")
        );
        assert_eq!(
            addresses.client(proxy, ["6.6.6.6, 198.51.100.9"]),
            ip("198.51.100.9")
        );
        assert_eq!(
            addresses.client(proxy, ["198.51.100.9", "10.0.0.3"]),
            ip("198.51.100.9")
        );
        assert_eq!(addresses.client(proxy, []), proxy);
        assert_eq!(
            addresses.client(ip("::ffff:192.0.2.7"), []),
            ip("192.0.2.7")
        );
        assert!(addresses.is_trusted(ip("::ffff:10.0.0.1")));
        assert!(ClientAddresses::new(&["proxy.local".into()], 64).is_err());
        assert!(ClientAddresses::new(&[], 129).is_err());
    }

    #[test]
    fn addresses_count_ipv6_by_network() {
        let addresses = ClientAddresses::new(&[], 64).unwrap();
        assert_eq!(
            addresses.key("2001:db8:1:2:3:4:5:6".parse().unwrap()),
            "2001:db8:1:2::/64"
        );
        assert_eq!(addresses.key("192.0.2.7".parse().unwrap()), "192.0.2.7");
    }

    #[test]
    fn the_local_limiter_counts_all_or_nothing() {
        let limiter = LocalLimiter::<&str>::default();
        let tight = limit(1, 60.0, None).rate();
        let loose = limit(100, 60.0, None).rate();
        assert!(limiter.check(vec![("a", tight), ("b", loose)]).is_ok());
        assert!(limiter.check(vec![("a", tight), ("b", loose)]).is_err());
        // The refusal spent nothing from the loose bucket: 99 more fit.
        for _ in 0..99 {
            assert!(limiter.check(vec![("b", loose)]).is_ok());
        }
        assert!(limiter.check(vec![("b", loose)]).is_err());
    }
}
