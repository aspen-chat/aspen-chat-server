//! `aspen-bench`: says whether a deployment is up to the job its operator has in mind.
//!
//! 1. `aspen-bench plan <profile> --run <tag>` writes the population to seed.
//! 2. `aspen-chat-server bench seed --plan plan.json --out manifest.json` writes it into the
//!    deployment's database.
//! 3. `aspen-bench run <profile> --manifest manifest.json --out <dir>` runs the workload and
//!    writes `report.json` and `report.html`, exiting 0 when every service level held and 1 when
//!    one did not. `--mode capacity` finds how many users the deployment holds instead;
//!    `--agents N` spreads the load over agents on other machines (`aspen-bench agent`).
//! 4. `aspen-chat-server bench purge --run <tag>` removes the population.
//!
//! `aspen-bench compare old.json new.json` fails when the new run is slower, for CI.

mod agent;
mod clock;
mod coordinator;
mod engine;
mod html;
mod profile;
mod report;
mod scenarios;
mod scrape;
mod stats;
mod user;
mod voice;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, Subcommand, ValueEnum};
use profile::Profile;
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser)]
#[command(about = "Aspen's benchmark: is a deployment up to the job?")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write the seed plan for a profile's population.
    Plan {
        /// A profile file, or the name of a built-in scenario.
        profile: String,
        /// Tags everything seeded, for `bench purge`.
        #[arg(long)]
        run: String,
        #[arg(long)]
        out: PathBuf,
    },
    /// Run a profile against its deployment.
    Run {
        profile: String,
        /// What `aspen-chat-server bench seed` wrote.
        #[arg(long)]
        manifest: PathBuf,
        /// Where to write `report.json` and `report.html`.
        #[arg(long)]
        out: PathBuf,
        #[arg(long, value_enum, default_value_t = Mode::Check)]
        mode: Mode,
        /// Agents on other machines to wait for; 0 plays every user in this process.
        #[arg(long, default_value_t = 0)]
        agents: u32,
        /// Where remote agents connect.
        #[arg(long, default_value = "0.0.0.0:7700")]
        listen: SocketAddr,
        /// Overrides the profile's `target.api`.
        #[arg(long)]
        api: Option<String>,
        /// Overrides the profile's `target.metrics`; repeatable.
        #[arg(long = "metrics")]
        metrics: Vec<String>,
    },
    /// Play users for a coordinator on another machine.
    Agent {
        /// The coordinator, `ws://host:7700`.
        #[arg(long)]
        coordinator: String,
        #[arg(long, default_value_t = default_agent_name())]
        name: String,
    },
    /// Compare two reports' steady phases; fails when the new one is slower.
    Compare {
        old: PathBuf,
        new: PathBuf,
        /// How much slower a p99 may be before it counts, as a share.
        #[arg(long, default_value_t = 0.1)]
        tolerance: f64,
    },
    /// Render a report as HTML again.
    Html {
        report: PathBuf,
        #[arg(long)]
        out: PathBuf,
    },
    /// List the built-in scenarios, or print one.
    Scenarios { name: Option<String> },
}

#[derive(Clone, Copy, ValueEnum)]
enum Mode {
    /// One run at the profile's load, judged against its service levels.
    Check,
    /// Raise the online share step by step until a service level breaks.
    Capacity,
}

fn default_agent_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .map(|h| h.trim().to_string())
        .unwrap_or_else(|_| "agent".into())
}

/// A profile from a file, or a built-in scenario by name.
fn load_profile(name: &str) -> Result<(Profile, String)> {
    let text = match scenarios::find(name) {
        Some(text) => text.to_string(),
        None => std::fs::read_to_string(name)
            .with_context(|| format!("could not read the profile {name}"))?,
    };
    let profile = Profile::from_toml(&text).map_err(|e| anyhow!("{name}: {e}"))?;
    Ok((profile, text))
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ASPEN_LOG")
                .unwrap_or_else(|_| "warn".into()),
        )
        .init();
    match run(Cli::parse()).await {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(2);
        }
    }
}

