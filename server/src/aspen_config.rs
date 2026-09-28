pub use aspen_limits::{Limit, LimitSetting, RuleTable};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug, Deserialize)]
pub struct AspenConfig {
    #[serde(default = "default_event_queue_size")]
    pub event_queue_size: usize,
    /// Tasks that route events to this server's event stream connections, one per logical CPU
    /// by default (`app::event_feed`).
    #[serde(default = "default_event_feed_shards")]
    pub event_feed_shards: usize,
    pub database_url: String,
    pub nats_url: String,
    pub nats_auth_token: String,
    pub valkey_url: String,
    #[serde(default)]
    pub media: MediaConfig,
    #[serde(default)]
    pub cors: CorsConfig,
    #[serde(default)]
    pub voice: VoiceConfig,
    #[serde(default)]
    pub limits: LimitsConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub presence: PresenceConfig,
    #[serde(default)]
    pub registration: RegistrationConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
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
#[derive(Clone, Debug, Deserialize)]
pub struct MetricsConfig {
    #[serde(default = "default_metrics_enabled")]
    pub enabled: bool,
    /// Where `GET /metrics` is served. Loopback by default: the figures describe the
    /// deployment's inside, so expose them only to whatever scrapes them.
    #[serde(default = "default_metrics_listen_addr")]
    pub listen_addr: std::net::SocketAddr,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            enabled: default_metrics_enabled(),
            listen_addr: default_metrics_listen_addr(),
        }
    }
}

fn default_metrics_enabled() -> bool {
    true
}

fn default_metrics_listen_addr() -> std::net::SocketAddr {
    std::net::SocketAddr::from(([127, 0, 0, 1], 9464))
}

/// Who may create an account (`app::registration_invite`).
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RegistrationConfig {
    /// Whether creating an account takes an invite from the deployment's administrators. Off,
    /// anyone who reaches the server may register. On, the first account's invite is made
    /// from the terminal (`aspen-chat-server invites create`).
    #[serde(default)]
    pub invite_required: bool,
}

/// Whether people show as online, away, or offline (`app::user_status`).
#[derive(Clone, Debug, Deserialize)]
pub struct PresenceConfig {
    /// A connected user who has not used Aspen for this long shows as away. Clients report
    /// activity at most once a minute, so values much under a few minutes make people flicker
    /// between away and online.
    #[serde(default = "default_presence_away_after_seconds")]
    pub away_after_seconds: u64,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            away_after_seconds: default_presence_away_after_seconds(),
        }
    }
}

/// Ten minutes.
fn default_presence_away_after_seconds() -> u64 {
    600
}

/// Sign-in: second factors, passkeys, and how recent a verification must be.
#[derive(Clone, Debug, Deserialize)]
pub struct AuthConfig {
    /// Every account must have a second factor. A session of an account without one can only
    /// add one (or sign out) until it does.
    #[serde(default)]
    pub require_two_factor: bool,
    /// How the server names itself to authenticators: the label beside an authenticator app's
    /// codes and the name a passkey prompt shows.
    #[serde(default = "default_auth_service_name")]
    pub service_name: String,
    /// A change to security settings needs the session to have proved who its user is within
    /// this many seconds.
    #[serde(default = "default_auth_reverify_seconds")]
    pub reverify_seconds: u64,
    /// Passkeys are offered only when this is set.
    #[serde(default)]
    pub passkeys: Option<PasskeyConfig>,
    /// Threads password hashing and checking may use at once, one per logical CPU by default.
    /// Each Argon2 hash holds 19 MiB while it runs.
    #[serde(default = "crate::app::login::default_password_hashing_threads")]
    pub password_hashing_threads: usize,
    /// How long a sign-in or other password check waits for a thread before it is refused
    /// with `serverBusy`.
    #[serde(default = "default_password_hashing_wait_seconds")]
    pub password_hashing_wait_seconds: u64,
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            require_two_factor: false,
            service_name: default_auth_service_name(),
            reverify_seconds: default_auth_reverify_seconds(),
            passkeys: None,
            password_hashing_threads: crate::app::login::default_password_hashing_threads(),
            password_hashing_wait_seconds: default_password_hashing_wait_seconds(),
        }
    }
}

