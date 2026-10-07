//! Rate limits.
//!
//! Every endpoint has a set of rules, each a limit counted along one dimension: everyone's
//! requests together (`global`), one client address's (`ip`), one user's (`user`), one
//! username's at sign-in (`username`), or any of the last three per value of a path parameter
//! (`per_channel`, `user_per_channel`, `ip_per_channel`, ...). A request passes only if every
//! rule of its endpoint allows it. An endpoint's rules are the configured defaults, which apply
//! to every endpoint, plus those of the groups that list it, with the endpoint's own settings
//! replacing its groups' dimension by dimension; setting a dimension to `false` on a group or
//! endpoint removes it there, defaults included. `aspen_config::RateLimitConfig` describes the
//! configuration and `rate_limits.toml` holds the built-in values. The rules are
//! compiled and checked against the real routes at startup, so a typo in an endpoint name, a
//! parameter the path lacks, or a per-user limit on an endpoint nobody signs in to is a
//! configuration error rather than a limit that silently never applies.
//!
//! Each rule counts in a bucket, a Valkey key shared by every API server, with GCRA (the
//! generic cell rate algorithm): the key holds the time at which the bucket will be empty
//! again, and a request is allowed if that is no further ahead than the burst allows. One
//! script per key does the check and the update atomically on Valkey's clock, and all of a
//! request's keys go in one round trip; keys are evaluated independently, so they may
//! live on different shards. A key that refuses is left unchanged; the others still count the
//! request, so hammering past one limit also spends the wider ones. When Valkey cannot be
//! reached the limiter lets requests through and logs, since most of the API does not otherwise
//! need Valkey, except on the endpoints listed in `fail_closed` (those that guess a secret: a
//! password, a code, an invite) and for the username limit of sign-in, which it refuses
//! (`Decision::Unavailable`) so an outage does not open them to unlimited guessing.
//!
//! An operator may suspend the limits for a while (`aspen_limits::suspension`): for requests
//! from given networks, the limits that count by address are skipped, or with `scope = all`
//! every limit is. The suspension ends by itself.
//!
//! Rules are checked at three points of a request, depending on what they need to know:
//! `Stage::Request` (global, address, and path parameter rules) before the handler runs,
//! `Stage::Session` (user rules) once the session token has been resolved, and
//! `Stage::Username` inside the sign-in handler, which alone knows the username.

use crate::UserId;
use crate::aspen_config::{Limit, LimitSetting, RateLimitConfig};
use aspen_limits::ClientAddresses;
use aspen_limits::suspension::{Exemption, SuspensionState};
use fred::clients::Client;
use fred::interfaces::LuaInterface;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The one endpoint whose handler supplies a username.
pub const SIGN_IN_ROUTE: &str = "POST /auth/login";

/// Checks and updates one GCRA bucket. `KEYS[1]` is the bucket; `ARGV[1]` the emission interval
/// (milliseconds per request) and `ARGV[2]` the burst tolerance (milliseconds of credit beyond
/// one request). Returns 0 when the request is allowed, otherwise the milliseconds until it
/// would be.
const GCRA: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local emission = tonumber(ARGV[1])
local tolerance = tonumber(ARGV[2])
local tat = tonumber(redis.call('GET', KEYS[1]) or now)
if tat < now then
  tat = now
end
local allow_at = tat - tolerance
if now < allow_at then
  return allow_at - now
end
local next_tat = tat + emission
redis.call('SET', KEYS[1], next_tat, 'PX', next_tat - now)
return 0
";

/// Takes one request from the GCRA bucket `key` at `rate`, for limits kept outside the API's
/// routes (`app::email::outbox`'s sending rate): `None` when it is allowed, or how long until it
/// would be.
pub async fn take(
    valkey: &Client,
    key: &str,
    rate: aspen_limits::Rate,
) -> Result<Option<Duration>, fred::error::Error> {
    let wait: i64 = valkey
        .eval(
            GCRA,
            key.to_string(),
            vec![rate.emission_ms, rate.tolerance_ms],
        )
        .await?;
    Ok((wait > 0).then(|| Duration::from_millis(wait.unsigned_abs())))
}

