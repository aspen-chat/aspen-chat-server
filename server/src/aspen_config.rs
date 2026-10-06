pub use aspen_limits::{Limit, LimitSetting, RuleTable};
use serde::Deserialize;
use smart_default::SmartDefault;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

#[derive(Clone, Debug, Deserialize)]
pub struct AspenConfig {
    /// The one address of this deployment, such as `https://chat.example.org`: the API, the web
    /// client this server serves, and the pages it opens are all at this origin. Every link to
    /// the deployment is built on it (invites, sign-in codes, mail), and the rest of what names
    /// the deployment follows from it as the config loads
    /// ([`AspenConfig::derive_from_public_url`]): passkeys belong to its host, and over `https`
    /// its host is the federation domain.
    pub public_url: String,
    #[serde(default = "default_event_queue_size")]
    pub event_queue_size: usize,
    /// Tasks that route events to this server's event stream connections, one per logical CPU
    /// by default (`app::event_feed`).
    #[serde(default = "default_event_feed_shards")]
    pub event_feed_shards: usize,
    pub database_url: String,
    /// The most database connections this server holds at once; left out, two per logical CPU. Every
    /// write holds one until its event is acknowledged, so a busy server may want more, within
    /// what PostgreSQL's `max_connections` allows for every server together.
    pub database_pool_size: Option<usize>,
    pub nats_url: String,
    pub nats_auth_token: String,
    pub valkey_url: String,
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub voice: VoiceConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub presence: PresenceConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub federation: FederationConfig,
    #[serde(default)]
    pub push: PushConfig,
    #[serde(default)]
    pub web_client: WebClientConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
    /// Sending mail (`app::email`); left out, the deployment sends none, and its administrators
    /// cannot require or offer what needs it.
    pub email: Option<EmailConfig>,
    /// What `aspen.toml` says about rate limits; `rate_limits` is the result.
    #[serde(default, rename = "rate_limits")]
    pub rate_limit_overrides: RateLimitOverrides,
    /// The rate limits in force: the built-in ones with `rate_limit_overrides` laid over them.
    #[serde(skip)]
    pub rate_limits: RateLimitConfig,
}

/// Rate limits (`app::rate_limit`), as resolved from the built-in `rate_limits.toml` and the
/// `[rate_limits]` of `aspen.toml` (see `RateLimitOverrides`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RateLimitConfig {
    pub enabled: bool,
    /// Reverse proxies whose `X-Forwarded-For` is believed: addresses or CIDR networks.
    #[serde(default)]
    pub trusted_proxies: Vec<String>,
    /// An IPv6 client is counted by its network of this many leading bits, since one
    /// subscriber usually holds a whole /64.
    pub ipv6_prefix: u8,
    /// The longest a suspension of the limits is honoured, from when it started.
    pub max_suspension_seconds: u64,
    /// Limits every endpoint has, in addition to its groups' and its own.
    #[serde(default)]
    pub default: RuleTable,
    /// Named families of endpoints sharing limits.
    #[serde(default)]
    pub groups: BTreeMap<String, RateLimitGroup>,
    /// Limits of single endpoints, keyed by method and path template (`"POST
    /// /channels/{channel}/messages"`).
    #[serde(default)]
    pub endpoints: HashMap<String, RuleTable>,
}

/// The `[rate_limits]` of `aspen.toml`, laid over the built-in limits. Each limit given
/// replaces the built-in one for the same place and dimension whole, so a limit written without
/// `burst` has the default burst rather than the built-in one's; everything not given keeps its
/// built-in value. A group given here with a name the built-ins use keeps their endpoint list
/// unless it lists its own.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RateLimitOverrides {
    pub enabled: Option<bool>,
    pub trusted_proxies: Option<Vec<String>>,
    pub ipv6_prefix: Option<u8>,
    pub max_suspension_seconds: Option<u64>,
    #[serde(default)]
    pub default: RuleTable,
    #[serde(default)]
    pub groups: BTreeMap<String, RateLimitGroupOverride>,
    #[serde(default)]
    pub endpoints: HashMap<String, RuleTable>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RateLimitGroupOverride {
    pub endpoints: Option<Vec<String>>,
    #[serde(default)]
    pub limits: RuleTable,
}