fn default_password_hashing_wait_seconds() -> u64 {
    10
}

/// WebAuthn relying party settings.
///
/// A passkey belongs to one domain, `rp_id`, and a browser offers it only to pages whose host is
/// that domain or under it. `origins` lists every page origin allowed to complete a passkey
/// ceremony: this server's own public origin, which serves the page the desktop and mobile
/// apps open in the system browser, and any web client origin under `rp_id` (such as
/// `https://chat.example.org` for `rp_id = "chat.example.org"`). Changing `rp_id` orphans every
/// passkey already registered.
#[derive(Clone, Debug, Deserialize)]
pub struct PasskeyConfig {
    pub rp_id: String,
    pub origins: Vec<String>,
}

fn default_auth_service_name() -> String {
    "Aspen".to_string()
}

/// Ten minutes.
fn default_auth_reverify_seconds() -> u64 {
    600
}

/// Ceilings that keep one user's footprint bounded.
#[derive(Clone, Debug, Deserialize)]
pub struct LimitsConfig {
    /// The most communities one user may belong to. It bounds how many subjects an event
    /// stream connection reads and how many copies of a profile change are published.
    #[serde(default = "default_max_communities_per_user")]
    pub max_communities_per_user: u32,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_communities_per_user: default_max_communities_per_user(),
        }
    }
}

fn default_max_communities_per_user() -> u32 {
    500
}

/// Voice calls. The servers listed here are seeded into the `voice_server` table at startup,
/// matched by name, and can then be managed through the `/voice-servers` endpoints.
#[derive(Clone, Debug, Deserialize)]
pub struct VoiceConfig {
    /// Shared with every voice server; signs the join tokens they verify.
    #[serde(default = "default_voice_token_secret")]
    pub token_secret: String,
    /// Distinct users whose session creation failed within `failure_window_seconds` before a
    /// server is disabled.
    #[serde(default = "default_voice_failure_threshold")]
    pub failure_threshold: u32,
    #[serde(default = "default_voice_failure_window_seconds")]
    pub failure_window_seconds: u64,
    /// How long a join token stays valid: long enough to try every candidate server.
    #[serde(default = "default_voice_join_token_ttl_seconds")]
    pub join_token_ttl_seconds: u64,
    /// The most candidate servers one join offer names.
    #[serde(default = "default_voice_candidate_limit")]
    pub candidate_limit: usize,
    /// A voice server silent for this long is not offered to anyone joining a call: it is
    /// probably down, and offering it would cost clients failed attempts and count against it.
    #[serde(default = "default_voice_offer_silence_seconds")]
    pub offer_silence_seconds: u64,
    /// A voice server silent for this long has its sessions ended, so a server that died
    /// leaves no phantom calls. Long, because a call outliving a brief outage of the report
    /// link is worth more than ending it early.
    #[serde(default = "default_voice_session_silence_seconds")]
    pub session_silence_seconds: u64,
    /// A call that has gone this long without ever holding two people at once is ended and
    /// its lone participant told why, so a forgotten client cannot hold a voice server slot
    /// indefinitely.
    #[serde(default = "default_voice_idle_session_seconds")]
    pub idle_session_seconds: u64,
    #[serde(default)]
    pub servers: Vec<VoiceServerSeed>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct VoiceServerSeed {
    pub name: String,
    /// The base URL clients open for signalling and measure latency against.
    pub url: String,
    /// The most participants the server carries at once.
    pub capacity: u32,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            token_secret: default_voice_token_secret(),
            failure_threshold: default_voice_failure_threshold(),
            failure_window_seconds: default_voice_failure_window_seconds(),
            join_token_ttl_seconds: default_voice_join_token_ttl_seconds(),
            candidate_limit: default_voice_candidate_limit(),
            offer_silence_seconds: default_voice_offer_silence_seconds(),
            session_silence_seconds: default_voice_session_silence_seconds(),
            idle_session_seconds: default_voice_idle_session_seconds(),
            servers: Vec::new(),
        }
    }
}

