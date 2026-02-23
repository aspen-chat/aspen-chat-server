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
    256
}

static CONFIG: LazyLock<RwLock<Option<AspenConfig>>> = LazyLock::new(|| RwLock::new(None));

/// Loads or reloads the config.
pub fn load_config() -> Result<(), config::ConfigError> {
    let loaded = config::Config::builder()
        .add_source(config::Environment::with_prefix("ASPEN"))
        .add_source(config::File::new("aspen.toml", config::FileFormat::Toml))
        .build()?
        .try_deserialize::<AspenConfig>()?;
    *CONFIG.blocking_write() = Some(loaded);
    Ok(())
}

/// Fetches the active config. Will panic if `load_config()` was not called at least once prior.
pub async fn aspen_config() -> AspenConfig {
    CONFIG
        .read()
        .await
        .as_ref()
        .expect("config was not yet loaded!")
        .clone()
}
