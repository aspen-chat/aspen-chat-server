use serde::Deserialize;
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
    pub nats_auth_token: String,
    /// Where the HTTP server, health check, and signalling socket listen.
    #[serde(default = "default_listen_addr")]
    pub listen_addr: SocketAddr,
    #[serde(default)]
    pub rtc: RtcConfig,
    /// mediasoup workers, each a process carrying some of the calls. Defaults to the number
    /// of CPUs.
    #[serde(default = "default_workers")]
    pub workers: usize,
}

/// Where WebRTC media is received.
#[derive(Clone, Debug, Deserialize)]
pub struct RtcConfig {
    /// The interface media is bound to.
    #[serde(default = "default_rtc_ip")]
    pub ip: IpAddr,
    /// The address clients are told to send media to. Required in effect when `ip` is
    /// unspecified (`0.0.0.0`), since a client cannot send to that; left unset, the host's
    /// primary interface address is announced, which suits a development machine and a server
    /// with a public address on an interface. A server behind NAT sets its public address.
    /// It must never be a loopback address: Firefox does not pair its own host candidates
    /// with a loopback peer, so media never connects.
    #[serde(default)]
    pub announced_address: Option<String>,
    #[serde(default = "default_rtc_min_port")]
    pub min_port: u16,
    #[serde(default = "default_rtc_max_port")]
    pub max_port: u16,
}

impl Default for RtcConfig {
    fn default() -> Self {
        Self {
            ip: default_rtc_ip(),
            announced_address: None,
            min_port: default_rtc_min_port(),
            max_port: default_rtc_max_port(),
        }
    }
}

fn default_workers() -> usize {
    num_cpus::get().max(1)
}

fn default_listen_addr() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 9001)
}

fn default_rtc_ip() -> IpAddr {
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
}

fn default_rtc_min_port() -> u16 {
    40000
}

fn default_rtc_max_port() -> u16 {
    40999
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

pub fn load_config() -> Result<VoiceServerConfig, config::ConfigError> {
    config::Config::builder()
        .add_source(
            config::Environment::with_prefix("ASPEN_VOICE_SERVER")
                .prefix_separator("_")
                .separator("__"),
        )
        .add_source(
            config::File::new("voice_server.toml", config::FileFormat::Toml).required(false),
        )
        .build()?
        .try_deserialize()
}