impl RateLimitConfig {
    /// Lays `overrides` over these limits.
    pub fn overlay(mut self, overrides: RateLimitOverrides) -> Result<Self, config::ConfigError> {
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
        self.default.extend(overrides.default);
        for (name, group) in overrides.groups {
            match self.groups.get_mut(&name) {
                Some(existing) => {
                    if let Some(endpoints) = group.endpoints {
                        existing.endpoints = endpoints;
                    }
                    existing.limits.extend(group.limits);
                }
                None => {
                    let endpoints = group.endpoints.ok_or_else(|| {
                        config::ConfigError::Message(format!(
                            "rate_limits.groups.{name} is a new group and must list its endpoints"
                        ))
                    })?;
                    self.groups.insert(
                        name,
                        RateLimitGroup {
                            endpoints,
                            limits: group.limits,
                        },
                    );
                }
            }
        }
        aspen_limits::overlay_tables(&mut self.endpoints, overrides.endpoints);
        Ok(self)
    }

    /// The built-in limits (`rate_limits.toml`).
    pub fn built_in() -> Result<Self, config::ConfigError> {
        config::Config::builder()
            .add_source(config::File::from_str(
                DEFAULT_RATE_LIMITS,
                config::FileFormat::Toml,
            ))
            .build()?
            .get("rate_limits")
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct RateLimitGroup {
    /// Endpoint names; `*` in one matches any run of characters.
    pub endpoints: Vec<String>,
    pub limits: RuleTable,
}

/// Prometheus metrics (`aspen_metrics`), served on a listener of their own.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct MetricsConfig {
    #[default = true]
    pub enabled: bool,
    /// Where `GET /metrics` is served. Loopback by default: the figures describe the
    /// deployment's inside, so expose them only to whatever scrapes them.
    #[default(std::net::SocketAddr::from(([127, 0, 0, 1], 9464)))]
    pub listen_addr: std::net::SocketAddr,
}

/// Whether people show as online, away, or offline (`app::user_status`).
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct PresenceConfig {
    /// A connected user who has not used Aspen for this long shows as away; ten minutes by
    /// default. Clients report activity at most once a minute, so values much under a few
    /// minutes make people flicker between away and online.
    #[default = 600]
    pub away_after_seconds: u64,
}

/// Sign-in: second factors, passkeys, and how recent a verification must be.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct AuthConfig {
    /// A change to security settings needs the session to have proved who its user is within
    /// this many seconds; ten minutes by default.
    #[default = 600]
    pub reverify_seconds: u64,
    /// The domain passkeys belong to, from `public_url`: its host, where a browser lets a page
    /// use passkeys (over `https`, or at `localhost` and names under it), and `None` elsewhere,
    /// where passkeys are not offered. Every page that runs a ceremony is at `public_url`, so it
    /// is the one origin allowed to. Changing the host orphans every passkey registered; over
    /// `https` it also changes the federation domain, which `app::deployment_settings::pin_domain`
    /// refuses.
    #[serde(skip)]
    pub rp_id: Option<String>,
    /// Threads password hashing and checking may use at once, one per logical CPU by default.
    /// Each Argon2 hash holds 19 MiB while it runs.
    #[default(crate::app::login::default_password_hashing_threads())]
    pub password_hashing_threads: usize,
    /// How long a sign-in or other password check waits for a thread before it is refused
    /// with `serverBusy`.
    #[default = 10]
    pub password_hashing_wait_seconds: u64,
}

/// Ceilings that keep one user's footprint bounded.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct LimitsConfig {
    /// The most communities one user may belong to. It bounds how many subjects an event
    /// stream connection reads and how many copies of a profile change are published.
    #[default = 500]
    pub max_communities_per_user: u32,
}

