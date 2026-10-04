//! Commands an operator runs against a deployment with its own credentials, rather than through
//! the API, for powers no account should hold.
//!
//! `limits suspend` lifts rate limits for a while (see `aspen_limits::suspension`), `limits
//! resume` restores them, and `limits status` says which is in force. `bench seed` writes a
//! benchmark population straight into the database and `bench purge` removes one
//! (`app::benchmark`). `admin grant` and `admin revoke` decide who may open the Administration
//! Dashboard, which nothing over the API can do. `invites create` makes a registration invite,
//! which is how an invite-only deployment gets its first account (`app::registration_invite`).
//! `federation` manages the directory of other deployments and this deployment's key
//! (`app::federation`), as Manage federation does from the dashboard, and can also replace the
//! key, which no account can. `settings` shows and changes the deployment's settings
//! (`app::deployment_settings`), and `voice-servers` keeps the registry of voice servers
//! (`app::voice`), as the dashboard does.

mod admin;
mod bench;
mod communities;
mod federation;
mod invites;
mod limits;
mod settings;
mod voice_servers;

pub use admin::{AdminCommand, admin};
pub use bench::{BenchCommand, bench};
pub use communities::{CommunitiesCommand, communities};
pub use federation::{FederationCommand, federation};
pub use invites::{InvitesCommand, invites};
pub use limits::{LimitsCommand, limits};
pub use settings::{SettingsCommand, settings};
pub use voice_servers::{VoiceServersCommand, voice_servers};

use crate::aspen_config::AspenConfig;
use anyhow::{Context, Result};

/// Who is running the command, for the servers' logs.
fn operator() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".into());
    let host = std::fs::read_to_string("/etc/hostname")
        .map(|h| h.trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    format!("{user}@{host}")
}

/// What an operator command announces its changes with, so connected clients and the servers'
/// event streams see them as they would the API's.
async fn publisher(config: &AspenConfig) -> Result<crate::app::events::Publisher> {
    crate::app::events::Publisher::connect(config)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))
        .context("could not connect to NATS, which announces the change; is it running?")
}

async fn database(config: &AspenConfig) -> Result<diesel_async::AsyncPgConnection> {
    use diesel_async::AsyncConnection;
    diesel_async::AsyncPgConnection::establish(&config.database_url)
        .await
        .context("could not connect to the database")
}
