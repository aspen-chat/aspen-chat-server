//! `plugins install`, `list`, `show`, `settings`, `mode`, `order`, `enable`, `disable`,
//! `remove`, and `purge`: the deployment's plugins (`app::plugin::install`). Installing,
//! upgrading, removing, and purging are the terminal's alone, as is granting a plugin its
//! permissions, which the operator reads and accepts before it is installed.

use super::{database, operator, publisher};
use crate::app::events::{Publisher, noting, settle_in};
use crate::app::plugin::manifest::Manifest;
use crate::app::plugin::{Mode, PluginPermission, install};
use crate::aspen_config::AspenConfig;
use anyhow::{Context, Result, anyhow, bail};
use clap::Subcommand;
use std::io::{BufRead, Write};
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum PluginsCommand {
    /// Install a plugin, or upgrade the installed plugin of its id, from its manifest
    /// (`aspen-plugin.json`), after showing what it asks for. A new plugin is installed off.
    Install {
        /// The manifest; its `component` is found beside it.
        manifest: PathBuf,
        /// How far a new plugin reaches: `optIn` (communities turn it on) or `everywhere`.
        #[clap(long, default_value = "optIn")]
        mode: String,
        /// Grant the plugin `dms`, letting it read and act in DMs, when its manifest asks.
        #[clap(long)]
        grant_dms: bool,
        /// Accept what it asks for without asking.
        #[clap(long)]
        yes: bool,
    },
    /// List the installed plugins, removed ones too.
    List,
    /// Show what an installed plugin asks for and how it is configured.
    Show { id: String },
    /// Change a plugin's settings: a JSON object of settings by name, `null` restoring one's
    /// default.
    Settings { id: String, settings: String },
    /// Set how far a plugin reaches: `optIn` or `everywhere`.
    Mode { id: String, mode: String },
    /// Order the plugins, first to run first; those left out follow.
    Order { ids: Vec<String> },
    /// Turn a plugin on. It needs every required setting.
    Enable { id: String },
    /// Turn a plugin off everywhere.
    Disable { id: String },
    /// Remove a plugin: it stops running, its annotations go, and its account leaves every
    /// community. What it kept stays until it is purged.
    Remove {
        id: String,
        #[clap(long)]
        yes: bool,
    },
    /// Delete everything a removed plugin kept: its data, and its settings, the deployment's
    /// and every community's.
    Purge {
        id: String,
        #[clap(long)]
        yes: bool,
    },
}

/// What a permission lets a plugin do, for the operator deciding whether to grant it.
fn explain(permission: PluginPermission) -> &'static str {
    match permission {
        PluginPermission::MessagesRead => "read messages where it runs",
        PluginPermission::MessagesRewrite => "change what people write before it is saved",
        PluginPermission::MessagesRefuse => "refuse messages before they are saved",
        PluginPermission::MessagesAnnotate => "show notes beside messages",
        PluginPermission::UsersAnnotate => "show notes on people's profiles",
        PluginPermission::AttachmentsRead => "read the files attached to messages it is shown",
        PluginPermission::Dms => "read and act in DMs",
        PluginPermission::Network => "send what it sees to the hosts it lists",
        PluginPermission::Storage => "keep data of its own",
        PluginPermission::Routes => "answer requests at routes of its own",
        PluginPermission::Events => "send events to people's apps",
        PluginPermission::Act => "act through an account of its own where communities let it",
    }
}

fn parse_mode(mode: &str) -> Result<Mode> {
    mode.parse()
        .map_err(|_| anyhow!("{mode:?} is not a mode; use optIn or everywhere"))
}

fn confirm(question: &str) -> Result<bool> {
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes" | "Yes"))
}

fn text(manifest: &Manifest, key: &str) -> String {
    crate::app::plugin::render(
        &manifest.messages,
        &manifest.default_language,
        &manifest.default_language,
        &crate::app::plugin::PluginText {
            key: key.to_string(),
            args: Default::default(),
        },
    )
}

