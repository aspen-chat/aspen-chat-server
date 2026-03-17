use std::sync::LazyLock;

use serde::Deserialize;
use tokio::sync::RwLock;

#[derive(Clone, Debug, Deserialize)]
pub struct AspenConfig {
    #[serde(default = "default_event_queue_size")]
    pub event_queue_size: usize,
    pub database_url: String,
    pub nats_url: String,
    pub nats_auth_token: String,
}

pub fn default_event_queue_size() -> usize {
    512
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
