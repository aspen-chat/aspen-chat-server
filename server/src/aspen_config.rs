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
}

impl Default for MediaS3Config {
    fn default() -> Self {
        Self {
            endpoint: default_media_s3_endpoint(),
            region: default_media_s3_region(),
            bucket: default_media_s3_bucket(),
            access_key: default_media_s3_access_key(),
            secret_key: default_media_s3_secret_key(),
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

/// Loads or reloads the config.
pub fn load_config() -> Result<AspenConfig, config::ConfigError> {
    let loaded = config::Config::builder()
        .add_source(config::Environment::with_prefix("ASPEN"))
        .add_source(config::File::new("aspen.toml", config::FileFormat::Toml))
        .build()?
        .try_deserialize::<AspenConfig>()?;
    Ok(loaded)
}