/// Federation: this deployment's name among deployments and how it checks on the users of
/// others (`app::federation`). Who may cross is the gates, which are deployment settings
/// (`app::deployment_settings`), changed from the dashboard or the terminal.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default, deny_unknown_fields)]
pub struct FederationConfig {
    /// This deployment's name among deployments, from `public_url`: its host, with `:port` when
    /// that is not 443, such as `chat.example.org`, when it is `https`, and `None` over `http`,
    /// which other deployments do not call, so no gate opens. Other deployments pin the key they
    /// find at this name, so once a server has started with it, it may not change
    /// (`app::deployment_settings::pin_domain`).
    #[serde(skip)]
    pub domain: Option<String>,
    /// How often this deployment asks the homes of the users from elsewhere signed in here
    /// whether they are still in good standing there (`app::federation::standing`): an hour.
    #[default = 3600]
    pub standing_interval_seconds: u64,
    /// How long a home may go unreached before its users' sessions here end: a day.
    #[default = 86400]
    pub standing_grace_seconds: u64,
    /// Settings for trying federation on one machine; a deployment others use leaves them out.
    pub development: FederationDevelopment,
}

/// Settings for running deployments side by side on one machine.
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct FederationDevelopment {
    /// PEM files of certificate authorities trusted, besides the system's, when this server
    /// calls other deployments: a development authority that signed certificates for names
    /// such as `beta.localhost`.
    pub extra_root_certificates: Vec<std::path::PathBuf>,
    /// Lets this server call deployments at loopback and private network addresses, which it
    /// otherwise refuses so that naming a deployment cannot make it reach inside its own
    /// network.
    pub allow_private_addresses: bool,
}

/// Waking phones that are not running Aspen (`app::push`, `spec/push.md`).
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct PushConfig {
    /// Whether apps may ask to be woken, and messages wake them.
    #[default = true]
    pub enabled: bool,
}

/// How plugins run (`app::plugin`): how long each call may take and how much memory it may use.
/// Which plugins are installed, and their settings, are in the database.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct PluginsConfig {
    /// How long a plugin has to decide a message about to be saved, which its author waits on.
    #[default = 25]
    pub intercept_millis: u64,
    /// How long a plugin has to handle something it observes.
    #[default = 10_000]
    pub observe_millis: u64,
    /// How long a plugin has to answer a request to one of its routes.
    #[default = 3_000]
    pub route_millis: u64,
    /// The most memory one call of a plugin may use, in MiB.
    #[default = 64]
    pub memory_mib: u64,
}

/// Voice calls. The voice servers themselves are rows of `voice_server`, added from the
/// dashboard or the terminal (`aspen-chat-server voice-servers`).
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct VoiceConfig {
    /// Shared with every voice server; signs the join tokens they verify.
    #[default = "aspen_dev_voice_secret"]
    pub token_secret: String,
    /// Distinct users whose session creation failed within `failure_window_seconds` before a
    /// server is disabled.
    #[default = 5]
    pub failure_threshold: u32,
    #[default = 3600]
    pub failure_window_seconds: u64,
    /// How long a join token stays valid: long enough to try every candidate server.
    #[default = 60]
    pub join_token_ttl_seconds: u64,
    /// The most candidate servers one join offer names.
    #[default = 10]
    pub candidate_limit: usize,
    /// A voice server silent for this long is not offered to anyone joining a call: it is
    /// probably down, and offering it would cost clients failed attempts and count against it.
    #[default = 60]
    pub offer_silence_seconds: u64,
    /// A voice server silent for this long (a day by default) has its sessions ended, so a
    /// server that died leaves no phantom calls. Long, because a call outliving a brief outage
    /// of the report link is worth more than ending it early.
    #[default(24 * 60 * 60)]
    pub session_silence_seconds: u64,
    /// A call that has gone this long (a day by default) without ever holding two people at
    /// once is ended and its lone participant told why, so a forgotten client cannot hold a
    /// voice server slot indefinitely.
    #[default(24 * 60 * 60)]
    pub idle_session_seconds: u64,
}

