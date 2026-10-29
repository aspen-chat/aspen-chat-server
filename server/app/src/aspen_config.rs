pub use aspen_limits::{Limit, LimitSetting, RuleTable};
pub use aspen_previews::PreviewConfig;
use serde::Deserialize;
use smart_default::SmartDefault;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

pub use aspen_tls::TlsFiles;

/// Its `Debug` leaves out the secrets it holds (`AspenConfig`'s own impl, below), so a config
/// written to a log gives none away.
#[derive(Clone, Deserialize)]
pub struct AspenConfig {
    /// The one address of this deployment, such as `https://chat.example.org`: the API, the web
    /// client this server serves, and the pages it opens are all at this origin. Every link to
    /// the deployment is built on it (invites, sign-in codes, mail), and the rest of what names
    /// the deployment follows from it as the config loads
    /// ([`AspenConfig::derive_from_public_url`]): passkeys belong to its host, and over `https`
    /// its host is the federation domain. It is `http` only at this machine (`localhost`, a name
    /// under it, or a loopback address), where browsers treat plain HTTP as secure.
    pub public_url: String,
    #[serde(default = "default_event_queue_size")]
    pub event_queue_size: usize,
    /// Tasks that route events to this server's event stream connections, one per logical CPU
    /// by default (`app::event_feed`).
    #[serde(default = "default_event_feed_shards")]
    pub event_feed_shards: usize,
    /// The most of the last minute's events, in MiB of their text, each API server keeps for
    /// catching connections up (`app::event_feed`); past it the oldest are let go early, and a
    /// client resuming from before them reads its state again.
    #[serde(default = "default_event_retained_mib")]
    pub event_retained_mib: usize,
    pub database_url: String,
    /// The most database connections this server holds at once; left out, two per logical CPU. Every
    /// write holds one until its event is acknowledged, so a busy server may want more, within
    /// what PostgreSQL's `max_connections` allows for every server together.
    pub database_pool_size: Option<usize>,
    /// How long a request or task waits for one of those connections before it gives up with
    /// `serverBusy`, so a pool that runs dry refuses work rather than holding it forever.
    #[serde(default = "default_database_pool_wait_seconds")]
    pub database_pool_wait_seconds: u64,
    pub nats_url: String,
    /// The token NATS was started with, when it signs everyone in by one token. Exactly one of
    /// this and `[nats_user]` is given (`AspenConfig::nats_options`).
    #[serde(default)]
    pub nats_auth_token: Option<String>,
    /// A NATS user for the API servers, when NATS has users: so that each voice server signs
    /// in as a user allowed only its own subjects
    /// (`docs/operators/installing/6-voice-servers.md`).
    #[serde(default)]
    pub nats_user: Option<NatsUser>,
    /// TLS to NATS: `[nats.tls]`'s certificate files, which also make TLS required.
    #[serde(default)]
    pub nats: NatsConfig,
    /// Valkey, as `redis://` or, over TLS, `rediss://`. A password in it is refused unencrypted
    /// to anything but this machine (`AspenConfig::check_valkey`).
    pub valkey_url: String,
    /// TLS to Valkey: `[valkey.tls]`'s certificate files, which need a `rediss://` `valkey_url`.
    #[serde(default)]
    pub valkey: ValkeyConfig,
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
    pub search: SearchConfig,
    #[serde(default)]
    pub metrics: MetricsConfig,
    #[serde(default)]
    pub health: HealthConfig,
    #[serde(default)]
    pub connections: ConnectionsConfig,
    #[serde(default)]
    pub federation: FederationConfig,
    #[serde(default)]
    pub push: PushConfig,
    #[serde(default)]
    pub identity_providers: IdentityProvidersConfig,
    #[serde(default)]
    pub web_client: WebClientConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
    /// Running background jobs (`app::jobs`).
    #[serde(default)]
    pub jobs: JobsConfig,
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
    /// Endpoints (`*` matching any run of characters) refused while the limits cannot be
    /// counted, rather than let through: those that guess a secret (a password, a code, an
    /// invite). The username limit of `POST /auth/login` is always counted so.
    #[serde(default)]
    pub fail_closed: Vec<String>,
    /// Endpoints (`*` matching any run of characters) a request without a session may call
    /// without a device credential (`app::device_credential`): those called by what is not an
    /// Aspen client, such as browsers opening a page and other deployments' servers.
    #[serde(default)]
    pub device_exempt: Vec<String>,
    /// How many leading zero bits a device credential's puzzle asks for, at most
    /// `device_credential::MAX_PUZZLE_BITS`: each more doubles the work, about a second on a
    /// mid-range phone at 20.
    pub device_puzzle_bits: u8,
    /// How many times a person's limits a bot's own are (`user`, `user_per_<param>`), its
    /// sustained rate and its burst both: a bot serves a whole community, as no person does.
    pub bot_multiplier: u32,
}

