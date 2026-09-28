pub use aspen_limits::{Limit, LimitSetting, RuleTable};
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use std::collections::{BTreeMap, HashMap};
use utoipa::ToSchema;

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
    pub bots: BotsConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub presence: PresenceConfig,
    #[serde(default)]
    pub registration: RegistrationConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub federation: FederationConfig,
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

/// Who may create an account (`app::registration_invite`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct RegistrationConfig {
    /// Whether creating an account takes an invite from the deployment's administrators. Off,
    /// anyone who reaches the server may register. On, the first account's invite is made
    /// from the terminal (`aspen-chat-server invites create`).
    pub invite_required: bool,
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
    /// Every account must have a second factor. A session of an account without one can only
    /// add one (or sign out) until it does.
    pub require_two_factor: bool,
    /// How the server names itself to authenticators: the label beside an authenticator app's
    /// codes and the name a passkey prompt shows.
    #[default = "Aspen"]
    pub service_name: String,
    /// A change to security settings needs the session to have proved who its user is within
    /// this many seconds; ten minutes by default.
    #[default = 600]
    pub reverify_seconds: u64,
    /// Passkeys are offered only when this is set.
    pub passkeys: Option<PasskeyConfig>,
    /// Threads password hashing and checking may use at once, one per logical CPU by default.
    /// Each Argon2 hash holds 19 MiB while it runs.
    #[default(crate::app::login::default_password_hashing_threads())]
    pub password_hashing_threads: usize,
    /// How long a sign-in or other password check waits for a thread before it is refused
    /// with `serverBusy`.
    #[default = 10]
    pub password_hashing_wait_seconds: u64,
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

/// Ceilings that keep one user's footprint bounded.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct LimitsConfig {
    /// The most communities one user may belong to. It bounds how many subjects an event
    /// stream connection reads and how many copies of a profile change are published.
    #[default = 500]
    pub max_communities_per_user: u32,
}

/// Federation: which of this deployment's users and bots may use other deployments, and whose
/// may use this one (`app::federation`).
///
/// Each direction has a gate. The deployment's policy, in the terms operators use, is the gates
/// of its users: none (both closed, the default), emigration (only `emigration` open or on a
/// list), immigration (only `immigration`), and full (both), each either open or with a list.
/// Bots have gates of their own, which work the same way.
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct FederationConfig {
    /// This deployment's name among deployments: the domain it is served at, with `:port` when
    /// that is not 443, such as `chat.example.org`. Required when any gate is not closed.
    /// Other deployments pin the key they find at this name, so it must not change.
    pub domain: Option<String>,
    pub users: MigrationRules,
    pub bots: MigrationRules,
    /// Settings for trying federation on one machine; a deployment others use leaves them out.
    pub development: FederationDevelopment,
}

/// Who may cross between this deployment and others, one direction at a time.
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct MigrationRules {
    /// This deployment's accounts using other deployments.
    pub emigration: Gate,
    /// Other deployments' accounts using this one.
    pub immigration: Gate,
    /// Both directions read one list instead of a list each. Both gates must then use a list,
    /// and the same kind of list.
    pub shared_list: bool,
    /// Whether an account of another deployment arriving here for the first time needs a
    /// registration invite (`app::registration_invite`), as `[registration] invite_required`
    /// asks of accounts made here.
    pub immigration_invite_required: bool,
}

/// One direction's gate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum Gate {
    /// No one crosses.
    #[default]
    Closed,
    /// Anyone crosses, to or from any deployment.
    Open,
    /// Only to or from the deployments on this direction's allow list.
    AllowList,
    /// To or from any deployment but those on this direction's block list.
    BlockList,
}

crate::app::wire_name_traits!(Gate);

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

impl FederationConfig {
    /// Whether any gate lets anyone cross.
    pub fn enabled(&self) -> bool {
        [&self.users, &self.bots]
            .iter()
            .any(|rules| rules.emigration != Gate::Closed || rules.immigration != Gate::Closed)
    }

