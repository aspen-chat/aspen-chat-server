use aspen_limits::RuleTable;
use serde::Deserialize;
use smart_default::SmartDefault;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use uuid::Uuid;

/// Read from `voice_server.toml` in the working directory, with `ASPEN_VOICE_SERVER_`
/// environment variables overriding it (`ASPEN_VOICE_SERVER_RTC__ANNOUNCED_ADDRESS` sets
/// `rtc.announced_address`).
#[derive(Clone, Debug, Deserialize)]
pub struct VoiceServerConfig {
    /// This server's row in the API server's `voice_server` table. Join tokens name the servers
    /// they are good for by this id, and every report carries it.
    pub id: Uuid,
    /// Shared with the API server; verifies join tokens.
    pub token_secret: String,
    pub nats_url: String,
    /// How this server signs in to NATS: a user of its own (`[nats]`), allowed only this
    /// server's subjects, or the deployment's token, which lets it do anything the API servers
    /// can. Exactly one is given.
    #[serde(default)]
    pub nats: Option<NatsUser>,
    #[serde(default)]
    pub nats_auth_token: Option<String>,
    /// Where the HTTP server, health check, and signalling socket listen.
    #[serde(default = "default_listen_addr")]
    pub listen_addr: SocketAddr,
    #[serde(default)]
    pub rtc: RtcConfig,
    #[serde(default)]
    pub transfer: TransferConfig,
    /// mediasoup workers, each a process carrying some of the calls. Defaults to the number
    /// of CPUs.
    #[serde(default = "default_workers")]
    pub workers: usize,
    #[serde(default)]
    pub metrics: MetricsConfig,
    /// What `voice_server.toml` says about limits; `rate_limits` is the result.
    #[serde(default, rename = "rate_limits")]
    pub rate_limit_overrides: LimitOverrides,
    /// The limits in force: the built-in ones (`limits.toml`) with the overrides laid over them.
    #[serde(skip)]
    pub rate_limits: LimitSettings,
}

/// A NATS user for this voice server alone. `docs/operators/installing.md` gives the
/// permissions it needs: publishing this server's reports, reading its commands and the rate
/// limit suspension, and replies to its own inbox (`Reporter::inbox_prefix`).
#[derive(Clone, Debug, Deserialize)]
pub struct NatsUser {
    pub user: String,
    pub password: String,
}

/// How a voice server signs in to NATS.
pub enum NatsAuth {
    User(NatsUser),
    Token(String),
}

impl VoiceServerConfig {
    /// The one way of signing in to NATS the settings give.
    pub fn nats_auth(&self) -> anyhow::Result<NatsAuth> {
        match (&self.nats, &self.nats_auth_token) {
            (Some(user), None) => Ok(NatsAuth::User(user.clone())),
            (None, Some(token)) => Ok(NatsAuth::Token(token.clone())),
            (None, None) => anyhow::bail!(
                "give [nats] user and password (a NATS user for voice servers), or nats_auth_token"
            ),
            (Some(_), Some(_)) => {
                anyhow::bail!("give either [nats] user and password or nats_auth_token, not both")
            }
        }
    }
}

/// Prometheus metrics (`aspen_metrics::voice`), served on a listener of their own.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct MetricsConfig {
    #[default = true]
    pub enabled: bool,
    /// Where `GET /metrics` is served; loopback by default.
    #[default(SocketAddr::from(([127, 0, 0, 1], 9465)))]
    pub listen_addr: SocketAddr,
}

/// The limits in force (`limits.rs`); `limits.toml` documents each.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct LimitSettings {
    pub enabled: bool,
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    pub ipv6_prefix: u8,
    pub max_suspension_seconds: u64,
    pub max_message_bytes: usize,
    pub max_pending_sockets_per_ip: u32,
    /// By route: `health`, `signalling`.
    #[serde(default)]
    pub http: HashMap<String, RuleTable>,
    /// By frame type, or `any`.
    #[serde(default)]
    pub frames: HashMap<String, RuleTable>,
}

/// The `[rate_limits]` of `voice_server.toml`. Each limit given replaces the built-in one for
/// the same place and dimension whole, so one written without `burst` has the default burst
/// rather than the built-in one's; everything not given keeps its built-in value.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct LimitOverrides {
    pub enabled: Option<bool>,
    pub trusted_proxies: Option<Vec<String>>,
    pub ipv6_prefix: Option<u8>,
    pub max_suspension_seconds: Option<u64>,
    pub max_message_bytes: Option<usize>,
    pub max_pending_sockets_per_ip: Option<u32>,
    #[serde(default)]
    pub http: HashMap<String, RuleTable>,
    #[serde(default)]
    pub frames: HashMap<String, RuleTable>,
}

const BUILT_IN_LIMITS: &str = include_str!("limits.toml");

