use aspen_limits::RuleTable;
use aspen_sockets::EVERY_INTERFACE;
use serde::Deserialize;
use smart_default::SmartDefault;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use uuid::Uuid;

/// Read from `voice_server.toml` in the working directory, with `ASPEN_VOICE_SERVER_`
/// environment variables overriding it (`ASPEN_VOICE_SERVER_RTC__ANNOUNCED_ADDRESS` sets
/// `rtc.announced_address`).
#[derive(Clone, Debug, Deserialize)]
pub struct VoiceServerConfig {
    /// This server's row in the API server's `voice_server` table. Join tokens name the servers
    /// they are good for by this id, and every report carries it. The server stops at startup
    /// when the API servers say no registered server has it (`token_keys`).
    pub id: Uuid,
    /// Join tokens are signed by the API servers' key, whose public half this server asks them
    /// for (`signalling::TokenKeys`). Given this too, it also takes tokens of the shared-secret
    /// form under it (`voice_protocol::token::verify_shared`), which API servers that do not
    /// sign with a key of their own make; it refuses to start with the development value or one
    /// shorter than `MIN_TOKEN_SECRET_BYTES` unless `development`.
    #[serde(default)]
    pub token_secret: Option<String>,
    /// Lets the server start with a token secret anyone may know, for a development machine.
    #[serde(default)]
    pub development: bool,
    pub nats_url: String,
    /// How this server signs in to NATS: a user of its own (`[nats_user]`), allowed only this
    /// server's subjects, or the deployment's token, which lets it do anything the API servers
    /// can. Exactly one is given.
    #[serde(default)]
    pub nats_user: Option<NatsUser>,
    #[serde(default)]
    pub nats_auth_token: Option<String>,
    /// TLS to NATS: `[nats.tls]`'s certificate files, which also make TLS required.
    #[serde(default)]
    pub nats: NatsConfig,
    /// Where the HTTP server, health check, and signalling socket listen: one address or
    /// several. Left unset, `DEFAULT_PORT` on every interface over IPv4 and IPv6
    /// (`listen_addrs`).
    #[serde(default, deserialize_with = "aspen_config_values::optional_list")]
    pub listen_addr: Option<Vec<SocketAddr>>,
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

/// A NATS user for this voice server alone. `docs/operators/installing/6-voice-servers.md`
/// gives the permissions it needs: publishing this server's reports, reading its commands and
/// the rate limit suspension, and replies to its own inbox
/// (`voice_protocol::control::inbox_prefix`).
#[derive(Clone, Debug, Deserialize)]
pub struct NatsUser {
    pub user: String,
    pub password: String,
}

/// `[nats]`: how NATS is reached besides its address and credentials.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NatsConfig {
    pub tls: Option<aspen_tls::TlsFiles>,
}

/// How a voice server signs in to NATS.
pub enum NatsAuth {
    User(NatsUser),
    Token(String),
}

