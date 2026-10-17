//! `voice-servers list`, `add`, `set`, and `remove`: the registry of voice servers
//! (`app::voice`), as Manage voice servers keeps it from the dashboard. `add` may be run again
//! with the same arguments, so a deployment's scripts can declare their servers, and prints what
//! the voice server must be given: the id it reports as, in `voice_server.toml` and in its NATS
//! user's permissions.

use super::{database, operator, publisher};
use anyhow::{Context, Result, anyhow, bail};
use aspen_app::VoiceServerId;
use aspen_app::aspen_config::AspenConfig;
use aspen_app::voice::{self, VoiceServer, VoiceServerChangeset};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum VoiceServersCommand {
    /// List every registered voice server.
    List,
    /// Register a voice server, or give the one already registered by that name this address
    /// and capacity, and print the id to give it.
    Add {
        /// What it is called, which the dashboard shows and the other commands name it by.
        name: String,
        /// The base URL clients open for signalling and measure latency against.
        #[clap(long)]
        url: String,
        /// The most participants it carries at once.
        #[clap(long)]
        capacity: u32,
        /// The id to register it under, for a deployment whose configuration names its voice
        /// servers' ids before they are registered. Left out, one is made.
        #[clap(long)]
        id: Option<uuid::Uuid>,
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
        "{}  {}  capacity {}  id {}{}",
        server.name,
        server.url,
        server.capacity,
        server.id.0,
        if server.enabled { "" } else { "  disabled" }
    );
}

/// What the voice server `server` names must be given to report as it, after `add`.
fn print_setup(server: &VoiceServer) {
    let id = server.id.0;
    println!(
        "\nGive the voice server this id in voice_server.toml (or ASPEN_VOICE_SERVER_ID):\n\n    id = \"{id}\"\n"
    );
    println!(
        "Give it a NATS user of its own, allowed only what it needs: add this to the users of \
         NATS's authorization, with a password of your own, which its voice_server.toml gives \
         under [nats_user] (docs/operators/installing/6-voice-servers.md):\n"
    );
    print!(
        "{}",
        voice_protocol::control::nats_permissions(id).nats_user(&server.name, "PASSWORD")
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
            id,
        } => {
            let id = id.map(VoiceServerId::from);
            let server = match servers.iter().find(|server| server.name == name) {
                Some(existing) => {
                    // A voice server's NATS user and its join tokens name its id, so giving
                    // a registered server another would cut off the one running under it.
                    if let Some(id) = id.filter(|id| *id != existing.id) {
                        bail!(
                            "{name} is already registered with the id {}, not {}, and a \
                             registered server's id never changes; to give it another, remove \
                             it and add it again",
                            existing.id.0,
                            id.0
                        );
                    }
                    let changes = VoiceServerChangeset {
                        name: None,
                        url: Some(url),
                        capacity: Some(capacity(wanted)?),
                        enabled: None,
                    };
                    voice::update_server_in(config, &mut conn, existing.id, changes)
                        .await
                        .map_err(fail)?
                }
                None => {
                    if let Some(holder) = servers.iter().find(|server| Some(server.id) == id) {
                        bail!(
                            "the id {} is already {}'s; give {name} an id of its own",
                            holder.id.0,
                            holder.name
                        );
                    }
                    let id = id.unwrap_or_else(VoiceServerId::new);
                    voice::create_server_in(config, &mut conn, id, name, url, capacity(wanted)?)
                        .await
                        .map_err(fail)?
                }
            };
            tracing::info!(
                name = server.name,
                url = server.url,
                operator = operator(),
                "registered a voice server"
            );
            print(&server);
            print_setup(&server);
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
                    voice::update_server_in(config, &mut conn, server.id, changes)
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