/// Who may call a route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// No session: user rules can never apply.
    Anonymous,
    /// A session is optional; user rules apply when one is presented.
    Optional,
    Authenticated,
}

/// A route as the rules see it.
#[derive(Clone, Debug)]
pub struct Route {
    /// `"POST /channels/{channel}/messages"`: the method and the path template under
    /// `/api/v1`.
    pub key: String,
    pub access: Access,
    /// The path template's parameters, `channel` for the example above.
    pub params: Vec<String>,
}

impl Route {
    pub fn new(method: &str, template: &str, access: Access) -> Self {
        let params = template
            .split('/')
            .filter_map(|segment| segment.strip_prefix('{')?.strip_suffix('}'))
            .map(str::to_string)
            .collect();
        Self {
            key: route_key(method, template),
            access,
            params,
        }
    }
}

/// The methods a route key names as they are. Any other method token a client sends is named
/// `OTHER` (`OTHER_METHOD`), so made-up methods cannot make new keys, and with them new metric
/// series and buckets.
const METHODS: [&str; 9] = [
    "GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "CONNECT", "TRACE",
];

/// What a route key calls a method that is none of `METHODS`.
pub const OTHER_METHOD: &str = "OTHER";

/// The key a request's limits and metrics go by: its method and its path template.
pub fn route_key(method: &str, template: &str) -> String {
    let method = METHODS
        .iter()
        .find(|known| known.eq_ignore_ascii_case(method))
        .copied()
        .unwrap_or(OTHER_METHOD);
    format!("{method} {template}")
}

/// Whose requests a rule counts together.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Dimension {
    Global,
    Ip,
    User,
    Username,
    Per(String),
    UserPer(String),
    IpPer(String),
}

impl Dimension {
    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "global" => Dimension::Global,
            "ip" => Dimension::Ip,
            "user" => Dimension::User,
            "username" => Dimension::Username,
            _ => {
                if let Some(param) = name.strip_prefix("user_per_") {
                    Dimension::UserPer(param.to_string())
                } else if let Some(param) = name.strip_prefix("ip_per_") {
                    Dimension::IpPer(param.to_string())
                } else {
                    let param = name.strip_prefix("per_")?;
                    Dimension::Per(param.to_string())
                }
            }
        })
    }

    fn name(&self) -> String {
        match self {
            Dimension::Global => "global".into(),
            Dimension::Ip => "ip".into(),
            Dimension::User => "user".into(),
            Dimension::Username => "username".into(),
            Dimension::Per(param) => format!("per_{param}"),
            Dimension::UserPer(param) => format!("user_per_{param}"),
            Dimension::IpPer(param) => format!("ip_per_{param}"),
        }
    }

    fn stage(&self) -> Stage {
        match self {
            Dimension::Global | Dimension::Ip | Dimension::Per(_) | Dimension::IpPer(_) => {
                Stage::Request
            }
            Dimension::User | Dimension::UserPer(_) => Stage::Session,
            Dimension::Username => Stage::Username,
        }
    }

    fn param(&self) -> Option<&str> {
        match self {
            Dimension::Per(param) | Dimension::UserPer(param) | Dimension::IpPer(param) => {
                Some(param)
            }
            _ => None,
        }
    }

    fn needs_user(&self) -> bool {
        matches!(self, Dimension::User | Dimension::UserPer(_))
    }
}

/// When during a request a rule can be checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Request,
    Session,
    Username,
}

#[derive(Clone, Debug)]
struct Rule {
    dimension: Dimension,
    bucket: String,
    emission_ms: u64,
    tolerance_ms: u64,
}

/// What a request is known by when its rules are checked.
#[derive(Default)]
pub struct Identity<'a> {
    pub ip: Option<IpAddr>,
    pub user: Option<UserId>,
    pub username: Option<&'a str>,
    pub params: Vec<(&'a str, &'a str)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    Limited {
        retry_after: Duration,
    },
    /// The limits could not be counted, and this endpoint is refused rather than let through
    /// while they cannot.
    Unavailable,
}