impl VoiceServerConfig {
    /// The one way of signing in to NATS the settings give.
    pub fn nats_auth(&self) -> anyhow::Result<NatsAuth> {
        if let Some(tls) = &self.nats.tls {
            tls.identity("nats.tls").map_err(anyhow::Error::msg)?;
        }
        match (&self.nats_user, &self.nats_auth_token) {
            (Some(user), None) => Ok(NatsAuth::User(user.clone())),
            (None, Some(token)) => Ok(NatsAuth::Token(token.clone())),
            (None, None) => anyhow::bail!(
                "give [nats_user] user and password (a NATS user for voice servers), or nats_auth_token"
            ),
            (Some(_), Some(_)) => {
                anyhow::bail!(
                    "give either [nats_user] user and password or nats_auth_token, not both"
                )
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
    /// Connections the HTTP listener holds at once, signalling sockets included.
    pub max_connections: usize,
    /// Calls one user may be in at once on this server (`rooms::Rooms::join`).
    pub max_seats_per_user: usize,
    /// People one call on this server may hold at once.
    pub max_participants_per_call: usize,
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
    #[serde(default, deserialize_with = "aspen_config_values::optional_list")]
    pub trusted_proxies: Option<Vec<String>>,
    pub ipv6_prefix: Option<u8>,
    pub max_suspension_seconds: Option<u64>,
    pub max_message_bytes: Option<usize>,
    pub max_pending_sockets_per_ip: Option<u32>,
    pub max_connections: Option<usize>,
    pub max_seats_per_user: Option<usize>,
    pub max_participants_per_call: Option<usize>,
    #[serde(default, deserialize_with = "aspen_config_values::table")]
    pub http: HashMap<String, RuleTable>,
    #[serde(default, deserialize_with = "aspen_config_values::table")]
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
        if let Some(connections) = overrides.max_connections {
            self.max_connections = connections;
        }
        if let Some(seats) = overrides.max_seats_per_user {
            self.max_seats_per_user = seats;
        }
        if let Some(people) = overrides.max_participants_per_call {
            self.max_participants_per_call = people;
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
    /// The interfaces media is bound to, at most one address of each family. Left unset,
    /// every interface over IPv4 and IPv6 (`0.0.0.0` and `::`), each family only where this
    /// machine has it (`media_addresses`).
    #[serde(deserialize_with = "aspen_config_values::optional_list")]
    pub ip: Option<Vec<IpAddr>>,
    /// The addresses clients are told to send media to: an IPv4 address for the IPv4
    /// interface, an IPv6 one for the IPv6 interface, or a host name, which stands for every
    /// interface that has no address of its own here. Required in effect for an unspecified
    /// interface (`0.0.0.0`, `::`), since a client cannot send to that; left unset, the
    /// address the host reaches the internet from in that family is announced, which suits a
    /// development machine and a server with a public address on an interface. A server
    /// behind NAT sets its public address. None may be a loopback address: Firefox does not
    /// pair its own host candidates with a loopback peer, so media never connects.
    #[serde(deserialize_with = "aspen_config_values::list")]
    pub announced_address: Vec<String>,
    #[default = 40000]
    pub min_port: u16,
    #[default = 40999]
    pub max_port: u16,
}

/// File transfers between the people of a call (`transfer`): a STUN and TURN server on one UDP
/// port, on the media interfaces (`rtc.ip`) and announced at `rtc.announced_address` like media.
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

impl TransferConfig {
    /// Refuses relay ports that overlap the media range or the STUN and TURN port: the relay
    /// would then take ports media needs, or relay to them.
    pub fn check(&self, rtc: &RtcConfig) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.relay_min_port <= self.relay_max_port,
            "transfer.relay_min_port is above transfer.relay_max_port"
        );
        let relay = self.relay_min_port..=self.relay_max_port;
        anyhow::ensure!(
            self.relay_max_port < rtc.min_port || rtc.max_port < self.relay_min_port,
            "transfer.relay_min_port to relay_max_port overlaps rtc.min_port to max_port; give \
             relayed transfers ports of their own"
        );
        anyhow::ensure!(
            !relay.contains(&self.port) && !(rtc.min_port..=rtc.max_port).contains(&self.port),
            "transfer.port is inside the relay or media port range"
        );
        Ok(())
    }
}

fn default_workers() -> usize {
    num_cpus::get().max(1)
}

/// The port the HTTP server listens on when `listen_addr` is unset.
const DEFAULT_PORT: u16 = 9001;

impl VoiceServerConfig {
    /// Where the HTTP server listens, and whether each must be bound: an address the settings
    /// name must, while of the defaults a family this machine lacks is left out
    /// (`aspen_sockets::tcp_listeners`).
    pub fn listen_addrs(&self) -> (Vec<SocketAddr>, bool) {
        match &self.listen_addr {
            Some(addrs) => (addrs.clone(), true),
            None => (
                EVERY_INTERFACE
                    .map(|ip| SocketAddr::new(ip, DEFAULT_PORT))
                    .to_vec(),
                false,
            ),
        }
    }
}

/// One interface media is received on, and what clients are told to send to in its place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaAddress {
    pub ip: IpAddr,
    /// An address or host name; `None` announces `ip` itself.
    pub announced: Option<String>,
}

impl MediaAddress {
    /// What clients are told to send to.
    pub fn host(&self) -> String {
        self.announced
            .clone()
            .unwrap_or_else(|| self.ip.to_string())
    }
}

fn family(ip: IpAddr) -> &'static str {
    if ip.is_ipv4() { "IPv4" } else { "IPv6" }
}