fn default_voice_token_secret() -> String {
    "aspen_dev_voice_secret".to_string()
}

fn default_voice_failure_threshold() -> u32 {
    5
}

fn default_voice_failure_window_seconds() -> u64 {
    3600
}

fn default_voice_join_token_ttl_seconds() -> u64 {
    60
}

fn default_voice_candidate_limit() -> usize {
    10
}

fn default_voice_offer_silence_seconds() -> u64 {
    60
}

/// A day.
fn default_voice_session_silence_seconds() -> u64 {
    24 * 60 * 60
}

/// A day.
fn default_voice_idle_session_seconds() -> u64 {
    24 * 60 * 60
}

/// Cross-Origin Resource Sharing.
///
/// Browsers (including the Electron and Capacitor shells, which are browsers) refuse to read a
/// response from an origin other than the page's own unless the server opts in with CORS
/// headers. `allowed_origins` lists the page origins permitted to call the API, such as
/// `https://chat.example.org` or the Vite dev server's `http://localhost:5173`. The single entry
/// `"*"` allows every origin, which is acceptable only because Aspen authenticates with a bearer
/// header rather than cookies. An empty list (the default) sends no CORS headers at all, which
/// is correct when the API and the web client are served from the same origin.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct CorsConfig {
    #[serde(default)]
    pub allowed_origins: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Default)]
pub struct MediaConfig {
    #[serde(default)]
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
#[derive(Clone, Debug, Deserialize)]
pub struct MediaS3Config {
    #[serde(default = "default_media_s3_endpoint")]
    pub endpoint: String,
    #[serde(default)]
    pub public_endpoint: Option<String>,
    #[serde(default = "default_media_s3_region")]
    pub region: String,
    #[serde(default = "default_media_s3_bucket")]
    pub bucket: String,
    #[serde(default = "default_media_s3_access_key")]
    pub access_key: String,
    #[serde(default = "default_media_s3_secret_key")]
    pub secret_key: String,
    #[serde(default = "default_media_s3_public_base_url")]
    pub public_base_url: String,
    #[serde(default = "default_media_s3_upload_url_ttl_seconds")]
    pub upload_url_ttl_seconds: u64,
}

impl Default for MediaS3Config {
    fn default() -> Self {
        Self {
            endpoint: default_media_s3_endpoint(),
            public_endpoint: None,
            region: default_media_s3_region(),
            bucket: default_media_s3_bucket(),
            access_key: default_media_s3_access_key(),
            secret_key: default_media_s3_secret_key(),
            public_base_url: default_media_s3_public_base_url(),
            upload_url_ttl_seconds: default_media_s3_upload_url_ttl_seconds(),
        }
    }
}

pub fn default_event_feed_shards() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

pub fn default_event_queue_size() -> usize {
    512
}

fn default_media_s3_endpoint() -> String {
    "http://127.0.0.1:3900".to_string()
}

fn default_media_s3_region() -> String {
    "garage".to_string()
}

fn default_media_s3_bucket() -> String {
    "aspen-media".to_string()
}

fn default_media_s3_access_key() -> String {
    "aspen_dev_key".to_string()
}

fn default_media_s3_secret_key() -> String {
    "aspen_dev_secret".to_string()
}

/// Local Garage `s3_web` endpoint, exposed on port 3902 by `docker-compose.yaml`.
/// Operators deploying against AWS S3 should override this to a CloudFront /
/// custom-domain URL pointed at the public-read bucket.
fn default_media_s3_public_base_url() -> String {
    "http://127.0.0.1:3902/aspen-media".to_string()
}

/// Fifteen minutes is long enough for a multi-minute upload from a slow
/// mobile connection while keeping a stale URL useless to anyone who
/// fishes it out of a log file later.
fn default_media_s3_upload_url_ttl_seconds() -> u64 {
    900
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
    Ok(loaded)
}