impl LimitSettings {
    pub fn built_in() -> Result<Self, config::ConfigError> {
        config::Config::builder()
            .add_source(config::File::from_str(
                BUILT_IN_LIMITS,
                config::FileFormat::Toml,
            ))
            .build()?
            .get("rate_limits")
    }

    pub fn overlay(mut self, overrides: LimitOverrides) -> Self {
        if let Some(enabled) = overrides.enabled {
            self.enabled = enabled;
        }
        if let Some(proxies) = overrides.trusted_proxies {
            self.trusted_proxies = proxies;
        }
        if let Some(prefix) = overrides.ipv6_prefix {
            self.ipv6_prefix = prefix;
        }
        if let Some(max) = overrides.max_suspension_seconds {
            self.max_suspension_seconds = max;
        }
        if let Some(bytes) = overrides.max_message_bytes {
            self.max_message_bytes = bytes;
        }
        if let Some(sockets) = overrides.max_pending_sockets_per_ip {
            self.max_pending_sockets_per_ip = sockets;
        }
        aspen_limits::overlay_tables(&mut self.http, overrides.http);
        aspen_limits::overlay_tables(&mut self.frames, overrides.frames);
        self
    }
}

/// Where WebRTC media is received.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct RtcConfig {
    /// The interface media is bound to; every interface by default.
    #[default(IpAddr::V4(Ipv4Addr::UNSPECIFIED))]
    pub ip: IpAddr,
    /// The address clients are told to send media to. Required in effect when `ip` is
    /// unspecified (`0.0.0.0`), since a client cannot send to that; left unset, the host's
    /// primary interface address is announced, which suits a development machine and a server
    /// with a public address on an interface. A server behind NAT sets its public address.
    /// It must never be a loopback address: Firefox does not pair its own host candidates
    /// with a loopback peer, so media never connects.
    pub announced_address: Option<String>,
    #[default = 40000]
    pub min_port: u16,
    #[default = 40999]
    pub max_port: u16,
}

/// File transfers between the people of a call (`transfer`): a STUN and TURN server on one UDP
/// port, on the media interface (`rtc.ip`) and announced at `rtc.announced_address` like media.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct TransferConfig {
    /// The most every transfer relayed through this server may carry together, in megabits a
    /// second. 0 turns relaying off: transfers then go directly between devices or not at all,
    /// and STUN still answers so that direct connections can be made.
    #[default = 50]
    pub relay_mbps: u32,
    /// The UDP port STUN and TURN answer on.
    #[default = 3478]
    pub port: u16,
    /// The UDP ports relayed transfers take, one per side of each; outside the media range.
    #[default = 42000]
    pub relay_min_port: u16,
    #[default = 42999]
    pub relay_max_port: u16,
}

fn default_workers() -> usize {
    num_cpus::get().max(1)
}

fn default_listen_addr() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 9001)
}

impl RtcConfig {
    /// The address to put in ICE candidates: the configured one, else the primary interface's
    /// address when bound to every interface, else nothing, which announces `ip` itself.
    pub fn resolved_announced_address(&self) -> anyhow::Result<Option<String>> {
        if let Some(configured) = &self.announced_address {
            return Ok(Some(configured.clone()));
        }
        if !self.ip.is_unspecified() {
            return Ok(None);
        }
        let detected = local_ip_address::local_ip().map_err(|e| {
            anyhow::anyhow!(
                "rtc.ip is unspecified and no primary interface address could be found ({e}); set rtc.announced_address"
            )
        })?;
        Ok(Some(detected.to_string()))
    }
}

/// Where settings come from: `voice_server.toml`, then the environment, which overrides it.
fn sources() -> Result<config::Config, config::ConfigError> {
    config::Config::builder()
        .add_source(
            config::File::new("voice_server.toml", config::FileFormat::Toml).required(false),
        )
        .add_source(
            config::Environment::with_prefix("ASPEN_VOICE_SERVER")
                .prefix_separator("_")
                .separator("__"),
        )
        .build()
}

/// The media settings alone, which `estimate-capacity` reads: it needs no registry id, secret,
/// or NATS, so it runs on a machine not yet set up as a voice server.
#[derive(Clone, Debug, Deserialize)]
pub struct MediaConfig {
    #[serde(default)]
    pub rtc: RtcConfig,
    #[serde(default = "default_workers")]
    pub workers: usize,
}

pub fn load_media_config() -> Result<MediaConfig, config::ConfigError> {
    sources()?.try_deserialize::<MediaConfig>()
}

pub fn load_config() -> Result<VoiceServerConfig, config::ConfigError> {
    sources()?
        .try_deserialize::<VoiceServerConfig>()
        .and_then(|mut config| {
            // Merged here rather than as one more config source, which would merge a limit
            // given in `voice_server.toml` into the built-in one field by field.
            config.rate_limits = LimitSettings::built_in()?
                .overlay(std::mem::take(&mut config.rate_limit_overrides));
            Ok(config)
        })
}
