//! `aspen-ul.toml`, in the data directory: what an operator says about their deployment. Every
//! other setting the API and voice servers need, UL derives from it and from the machine, and
//! `[server]` is laid over what it derives for the rest of `aspen.toml`'s settings.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// The settings UL derives, which `[server]` may not give.
const DERIVED: &[&str] = &[
    "public_url",
    "database_url",
    "database_pool_size",
    "nats_url",
    "nats_auth_token",
    "nats_user",
    "nats",
    "valkey_url",
    "valkey",
    "web_client",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The deployment's one address, as `aspen.toml`'s `public_url`.
    pub public_url: String,
    /// Where the deployment's files are served: an origin of their own, `media.` and
    /// `public_url`'s host by default, at its scheme and port.
    pub media_url: Option<String>,
    /// The addresses listened on, every interface over IPv4 and IPv6 when not given.
    #[serde(default)]
    pub listen: Vec<IpAddr>,
    /// The port the deployment is served at.
    #[serde(default = "default_port")]
    pub port: u16,
    pub tls: Tls,
    /// The memory this deployment may use, in MiB, which sizes PostgreSQL, the database pool, and
    /// what the bus and short-lived store hold. The machine's (or its cgroup's) when not given.
    pub memory_mib: Option<u64>,
    #[serde(default)]
    pub voice: Voice,
    /// Any other setting of `aspen.toml`, as it would be written there: `[email]`,
    /// `[rate_limits]`, `[media.previews]`, and the rest.
    #[serde(default)]
    pub server: Option<config::Value>,
}

fn default_port() -> u16 {
    443
}

#[derive(Debug, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
pub enum Tls {
    /// Plain HTTP, for a proxy on this machine that terminates TLS, or a deployment at
    /// `localhost`.
    Off,
    /// A certificate and key in files, PEM or DER, for every name the deployment answers.
    Files { cert: PathBuf, key: PathBuf },
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Voice {
    /// The port calls' media arrive at, over UDP and TCP.
    pub media_port: u16,
    /// The port of STUN and TURN for file transfers, over UDP and TCP.
    pub transfer_port: u16,
    /// The first and last ports relayed file transfers take.
    pub relay_ports: (u16, u16),
    /// The addresses clients send media to, when the one this machine reaches the internet from
    /// is not it (behind NAT, it is the router's public address).
    pub announced_address: Vec<String>,
    /// The most people in calls at once.
    pub capacity: i32,
}

impl Default for Voice {
    fn default() -> Self {
        Self {
            media_port: 40000,
            transfer_port: 3478,
            relay_ports: (40100, 40199),
            announced_address: Vec::new(),
            capacity: 100,
        }
    }
}

impl Config {
    pub fn read(path: &Path) -> Result<Self> {
        let config: Self = config::Config::builder()
            .add_source(config::File::from(path).format(config::FileFormat::Toml))
            .build()
            .and_then(config::Config::try_deserialize)
            .with_context(|| format!("could not read {}", path.display()))?;
        if let Some(server) = &config.server {
            let table = server
                .clone()
                .into_table()
                .context("[server] is a table of aspen.toml's settings")?;
            if let Some(derived) = DERIVED.iter().find(|key| table.contains_key(**key)) {
                bail!(
                    "[server] gives {derived}, which aspen-ul sets itself; give the deployment's \
                     address as public_url at the top of {}",
                    path.display()
                );
            }
            if table
                .get("media")
                .and_then(|media| media.clone().into_table().ok())
                .is_some_and(|media| media.contains_key("directory") || media.contains_key("s3"))
            {
                bail!(
                    "[server.media] gives where files are kept, which aspen-ul sets itself; give \
                     media_url at the top of {}",
                    path.display()
                );
            }
        }
        Ok(config)
    }

    /// `media_url`, or `media.` and `public_url`'s host at its scheme and port.
    pub fn media_url(&self) -> Result<String> {
        if let Some(media) = &self.media_url {
            return Ok(media.trim_end_matches('/').to_string());
        }
        let mut url = url::Url::parse(&self.public_url)
            .with_context(|| format!("public_url {:?} is not an address", self.public_url))?;
        let host = url
            .host_str()
            .context("public_url names no host")?
            .to_string();
        url.set_host(Some(&format!("media.{host}")))
            .context("media.<public_url's host> is not a host name; give media_url")?;
        Ok(url.as_str().trim_end_matches('/').to_string())
    }

    /// `[server]`, as a source of `aspen.toml`'s settings, laid over what UL derives.
    pub fn server_settings(&self) -> Result<ServerSettings> {
        Ok(ServerSettings(match &self.server {
            None => config::Map::new(),
            Some(server) => server.clone().into_table()?,
        }))
    }
}

/// `[server]`'s settings, as one more source of the API server's configuration.
#[derive(Clone, Debug)]
pub struct ServerSettings(config::Map<String, config::Value>);

impl config::Source for ServerSettings {
    fn clone_into_box(&self) -> Box<dyn config::Source + Send + Sync> {
        Box::new(self.clone())
    }

    fn collect(&self) -> Result<config::Map<String, config::Value>, config::ConfigError> {
        Ok(self.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<Config> {
        let dir = std::env::temp_dir().join(format!("aspen-ul-config-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("aspen-ul.toml");
        std::fs::write(&path, text).unwrap();
        let read = Config::read(&path);
        std::fs::remove_dir_all(&dir).unwrap();
        read
    }

    #[test]
    fn the_media_origin_is_its_own_host_unless_given() {
        let config =
            read("public_url = \"https://chat.example.org\"\n[tls]\nmode = \"off\"\n").unwrap();
        assert_eq!(
            config.media_url().unwrap(),
            "https://media.chat.example.org"
        );
        let config = read(
            "public_url = \"http://localhost:8800\"\nmedia_url = \"http://files.localhost:8800/\"\n[tls]\nmode = \"off\"\n",
        )
        .unwrap();
        assert_eq!(config.media_url().unwrap(), "http://files.localhost:8800");
        let config =
            read("public_url = \"http://localhost:8800\"\n[tls]\nmode = \"off\"\n").unwrap();
        assert_eq!(config.media_url().unwrap(), "http://media.localhost:8800");
    }

    #[test]
    fn server_settings_pass_through_and_derived_ones_are_refused() {
        use config::Source;
        let config = read(
            "public_url = \"https://chat.example.org\"\n[tls]\nmode = \"off\"\n\
             [server]\nevent_queue_size = 256\n[server.email]\nfrom = \"a@example.org\"\n",
        )
        .unwrap();
        let settings = config.server_settings().unwrap().collect().unwrap();
        assert_eq!(
            settings["event_queue_size"].clone().into_int().unwrap(),
            256
        );
        let email = settings["email"].clone().into_table().unwrap();
        assert_eq!(
            email["from"].clone().into_string().unwrap(),
            "a@example.org"
        );
        for derived in [
            "database_url = \"x\"",
            "nats_url = \"x\"",
            "[server.web_client]\ndir = \"x\"",
            "[server.media.directory]\npath = \"x\"",
        ] {
            let text = format!(
                "public_url = \"https://chat.example.org\"\n[tls]\nmode = \"off\"\n[server]\n{derived}\n"
            );
            assert!(read(&text).is_err(), "{derived}");
        }
        assert!(
            read("public_url = \"https://chat.example.org\"\n").is_err(),
            "tls must be said"
        );
    }
}
