//! What one client may ask of this voice server, and how fast.
//!
//! Two HTTP routes are limited by client address: `health`, the latency probe every joining
//! client sends each candidate, and `signalling`, opening the socket. An address may also hold
//! only `max_pending_sockets_per_ip` sockets that have not identified yet, since each costs a
//! task and ten seconds of waiting, and no signalling message may exceed `max_message_bytes`.
//! Once a socket has identified, each frame is limited by its type, and every frame by the
//! limits of `any`, counted per caller (`user`), per address (`ip`), per call (`channel`), or
//! over the whole server (`global`). A refused frame is dropped and answered with a non-fatal
//! error saying how long to wait.
//!
//! An operator may suspend the limits for a while (`aspen_limits::suspension`): addresses in the
//! suspension's networks skip the limits that count by address, including the cap on
//! unidentified sockets, or with `scope = all` every limit is lifted. It ends by itself.
//!
//! The counters live in this process: every client of a call talks to this one server, so no
//! other server needs them. A request passes only if all of its limits allow it, and a refused
//! one spends none of them. `limits.toml` holds the built-in values, laid under the
//! `[rate_limits]` of `voice_server.toml`; a limit naming an unknown route, frame type, or
//! dimension stops the server at startup.

use crate::config::LimitSettings;
use aspen_limits::suspension::{Exemption, SuspensionState};
use aspen_limits::{ClientAddresses, LocalLimiter, Rate, RuleTable};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use uuid::Uuid;
use voice_protocol::signal::ClientMessage;

/// The limited HTTP routes, as `http` names them.
pub const HEALTH: &str = "health";
pub const SIGNALLING: &str = "signalling";
const HTTP_ROUTES: [&str; 2] = [HEALTH, SIGNALLING];
/// The `frames` entry that limits every frame.
const ANY_FRAME: &str = "any";

#[derive(Clone, Copy, Debug, PartialEq, Eq, strum::IntoStaticStr)]
#[strum(serialize_all = "lowercase")]
enum Dimension {
    Global,
    Ip,
    User,
    Channel,
}

impl Dimension {
    /// Its name in the limits' configuration.
    fn name(self) -> &'static str {
        self.into()
    }
}

#[derive(Clone, Debug)]
struct Rule {
    dimension: Dimension,
    bucket: String,
    rate: Rate,
}

/// Who a request is from, as far as it is known.
pub struct Caller {
    pub ip: Option<IpAddr>,
    pub user: Option<Uuid>,
    pub channel: Option<Uuid>,
}

pub struct Limits {
    enabled: bool,
    addresses: ClientAddresses,
    suspension: SuspensionState,
    pub max_message_bytes: usize,
    max_pending_sockets_per_ip: u32,
    http: HashMap<&'static str, Vec<Rule>>,
    frames: HashMap<&'static str, Vec<Rule>>,
    limiter: LocalLimiter<String>,
    /// Sockets per address that have not identified yet.
    pending: Mutex<HashMap<String, u32>>,
}

impl Limits {
    /// Compiles the settings; the error names what is wrong.
    pub fn new(settings: &LimitSettings) -> Result<Self, String> {
        let addresses = ClientAddresses::new(&settings.trusted_proxies, settings.ipv6_prefix)?;
        let http = compile(
            &settings.http,
            &HTTP_ROUTES,
            &[Dimension::Global, Dimension::Ip],
            "http",
        )?;
        let mut frame_kinds: Vec<&'static str> = ClientMessage::KINDS.to_vec();
        frame_kinds.push(ANY_FRAME);
        let frames = compile(
            &settings.frames,
            &frame_kinds,
            &[
                Dimension::Global,
                Dimension::Ip,
                Dimension::User,
                Dimension::Channel,
            ],
            "frames",
        )?;
        Ok(Self {
            enabled: settings.enabled,
            addresses,
            suspension: SuspensionState::new(Duration::from_secs(settings.max_suspension_seconds)),
            max_message_bytes: settings.max_message_bytes,
            max_pending_sockets_per_ip: settings.max_pending_sockets_per_ip,
            http,
            frames,
            limiter: LocalLimiter::default(),
            pending: Mutex::new(HashMap::new()),
        })
    }

    /// The suspension of these limits in force, kept current by
    /// `aspen_limits::suspension::watch`.
    pub fn suspension(&self) -> &SuspensionState {
        &self.suspension
    }

