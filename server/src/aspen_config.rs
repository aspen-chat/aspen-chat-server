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
        .add_source(config::Environment::with_prefix("ASPEN"))
        .add_source(config::File::new("aspen.toml", config::FileFormat::Toml))
        .build()?
        .try_deserialize::<AspenConfig>()?;
    Ok(loaded)
}