/// The `[rate_limits]` of `aspen.toml`, laid over the built-in limits. Each limit given
/// replaces the built-in one for the same place and dimension whole, so a limit written without
/// `burst` has the default burst rather than the built-in one's; everything not given keeps its
/// built-in value. A group given here with a name the built-ins use keeps their endpoint list
/// unless it lists its own.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RateLimitOverrides {
    pub enabled: Option<bool>,
    #[serde(default, deserialize_with = "aspen_config_values::optional_list")]
    pub trusted_proxies: Option<Vec<String>>,
    pub ipv6_prefix: Option<u8>,
    pub max_suspension_seconds: Option<u64>,
    /// Replaces the built-in list whole.
    #[serde(default, deserialize_with = "aspen_config_values::optional_list")]
    pub fail_closed: Option<Vec<String>>,
    /// Replaces the built-in list whole.
    #[serde(default, deserialize_with = "aspen_config_values::optional_list")]
    pub device_exempt: Option<Vec<String>>,
    pub device_puzzle_bits: Option<u8>,
    pub bot_multiplier: Option<u32>,
    #[serde(default, deserialize_with = "aspen_config_values::table")]
    pub default: RuleTable,
    #[serde(default, deserialize_with = "aspen_config_values::table")]
    pub groups: BTreeMap<String, RateLimitGroupOverride>,
    /// Keyed by method and path, which no environment variable's name can spell: from the
    /// environment, a JSON object of them all (`aspen_config_values::table`).
    #[serde(default, deserialize_with = "aspen_config_values::table")]
    pub endpoints: HashMap<String, RuleTable>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct RateLimitGroupOverride {
    #[serde(default, deserialize_with = "aspen_config_values::optional_list")]
    pub endpoints: Option<Vec<String>>,
    #[serde(default, deserialize_with = "aspen_config_values::table")]
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
        if let Some(fail_closed) = overrides.fail_closed {
            self.fail_closed = fail_closed;
        }
        if let Some(exempt) = overrides.device_exempt {
            self.device_exempt = exempt;
        }
        if let Some(bits) = overrides.device_puzzle_bits {
            self.device_puzzle_bits = bits;
        }
        if let Some(multiplier) = overrides.bot_multiplier {
            self.bot_multiplier = multiplier;
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

/// A private worker's health checks (`api::health`), on a listener of their own, since it opens no
/// other. A server that takes requests answers them on its own listener and opens none for them.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct HealthConfig {
    /// Loopback by default, where a container's own health check asks.
    #[default(std::net::SocketAddr::from(([127, 0, 0, 1], 9467)))]
    pub listen_addr: std::net::SocketAddr,
}

/// What the API server's listener admits (`connections`), so connections held open slowly or
/// in numbers cannot take every socket.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct ConnectionsConfig {
    /// The most connections this server holds open at once, event streams included; one more
    /// is closed as soon as it is accepted. Keep it below the process's open file limit.
    #[default = 100_000]
    pub max: usize,
    /// The most one address holds open at once (an IPv6 address by its `[rate_limits]
    /// ipv6_prefix` network). Reverse proxies in `[rate_limits] trusted_proxies` count only
    /// toward `max`.
    #[default = 512]
    pub max_per_ip: usize,
    /// The most one network holds open at once: an IPv4 /24 or an IPv6 /48, counted alongside
    /// `max_per_ip`, so a holder of many addresses cannot spread past it. Reverse proxies in
    /// `[rate_limits] trusted_proxies` count only toward `max`.
    #[default = 4096]
    pub max_per_network: usize,
    /// The most connections one user holds open at once once they have signed in on them. A
    /// connection counts toward `max_per_ip` and `max_per_network` until a request on it
    /// presents a session or its event stream identifies, and then toward its user's share
    /// instead while the user has room, so many people behind one address (a carrier-grade NAT)
    /// are each held to their own share rather than all to the address's.
    #[default = 64]
    pub max_per_user: usize,
    /// How long a client has to finish its TLS handshake.
    #[default = 10]
    pub handshake_seconds: u64,
    /// How long an HTTP/1.1 client has to send a request's headers, counted from when the
    /// server starts waiting for them, so an idle connection kept alive closes after this too.
    #[default = 30]
    pub header_read_seconds: u64,
    /// How long a connection may stay open with no request in it before it is closed. HTTP/2
    /// clients keep connections open between requests; an HTTP/1.1 connection kept alive closes
    /// after `header_read_seconds` first. An event stream's WebSocket leaves its HTTP connection
    /// when it opens, so this does not close it.
    #[default = 120]
    pub idle_seconds: u64,
    /// How long a request may take, from when it arrives until its response's headers are
    /// ready, its body read included; one that takes longer is answered `requestTimedOut`
    /// (`api::deadline`). It must leave a search's `[search] round_seconds` ten seconds to
    /// spare. A response's body, once under way, is bounded by `idle_seconds` instead: it keeps
    /// its connection open only while it makes progress.
    #[default = 60]
    pub request_seconds: u64,
    /// How long a connection asked to close (idle, or the server shutting down) has to finish
    /// what is in flight before it is dropped, so a client that never reads its responses
    /// cannot hold it open.
    #[default = 30]
    pub shutdown_seconds: u64,
    /// The most requests one HTTP/2 connection may have open at once.
    #[default = 64]
    pub max_streams: u32,
}

impl ConnectionsConfig {
    /// Refuses a request deadline that would cut off a search's round, and limits of none.
    fn check(&self, search: &SearchConfig) -> Result<(), config::ConfigError> {
        let message = |text: String| config::ConfigError::Message(text);
        if self.request_seconds < search.round_seconds + REQUEST_SPARE_SECONDS {
            return Err(message(format!(
                "connections.request_seconds ({}) must be at least search.round_seconds ({}) \
                 plus {REQUEST_SPARE_SECONDS}, or searches through history are cut off",
                self.request_seconds, search.round_seconds
            )));
        }
        if self.shutdown_seconds == 0 || self.max_streams == 0 {
            return Err(message(
                "connections.shutdown_seconds and connections.max_streams must be at least 1"
                    .to_string(),
            ));
        }
        Ok(())
    }
}

/// How much longer than a search's round a request may take: the round's last step, and
/// building its page, run past it.
const REQUEST_SPARE_SECONDS: u64 = 10;

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

/// Searches that read on through history after finding nothing near the present
/// (`app::search`, `app::deep_search`).
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct SearchConfig {
    /// How many such searches' steps run at once on the whole deployment, each a core of the
    /// database's for as long as it runs; two by default, which suits a small machine. About a
    /// quarter of the database server's cores leaves the rest for everyone else. Every server
    /// should be given the same number.
    #[default = 2]
    pub deep_concurrency: u32,
    /// How long one request reads on for before it answers with how far it got, and its client
    /// asks again from there; twenty seconds by default. Keep it under the shortest timeout of
    /// anything between clients and this server (a reverse proxy's is commonly sixty seconds).
    #[default = 20]
    pub round_seconds: u64,
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
    #[default(crate::login::default_password_hashing_threads())]
    pub password_hashing_threads: usize,
    /// How long a sign-in or other password check waits for a thread before it is refused
    /// with `serverBusy`.
    #[default = 10]
    pub password_hashing_wait_seconds: u64,
    /// Android apps, besides the official one (`passkey::OFFICIAL_ANDROID_APPS`), that may use
    /// this deployment's passkeys through Android's Credential Manager, each written
    /// `<package>=<SHA-256 fingerprint of its signing certificate>` (`passkey::AndroidApp`): a
    /// fork, or a build signed with another key. Each is named in `/.well-known/assetlinks.json`
    /// and its origin accepted in ceremonies. Without passkeys (`rp_id`) none is.
    #[serde(deserialize_with = "aspen_config_values::list")]
    pub android_apps: Vec<crate::passkey::AndroidApp>,
}