/// The web client this server serves at `public_url` (`api::web_client`).
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default, deny_unknown_fields)]
pub struct WebClientConfig {
    /// The built web client (`pnpm build` in `client/` writes `client/packages/app/dist`, the
    /// default, relative to where the server runs). The server will not start without it. Its
    /// files are read as they are asked for, so a new release is served once it is in place.
    #[default(PathBuf::from("client/packages/app/dist"))]
    pub dir: PathBuf,
}

/// Sending mail (`app::email`): verification and password reset codes, the daily digest, and the
/// newsletter.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmailConfig {
    /// The SMTP server mail is handed to, with its credentials: `smtps://user:password@host`
    /// (TLS from the start, port 465), `smtp://user:password@host?tls=required` (STARTTLS, port
    /// 587), or `smtp://host:1025` (no encryption, for a development mail catcher). Characters
    /// in the user and password that a URL reserves are percent-encoded. Required where `send`
    /// is on; a server that sends nothing needs no credentials.
    pub smtp_url: Option<String>,
    /// Whether this server sends mail and makes digests. Every server with `[email]` takes
    /// addresses and queues mail; turned off on some, the others send it, so the SMTP
    /// credentials and the work of sending stay on servers chosen for it. At least one server
    /// of the deployment must send.
    #[serde(default = "EmailConfig::default_send")]
    pub send: bool,
    /// The most mail the whole deployment hands to the SMTP server each second, however many
    /// servers send, as the provider's quota allows; left out, no limit. Counted in Valkey.
    pub max_per_second: Option<u32>,
    /// Who mail comes from, such as `Aspen <noreply@chat.example.org>`.
    pub from: String,
}

impl EmailConfig {
    fn default_send() -> bool {
        true
    }

    /// Checks `from`, `smtp_url`, and `max_per_second`.
    fn validate(&self) -> Result<(), config::ConfigError> {
        let message = |text: String| config::ConfigError::Message(text);
        if self.send && self.smtp_url.is_none() {
            return Err(message(
                "[email] needs smtp_url on a server that sends mail; set send = false on a \
                 server that only queues it"
                    .to_string(),
            ));
        }
        if self.max_per_second == Some(0) {
            return Err(message(
                "email.max_per_second must be at least 1; leave it out for no limit".to_string(),
            ));
        }
        self.from.parse::<lettre::message::Mailbox>().map_err(|e| {
            message(format!(
                "email.from {:?} is not an address like `Aspen <noreply@example.org>`: {e}",
                self.from
            ))
        })?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default)]
pub struct MediaConfig {
    pub s3: MediaS3Config,
}

/// Object-storage configuration.
///
/// `endpoint` is the authenticated S3 API the server talks to (PUTs preview
/// images, deletes objects, checks uploads). `public_endpoint`, when set, is the
/// same API as clients reach it, and is the host the presigned upload URLs they
/// are handed name; without it they name `endpoint`, which is right only when
/// clients reach storage at the same address the server does. `public_base_url` is what
/// clients see in `downloadUrl` fields and is expected to be served by an
/// operator-configured anonymous read path (e.g. Garage's `s3_web` website
/// endpoint, or an AWS bucket with `BlockPublicAccess=false` plus a
/// `s3:GetObject` allow-all policy). The two URLs may point at completely
/// different hosts; the public path does not need to be reachable from the
/// server itself.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct MediaS3Config {
    #[default = "http://127.0.0.1:3900"]
    pub endpoint: String,
    pub public_endpoint: Option<String>,
    #[default = "garage"]
    pub region: String,
    #[default = "aspen-media"]
    pub bucket: String,
    #[default = "aspen_dev_key"]
    pub access_key: String,
    #[default = "aspen_dev_secret"]
    pub secret_key: String,
    /// By default the local Garage `s3_web` endpoint, exposed on port 3902 by
    /// `docker-compose.yaml`. Operators deploying against AWS S3 should override this to a
    /// CloudFront or custom-domain URL pointed at the public-read bucket.
    #[default = "http://127.0.0.1:3902/aspen-media"]
    pub public_base_url: String,
    /// Fifteen minutes by default: long enough for a multi-minute upload from a slow mobile
    /// connection while keeping a stale URL useless to anyone who fishes it out of a log file
    /// later.
    #[default = 900]
    pub upload_url_ttl_seconds: u64,
}