#[derive(Debug)]
pub struct RateLimiter {
    enabled: bool,
    addresses: ClientAddresses,
    suspension: SuspensionState,
    rules: HashMap<String, Vec<Rule>>,
    /// The rules of a request whose route key has none of its own (a method the route does not
    /// have, refused by the router): the default ones that need neither a session nor a path
    /// parameter, so such requests are counted too.
    unknown: Vec<Rule>,
    /// The routes refused while the limits cannot be counted.
    fail_closed: HashSet<String>,
    /// Unix milliseconds of the last "Valkey unreachable" log line, so an outage logs twice a
    /// minute rather than on every request.
    last_failure_log: AtomicU64,
}

/// How often an unreachable Valkey is logged.
const FAILURE_LOG_INTERVAL_MS: u64 = 30_000;
/// Longer key parts are hashed, keeping keys short whatever a path parameter holds.
const MAX_KEY_PART: usize = 64;

impl RateLimiter {
    /// The names of the dimensions `route` is counted along, sorted.
    pub fn dimensions(&self, route: &str) -> Vec<String> {
        let mut names: Vec<String> = self
            .rules
            .get(route)
            .unwrap_or(&self.unknown)
            .iter()
            .map(|rule| rule.dimension.name())
            .collect();
        names.sort();
        names
    }

    /// Whether `route` is refused while the limits cannot be counted.
    pub fn fails_closed(&self, route: &str) -> bool {
        self.fail_closed.contains(route)
    }

    /// Compiles the configuration against the server's routes. The error names what is wrong.
    pub fn compile(config: &RateLimitConfig, routes: &[Route]) -> Result<Self, String> {
        let addresses = ClientAddresses::new(&config.trusted_proxies, config.ipv6_prefix)?;
        let by_key: HashMap<&str, &Route> = routes.iter().map(|r| (r.key.as_str(), r)).collect();
        for key in config.endpoints.keys() {
            if !by_key.contains_key(key.as_str()) {
                return Err(format!(
                    "rate_limits.endpoints.{key:?} names no endpoint (write the method in capitals and the path template without /api/v1, as in openapi.yaml)"
                ));
            }
        }
        let mut fail_closed = HashSet::new();
        for pattern in &config.fail_closed {
            let matched: Vec<&Route> = routes.iter().filter(|r| glob(pattern, &r.key)).collect();
            if matched.is_empty() {
                return Err(format!(
                    "rate_limits.fail_closed: {pattern:?} matches no endpoint"
                ));
            }
            fail_closed.extend(matched.into_iter().map(|r| r.key.clone()));
        }
        let mut groups = Vec::new();
        for (name, group) in &config.groups {
            let members: Vec<&str> = routes
                .iter()
                .map(|r| r.key.as_str())
                .filter(|key| group.endpoints.iter().any(|pattern| glob(pattern, key)))
                .collect();
            for pattern in &group.endpoints {
                if !routes.iter().any(|r| glob(pattern, &r.key)) {
                    return Err(format!(
                        "rate_limits.groups.{name}: {pattern:?} matches no endpoint"
                    ));
                }
            }
            groups.push((name, members, &group.limits));
        }

        let mut rules = HashMap::new();
        for route in routes {
            let origin = |dimension: &str, source: &str| {
                format!(
                    "rate_limits.{source} sets {dimension:?} for {:?}",
                    route.key
                )
            };
            let defaults = defaults_for(config, route)?;
            // An endpoint's own setting replaces its group's, dimension by dimension; `false`
            // (a `None` here) also removes the default of that dimension.
            let mut merged: HashMap<Dimension, Option<Limit>> = HashMap::new();
            let mut from_groups: HashMap<Dimension, &str> = HashMap::new();
            for (name, members, limits) in &groups {
                if !members.contains(&route.key.as_str()) {
                    continue;
                }
                for (dimension, setting) in *limits {
                    let source = format!("groups.{name}");
                    let parsed = parse_dimension(dimension, &source)?;
                    check_applicable(&parsed, route, &origin(dimension, &source))?;
                    if let Some(other) = from_groups.insert(parsed.clone(), name) {
                        return Err(format!(
                            "{} and rate_limits.groups.{other} both set {dimension:?} for {:?}; set it on the endpoint instead",
                            origin(dimension, &source),
                            route.key
                        ));
                    }
                    merged.insert(parsed, limit_of(setting, &source, dimension)?);
                }
            }
            if let Some(table) = config.endpoints.get(&route.key) {
                let source = format!("endpoints.{:?}", route.key);
                for (dimension, setting) in table {
                    let parsed = parse_dimension(dimension, &source)?;
                    check_applicable(&parsed, route, &origin(dimension, &source))?;
                    merged.insert(parsed, limit_of(setting, &source, dimension)?);
                }
            }
            let mut compiled: Vec<Rule> = defaults
                .into_iter()
                .filter(|(dimension, _)| !matches!(merged.get(dimension), Some(None)))
                .map(|(dimension, limit)| {
                    compile_rule(dimension, &limit, &format!("default:{}", route.key))
                })
                .collect();
            compiled.extend(merged.into_iter().filter_map(|(dimension, limit)| {
                limit.map(|limit| compile_rule(dimension, &limit, &route.key))
            }));
            rules.insert(route.key.clone(), compiled);
        }
        let unknown_route = Route::new(OTHER_METHOD, "/*", Access::Anonymous);
        let unknown = defaults_for(config, &unknown_route)?
            .into_iter()
            .map(|(dimension, limit)| {
                compile_rule(dimension, &limit, &format!("default:{}", unknown_route.key))
            })
            .collect();
        Ok(Self {
            enabled: config.enabled,
            addresses,
            suspension: SuspensionState::new(Duration::from_secs(config.max_suspension_seconds)),
            rules,
            unknown,
            fail_closed,
            last_failure_log: AtomicU64::new(0),
        })
    }