/// Prints what `manifest` is and asks for, marking what `granted` lacks as new.
fn describe(manifest: &Manifest, granted: Option<&std::collections::BTreeSet<PluginPermission>>) {
    println!(
        "{} {} ({})",
        text(manifest, &manifest.name),
        manifest.version,
        manifest.id
    );
    println!("  {}", text(manifest, &manifest.description));
    if let Some(author) = &manifest.author {
        println!("  by {author}");
    }
    if let Some(homepage) = &manifest.homepage {
        println!("  {homepage}");
    }
    println!("It asks to:");
    for permission in &manifest.permissions {
        let new = granted.is_some_and(|g| !g.contains(permission));
        println!(
            "  {}{permission}: {}",
            if new { "NEW " } else { "" },
            explain(*permission)
        );
    }
    if !manifest.hosts.is_empty() {
        println!("It may call: {}", manifest.hosts.join(", "));
    }
    if let Some(quota) = manifest.storage_quota {
        println!("It may keep up to {quota} bytes.");
    }
    if let Some(principal) = &manifest.principal {
        let permissions: Vec<String> = principal
            .permissions
            .iter()
            .map(|p| p.to_string())
            .collect();
        println!(
            "Its account, @{}, asks communities for: {}",
            principal.username,
            if permissions.is_empty() {
                "nothing".to_string()
            } else {
                permissions.join(", ")
            }
        );
    }
    println!("What it keeps: {}", text(manifest, &manifest.retention));
}

pub async fn plugins(config: &AspenConfig, command: PluginsCommand) -> Result<()> {
    let mut conn = database(config).await?;
    let publisher = publisher(config).await?;
    let (result, noted) = noting(run(&publisher, &mut conn, command)).await;
    settle_in(
        &publisher,
        &mut conn,
        config.voice.file_transfers,
        noted,
        result.is_err(),
    )
    .await;
    result
}

async fn announce(publisher: &Publisher) -> Result<()> {
    use crate::app::events::Publishing;
    crate::app::plugin::registry::announce(&publisher.nats().client(), None)
        .await
        .map_err(|e| anyhow!("{e}"))
}