/// Ceilings that keep one user's footprint bounded.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct LimitsConfig {
    /// The most communities one user may belong to. It bounds how many subjects an event
    /// stream connection reads and how many copies of a profile change are published.
    #[default = 500]
    pub max_communities_per_user: u32,
    /// The most event streams one user may hold open on one API server at once
    /// (`app::event_feed::StreamCaps`); one more is closed with `tooManyStreams`. Each person's
    /// app holds one per window or device, and a bot one per process.
    #[default = 20]
    pub max_event_streams_per_user: usize,
    /// The most event streams one client address (an IPv6 one by its `[rate_limits]
    /// ipv6_prefix` network) may hold open on one API server before they identify, counted from
    /// the upgrade; one more is closed with `tooManyStreamsFromAddress`. An identified stream
    /// counts toward its user's cap instead, so this bounds only sockets that have not said who
    /// they are, however many people share the address.
    #[default = 200]
    pub max_event_streams_per_address: usize,
    /// The most event streams one API server identifies at once; half of the database pool
    /// when left out. Each identify reads the database, and the stream it opens is soon
    /// followed by its client reading its state again, so after a mass reconnect this keeps
    /// requests their share of the pool. A stream that cannot start identifying soon, or that
    /// arrives while requests already queue for the pool, is closed with `serverBusy` and told
    /// when to come back (`api::event_stream`).
    pub max_identifying_event_streams: Option<usize>,
    /// The most DM partners whose profile events one event stream connection follows
    /// (`app::event_feed::Following`): the people in the user's most recent DMs, as many DMs as
    /// keep their partners within this many. A connection learns of a change to the profile of
    /// someone it shares no community with (a name, an avatar, a deleted account) only through
    /// this, so a partner from an older DM is shown as they were until the client reads them
    /// again. Each followed partner is one entry in the server's index of who follows whom.
    #[default = 100]
    pub max_followed_dm_partners: usize,
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
    /// whether they are still in good standing there, and reads again the documents of the
    /// deployments in use that a gate admits (`app::federation::standing`): an hour.
    #[default = 3600]
    pub standing_interval_seconds: u64,
    /// How long a home may go unreached before its users' sessions here end: a day.
    #[default = 86400]
    pub standing_grace_seconds: u64,
    /// The most users and bots of one other deployment that may arrive here for the first time
    /// in one day (UTC), counted across every server (`app::federation::abroad`), so a home
    /// that mints accounts cannot fill this deployment with them: five hundred.
    #[default = 500]
    pub max_arrivals_per_home_per_day: u64,
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
    #[serde(deserialize_with = "aspen_config_values::list")]
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

/// Signing in through OpenID Connect providers (`app::identity_provider`). Which providers there
/// are is in the database.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct IdentityProvidersConfig {
    /// Whether a provider's issuer may resolve to an address inside a network (a Keycloak on the
    /// LAN). Off, the server reaches providers only on the public internet, so whoever may add a
    /// provider in the dashboard cannot make it call services only it can see.
    pub allow_private_addresses: bool,
    /// Certificate authorities, in PEM, trusted for providers besides the system's: a company's
    /// own authority, which its identity provider's certificate is issued by.
    pub ca_file: Option<PathBuf>,
}

/// How this server runs background jobs (`app::jobs`). Which jobs there are is in the database;
/// every server that runs jobs takes its share of them.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct JobsConfig {
    /// Whether this server runs jobs at all. A deployment needs at least one server that does.
    #[default = true]
    pub run: bool,
    /// The most jobs this server runs at once that any class may take, beside the one place
    /// each class keeps for itself; two per logical CPU by default.
    #[default(_code = "2 * default_event_feed_shards()")]
    pub concurrency: usize,
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
    /// The most calls of plugins this server runs at once, of every plugin together; two per
    /// logical CPU by default. A call waits for a place within its own time limit, and counts
    /// as failed when none comes in time. With `memory_mib`, it bounds what plugins can take of
    /// the server's memory. A quarter of the places are kept for intercepting calls
    /// (`plugin::registry::Places`).
    #[default(_code = "2 * default_event_feed_shards()")]
    pub concurrency: usize,
    /// The most calls of any one plugin this server runs at once, so one busy plugin leaves
    /// room for the rest; one per logical CPU by default. A quarter of these too are kept for
    /// intercepting calls.
    #[default(_code = "default_event_feed_shards()")]
    pub concurrency_per_plugin: usize,
    /// The most notices one plugin may give one person in a minute (`notify`).
    #[default = 10]
    pub notify_per_minute: u32,
    /// The most notices one plugin may give one person in a day.
    #[default = 100]
    pub notify_per_day: u32,
    /// The most one plugin may keep, every community's, DM's, and person's share together, in
    /// GiB; 0 sets no limit beyond each share's.
    #[default = 16]
    pub storage_total_gib: u64,
}

/// Voice calls. The voice servers themselves are rows of `voice_server`, added from the
/// dashboard or the terminal (`aspen-chat-server voice-servers`). Join tokens are signed with
/// the deployment's join token key (`app::server_secret::JoinTokenKey`), which is in the
/// database, not here.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct VoiceConfig {
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
    /// A voice server silent for this long (fifteen minutes by default) has its sessions ended by
    /// the reaper, so a server that died leaves no phantom calls once the first person to try
    /// joining each of its calls has not already ended it (`voice::join_offer`). Minutes rather
    /// than seconds, so a call survives a brief outage of the report link (NATS restarting, a
    /// voice server reconnecting), whose lost reports its next snapshot repairs; minutes rather
    /// than hours, since a dead server's calls stay on record, showing their people in them and
    /// taking the commands no one hears, for at most this long.
    #[default(15 * 60)]
    pub session_silence_seconds: u64,
    /// A call that has gone this long (a day by default) without ever holding two people at
    /// once is ended and its lone participant told why, so a forgotten client cannot hold a
    /// voice server slot indefinitely.
    #[default(24 * 60 * 60)]
    pub idle_session_seconds: u64,
}

/// A NATS user and its password.
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NatsUser {
    pub user: String,
    pub password: String,
}

/// `[nats]`: how NATS is reached besides its address and credentials.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NatsConfig {
    pub tls: Option<TlsFiles>,
    /// The NATS server's monitoring port (`http://nats:8222`), where the server says how much
    /// memory its streams may reserve (`app::nats_memory`). Left out, the streams are sized
    /// without knowing, unless NATS limits this deployment's account.
    pub monitor_url: Option<String>,
}

/// `[valkey]`: how Valkey is reached besides its address.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValkeyConfig {
    pub tls: Option<TlsFiles>,
}

/// Stands for a secret in a `Debug` impl.
struct Redacted;

impl std::fmt::Debug for Redacted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("<redacted>")
    }
}

/// A URL in a `Debug` impl, with any password it holds replaced, and the whole replaced when it
/// does not parse, since then where its password is cannot be told.
struct RedactedUrl<'a>(&'a str);

impl std::fmt::Debug for RedactedUrl<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match url::Url::parse(self.0) {
            Ok(mut url) => {
                if url.password().is_some() {
                    let _ = url.set_password(Some("<redacted>"));
                }
                write!(f, "{:?}", url.as_str())
            }
            Err(_) => Redacted.fmt(f),
        }
    }
}