    /// The suspension of these limits in force, which `aspen_limits::suspension::watch` keeps
    /// current.
    pub fn suspension(&self) -> &SuspensionState {
        &self.suspension
    }

    /// Where requests come from, behind the configured proxies.
    pub fn addresses(&self) -> &ClientAddresses {
        &self.addresses
    }

    /// Checks the rules of `route` that belong to `stage`, counting the request against each.
    pub async fn check(
        &self,
        valkey: &Client,
        route: &str,
        stage: Stage,
        identity: &Identity<'_>,
    ) -> Decision {
        if !self.enabled {
            return Decision::Allowed;
        }
        let rules = self.rules.get(route).unwrap_or(&self.unknown);
        let exemption = self.suspension.exemption(identity.ip);
        if exemption == Exemption::All {
            return Decision::Allowed;
        }
        let keyed: Vec<(&Rule, String)> = rules
            .iter()
            .filter(|rule| rule.dimension.stage() == stage)
            .filter(|rule| {
                exemption != Exemption::AddressLimits
                    || !matches!(rule.dimension, Dimension::Ip | Dimension::IpPer(_))
            })
            .filter_map(|rule| Some((rule, self.key(rule, identity)?)))
            .collect();
        if keyed.is_empty() {
            return Decision::Allowed;
        }
        let started = std::time::Instant::now();
        // Sent concurrently, which the client pipelines on its connection: one round trip.
        let waits = futures_util::future::join_all(keyed.iter().map(|(rule, key)| {
            valkey.eval::<i64, _, _, _>(
                GCRA,
                key.clone(),
                vec![rule.emission_ms, rule.tolerance_ms],
            )
        }))
        .await;
        let mut longest = 0;
        for wait in waits {
            match wait {
                Ok(wait) => longest = longest.max(wait),
                Err(e) => {
                    let refused = stage == Stage::Username || self.fail_closed.contains(route);
                    self.log_failure(&e);
                    return if refused {
                        Decision::Unavailable
                    } else {
                        Decision::Allowed
                    };
                }
            }
        }
        metrics::histogram!(aspen_metrics::api::RATE_LIMIT_CHECK_DURATION)
            .record(started.elapsed().as_secs_f64());
        if longest > 0 {
            metrics::counter!(aspen_metrics::api::RATE_LIMIT_REFUSALS, "route" => route.to_string())
                .increment(1);
            Decision::Limited {
                retry_after: Duration::from_millis(longest.unsigned_abs()),
            }
        } else {
            Decision::Allowed
        }
    }