async fn run(
    publisher: &Publisher,
    conn: &mut diesel_async::AsyncPgConnection,
    command: PluginsCommand,
) -> Result<()> {
    match command {
        PluginsCommand::Install {
            manifest: path,
            mode,
            grant_dms,
            yes,
        } => {
            let mode = parse_mode(&mode)?;
            let manifest: Manifest = serde_json::from_slice(
                &std::fs::read(&path).with_context(|| format!("could not read {path:?}"))?,
            )
            .with_context(|| format!("{path:?} is not a plugin manifest"))?;
            let wrong = manifest.check();
            if !wrong.is_empty() {
                bail!("the manifest is not valid:\n  {}", wrong.join("\n  "));
            }
            let component_path = path
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join(&manifest.component);
            let component = std::fs::read(&component_path)
                .with_context(|| format!("could not read the component, {component_path:?}"))?;
            crate::app::plugin::Plugins::new()
                .map_err(|e| anyhow!("{e}"))?
                .check_component(&component)
                .map_err(|e| anyhow!("the component is not a plugin this Aspen runs: {e}"))?;
            let existing = install::find(conn, &manifest.id).await.ok();
            describe(&manifest, existing.as_ref().map(|e| &e.granted));
            if manifest.asks(PluginPermission::Dms) && !grant_dms {
                println!(
                    "It asks to run in DMs, which it will not unless installed with --grant-dms."
                );
            }
            if let Some(existing) = &existing
                && !existing.removed
            {
                let more = manifest.more_than(&existing.granted);
                println!(
                    "This upgrades {} from {}{}.",
                    manifest.id,
                    existing.version,
                    if more.is_empty() {
                        String::new()
                    } else {
                        ", and asks for more than it was granted".to_string()
                    }
                );
            }
            if !yes && !confirm("Install it, granting what it asks?")? {
                bail!("not installed");
            }
            let outcome = install::install(publisher, conn, &manifest, &component, mode, grant_dms)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            announce(publisher).await?;
            tracing::info!(
                plugin = manifest.id,
                version = manifest.version,
                operator = operator(),
                "installed a plugin"
            );
            match outcome {
                install::Outcome::New => println!(
                    "installed {}; configure it with `plugins settings`, then turn it on with \
                     `plugins enable {}`",
                    manifest.id, manifest.id
                ),
                install::Outcome::Restored => println!(
                    "installed {} again, with whatever it kept; turn it on with `plugins enable {}`",
                    manifest.id, manifest.id
                ),
                install::Outcome::Upgraded { from } => {
                    println!(
                        "upgraded {} from {from} to {}",
                        manifest.id, manifest.version
                    )
                }
            }
        }
        PluginsCommand::List => {
            let plugins = install::list(conn, true)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            if plugins.is_empty() {
                println!("no plugins are installed");
            }
            for plugin in plugins {
                let state = if plugin.removed {
                    "removed"
                } else if plugin.enabled {
                    "on"
                } else {
                    "off"
                };
                println!(
                    "{}  {}  {}  {}  {}",
                    plugin.position, plugin.id, plugin.version, plugin.mode, state
                );
            }
        }
        PluginsCommand::Show { id } => {
            let plugin = install::find(conn, &id).await.map_err(|e| anyhow!("{e}"))?;
            describe(&plugin.manifest, None);
            let granted: Vec<String> = plugin.granted.iter().map(|p| p.to_string()).collect();
            println!("Granted: {}", granted.join(", "));
            println!(
                "Mode: {}; {}",
                plugin.mode,
                if plugin.enabled { "on" } else { "off" }
            );
            let (shown, secrets) =
                crate::app::plugin::settings::readable(&plugin.manifest.settings, &plugin.settings);
            println!("Settings: {}", serde_json::Value::Object(shown));
            if !secrets.is_empty() {
                println!("Secrets set: {}", secrets.join(", "));
            }
            println!("Storage: {} bytes", plugin.storage_bytes);
            println!(
                "Installed: {}",
                plugin.installed_at.format("%Y-%m-%d %H:%M UTC")
            );
        }
        PluginsCommand::Settings { id, settings } => {
            let patch: serde_json::Map<String, serde_json::Value> = serde_json::from_str(&settings)
                .context("settings must be a JSON object of settings by name")?;
            install::update(conn, &id, None, None, Some(&patch))
                .await
                .map_err(|e| anyhow!("{e}"))?;
            announce(publisher).await?;
            tracing::info!(
                plugin = id,
                operator = operator(),
                "changed a plugin's settings"
            );
            println!("changed {id}'s settings");
        }
        PluginsCommand::Mode { id, mode } => {
            let mode = parse_mode(&mode)?;
            install::update(conn, &id, None, Some(mode), None)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            announce(publisher).await?;
            tracing::info!(plugin = id, %mode, operator = operator(), "changed a plugin's mode");
            println!(
                "{id} now runs {}",
                match mode {
                    Mode::Everywhere => "in every community",
                    Mode::OptIn => "in the communities that turn it on",
                }
            );
        }
        PluginsCommand::Order { ids } => {
            install::order(conn, &ids)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            announce(publisher).await?;
            println!("ordered the plugins");
        }
        PluginsCommand::Enable { id } | PluginsCommand::Disable { id }
            if install::find(conn, &id).await.is_err() =>
        {
            bail!("no plugin {id} is installed");
        }
        PluginsCommand::Enable { id } => {
            install::update(conn, &id, Some(true), None, None)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            announce(publisher).await?;
            tracing::info!(plugin = id, operator = operator(), "turned a plugin on");
            println!("{id} is on");
        }
        PluginsCommand::Disable { id } => {
            install::update(conn, &id, Some(false), None, None)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            announce(publisher).await?;
            tracing::info!(plugin = id, operator = operator(), "turned a plugin off");
            println!("{id} is off");
        }
        PluginsCommand::Remove { id, yes } => {
            if !yes
                && !confirm(&format!(
                    "Remove {id}? It stops running, its notes go, and its account leaves every \
                     community."
                ))?
            {
                bail!("not removed");
            }
            install::remove(publisher, conn, &id)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            announce(publisher).await?;
            tracing::info!(plugin = id, operator = operator(), "removed a plugin");
            println!("removed {id}; what it kept stays until `plugins purge {id}`");
        }
        PluginsCommand::Purge { id, yes } => {
            if !yes
                && !confirm(&format!(
                    "Delete everything {id} kept? This cannot be undone."
                ))?
            {
                bail!("not purged");
            }
            install::purge(conn, &id)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            tracing::info!(plugin = id, operator = operator(), "purged a plugin");
            println!("purged {id}");
        }
    }
    Ok(())
}