/// What a machine has of an address family, which `RtcConfig::media_addresses` asks of this
/// one and its tests of ones they make up.
pub(crate) trait Interfaces {
    /// Whether media can be bound to `ip`.
    fn bind(&self, ip: IpAddr) -> std::io::Result<()>;
    /// The address this machine reaches the internet from in `like`'s family, which clients
    /// can send to when it is a public one.
    fn primary(&self, like: IpAddr) -> Result<IpAddr, String>;
}

struct ThisMachine;

impl Interfaces for ThisMachine {
    fn bind(&self, ip: IpAddr) -> std::io::Result<()> {
        std::net::UdpSocket::bind((ip, 0)).map(drop)
    }

    fn primary(&self, like: IpAddr) -> Result<IpAddr, String> {
        let found = if like.is_ipv4() {
            local_ip_address::local_ip()
        } else {
            local_ip_address::local_ipv6()
        }
        .map_err(|e| e.to_string())?;
        // A link-local address means nothing without the interface it is on, which an ICE
        // candidate cannot name.
        let link_local = match found {
            IpAddr::V4(ip) => ip.is_link_local(),
            IpAddr::V6(ip) => ip.is_unicast_link_local(),
        };
        if found.is_loopback() || found.is_unspecified() || link_local {
            return Err(format!("{found} is not an address clients can send to"));
        }
        Ok(found)
    }
}

impl RtcConfig {
    /// Where media is received and what each interface announces: `ip` (or every interface),
    /// each with the `announced_address` of its family, else the host name among them, else,
    /// for an unspecified interface, this machine's own address in that family.
    ///
    /// A family the settings name (in `ip`, or by an announced address of that family) must
    /// be there, and the server does not start without it. One that only the defaults name is
    /// left out, with a log line, when this machine cannot bind it or has no address to
    /// announce, so a machine with one family runs on that one.
    pub fn media_addresses(&self) -> anyhow::Result<Vec<MediaAddress>> {
        self.media_addresses_on(&ThisMachine)
    }