impl std::fmt::Debug for AspenConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            public_url,
            event_queue_size,
            event_feed_shards,
            event_retained_mib,
            database_url,
            database_pool_size,
            database_pool_wait_seconds,
            nats_url,
            nats_auth_token,
            nats_user,
            nats,
            valkey_url,
            valkey,
            media,
            voice,
            limits,
            auth,
            presence,
            search,
            metrics,
            health,
            connections,
            federation,
            push,
            identity_providers,
            web_client,
            plugins,
            jobs,
            email,
            rate_limit_overrides,
            rate_limits,
        } = self;
        f.debug_struct("AspenConfig")
            .field("public_url", public_url)
            .field("event_queue_size", event_queue_size)
            .field("event_feed_shards", event_feed_shards)
            .field("event_retained_mib", event_retained_mib)
            .field("database_url", &RedactedUrl(database_url))
            .field("database_pool_size", database_pool_size)
            .field("database_pool_wait_seconds", database_pool_wait_seconds)
            .field("nats_url", &RedactedUrl(nats_url))
            .field(
                "nats_auth_token",
                &nats_auth_token.as_ref().map(|_| Redacted),
            )
            .field("nats_user", nats_user)
            .field("nats", nats)
            .field("valkey_url", &RedactedUrl(valkey_url))
            .field("valkey", valkey)
            .field("media", media)
            .field("voice", voice)
            .field("limits", limits)
            .field("auth", auth)
            .field("presence", presence)
            .field("search", search)
            .field("metrics", metrics)
            .field("health", health)
            .field("connections", connections)
            .field("federation", federation)
            .field("push", push)
            .field("identity_providers", identity_providers)
            .field("web_client", web_client)
            .field("plugins", plugins)
            .field("jobs", jobs)
            .field("email", email)
            .field("rate_limit_overrides", rate_limit_overrides)
            .field("rate_limits", rate_limits)
            .finish()
    }
}

impl std::fmt::Debug for NatsUser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NatsUser")
            .field("user", &self.user)
            .field("password", &Redacted)
            .finish()
    }
}

impl AspenConfig {
    /// How to reach NATS: signing in as the `[nats_user]` user or with `nats_auth_token`,
    /// whichever is given (`load_config` refuses both or neither), over TLS as `[nats.tls]` says.
    pub fn nats_options(&self) -> async_nats::ConnectOptions {
        let options = match (&self.nats_user, &self.nats_auth_token) {
            (Some(NatsUser { user, password }), _) => {
                async_nats::ConnectOptions::with_user_and_password(user.clone(), password.clone())
            }
            (None, token) => {
                async_nats::ConnectOptions::with_token(token.clone().unwrap_or_default())
            }
        };
        aspen_tls::nats_options(options, self.nats.tls.as_ref())
    }

    /// Connects to NATS as `nats_options` says.
    pub async fn connect_nats(&self) -> Result<async_nats::Client, async_nats::ConnectError> {
        async_nats::connect_with_options(&self.nats_url, self.nats_options()).await
    }

    fn check_nats(&self) -> Result<(), config::ConfigError> {
        if let Some(tls) = &self.nats.tls {
            tls.identity("nats.tls")
                .map_err(config::ConfigError::Message)?;
        }
        if let Some(monitor) = &self.nats.monitor_url
            && !url::Url::parse(monitor)
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.has_host())
        {
            return Err(config::ConfigError::Message(format!(
                "[nats] monitor_url: {monitor:?} is not an http or https address; give the NATS \
                 server's monitoring port, as http://localhost:8222"
            )));
        }
        match (&self.nats_user, &self.nats_auth_token) {
            (Some(_), None) | (None, Some(_)) => Ok(()),
            (None, None) => Err(config::ConfigError::Message(
                "give nats_auth_token, or [nats_user] user and password".to_string(),
            )),
            (Some(_), Some(_)) => Err(config::ConfigError::Message(
                "give either nats_auth_token or [nats_user] user and password, not both"
                    .to_string(),
            )),
        }
    }

    /// Refuses `[valkey.tls]` with a `valkey_url` that does not use TLS, which would ignore it,
    /// and a password in a `valkey_url` without TLS to anything but this machine, where whoever
    /// reads the network would read it.
    fn check_valkey(&self) -> Result<(), config::ConfigError> {
        let message = |text: &str| Err(config::ConfigError::Message(text.to_string()));
        let Ok(url) = url::Url::parse(&self.valkey_url) else {
            // fred reports what is wrong with it when the client is made.
            return Ok(());
        };
        let encrypted = matches!(url.scheme(), "rediss" | "valkeys");
        if let Some(tls) = &self.valkey.tls {
            tls.identity("valkey.tls")
                .map_err(config::ConfigError::Message)?;
            if !encrypted {
                return message(
                    "[valkey.tls] is given but valkey_url does not use TLS; name it with rediss://",
                );
            }
        }
        let loopback = url.host().is_some_and(|host| is_loopback_host(&host));
        if url.password().is_some() && !encrypted && !loopback {
            return message(
                "valkey_url would send its password unencrypted; name Valkey with rediss:// and \
                 give it a certificate (tls-port, tls-cert-file, tls-key-file)",
            );
        }
        Ok(())
    }
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
/// newsletter. Its `Debug` leaves out the password in `smtp_url`.
#[derive(Clone, Deserialize)]
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
    /// `[email.tls]`: authorities to trust besides the system's, and a client certificate, for
    /// an `smtp_url` that uses TLS.
    pub tls: Option<TlsFiles>,
}

impl std::fmt::Debug for EmailConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            smtp_url,
            send,
            max_per_second,
            from,
            tls,
        } = self;
        f.debug_struct("EmailConfig")
            .field("smtp_url", &smtp_url.as_deref().map(RedactedUrl))
            .field("send", send)
            .field("max_per_second", max_per_second)
            .field("from", from)
            .field("tls", tls)
            .finish()
    }
}

impl EmailConfig {
    fn default_send() -> bool {
        true
    }

    /// Refuses an `smtp_url` that would send its user and password unencrypted, `smtp://` without
    /// `tls=required` (which `tls=opportunistic` is not: whoever sits between the servers can
    /// strip STARTTLS), to anything but this machine, where a development mail catcher listens.
    fn check_smtp_encrypted(smtp_url: &str) -> Result<(), String> {
        let Ok(url) = url::Url::parse(smtp_url) else {
            // `from_url` reports what is wrong with it when the transport is made.
            return Ok(());
        };
        let has_credentials = !url.username().is_empty() || url.password().is_some();
        let encrypted = Self::encrypts(&url);
        let loopback = url.host().is_some_and(|host| is_loopback_host(&host));
        if has_credentials && !encrypted && !loopback {
            return Err(
                "email.smtp_url would send its user and password unencrypted; use smtps:// \
                 (port 465) or add ?tls=required (STARTTLS, port 587)"
                    .to_string(),
            );
        }
        Ok(())
    }