    /// The client address of a request from `peer` carrying these `X-Forwarded-For` values.
    pub fn client<'a>(&self, peer: IpAddr, forwarded: impl IntoIterator<Item = &'a str>) -> IpAddr {
        self.addresses.client(peer, forwarded)
    }

    /// Counts a request to an HTTP route.
    pub fn check_http(&self, route: &str, ip: IpAddr) -> Result<(), Duration> {
        let caller = Caller {
            ip: Some(ip),
            user: None,
            channel: None,
        };
        self.check(self.http.get(route).into_iter().flatten(), &caller)
    }

    /// Counts a signalling frame of type `kind` against its own limits and those of `any`.
    pub fn check_frame(&self, kind: &str, caller: &Caller) -> Result<(), Duration> {
        let rules = self
            .frames
            .get(kind)
            .into_iter()
            .flatten()
            .chain(self.frames.get(ANY_FRAME).into_iter().flatten());
        self.check(rules, caller)
    }

    fn check<'a>(
        &self,
        rules: impl Iterator<Item = &'a Rule>,
        caller: &Caller,
    ) -> Result<(), Duration> {
        if !self.enabled {
            return Ok(());
        }
        let exemption = self.suspension.exemption(caller.ip);
        if exemption == Exemption::All {
            return Ok(());
        }
        let buckets: Vec<(String, Rate)> = rules
            .filter(|rule| exemption != Exemption::AddressLimits || rule.dimension != Dimension::Ip)
            .filter_map(|rule| {
                let who = match rule.dimension {
                    Dimension::Global => "all".to_string(),
                    Dimension::Ip => self.addresses.key(caller.ip?),
                    Dimension::User => caller.user?.to_string(),
                    Dimension::Channel => caller.channel?.to_string(),
                };
                Some((
                    format!("{}:{}:{who}", rule.bucket, rule.dimension.name()),
                    rule.rate,
                ))
            })
            .collect();
        if buckets.is_empty() {
            return Ok(());
        }
        self.limiter.check(buckets)
    }

    /// Holds one of the address's unidentified socket places, or `None` when it has none left.
    /// The place is given back when the guard drops, at identification or disconnection.
    pub fn pending_socket(self: &Arc<Self>, ip: IpAddr) -> Option<PendingSocket> {
        if !self.enabled || self.suspension.exemption(Some(ip)) != Exemption::None {
            return Some(PendingSocket {
                limits: None,
                key: String::new(),
            });
        }
        let key = self.addresses.key(ip);
        let mut pending = self.pending.lock().expect("pending lock");
        let count = pending.entry(key.clone()).or_default();
        if *count >= self.max_pending_sockets_per_ip {
            return None;
        }
        *count += 1;
        Some(PendingSocket {
            limits: Some(Arc::clone(self)),
            key,
        })
    }
}

/// One unidentified socket's place; see `Limits::pending_socket`.
pub struct PendingSocket {
    limits: Option<Arc<Limits>>,
    key: String,
}

impl Drop for PendingSocket {
    fn drop(&mut self) {
        let Some(limits) = &self.limits else {
            return;
        };
        let mut pending = limits.pending.lock().expect("pending lock");
        if let Some(count) = pending.get_mut(&self.key) {
            *count -= 1;
            if *count == 0 {
                pending.remove(&self.key);
            }
        }
    }
}