    /// Whether an immigration gate lets accounts of other deployments in.
    pub fn admits_anyone(&self) -> bool {
        self.users.immigration != Gate::Closed || self.bots.immigration != Gate::Closed
    }

    fn validate(&self) -> Result<(), config::ConfigError> {
        if self.enabled() && self.domain.is_none() {
            return Err(config::ConfigError::Message(
                "federation.domain must be set when a federation gate is not closed".into(),
            ));
        }
        if let Some(domain) = &self.domain {
            crate::app::federation::Domain::parse(domain).map_err(|_| {
                config::ConfigError::Message(format!(
                    "federation.domain {domain:?} is not a domain, optionally with a port"
                ))
            })?;
        }
        for (name, rules) in [("users", &self.users), ("bots", &self.bots)] {
            let listed = |gate: Gate| matches!(gate, Gate::AllowList | Gate::BlockList);
            if rules.shared_list
                && !(listed(rules.emigration) && rules.emigration == rules.immigration)
            {
                return Err(config::ConfigError::Message(format!(
                    "federation.{name}.shared_list needs emigration and immigration to be the \
                     same kind of list"
                )));
            }
        }
        Ok(())
    }
}

/// Bots: accounts that sign in only with a token, each made and managed by a person
/// (`app::bot`).
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct BotsConfig {
    /// Whether people may make bots at all. Bots already made keep working either way.
    #[default = true]
    pub enabled: bool,
    /// The most bots one person may own.
    #[default = 25]
    pub max_per_user: u32,
}

/// Voice calls. The servers listed here are seeded into the `voice_server` table at startup,
/// matched by name, and can then be managed through the `/voice-servers` endpoints.
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
#[serde(default)]
pub struct CorsConfig {
    pub allowed_origins: Vec<String>,
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
    loaded.federation.validate()?;
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A section given in part keeps the defaults of what it leaves out, and a section left out
    /// is all defaults.
    #[test]
    fn missing_settings_take_their_defaults() {
        let config: AspenConfig = config::Config::builder()
            .add_source(config::File::from_str(
                r#"
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
        assert_eq!(config.auth.service_name, "Aspen");
        assert!(config.bots.enabled);
        assert_eq!(config.bots.max_per_user, 25);
        assert_eq!(config.presence.away_after_seconds, 600);
        assert_eq!(config.limits.max_communities_per_user, 500);
        assert!(config.metrics.enabled);
        assert!(!config.federation.enabled());
        assert_eq!(config.federation.users.emigration, Gate::Closed);
        assert_eq!(config.event_queue_size, 512);
        assert_eq!(
            config.voice.token_secret,
            VoiceConfig::default().token_secret
        );
    }

    fn federation(toml: &str) -> Result<(), config::ConfigError> {
        config::Config::builder()
            .add_source(config::File::from_str(toml, config::FileFormat::Toml))
            .build()?
            .try_deserialize::<FederationConfig>()?
            .validate()
    }

    /// An open gate needs the deployment's domain, and a shared list needs both directions to
    /// use the same kind of list.
    #[test]
    fn federation_settings_are_checked() {
        assert!(federation("[users]\nemigration = \"open\"").is_err());
        assert!(
            federation("domain = \"chat.example.org\"\n[users]\nemigration = \"open\"").is_ok()
        );
        assert!(federation("domain = \"https://chat.example.org\"").is_err());
        assert!(
            federation(
                "domain = \"a.example\"\n[users]\nemigration = \"allowList\"\nimmigration = \"blockList\"\nshared_list = true"
            )
            .is_err()
        );
        assert!(
            federation(
                "domain = \"a.example:8443\"\n[bots]\nemigration = \"blockList\"\nimmigration = \"blockList\"\nshared_list = true"
            )
            .is_ok()
        );
        assert!(federation("[users]\nemigration = \"sometimes\"").is_err());
    }
}