async fn run(cli: Cli) -> Result<i32> {
    match cli.command {
        Command::Plan { profile, run, out } => {
            let (profile, _) = load_profile(&profile)?;
            let plan = profile.seed_plan(&run);
            plan.validate(u32::MAX).map_err(|e| anyhow!(e))?;
            std::fs::write(&out, serde_json::to_vec(&plan)?)?;
            let memberships: usize = plan.communities.iter().map(|c| c.members.len()).sum();
            println!(
                "plan for {}: {} users, {} communities, {} memberships; seed it with\n  aspen-chat-server bench seed --plan {} --out manifest.json",
                profile.name,
                plan.users,
                plan.communities.len(),
                memberships,
                out.display()
            );
            Ok(0)
        }
        Command::Run {
            profile,
            manifest,
            out,
            mode,
            agents,
            listen,
            api,
            metrics,
        } => {
            let (mut profile, mut text) = load_profile(&profile)?;
            if api.is_some() || !metrics.is_empty() {
                if let Some(api) = api {
                    profile.target.api = api;
                }
                if !metrics.is_empty() {
                    profile.target.metrics = metrics;
                }
                // Agents read the profile as text, so the overrides go into it.
                text = toml::to_string(&profile)?;
            }
            let manifest: aspen_bench_protocol::Manifest = serde_json::from_slice(
                &std::fs::read(&manifest)
                    .with_context(|| format!("could not read {}", manifest.display()))?,
            )
            .context("not a manifest")?;
            if manifest.users.len() != profile.population.users as usize {
                bail!(
                    "the manifest has {} users but the profile's population has {}; seed the profile's own plan",
                    manifest.users.len(),
                    profile.population.users
                );
            }
            let mut links = if agents == 0 {
                vec![coordinator::local_agent()]
            } else {
                coordinator::remote_agents(listen, agents)
                    .await
                    .map_err(|e| anyhow!(e))?
            };
            let report = match mode {
                Mode::Check => coordinator::check(&profile, &text, &manifest, &mut links).await,
                Mode::Capacity => {
                    coordinator::capacity(&profile, &text, &manifest, &mut links).await
                }
            }
            .map_err(|e| anyhow!(e))?;
            std::fs::create_dir_all(&out)?;
            std::fs::write(out.join("report.json"), serde_json::to_vec_pretty(&report)?)?;
            std::fs::write(out.join("report.html"), html::render(&report))?;
            print!("{}", html::summary(&report));
            println!("report: {}", out.join("report.html").display());
            Ok(if report.verdict.pass { 0 } else { 1 })
        }
        Command::Agent { coordinator, name } => {
            agent::connect(&coordinator, name)
                .await
                .map_err(|e| anyhow!(e))?;
            Ok(0)
        }
        Command::Compare {
            old,
            new,
            tolerance,
        } => {
            let read = |path: &PathBuf| -> Result<report::Report> {
                serde_json::from_slice(&std::fs::read(path)?)
                    .with_context(|| format!("{} is not a report", path.display()))
            };
            let (old, new) = (read(&old)?, read(&new)?);
            let rows = report::compare(&old, &new, tolerance);
            if rows.is_empty() {
                bail!("the reports share no steady-phase measurements");
            }
            let mut worse = 0;
            println!("{:<55} {:>10} {:>10}", "p99 (ms)", "old", "new");
            for (name, before, after, regressed) in &rows {
                if *regressed {
                    worse += 1;
                }
                println!(
                    "{:<55} {:>10.1} {:>10.1}{}",
                    name,
                    before,
                    after,
                    if *regressed { "  slower" } else { "" }
                );
            }
            println!(
                "{worse} of {} measurements slower by more than {:.0}%",
                rows.len(),
                tolerance * 100.0
            );
            Ok(if worse > 0 { 1 } else { 0 })
        }
        Command::Html { report, out } => {
            let report: report::Report = serde_json::from_slice(&std::fs::read(&report)?)?;
            std::fs::write(&out, html::render(&report))?;
            Ok(0)
        }
        Command::Scenarios { name } => {
            match name {
                Some(name) => print!(
                    "{}",
                    scenarios::find(&name).ok_or_else(|| anyhow!("no scenario {name}"))?
                ),
                None => {
                    for (name, text) in scenarios::ALL {
                        let description = Profile::from_toml(text)
                            .map(|p| p.description)
                            .unwrap_or_default();
                        println!("{name:<18} {}", description.lines().next().unwrap_or(""));
                    }
                }
            }
            Ok(0)
        }
    }
}