/// Compiles one section (`http` or `frames`): every name must be one of `names`, and every
/// dimension one of `dimensions`.
fn compile(
    tables: &HashMap<String, RuleTable>,
    names: &[&'static str],
    dimensions: &[Dimension],
    section: &str,
) -> Result<HashMap<&'static str, Vec<Rule>>, String> {
    let mut compiled = HashMap::new();
    for (name, table) in tables {
        let name: &'static str = names
            .iter()
            .find(|known| **known == name.as_str())
            .copied()
            .ok_or_else(|| {
                format!(
                    "rate_limits.{section}.{name} is none of {}",
                    names.join(", ")
                )
            })?;
        let mut rules = Vec::new();
        for (dimension, setting) in table {
            let place = format!("rate_limits.{section}.{name}.{dimension}");
            let parsed = dimensions
                .iter()
                .find(|known| known.name() == dimension)
                .copied()
                .ok_or_else(|| {
                    let allowed: Vec<&str> = dimensions.iter().map(|d| d.name()).collect();
                    format!("{place}: not a dimension here ({})", allowed.join(", "))
                })?;
            if let Some(limit) = setting.limit(&place)? {
                rules.push(Rule {
                    dimension: parsed,
                    bucket: limit
                        .bucket
                        .clone()
                        .unwrap_or_else(|| format!("{section}.{name}")),
                    rate: limit.rate(),
                });
            }
        }
        compiled.insert(name, rules);
    }
    Ok(compiled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LimitOverrides;

    fn settings(overrides: &str) -> LimitSettings {
        let overrides: LimitOverrides = config::Config::builder()
            .add_source(config::File::from_str(overrides, config::FileFormat::Toml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        LimitSettings::built_in().unwrap().overlay(overrides)
    }

    fn caller(user: u128, channel: u128) -> Caller {
        Caller {
            ip: Some("192.0.2.1".parse().unwrap()),
            user: Some(Uuid::from_u128(user)),
            channel: Some(Uuid::from_u128(channel)),
        }
    }

    #[test]
    fn the_built_in_limits_compile() {
        let limits = Limits::new(&LimitSettings::built_in().unwrap()).unwrap();
        assert!(limits.frames.contains_key("createTransport"));
        assert!(limits.http.contains_key(HEALTH));
    }

    #[test]
    fn frames_count_by_type_and_by_any() {
        let limits = Limits::new(&settings(
            "[frames.produceRtp]\nuser = { requests = 2, per_seconds = 60 }\n[frames.any]\nuser = { requests = 3, per_seconds = 60 }\n",
        ))
        .unwrap();
        let alice = caller(1, 10);
        assert!(limits.check_frame("produceRtp", &alice).is_ok());
        assert!(limits.check_frame("produceRtp", &alice).is_ok());
        assert!(limits.check_frame("produceRtp", &alice).is_err());
        // The refused frame spent nothing; `any` has one left.
        assert!(limits.check_frame("setState", &alice).is_ok());
        assert!(limits.check_frame("setState", &alice).is_err());
        // Another caller has their own budgets.
        assert!(limits.check_frame("produceRtp", &caller(2, 10)).is_ok());
    }

    #[test]
    fn a_call_can_be_limited_whoever_sends() {
        let limits = Limits::new(&settings(
            "[frames.setState]\nuser = false\nchannel = { requests = 2, per_seconds = 60 }\n",
        ))
        .unwrap();
        assert!(limits.check_frame("setState", &caller(1, 10)).is_ok());
        assert!(limits.check_frame("setState", &caller(2, 10)).is_ok());
        assert!(limits.check_frame("setState", &caller(3, 10)).is_err());
        assert!(limits.check_frame("setState", &caller(3, 11)).is_ok());
    }

    #[test]
    fn http_routes_count_by_address() {
        let limits = Limits::new(&settings(
            "[http.health]\nip = { requests = 1, per_seconds = 60 }\n",
        ))
        .unwrap();
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        assert!(limits.check_http(HEALTH, ip).is_ok());
        let wait = limits.check_http(HEALTH, ip).unwrap_err();
        assert!(wait > Duration::from_secs(50));
        assert!(
            limits
                .check_http(HEALTH, "192.0.2.2".parse().unwrap())
                .is_ok()
        );
        assert!(limits.check_http(SIGNALLING, ip).is_ok());
    }

    #[test]
    fn pending_sockets_are_capped_per_address_and_given_back() {
        let limits = Arc::new(Limits::new(&settings("max_pending_sockets_per_ip = 2\n")).unwrap());
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        let first = limits.pending_socket(ip).unwrap();
        let _second = limits.pending_socket(ip).unwrap();
        assert!(limits.pending_socket(ip).is_none());
        assert!(
            limits
                .pending_socket("192.0.2.2".parse().unwrap())
                .is_some()
        );
        drop(first);
        assert!(limits.pending_socket(ip).is_some());
    }

    #[test]
    fn mistakes_stop_the_server() {
        let err = |overrides: &str| Limits::new(&settings(overrides)).err().unwrap();
        assert!(
            err("[frames.createTransports]\nuser = { requests = 1, per_seconds = 1 }\n")
                .contains("none of")
        );
        assert!(
            err("[http.health]\nuser = { requests = 1, per_seconds = 1 }\n")
                .contains("not a dimension")
        );
        assert!(
            err("[frames.produce]\nuser = { requests = 0, per_seconds = 1 }\n")
                .contains("positive")
        );
        assert!(err("trusted_proxies = [\"somewhere\"]\n").contains("trusted_proxies"));
    }

    #[test]
    fn disabled_limits_allow_everything() {
        let limits = Arc::new(
            Limits::new(&settings(
                "enabled = false\nmax_pending_sockets_per_ip = 0\n",
            ))
            .unwrap(),
        );
        let ip: IpAddr = "192.0.2.1".parse().unwrap();
        assert!(limits.pending_socket(ip).is_some());
        for _ in 0..1000 {
            assert!(limits.check_http(HEALTH, ip).is_ok());
        }
    }
}