    /// The Valkey key of a rule's bucket for this request, or `None` when the request lacks
    /// what the rule counts by (no session for a user rule, say).
    fn key(&self, rule: &Rule, identity: &Identity<'_>) -> Option<String> {
        let param = |name: &str| {
            identity
                .params
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| key_part(value))
        };
        let ip = || identity.ip.map(|ip| self.addresses.key(ip));
        let user = || identity.user.map(|user| user.0.to_string());
        let value = match &rule.dimension {
            Dimension::Global => "all".to_string(),
            Dimension::Ip => ip()?,
            Dimension::User => user()?,
            // One account answers to its name in any case, so every case of it shares a bucket.
            Dimension::Username => key_part(&identity.username?.to_lowercase()),
            Dimension::Per(name) => param(name)?,
            Dimension::UserPer(name) => format!("{}:{}", user()?, param(name)?),
            Dimension::IpPer(name) => format!("{}:{}", ip()?, param(name)?),
        };
        Some(format!(
            "rl:{}:{}:{value}",
            rule.bucket,
            rule.dimension.name()
        ))
    }

    fn log_failure(&self, error: &fred::error::Error) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        let last = self.last_failure_log.load(Ordering::Relaxed);
        if now.saturating_sub(last) >= FAILURE_LOG_INTERVAL_MS
            && self
                .last_failure_log
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
        {
            tracing::error!(
                %error,
                "rate limits unavailable, refusing the endpoints that fail closed and letting other requests through"
            );
        }
    }
}

fn key_part(value: &str) -> String {
    if value.len() <= MAX_KEY_PART && !value.contains(':') {
        value.to_string()
    } else {
        data_encoding::HEXLOWER.encode(&Sha256::digest(value.as_bytes()))
    }
}

fn parse_dimension(name: &str, source: &str) -> Result<Dimension, String> {
    Dimension::parse(name).ok_or_else(|| {
        format!(
            "rate_limits.{source}: {name:?} is not a dimension (global, ip, user, username, per_<param>, user_per_<param>, ip_per_<param>)"
        )
    })
}

fn limit_of(
    setting: &LimitSetting,
    source: &str,
    dimension: &str,
) -> Result<Option<Limit>, String> {
    setting
        .limit(&format!("rate_limits.{source}.{dimension}"))
        .map(|limit| limit.cloned())
}

/// Whether a dimension can count requests to `route` at all.
/// The default limits that apply to `route`, which is wherever they can: a user limit where
/// users sign in, a per-parameter limit where the path has the parameter.
fn defaults_for(
    config: &RateLimitConfig,
    route: &Route,
) -> Result<HashMap<Dimension, Limit>, String> {
    let mut defaults = HashMap::new();
    for (dimension, setting) in &config.default {
        let parsed = parse_dimension(dimension, "default")?;
        if let Some(limit) = limit_of(setting, "default", dimension)?
            && applies(&parsed, route)
        {
            defaults.insert(parsed, limit);
        }
    }
    Ok(defaults)
}

fn applies(dimension: &Dimension, route: &Route) -> bool {
    (!dimension.needs_user() || route.access != Access::Anonymous)
        && dimension
            .param()
            .is_none_or(|param| route.params.iter().any(|p| p == param))
        && (*dimension != Dimension::Username || route.key == SIGN_IN_ROUTE)
}

/// As `applies`, but an explicit setting that can never apply is an error.
fn check_applicable(dimension: &Dimension, route: &Route, origin: &str) -> Result<(), String> {
    if applies(dimension, route) {
        return Ok(());
    }
    Err(if dimension.needs_user() {
        format!("{origin}, but nobody signs in to call it")
    } else if let Some(param) = dimension.param() {
        format!("{origin}, but its path has no {{{param}}}")
    } else {
        format!("{origin}, but only {SIGN_IN_ROUTE:?} knows a username")
    })
}