pub fn default_event_feed_shards() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

pub fn default_event_queue_size() -> usize {
    512
}

/// The built-in rate limits, beneath whatever `aspen.toml` sets.
const DEFAULT_RATE_LIMITS: &str = include_str!("rate_limits.toml");

/// Loads or reloads the config.
pub fn load_config() -> Result<AspenConfig, config::ConfigError> {
    let mut loaded = config::Config::builder()
        .add_source(config::File::new("aspen.toml", config::FileFormat::Toml))
        // Sources added later take precedence, so the environment overrides `aspen.toml`.
        // `ASPEN_DATABASE_URL` sets `database_url`; `ASPEN_VOICE__IDLE_SESSION_SECONDS` sets
        // `voice.idle_session_seconds`. The prefix separator is set explicitly because it would
        // otherwise follow the nesting separator and every flat key would need two underscores.
        .add_source(
            config::Environment::with_prefix("ASPEN")
                .prefix_separator("_")
                .separator("__"),
        )
        .build()?
        .try_deserialize::<AspenConfig>()?;
    // Merged here rather than as one more config source, which would merge a limit given in
    // `aspen.toml` into the built-in one field by field.
    loaded.rate_limits =
        RateLimitConfig::built_in()?.overlay(std::mem::take(&mut loaded.rate_limit_overrides))?;
    loaded.derive_from_public_url()?;
    if let Some(email) = &loaded.email {
        email.validate()?;
    }
    Ok(loaded)
}

