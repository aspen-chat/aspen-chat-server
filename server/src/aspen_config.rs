use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct AspenConfig {
    #[serde(default = "default_event_queue_size")]
    pub event_queue_size: usize,
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
/// images, presigns uploads, deletes objects). `public_base_url` is what
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
            region: default_media_s3_region(),
            bucket: default_media_s3_bucket(),
            access_key: default_media_s3_access_key(),
            secret_key: default_media_s3_secret_key(),
            public_base_url: default_media_s3_public_base_url(),
            upload_url_ttl_seconds: default_media_s3_upload_url_ttl_seconds(),
        }
    }
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

/// Loads or reloads the config.
pub fn load_config() -> Result<AspenConfig, config::ConfigError> {
    let loaded = config::Config::builder()
        // `ASPEN_DATABASE_URL` sets `database_url`; `ASPEN_VOICE__IDLE_SESSION_SECONDS` sets
        // `voice.idle_session_seconds`. The prefix separator is set explicitly because it would
        // otherwise follow the nesting separator and every flat key would need two underscores.
        .add_source(
            config::Environment::with_prefix("ASPEN")
                .prefix_separator("_")
                .separator("__"),
        )
        .add_source(config::File::new("aspen.toml", config::FileFormat::Toml))
        .build()?
        .try_deserialize::<AspenConfig>()?;
    Ok(loaded)
}