    /// Whether `smtp_url` always uses TLS: `smtps://`, or STARTTLS with `tls=required`.
    pub fn encrypts(smtp_url: &url::Url) -> bool {
        smtp_url.scheme() == "smtps"
            || smtp_url
                .query_pairs()
                .any(|(key, value)| key == "tls" && value == "required")
    }

    /// Checks `from`, `smtp_url`, `max_per_second`, and `[email.tls]`.
    fn validate(&self) -> Result<(), config::ConfigError> {
        let message = |text: String| config::ConfigError::Message(text);
        if self.send && self.smtp_url.is_none() {
            return Err(message(
                "[email] needs smtp_url on a server that sends mail; set send = false on a \
                 server that only queues it"
                    .to_string(),
            ));
        }
        if let Some(smtp_url) = &self.smtp_url {
            Self::check_smtp_encrypted(smtp_url).map_err(message)?;
        }
        if let Some(tls) = &self.tls {
            tls.identity("email.tls").map_err(message)?;
            let encrypted = self
                .smtp_url
                .as_deref()
                .and_then(|url| url::Url::parse(url).ok())
                .is_some_and(|url| Self::encrypts(&url));
            if !encrypted {
                return Err(message(
                    "[email.tls] is given but email.smtp_url does not always use TLS; use \
                     smtps:// or add ?tls=required"
                        .to_string(),
                ));
            }
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

#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct MediaConfig {
    pub s3: MediaS3Config,
    pub previews: PreviewConfig,
    /// The largest attachment, in bytes, anyone may upload (256 MiB). Each upload URL is signed
    /// for the size its client declares, which may be no more, and confirming an upload that
    /// holds more deletes it.
    #[default = 268_435_456]
    pub max_attachment_bytes: u64,
}

/// Object-storage configuration.
///
/// `endpoint` is the authenticated S3 API the server talks to (writes preview images, copies
/// confirmed uploads into place, deletes objects, checks uploads). `public_endpoint`, when set, is
/// the same API as clients reach it, and is the host the presigned upload URLs they are handed
/// name; without it they name `endpoint`, which is right only when clients reach storage at the
/// same address the server does. `public_base_url` is what clients download objects from, an
/// anonymous read path the operator sets up that allows reading objects and nothing else, no
/// listing and no writes (Garage's `s3_web` website endpoint, an AWS bucket whose policy allows
/// `s3:GetObject` alone, or a CDN before either;
/// `docs/operators/installing/storage-read-path.md`). The addresses may name different hosts;
/// the read path need not be reachable from the server itself. The addresses' defaults are a
/// development storage's. Its `Debug` leaves out `secret_key`.
#[derive(Clone, Deserialize, SmartDefault)]
#[serde(default)]
pub struct MediaS3Config {
    #[default = "http://127.0.0.1:3900"]
    pub endpoint: String,
    pub public_endpoint: Option<String>,
    #[default = "garage"]
    pub region: String,
    #[default = "aspen-media"]
    pub bucket: String,
    /// The key pair the server signs with, both or neither. Neither, it takes credentials as
    /// AWS's own tools do: `AWS_ACCESS_KEY_ID` and the rest of the environment, then the role
    /// of the ECS task or EC2 machine it runs on (`check_credentials`).
    pub access_key: Option<String>,
    pub secret_key: Option<String>,
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
    /// `[media.s3.tls]`: authorities to trust besides the system's for an `https` `endpoint`
    /// (`ca_file` alone; the S3 client presents no client certificate).
    pub tls: Option<TlsFiles>,
}

impl std::fmt::Debug for MediaS3Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self {
            endpoint,
            public_endpoint,
            region,
            bucket,
            access_key,
            secret_key: _,
            public_base_url,
            upload_url_ttl_seconds,
            tls,
        } = self;
        f.debug_struct("MediaS3Config")
            .field("endpoint", endpoint)
            .field("public_endpoint", public_endpoint)
            .field("region", region)
            .field("bucket", bucket)
            .field("access_key", access_key)
            .field("secret_key", &Redacted)
            .field("public_base_url", public_base_url)
            .field("upload_url_ttl_seconds", upload_url_ttl_seconds)
            .field("tls", tls)
            .finish()
    }
}

impl MediaS3Config {
    /// Refuses a key pair given half.
    fn check_credentials(&self) -> Result<(), config::ConfigError> {
        if self.access_key.is_some() != self.secret_key.is_some() {
            return Err(config::ConfigError::Message(
                "[media.s3] access_key and secret_key are given together, or neither to take \
                 credentials from the environment or the role this server runs as"
                    .to_string(),
            ));
        }
        Ok(())
    }

    /// Refuses `[media.s3.tls]` with an `endpoint` that is not `https`, which would ignore it, and
    /// a client certificate, which the S3 client cannot present.
    fn check_tls(&self) -> Result<(), config::ConfigError> {
        let Some(tls) = &self.tls else {
            return Ok(());
        };
        let message = |text: &str| Err(config::ConfigError::Message(text.to_string()));
        if tls.cert_file.is_some() || tls.key_file.is_some() {
            return message(
                "[media.s3.tls] takes only ca_file: the S3 client presents no client certificate",
            );
        }
        if !self.endpoint.starts_with("https://") {
            return message("[media.s3.tls] is given but media.s3.endpoint is not https://");
        }
        Ok(())
    }
}

pub fn default_event_feed_shards() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

pub fn default_database_pool_wait_seconds() -> u64 {
    10
}

pub fn default_event_queue_size() -> usize {
    512
}

pub fn default_event_retained_mib() -> usize {
    256
}

/// The built-in rate limits, beneath whatever `aspen.toml` sets.
const DEFAULT_RATE_LIMITS: &str = include_str!("rate_limits.toml");

/// Loads or reloads the config: `aspen.toml` in the working directory, and the environment.
pub fn load_config() -> Result<AspenConfig, config::ConfigError> {
    load_from(
        config::File::new("aspen.toml", config::FileFormat::Toml),
        environment(),
    )
}

/// The `ASPEN_` variables, which override the file. `ASPEN_DATABASE_URL` sets `database_url`;
/// `ASPEN_VOICE__IDLE_SESSION_SECONDS` sets `voice.idle_session_seconds`. The prefix separator
/// is set explicitly because it would otherwise follow the nesting separator and every flat key
/// would need two underscores. Settings that are lists or tables read text as
/// `aspen_config_values` says.
fn environment() -> config::Environment {
    config::Environment::with_prefix("ASPEN")
        .prefix_separator("_")
        .separator("__")
}

