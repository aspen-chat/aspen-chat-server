//! `voice-servers list`, `add`, `set`, and `remove`: the registry of voice servers
//! (`app::voice`), as Manage voice servers keeps it from the dashboard. `add` may be run again
//! with the same arguments, so a deployment's scripts can declare their servers.

use super::{database, operator, publisher};
use anyhow::{Context, Result, anyhow};
use aspen_app::aspen_config::AspenConfig;
use aspen_app::voice::{self, VoiceServer, VoiceServerChangeset};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum VoiceServersCommand {
    /// List every registered voice server.
    List,
    /// Register a voice server, or give the one already registered by that name this address
    /// and capacity.
    Add {
        /// What it is called, which the dashboard shows and the other commands name it by.
        name: String,
        /// The base URL clients open for signalling and measure latency against.
        #[clap(long)]
        url: String,
        /// The most participants it carries at once.
        #[clap(long)]
        capacity: u32,
    },
    /// Change a registered voice server.
    Set {
        name: String,
        #[clap(long)]
        url: Option<String>,
        #[clap(long)]
        capacity: Option<u32>,
        /// Whether new calls may start on it; calls already there go on either way.
        #[clap(long)]
        enabled: Option<bool>,
    },
    /// Remove a voice server that holds no calls.
    Remove { name: String },
}

fn print(server: &VoiceServer) {
    println!(
        "{}  {}  capacity {}{}",
        server.name,
        server.url,
        server.capacity,
        if server.enabled { "" } else { "  disabled" }
    );
}

fn capacity(capacity: u32) -> Result<i32> {
    i32::try_from(capacity).context("a voice server's capacity can be at most 2147483647")
}

pub async fn voice_servers(config: &AspenConfig, command: VoiceServersCommand) -> Result<()> {
    let fail = |e: aspen_app::Error| anyhow!("{e}");
    let mut conn = database(config).await?;
    let servers = voice::list_servers_in(&mut conn).await.map_err(fail)?;
    let named = |name: &str| {
        servers
            .iter()
            .find(|server| server.name == name)
            .ok_or_else(|| {
                anyhow!("no voice server is called {name}; `voice-servers list` lists them")
            })
    };
    match command {
        VoiceServersCommand::List => servers.iter().for_each(print),
        VoiceServersCommand::Add {
            name,
            url,
            capacity: wanted,
        } => {
            let server = match servers.iter().find(|server| server.name == name) {
                Some(existing) => {
                    let changes = VoiceServerChangeset {
                        name: None,
                        url: Some(url),
                        capacity: Some(capacity(wanted)?),
                        enabled: None,
                    };
                    voice::update_server_in(&mut conn, existing.id, changes)
                        .await
                        .map_err(fail)?
                }
                None => voice::create_server_in(&mut conn, name, url, capacity(wanted)?)
                    .await
                    .map_err(fail)?,
            };
            tracing::info!(
                name = server.name,
                url = server.url,
                operator = operator(),
                "registered a voice server"
            );
            print(&server);
        }
        VoiceServersCommand::Set {
            name,
            url,
            capacity: wanted,
            enabled,
        } => {
            let changes = VoiceServerChangeset {
                name: None,
                url,
                capacity: wanted.map(capacity).transpose()?,
                enabled,
            };
            let server = named(&name)?;
            let server =
                if changes.url.is_none() && changes.capacity.is_none() && changes.enabled.is_none()
                {
                    server.clone()
                } else {
                    voice::update_server_in(&mut conn, server.id, changes)
                        .await
                        .map_err(fail)?
                };
            tracing::info!(
                name = server.name,
                operator = operator(),
                "changed a voice server"
            );
            print(&server);
        }
        VoiceServersCommand::Remove { name } => {
            let id = named(&name)?.id;
            let publisher = publisher(config).await?;
            voice::delete_idle_server(&publisher, &mut conn, id)
                .await
                .map_err(fail)?;
            tracing::info!(name, operator = operator(), "removed a voice server");
            println!("removed {name}");
        }
    }
    Ok(())
}