fn compile_rule(dimension: Dimension, limit: &Limit, route: &str) -> Rule {
    let rate = limit.rate();
    Rule {
        dimension,
        bucket: limit.bucket.clone().unwrap_or_else(|| route.to_string()),
        emission_ms: rate.emission_ms,
        tolerance_ms: rate.tolerance_ms,
    }
}

/// Matches `text` against `pattern`, where `*` stands for any run of characters.
pub fn glob(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(at) => rest = &rest[at + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aspen_config::{RateLimitGroup, RateLimitOverrides, RuleTable};

    fn routes() -> Vec<Route> {
        vec![
            Route::new("post", "/auth/login", Access::Anonymous),
            Route::new("POST", "/users", Access::Anonymous),
            Route::new(
                "POST",
                "/channels/{channel}/messages",
                Access::Authenticated,
            ),
            Route::new("GET", "/channels/{channel}", Access::Authenticated),
            Route::new("POST", "/auth/passkey-ceremonies", Access::Optional),
        ]
    }

    fn limit(requests: u32, per_seconds: f64) -> LimitSetting {
        LimitSetting::Limit(Limit {
            requests,
            per_seconds,
            burst: None,
            bucket: None,
        })
    }

    fn config() -> RateLimitConfig {
        RateLimitConfig {
            enabled: true,
            trusted_proxies: vec!["10.0.0.0/8".into(), "::1".into()],
            ipv6_prefix: 64,
            max_suspension_seconds: 3600,
            default: RuleTable::from([
                ("user".into(), limit(100, 60.0)),
                ("ip".into(), limit(200, 60.0)),
                ("per_channel".into(), limit(1000, 60.0)),
            ]),
            groups: Default::default(),
            endpoints: Default::default(),
            fail_closed: Default::default(),
        }
    }

    fn dimensions(limiter: &RateLimiter, route: &str) -> Vec<String> {
        limiter.dimensions(route)
    }

    #[test]
    fn defaults_apply_only_where_they_can() {
        let limiter = RateLimiter::compile(&config(), &routes()).unwrap();
        assert_eq!(dimensions(&limiter, "POST /users"), ["ip"]);
        assert_eq!(
            dimensions(&limiter, "GET /channels/{channel}"),
            ["ip", "per_channel", "user"]
        );
        assert_eq!(
            dimensions(&limiter, "POST /auth/passkey-ceremonies"),
            ["ip", "user"]
        );
    }

    #[test]
    fn made_up_methods_share_one_key() {
        assert_eq!(route_key("post", "/users"), "POST /users");
        assert_eq!(route_key("FROBNICATE", "/users"), "OTHER /users");
        assert_eq!(route_key("x1", "/users"), route_key("x2", "/users"));
    }

    #[test]
    fn a_route_without_rules_of_its_own_gets_the_defaults_that_need_nothing() {
        let limiter = RateLimiter::compile(&config(), &routes()).unwrap();
        assert_eq!(dimensions(&limiter, "OTHER /channels/{channel}"), ["ip"]);
        assert_eq!(dimensions(&limiter, "DELETE /users"), ["ip"]);
    }

    fn rules_of<'a>(limiter: &'a RateLimiter, route: &str, dimension: &Dimension) -> Vec<&'a Rule> {
        limiter.rules[route]
            .iter()
            .filter(|rule| &rule.dimension == dimension)
            .collect()
    }

    #[test]
    fn endpoints_replace_groups_and_add_to_defaults() {
        let mut config = config();
        config.groups.insert(
            "writes".into(),
            RateLimitGroup {
                endpoints: vec!["POST /channels/*".into()],
                limits: RuleTable::from([
                    ("user".into(), limit(10, 60.0)),
                    ("per_channel".into(), limit(500, 60.0)),
                ]),
            },
        );
        config.endpoints.insert(
            "POST /channels/{channel}/messages".into(),
            RuleTable::from([
                ("ip".into(), LimitSetting::Off(false)),
                ("per_channel".into(), limit(50, 60.0)),
                ("user_per_channel".into(), limit(5, 5.0)),
            ]),
        );
        let limiter = RateLimiter::compile(&config, &routes()).unwrap();
        let route = "POST /channels/{channel}/messages";
        // The default user limit and the group's both count, in separate buckets.
        let users = rules_of(&limiter, route, &Dimension::User);
        let mut buckets: Vec<(&str, u64)> = users
            .iter()
            .map(|rule| (rule.bucket.as_str(), rule.emission_ms))
            .collect();
        buckets.sort();
        assert_eq!(
            buckets,
            [(route, 6000), (&format!("default:{route}")[..], 600)]
        );
        // `false` removes the default too.
        assert!(rules_of(&limiter, route, &Dimension::Ip).is_empty());
        // The endpoint's per-channel limit replaced the group's, beside the default's.
        let mut per_channel: Vec<u64> =
            rules_of(&limiter, route, &Dimension::Per("channel".into()))
                .iter()
                .map(|rule| rule.emission_ms)
                .collect();
        per_channel.sort();
        assert_eq!(per_channel, [60, 1200]);
        assert_eq!(
            rules_of(&limiter, route, &Dimension::UserPer("channel".into()))[0].emission_ms,
            1000
        );
    }

    #[test]
    fn overrides_replace_whole_limits() {
        let built_in = RateLimitConfig::built_in().unwrap();
        let route = "GET /auth/methods";
        let before = match &built_in.endpoints[route]["ip"] {
            LimitSetting::Limit(limit) => limit.clone(),
            LimitSetting::Off(_) => panic!("expected a built-in limit"),
        };
        assert!(before.burst.is_some());
        let overrides: RateLimitOverrides = config::Config::builder()
            .add_source(config::File::from_str(
                "[endpoints.\"GET /auth/methods\"]\nip = { requests = 3, per_seconds = 60 }\n[groups.extra]\nendpoints = [\"GET /users/statuses\"]\nlimits = { ip = { requests = 1, per_seconds = 1 } }\n",
                config::FileFormat::Toml,
            ))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        let merged = built_in.overlay(overrides).unwrap();
        let LimitSetting::Limit(after) = &merged.endpoints[route]["ip"] else {
            panic!("expected a limit");
        };
        assert_eq!((after.requests, after.burst), (3, None));
        assert!(merged.groups.contains_key("extra"));
        assert!(merged.groups.contains_key("sign_in"));
        let missing = RateLimitOverrides {
            groups: [(
                "new".to_string(),
                crate::aspen_config::RateLimitGroupOverride {
                    endpoints: None,
                    limits: RuleTable::new(),
                },
            )]
            .into(),
            ..RateLimitOverrides::default()
        };
        assert!(
            RateLimitConfig::built_in()
                .unwrap()
                .overlay(missing)
                .is_err()
        );
    }

    #[test]
    fn mistakes_are_configuration_errors() {
        let bad = |edit: &dyn Fn(&mut RateLimitConfig)| {
            let mut config = config();
            edit(&mut config);
            RateLimiter::compile(&config, &routes()).unwrap_err()
        };
        assert!(
            bad(&|c| {
                c.endpoints
                    .insert("POST /channels/{id}/messages".into(), RuleTable::new());
            })
            .contains("names no endpoint")
        );
        assert!(
            bad(&|c| {
                c.endpoints.insert(
                    "POST /users".into(),
                    RuleTable::from([("user".into(), limit(1, 1.0))]),
                );
            })
            .contains("nobody signs in")
        );
        assert!(
            bad(&|c| {
                c.endpoints.insert(
                    "POST /users".into(),
                    RuleTable::from([("per_channel".into(), limit(1, 1.0))]),
                );
            })
            .contains("no {channel}")
        );
        assert!(
            bad(&|c| {
                c.endpoints.insert(
                    "POST /users".into(),
                    RuleTable::from([("username".into(), limit(1, 1.0))]),
                );
            })
            .contains("knows a username")
        );
        assert!(
            bad(&|c| {
                c.endpoints.insert(
                    "POST /users".into(),
                    RuleTable::from([("per_second".into(), limit(0, 1.0))]),
                );
            })
            .contains("no {second}")
        );
        assert!(
            bad(&|c| {
                c.endpoints.insert(
                    "POST /users".into(),
                    RuleTable::from([("ip".into(), limit(0, 1.0))]),
                );
            })
            .contains("must be positive")
        );
        assert!(
            bad(&|c| {
                c.endpoints.insert(
                    "POST /users".into(),
                    RuleTable::from([("ip".into(), LimitSetting::Off(true))]),
                );
            })
            .contains("is not a limit")
        );
        assert!(
            bad(&|c| {
                c.endpoints.insert(
                    "POST /users".into(),
                    RuleTable::from([("everyone".into(), limit(1, 1.0))]),
                );
            })
            .contains("is not a dimension")
        );
        assert!(bad(&|c| c.trusted_proxies.push("proxy.local".into())).contains("trusted_proxies"));
        assert!(
            bad(&|c| {
                c.groups.insert(
                    "typo".into(),
                    RateLimitGroup {
                        endpoints: vec!["POST /nowhere".into()],
                        limits: RuleTable::new(),
                    },
                );
            })
            .contains("matches no endpoint")
        );
        assert!(
            bad(&|c| {
                for name in ["a", "b"] {
                    c.groups.insert(
                        name.into(),
                        RateLimitGroup {
                            endpoints: vec!["POST /users".into()],
                            limits: RuleTable::from([("ip".into(), limit(1, 1.0))]),
                        },
                    );
                }
            })
            .contains("both set")
        );
    }

    #[test]
    fn a_fail_closed_pattern_must_match_an_endpoint() {
        let mut config = config();
        config.fail_closed = vec!["POST /nowhere".into()];
        assert!(RateLimiter::compile(&config, &routes()).is_err());
    }

    #[test]
    fn keys_name_the_bucket_dimension_and_identity() {
        let limiter = RateLimiter::compile(&config(), &routes()).unwrap();
        let rule = |dimension: Dimension| Rule {
            dimension,
            bucket: "b".into(),
            emission_ms: 1,
            tolerance_ms: 0,
        };
        let user = UserId(uuid::Uuid::nil());
        let identity = Identity {
            ip: Some("2001:db8:1:2:3:4:5:6".parse().unwrap()),
            user: Some(user),
            username: None,
            params: vec![("channel", "c1")],
        };
        assert_eq!(
            limiter.key(&rule(Dimension::Ip), &identity).unwrap(),
            "rl:b:ip:2001:db8:1:2::/64"
        );
        assert_eq!(
            limiter
                .key(&rule(Dimension::UserPer("channel".into())), &identity)
                .unwrap(),
            format!("rl:b:user_per_channel:{}:c1", user.0)
        );
        assert!(limiter.key(&rule(Dimension::Username), &identity).is_none());
        let mapped = Identity {
            ip: Some("::ffff:192.0.2.7".parse().unwrap()),
            ..Identity::default()
        };
        assert_eq!(
            limiter.key(&rule(Dimension::Ip), &mapped).unwrap(),
            "rl:b:ip:192.0.2.7"
        );
    }

    #[test]
    fn trusted_proxies_come_from_the_configuration() {
        let limiter = RateLimiter::compile(&config(), &routes()).unwrap();
        assert!(limiter.addresses().is_trusted("10.1.2.3".parse().unwrap()));
        assert!(limiter.addresses().is_trusted("::1".parse().unwrap()));
        assert!(!limiter.addresses().is_trusted("192.0.2.1".parse().unwrap()));
    }

    #[test]
    fn glob_matches_runs_of_characters() {
        assert!(glob("POST /auth/*", "POST /auth/login"));
        assert!(glob("* /users/{user}", "GET /users/{user}"));
        assert!(glob("*/messages*", "POST /channels/{channel}/messages"));
        assert!(!glob("POST /auth/*", "GET /auth/methods"));
        assert!(glob("GET /events", "GET /events"));
        assert!(!glob("GET /events", "GET /events/x"));
    }
}