fn load_from(
    file: impl config::Source + Send + Sync + 'static,
    environment: config::Environment,
) -> Result<AspenConfig, config::ConfigError> {
    let mut loaded = config::Config::builder()
        // Sources added later take precedence.
        .add_source(file)
        .add_source(environment)
        .build()?
        .try_deserialize::<AspenConfig>()?;
    // Merged here rather than as one more config source, which would merge a limit given in
    // `aspen.toml` into the built-in one field by field.
    loaded.rate_limits =
        RateLimitConfig::built_in()?.overlay(std::mem::take(&mut loaded.rate_limit_overrides))?;
    loaded.derive_from_public_url()?;
    loaded.check_nats()?;
    loaded.check_valkey()?;
    loaded.media.s3.check_tls()?;
    loaded.media.s3.check_credentials()?;
    loaded.check_development_credentials()?;
    loaded.check_federation_development()?;
    loaded.connections.check(&loaded.search)?;
    if let Some(email) = &loaded.email {
        email.validate()?;
    }
    Ok(loaded)
}

/// Credentials written into this repository for development (`docker-compose.yaml` and the
/// scripts), which anyone can read.
const DEVELOPMENT_PASSWORDS: &[&str] = &["aspen_test"];
const DEVELOPMENT_S3_KEYS: &[&str] = &[
    "GK484e56c38fb7e14b182bf47a",
    "6b49da9e42f7959cc946d7987a504763f6ec405b88abeeec08aa926b61316027",
];

/// Whether `host` is this machine: `localhost`, a name under it, or a loopback address.
pub fn is_loopback_host(host: &url::Host<&str>) -> bool {
    match host {
        // A URL of a scheme `url` does not know (`smtp:`) holds even an address as a name.
        url::Host::Domain(name) => aspen_tls::is_loopback(name),
        url::Host::Ipv4(address) => address.is_loopback(),
        url::Host::Ipv6(address) => address.is_loopback(),
    }
}

impl AspenConfig {
    /// Whether this deployment is one others may reach: `public_url`'s host is not this machine,
    /// which only an `https` one may be ([`AspenConfig::derive_from_public_url`]). Development
    /// deployments, at `localhost` or a name under it (`scripts/dev_federation.py`), may use what
    /// is meant only for development.
    pub fn is_public(&self) -> bool {
        url::Url::parse(&self.public_url)
            .is_ok_and(|url| url.host().is_some_and(|host| !is_loopback_host(&host)))
    }

    /// Refuses `[federation.development]` unless `public_url`'s host is this machine (`localhost`,
    /// a name under it, or a loopback address): what it allows, trusting more certificate
    /// authorities and calling into private networks, is for deployments side by side on one
    /// machine, and would let a deployment others reach be pointed inside its own network.
    fn check_federation_development(&self) -> Result<(), config::ConfigError> {
        let development = &self.federation.development;
        if development.extra_root_certificates.is_empty() && !development.allow_private_addresses {
            return Ok(());
        }
        let local = url::Url::parse(&self.public_url)
            .is_ok_and(|url| url.host().is_some_and(|host| is_loopback_host(&host)));
        if local {
            return Ok(());
        }
        Err(config::ConfigError::Message(
            "[federation.development] is only for deployments whose public_url is at localhost \
             or a name under it; remove it"
                .to_string(),
        ))
    }