    pub(crate) fn media_addresses_on(
        &self,
        machine: &impl Interfaces,
    ) -> anyhow::Result<Vec<MediaAddress>> {
        let ips = self.ip.as_deref().unwrap_or(&EVERY_INTERFACE);
        anyhow::ensure!(!ips.is_empty(), "rtc.ip names no address");
        let of_family = |ip: IpAddr| {
            ips.iter()
                .copied()
                .filter(move |other| other.is_ipv4() == ip.is_ipv4())
        };
        for &ip in ips {
            anyhow::ensure!(
                of_family(ip).count() == 1,
                "rtc.ip names more than one {} address; it takes one of each family at most",
                family(ip)
            );
        }
        let mut literals: Vec<IpAddr> = Vec::new();
        let mut host_name = None;
        for announced in &self.announced_address {
            let Ok(ip) = announced.parse::<IpAddr>() else {
                anyhow::ensure!(
                    host_name.replace(announced).is_none(),
                    "rtc.announced_address names more than one host name"
                );
                continue;
            };
            anyhow::ensure!(
                !ip.is_loopback(),
                "rtc.announced_address {ip} is a loopback address, which browsers cannot send media to"
            );
            anyhow::ensure!(
                !ip.is_unspecified(),
                "rtc.announced_address {ip} is not an address clients can send to"
            );
            anyhow::ensure!(
                literals.iter().all(|other| other.is_ipv4() != ip.is_ipv4()),
                "rtc.announced_address names more than one {} address",
                family(ip)
            );
            anyhow::ensure!(
                of_family(ip).next().is_some(),
                "rtc.announced_address {ip} is an {} address, but rtc.ip has none to receive on",
                family(ip)
            );
            literals.push(ip);
        }

        let mut addresses = Vec::new();
        for &ip in ips {
            let family = family(ip);
            let literal = literals
                .iter()
                .find(|other| other.is_ipv4() == ip.is_ipv4());
            let required = self.ip.is_some() || literal.is_some();
            if let Err(e) = machine.bind(ip) {
                anyhow::ensure!(
                    !required && aspen_sockets::family_missing(&e),
                    "rtc.ip {ip} cannot be bound: {e}"
                );
                tracing::info!("no media over {family}: {ip} cannot be bound ({e})");
                continue;
            }
            let announced = match literal.map(IpAddr::to_string).or(host_name.cloned()) {
                Some(announced) => Some(announced),
                None if ip.is_unspecified() => match machine.primary(ip) {
                    Ok(primary) => Some(primary.to_string()),
                    Err(e) => {
                        anyhow::ensure!(
                            !required,
                            "rtc.ip is {ip} and this machine has no {family} address to                              announce ({e}); add one to rtc.announced_address"
                        );
                        tracing::info!(
                            "no media over {family}: this machine has no {family} address to                              announce ({e}); add one to rtc.announced_address if it has"
                        );
                        continue;
                    }
                },
                None => {
                    anyhow::ensure!(
                        !ip.is_loopback(),
                        "rtc.ip {ip} is a loopback address, which browsers cannot send media                          to; bind an interface or leave it unspecified"
                    );
                    None
                }
            };
            addresses.push(MediaAddress { ip, announced });
        }
        anyhow::ensure!(
            !addresses.is_empty(),
            "this machine has no address to receive media on over IPv4 or IPv6; set rtc.ip and              rtc.announced_address"
        );
        Ok(addresses)
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

/// The API server's development `[voice] token_secret`, which is in its source.
const DEVELOPMENT_TOKEN_SECRET: &str = "aspen_dev_voice_secret";
/// The shortest token secret a server not in `development` accepts, as the API server's
/// `https` deployments do: as long as the HMAC-SHA256 key it signs with.
const MIN_TOKEN_SECRET_BYTES: usize = 32;

impl VoiceServerConfig {
    /// Refuses a token secret anyone may know or guess, which lets whoever does sign their own
    /// way into any call, unless the server is in `development`.
    pub fn check_secret(&self) -> anyhow::Result<()> {
        let Some(secret) = &self.token_secret else {
            return Ok(());
        };
        let weak = secret == DEVELOPMENT_TOKEN_SECRET || secret.len() < MIN_TOKEN_SECRET_BYTES;
        if weak && !self.development {
            anyhow::bail!(
                "token_secret is the development value or shorter than {MIN_TOKEN_SECRET_BYTES} \
                 bytes, so whoever knows or guesses it can join any call; leave it out once every \
                 API server signs join tokens with its key, give it a long random one meanwhile, or \
                 set development = true on a development machine"
            );
        }
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine with the families and primary addresses a test gives it.
    #[derive(Default)]
    struct Machine {
        v4: Option<&'static str>,
        v6: Option<&'static str>,
        /// Whether `::` can be bound; an IPv4 address always can.
        binds_v6: bool,
    }

    impl Interfaces for Machine {
        fn bind(&self, ip: IpAddr) -> std::io::Result<()> {
            if ip.is_ipv4() || self.binds_v6 {
                Ok(())
            } else {
                Err(std::io::ErrorKind::AddrNotAvailable.into())
            }
        }

        fn primary(&self, like: IpAddr) -> Result<IpAddr, String> {
            let primary = if like.is_ipv4() { self.v4 } else { self.v6 };
            primary
                .map(|ip| ip.parse().expect("an address"))
                .ok_or_else(|| "none".to_string())
        }
    }

    const DUAL: Machine = Machine {
        v4: Some("192.0.2.7"),
        v6: Some("2001:db8::7"),
        binds_v6: true,
    };
    const V4_ONLY: Machine = Machine {
        v4: Some("192.0.2.7"),
        v6: None,
        binds_v6: true,
    };

    fn rtc(toml: &str) -> RtcConfig {
        config::Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .build()
            .expect("settings")
            .try_deserialize()
            .expect("an [rtc] table")
    }

    fn resolved(toml: &str, machine: &Machine) -> Vec<(String, Option<String>)> {
        rtc(toml)
            .media_addresses_on(machine)
            .expect("media addresses")
            .into_iter()
            .map(|address| (address.ip.to_string(), address.announced))
            .collect()
    }

    fn pair(ip: &str, announced: &str) -> (String, Option<String>) {
        (ip.to_string(), Some(announced.to_string()))
    }

    #[test]
    fn defaults_take_both_families() {
        assert_eq!(
            resolved("", &DUAL),
            [pair("0.0.0.0", "192.0.2.7"), pair("::", "2001:db8::7")]
        );
    }

    #[test]
    fn defaults_leave_out_a_family_the_machine_lacks() {
        assert_eq!(resolved("", &V4_ONLY), [pair("0.0.0.0", "192.0.2.7")]);
        let unbindable = Machine {
            binds_v6: false,
            ..DUAL
        };
        assert_eq!(resolved("", &unbindable), [pair("0.0.0.0", "192.0.2.7")]);
        let v6_only = Machine { v4: None, ..DUAL };
        assert_eq!(resolved("", &v6_only), [pair("::", "2001:db8::7")]);
        assert!(rtc("").media_addresses_on(&Machine::default()).is_err());
    }

    #[test]
    fn a_named_family_must_be_there() {
        assert!(
            rtc(r#"ip = ["0.0.0.0", "::"]"#)
                .media_addresses_on(&V4_ONLY)
                .is_err()
        );
        let unbindable = Machine {
            binds_v6: false,
            ..DUAL
        };
        assert!(
            rtc(r#"announced_address = ["203.0.113.10", "2001:db8::10"]"#)
                .media_addresses_on(&unbindable)
                .is_err()
        );
    }

    #[test]
    fn announced_addresses_go_to_their_family() {
        assert_eq!(
            resolved(
                r#"announced_address = ["2001:db8::10", "203.0.113.10"]"#,
                &DUAL
            ),
            [pair("0.0.0.0", "203.0.113.10"), pair("::", "2001:db8::10")]
        );
        // One address, as a server set up for IPv4 alone has it: the other family announces
        // the machine's own, or is left out.
        assert_eq!(
            resolved(r#"announced_address = "203.0.113.10""#, &DUAL),
            [pair("0.0.0.0", "203.0.113.10"), pair("::", "2001:db8::7")]
        );
        assert_eq!(
            resolved(r#"announced_address = "203.0.113.10""#, &V4_ONLY),
            [pair("0.0.0.0", "203.0.113.10")]
        );
        assert_eq!(
            resolved(
                r#"announced_address = ["voice.example.org", "203.0.113.10"]"#,
                &DUAL
            ),
            [
                pair("0.0.0.0", "203.0.113.10"),
                pair("::", "voice.example.org")
            ]
        );
    }

    #[test]
    fn one_interface_is_one_family() {
        assert_eq!(
            resolved(r#"ip = "0.0.0.0""#, &DUAL),
            [pair("0.0.0.0", "192.0.2.7")]
        );
        assert_eq!(
            resolved(r#"ip = "2001:db8::7""#, &DUAL),
            [("2001:db8::7".to_string(), None)]
        );
        // The form an environment variable gives a list in.
        assert_eq!(
            resolved(r#"ip = "0.0.0.0, ::""#, &DUAL),
            [pair("0.0.0.0", "192.0.2.7"), pair("::", "2001:db8::7")]
        );
    }

    #[test]
    fn refuses_what_clients_cannot_send_to() {
        for toml in [
            r#"ip = "127.0.0.1""#,
            r#"ip = "::1""#,
            r#"announced_address = "127.0.0.1""#,
            r#"announced_address = "::""#,
            r#"ip = []"#,
            r#"ip = ["192.0.2.7", "192.0.2.8"]"#,
            r#"announced_address = ["203.0.113.10", "203.0.113.11"]"#,
            r#"announced_address = ["a.example.org", "b.example.org"]"#,
            // An IPv6 address to announce, and no IPv6 interface to receive on.
            "ip = \"0.0.0.0\"\nannounced_address = \"2001:db8::10\"",
        ] {
            assert!(rtc(toml).media_addresses_on(&DUAL).is_err(), "{toml}");
        }
    }
}