impl AspenConfig {
    /// Checks `public_url`, an `http` or `https` origin alone, and fills in what follows from it:
    /// the federation domain and the passkeys' domain. It is kept as its origin, with no
    /// trailing slash.
    fn derive_from_public_url(&mut self) -> Result<(), config::ConfigError> {
        let given = &self.public_url;
        let invalid =
            |why: &str| config::ConfigError::Message(format!("public_url {given:?} {why}"));
        let parsed = url::Url::parse(given)
            .map_err(|_| invalid("is not an address such as https://chat.example.org"))?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(invalid("must be an http or https address"));
        }
        if parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(invalid(
                "must be an origin alone, such as https://chat.example.org: the web client is \
                 served at its root",
            ));
        }
        let https = parsed.scheme() == "https";
        self.federation.domain = match (https, parsed.host_str()) {
            (true, Some(host)) => {
                let authority = match parsed.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_string(),
                };
                let domain = crate::app::federation::Domain::parse(&authority).map_err(|_| {
                    invalid(
                        "names no domain, which an https deployment needs as its federation \
                         domain",
                    )
                })?;
                Some(String::from(domain))
            }
            _ => None,
        };
        self.auth.rp_id = match parsed.host() {
            Some(url::Host::Domain(host))
                if https || host == "localhost" || host.ends_with(".localhost") =>
            {
                Some(host.to_string())
            }
            _ => None,
        };
        self.public_url = parsed.origin().ascii_serialization();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type Derived = (String, Option<String>, Option<String>);

    fn derived(public_url: &str) -> Result<Derived, String> {
        let mut config: AspenConfig = config::Config::builder()
            .add_source(config::File::from_str(
                &format!(
                    "public_url = {public_url:?}\ndatabase_url = \"postgres://x\"\n\
                     nats_url = \"nats://x\"\nnats_auth_token = \"t\"\nvalkey_url = \"redis://x\""
                ),
                config::FileFormat::Toml,
            ))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        config.derive_from_public_url().map_err(|e| e.to_string())?;
        Ok((
            config.public_url,
            config.federation.domain,
            config.auth.rp_id,
        ))
    }

    /// Everything that names the deployment follows from its one address.
    #[test]
    fn the_public_url_names_the_deployment_everywhere() {
        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            derived("https://Chat.Example.org/").unwrap(),
            (
                "https://chat.example.org".into(),
                some("chat.example.org"),
                some("chat.example.org")
            )
        );
        assert_eq!(
            derived("https://alpha.localhost:8443").unwrap(),
            (
                "https://alpha.localhost:8443".into(),
                some("alpha.localhost:8443"),
                some("alpha.localhost")
            )
        );
        assert_eq!(
            derived("http://localhost:5173").unwrap(),
            ("http://localhost:5173".into(), None, some("localhost"))
        );
        assert_eq!(
            derived("http://192.168.2.220:8000").unwrap(),
            ("http://192.168.2.220:8000".into(), None, None)
        );
        assert!(derived("https://example.org/aspen").is_err());
        assert!(derived("https://example.org/?x=1").is_err());
        assert!(derived("https://user@example.org").is_err());
        assert!(derived("aspen://app").is_err());
        assert!(derived("chat.example.org").is_err());
    }

    /// A section given in part keeps the defaults of what it leaves out, and a section left out
    /// is all defaults.
    #[test]
    fn missing_settings_take_their_defaults() {
        let config: AspenConfig = config::Config::builder()
            .add_source(config::File::from_str(
                r#"
                public_url = "https://chat.example.org"
                database_url = "postgres://x"
                nats_url = "nats://x"
                nats_auth_token = "t"
                valkey_url = "redis://x"
                [voice]
                failure_threshold = 7
                [media.s3]
                bucket = "elsewhere"
                "#,
                config::FileFormat::Toml,
            ))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(config.voice.failure_threshold, 7);
        assert_eq!(config.voice.idle_session_seconds, 24 * 60 * 60);
        assert_eq!(config.media.s3.bucket, "elsewhere");
        assert_eq!(config.media.s3.upload_url_ttl_seconds, 900);
        assert_eq!(config.presence.away_after_seconds, 600);
        assert_eq!(config.limits.max_communities_per_user, 500);
        assert!(config.metrics.enabled);
        assert_eq!(config.event_queue_size, 512);
        assert_eq!(
            config.voice.token_secret,
            VoiceConfig::default().token_secret
        );
    }

    /// A server that sends needs an SMTP server; one that only queues does not.
    #[test]
    fn only_a_sending_server_needs_smtp() {
        let config = |toml: &str| -> Result<EmailConfig, String> {
            let config: EmailConfig = config::Config::builder()
                .add_source(config::File::from_str(toml, config::FileFormat::Toml))
                .build()
                .and_then(|built| built.try_deserialize())
                .map_err(|e| e.to_string())?;
            config.validate().map_err(|e| e.to_string())?;
            Ok(config)
        };
        assert!(config("from = \"a@example.org\"").is_err());
        let queuing = config("from = \"a@example.org\"\nsend = false").unwrap();
        assert!(!queuing.send && queuing.smtp_url.is_none());
        let sending = config("from = \"a@example.org\"\nsmtp_url = \"smtp://x\"").unwrap();
        assert!(sending.send && sending.max_per_second.is_none());
        assert!(
            config("from = \"a@example.org\"\nsmtp_url = \"smtp://x\"\nmax_per_second = 0")
                .is_err()
        );
    }

    /// The domain is the public URL's, and the gates are not set here.
    #[test]
    fn federation_settings_are_checked() {
        let federation = |toml: &str| {
            config::Config::builder()
                .add_source(config::File::from_str(toml, config::FileFormat::Toml))
                .build()
                .and_then(|built| built.try_deserialize::<FederationConfig>())
        };
        assert!(federation("standing_interval_seconds = 60").is_ok());
        assert!(federation("domain = \"chat.example.org\"").is_err());
        assert!(federation("[users]\nemigration = \"open\"").is_err());
    }
}