    /// Refuses, at a public deployment ([`AspenConfig::is_public`]), the database, NATS, and
    /// storage credentials this repository holds for development: whoever reads the repository
    /// would hold the deployment's data.
    fn check_development_credentials(&self) -> Result<(), config::ConfigError> {
        if !self.is_public() {
            return Ok(());
        }
        let refuse = |what: &str| {
            Err(config::ConfigError::Message(format!(
                "{what} is a development value published in Aspen's repository; a deployment at \
                 an https address needs one of its own, a long random string"
            )))
        };
        let database_password = crate::database::password(&self.database_url)
            .map_err(|e| config::ConfigError::Message(e.to_string()))?;
        if database_password.is_some_and(|password| {
            DEVELOPMENT_PASSWORDS
                .iter()
                .any(|development| development.as_bytes() == password)
        }) {
            return refuse("database_url's password");
        }
        if self
            .nats_auth_token
            .as_deref()
            .is_some_and(|token| DEVELOPMENT_PASSWORDS.contains(&token))
        {
            return refuse("nats_auth_token");
        }
        if self
            .nats_user
            .as_ref()
            .is_some_and(|nats| DEVELOPMENT_PASSWORDS.contains(&&*nats.password))
        {
            return refuse("[nats_user] password");
        }
        let s3 = &self.media.s3;
        let development = |key: &Option<String>| {
            key.as_deref()
                .is_some_and(|key| DEVELOPMENT_S3_KEYS.contains(&key))
        };
        if development(&s3.access_key) {
            return refuse("[media.s3] access_key");
        }
        if development(&s3.secret_key) {
            return refuse("[media.s3] secret_key");
        }
        Ok(())
    }
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
        if !https && !parsed.host().is_some_and(|host| is_loopback_host(&host)) {
            return Err(invalid(
                "is http at a host that is not this machine; browsers elsewhere refuse passkeys, \
                 Web Crypto, and service workers to such a page. Use https, with a certificate \
                 from scripts/dev_ca.py in development",
            ));
        }
        self.federation.domain = match (https, parsed.host_str()) {
            (true, Some(host)) => {
                let authority = match parsed.port() {
                    Some(port) => format!("{host}:{port}"),
                    None => host.to_string(),
                };
                let domain = crate::federation::Domain::parse(&authority).map_err(|_| {
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

    /// Every setting can be given in the environment, lists and tables too.
    #[test]
    fn the_environment_gives_lists_and_tables() {
        let file = "public_url = \"http://localhost:5173\"\ndatabase_url = \"postgres://x\"\n\
                    nats_url = \"nats://x\"\nnats_auth_token = \"t\"\nvalkey_url = \"redis://x\"\n";
        let variables = [
            (
                "ASPEN_RATE_LIMITS__TRUSTED_PROXIES",
                "127.0.0.1, 10.0.0.0/8",
            ),
            (
                "ASPEN_RATE_LIMITS__ENDPOINTS",
                r#"{"POST /users": {"ip": {"requests": 2, "per_seconds": 60}}}"#,
            ),
            ("ASPEN_RATE_LIMITS__DEFAULT__USER", "false"),
            ("ASPEN_RATE_LIMITS__DEFAULT__IP__REQUESTS", "100"),
            ("ASPEN_RATE_LIMITS__DEFAULT__IP__PER_SECONDS", "60"),
            (
                "ASPEN_FEDERATION__DEVELOPMENT__EXTRA_ROOT_CERTIFICATES",
                "/a.pem,/b.pem",
            ),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
        let config = load_from(
            config::File::from_str(file, config::FileFormat::Toml),
            environment().source(Some(variables)),
        )
        .unwrap();
        assert_eq!(
            config.rate_limits.trusted_proxies,
            ["127.0.0.1", "10.0.0.0/8"]
        );
        let users = &config.rate_limits.endpoints["POST /users"]["ip"];
        assert!(
            matches!(users, LimitSetting::Limit(limit) if limit.requests == 2),
            "{users:?}"
        );
        assert!(matches!(
            config.rate_limits.default["user"],
            LimitSetting::Off(false)
        ));
        let ip = &config.rate_limits.default["ip"];
        assert!(
            matches!(ip, LimitSetting::Limit(limit) if limit.requests == 100),
            "{ip:?}"
        );
        assert_eq!(
            config.federation.development.extra_root_certificates.len(),
            2
        );
    }

    /// Android apps named in the environment are accepted as passkey origins.
    #[test]
    fn android_apps_become_passkey_origins() {
        let file = "public_url = \"https://chat.example.org\"\ndatabase_url = \"postgres://x\"\n\
                    nats_url = \"nats://x\"\nnats_auth_token = \"t\"\nvalkey_url = \"redis://x\"\n";
        let fingerprint = ["AB"; 32].join(":");
        let variables = [(
            "ASPEN_AUTH__ANDROID_APPS".to_string(),
            format!("org.example.fork={fingerprint}"),
        )]
        .into_iter()
        .collect();
        let config = load_from(
            config::File::from_str(file, config::FileFormat::Toml),
            environment().source(Some(variables)),
        )
        .unwrap();
        let apps = crate::passkey::android_apps(&config);
        assert_eq!(apps.len(), config.auth.android_apps.len());
        assert_eq!(apps[0].package, "org.example.fork");
        assert_eq!(apps[0].fingerprint_hex(), fingerprint);
        let webauthn = crate::passkey::relying_party(&config, "Example")
            .unwrap()
            .unwrap();
        let origins: Vec<String> = webauthn
            .get_allowed_origins()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(
            origins,
            [
                "https://chat.example.org/".to_string(),
                format!(
                    "android:apk-key-hash:{}",
                    base64::Engine::encode(&base64::prelude::BASE64_URL_SAFE_NO_PAD, [0xAB; 32])
                )
            ]
        );
    }

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
            derived("http://127.0.0.1:8000").unwrap(),
            ("http://127.0.0.1:8000".into(), None, None)
        );
        assert_eq!(
            derived("https://192-168-2-220.nip.io").unwrap(),
            (
                "https://192-168-2-220.nip.io".into(),
                some("192-168-2-220.nip.io"),
                some("192-168-2-220.nip.io")
            )
        );
        assert!(derived("http://192.168.2.220:8000").is_err());
        assert!(derived("http://chat.example.org").is_err());
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
        assert_eq!(config.limits.max_followed_dm_partners, 100);
        assert!(config.metrics.enabled);
        assert_eq!(config.event_queue_size, 512);
    }

    /// NATS signs the server in by token or as a user, and the settings give exactly one.
    #[test]
    fn nats_takes_a_token_or_a_user() {
        let config = |auth: &str| -> AspenConfig {
            config::Config::builder()
                .add_source(config::File::from_str(
                    &format!(
                        "public_url = \"http://localhost\"\ndatabase_url = \"postgres://x\"\n\
                         nats_url = \"nats://x\"\nvalkey_url = \"redis://x\"\n{auth}"
                    ),
                    config::FileFormat::Toml,
                ))
                .build()
                .unwrap()
                .try_deserialize()
                .unwrap()
        };
        let user = "[nats_user]\nuser = \"aspen\"\npassword = \"p\"";
        assert!(config("nats_auth_token = \"t\"").check_nats().is_ok());
        assert!(config(user).check_nats().is_ok());
        assert!(config("").check_nats().is_err());
        assert!(
            config(&format!("nats_auth_token = \"t\"\n{user}"))
                .check_nats()
                .is_err()
        );
    }

    /// Certificate files for a service are refused where its address would not use them, and a
    /// client certificate without its key; Valkey's password travels only encrypted, except to
    /// this machine; and `[nats]` takes `tls` and an http or https `monitor_url`, a NATS user
    /// being `[nats_user]`.
    #[test]
    fn tls_files_go_with_tls() {
        let parse = |toml: &str| -> Result<AspenConfig, String> {
            config::Config::builder()
                .add_source(config::File::from_str(
                    &format!(
                        "public_url = \"http://localhost\"\ndatabase_url = \"postgres://x\"\n\
                         nats_url = \"nats://x\"\nnats_auth_token = \"t\"\n{toml}"
                    ),
                    config::FileFormat::Toml,
                ))
                .build()
                .and_then(|built| built.try_deserialize())
                .map_err(|e| e.to_string())
        };
        let valkey = |toml: &str| parse(toml).unwrap().check_valkey();
        assert!(valkey("valkey_url = \"redis://localhost:6379\"").is_ok());
        assert!(valkey("valkey_url = \"redis://:pw@127.0.0.1:6379\"").is_ok());
        assert!(valkey("valkey_url = \"redis://valkey.internal:6379\"").is_ok());
        assert!(valkey("valkey_url = \"redis://:pw@valkey.internal:6379\"").is_err());
        assert!(valkey("valkey_url = \"rediss://:pw@valkey.internal:6379\"").is_ok());
        let ca = "[valkey.tls]\nca_file = \"/ca.pem\"";
        assert!(valkey(&format!("valkey_url = \"rediss://valkey.internal\"\n{ca}")).is_ok());
        assert!(valkey(&format!("valkey_url = \"redis://valkey.internal\"\n{ca}")).is_err());
        assert!(
            valkey(
                "valkey_url = \"rediss://valkey.internal\"\n[valkey.tls]\ncert_file = \"/c.pem\""
            )
            .is_err()
        );

        let nats = |toml: &str| {
            parse(&format!("valkey_url = \"redis://x\"\n{toml}"))
                .unwrap()
                .check_nats()
        };
        assert!(nats("[nats.tls]\ncert_file = \"/c.pem\"\nkey_file = \"/k.pem\"").is_ok());
        assert!(nats("[nats.tls]\nkey_file = \"/k.pem\"").is_err());
        assert!(
            parse("valkey_url = \"redis://x\"\n[nats]\nuser = \"a\"\npassword = \"p\"").is_err()
        );
        assert!(nats("[nats]\nmonitor_url = \"http://nats.internal:8222\"").is_ok());
        assert!(nats("[nats]\nmonitor_url = \"nats://nats.internal:4222\"").is_err());
        assert!(nats("[nats]\nmonitor_url = \"nats.internal:8222\"").is_err());

        let s3 = |toml: &str| {
            parse(&format!("valkey_url = \"redis://x\"\n[media.s3]\n{toml}"))
                .unwrap()
                .media
                .s3
                .check_tls()
        };
        assert!(s3("endpoint = \"https://s3.internal\"\ntls = { ca_file = \"/ca.pem\" }").is_ok());
        assert!(s3("endpoint = \"http://s3.internal\"\ntls = { ca_file = \"/ca.pem\" }").is_err());
        assert!(
            s3("endpoint = \"https://s3.internal\"\ntls = { cert_file = \"/c\", key_file = \"/k\" }")
                .is_err()
        );

        let email = |smtp_url: &str| {
            parse(&format!(
                "valkey_url = \"redis://x\"\n[email]\nfrom = \"a@example.org\"\n\
                 smtp_url = \"{smtp_url}\"\n[email.tls]\nca_file = \"/ca.pem\""
            ))
            .unwrap()
            .email
            .unwrap()
            .validate()
        };
        assert!(email("smtps://mail.internal").is_ok());
        assert!(email("smtp://mail.internal?tls=required").is_ok());
        assert!(email("smtp://mail.internal?tls=opportunistic").is_err());
        assert!(email("smtp://localhost:1025").is_err());
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

    /// SMTP credentials travel only encrypted, except to this machine.
    #[test]
    fn smtp_credentials_need_tls() {
        let check = EmailConfig::check_smtp_encrypted;
        assert!(check("smtp://localhost:1025").is_ok());
        assert!(check("smtp://mail.example.org:25").is_ok());
        assert!(check("smtps://user:pw@mail.example.org").is_ok());
        assert!(check("smtp://user:pw@mail.example.org?tls=required").is_ok());
        assert!(check("smtp://user:pw@127.0.0.1:1025").is_ok());
        assert!(check("smtp://user:pw@mail.example.org").is_err());
        assert!(check("smtp://user:pw@mail.example.org?tls=opportunistic").is_err());
        assert!(check("smtp://user@mail.example.org:587").is_err());
    }

    /// A public deployment refuses the credentials this repository publishes for development;
    /// development deployments keep them.
    #[test]
    fn a_public_deployment_refuses_development_credentials() {
        let config = |public_url: &str, extra: &str| -> AspenConfig {
            let mut config: AspenConfig = config::Config::builder()
                .add_source(config::File::from_str(
                    &format!(
                        "public_url = {public_url:?}\n\
                         database_url = \"postgres://aspen:own-password@db/aspen\"\n\
                         nats_url = \"nats://x\"\nnats_auth_token = \"own-token\"\n\
                         valkey_url = \"redis://x\"\n{extra}"
                    ),
                    config::FileFormat::Toml,
                ))
                .build()
                .unwrap()
                .try_deserialize()
                .unwrap();
            config.derive_from_public_url().unwrap();
            config
        };
        let own_s3 = "[media.s3]\naccess_key = \"own\"\nsecret_key = \"own-secret\"\n";
        let public = "https://chat.example.org";
        assert!(
            config(public, own_s3)
                .check_development_credentials()
                .is_ok()
        );
        // No key pair: credentials come from the environment or a role.
        assert!(config(public, "").check_development_credentials().is_ok());
        assert!(
            config(public, "[media.s3]\naccess_key = \"own\"\n")
                .media
                .s3
                .check_credentials()
                .is_err()
        );
        for development in ["http://localhost:5173", "https://alpha.localhost:8443"] {
            assert!(
                config(development, "")
                    .check_development_credentials()
                    .is_ok()
            );
        }
        let mut database = config(public, own_s3);
        database.database_url = "postgres://postgres:aspen_test@db:5432".to_string();
        assert!(database.check_development_credentials().is_err());
        database.database_url = "host=db user=postgres password=aspen_test".to_string();
        assert!(database.check_development_credentials().is_err());
        let mut nats = config(public, own_s3);
        nats.nats_auth_token = Some("aspen_test".to_string());
        assert!(nats.check_development_credentials().is_err());
        let mut s3 = config(public, own_s3);
        s3.media.s3.access_key = Some("GK484e56c38fb7e14b182bf47a".to_string());
        assert!(s3.check_development_credentials().is_err());
    }

    /// Development federation settings are only for deployments on this machine.
    #[test]
    fn federation_development_is_only_for_this_machine() {
        let config = |public_url: &str, development: &str| -> AspenConfig {
            let mut config: AspenConfig = config::Config::builder()
                .add_source(config::File::from_str(
                    &format!(
                        "public_url = {public_url:?}\ndatabase_url = \"postgres://x\"\n\
                         nats_url = \"nats://x\"\nnats_auth_token = \"t\"\n\
                         valkey_url = \"redis://x\"\n[federation.development]\n{development}"
                    ),
                    config::FileFormat::Toml,
                ))
                .build()
                .unwrap()
                .try_deserialize()
                .unwrap();
            config.derive_from_public_url().unwrap();
            config
        };
        let private = "allow_private_addresses = true";
        let roots = "extra_root_certificates = [\"ca.pem\"]";
        for local in ["https://alpha.localhost:8443", "http://localhost:5173"] {
            assert!(
                config(local, private)
                    .check_federation_development()
                    .is_ok()
            );
        }
        for elsewhere in ["https://chat.example.org", "https://192-168-2-220.nip.io"] {
            assert!(config(elsewhere, "").check_federation_development().is_ok());
            assert!(
                config(elsewhere, private)
                    .check_federation_development()
                    .is_err()
            );
            assert!(
                config(elsewhere, roots)
                    .check_federation_development()
                    .is_err()
            );
        }
    }

    /// A config written to a log gives none of its secrets away.
    #[test]
    fn debug_leaves_out_secrets() {
        let mut config: AspenConfig = config::Config::builder()
            .add_source(config::File::from_str(
                r#"
                public_url = "https://chat.example.org"
                database_url = "postgres://aspen:db-secret@db/aspen"
                nats_url = "nats://x"
                nats_auth_token = "nats-secret"
                valkey_url = "redis://:valkey-secret@valkey:6379"
                [nats_user]
                user = "aspen"
                password = "nats-user-secret"
                [media.s3]
                secret_key = "s3-secret"
                [email]
                from = "a@example.org"
                smtp_url = "smtps://mail:smtp-secret@mail.example.org"
                "#,
                config::FileFormat::Toml,
            ))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        config.derive_from_public_url().unwrap();
        let written = format!("{config:?}");
        for secret in [
            "db-secret",
            "nats-secret",
            "valkey-secret",
            "nats-user-secret",
            "s3-secret",
            "smtp-secret",
        ] {
            assert!(!written.contains(secret), "{secret} in {written}");
        }
        assert!(written.contains("chat.example.org"));
        assert!(written.contains("db/aspen"));
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
