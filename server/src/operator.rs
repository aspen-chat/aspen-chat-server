//! Commands an operator runs against a deployment with its own credentials, rather than through
//! the API, which every user may call under the Insanity.
//!
//! `limits suspend` lifts rate limits for a while (see `aspen_limits::suspension`), `limits
//! resume` restores them, and `limits status` says which is in force. `bench seed` writes a
//! benchmark population straight into the database and `bench purge` removes one
//! (`app::benchmark`).

use crate::aspen_config::AspenConfig;
use anyhow::{Context, Result, anyhow, bail};
use aspen_limits::suspension::{self, Scope, Suspension};
use clap::{Subcommand, ValueEnum};
use std::time::Duration;

#[derive(Subcommand, Debug)]
pub enum LimitsCommand {
    /// Suspend rate limits until `--for` has passed. They come back by themselves.
    Suspend {
        /// How long, as `90s`, `30m`, `2h`, or `1d`; at most `max_suspension_seconds`.
        #[clap(long = "for", default_value = "2h")]
        duration: String,
        /// `networks`: requests from `--network` skip the limits that count by address, and
        /// every other limit stays. `all`: every limit is lifted for everyone.
        #[clap(long, value_enum, default_value_t = ScopeArg::Networks)]
        scope: ScopeArg,
        /// A network (CIDR) or address exempted by `--scope networks`; repeatable.
        #[clap(long = "network")]
        networks: Vec<String>,
        /// Why, for the servers' logs.
        #[clap(long)]
        reason: String,
    },
    /// End a suspension now.
    Resume,
    /// Say whether a suspension is in force.
    Status,
}

#[derive(Subcommand, Debug)]
pub enum BenchCommand {
    /// Write the population a seed plan describes, and the manifest of what was made.
    Seed {
        /// The plan, as `aspen-bench plan` writes it (JSON).
        #[clap(long)]
        plan: std::path::PathBuf,
        /// Where to write the manifest (JSON).
        #[clap(long)]
        out: std::path::PathBuf,
    },
    /// Remove a run's population and everything that came to depend on it.
    Purge {
        #[clap(long)]
        run: String,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeArg {
    Networks,
    All,
}

/// Parses `90s`, `30m`, `2h`, `1d`, or a bare number of seconds.
pub fn parse_duration(text: &str) -> Result<Duration> {
    let text = text.trim();
    let (number, unit) = text.split_at(
        text.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len()),
    );
    let value: u64 = number
        .parse()
        .map_err(|_| anyhow!("{text:?} is not a duration such as 90s, 30m, 2h, or 1d"))?;
    let seconds = match unit {
        "" | "s" => value,
        "m" => value * 60,
        "h" => value * 3600,
        "d" => value * 86_400,
        _ => bail!("{text:?} is not a duration such as 90s, 30m, 2h, or 1d"),
    };
    Ok(Duration::from_secs(seconds))
}

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

fn describe(record: &Suspension, max: Duration) -> String {
    let ends = record.effective_until(max);
    let remaining = ends.saturating_sub(suspension::now_ms()) / 1000;
    let scope = match &record.scope {
        Scope::All => "every limit, for everyone".to_string(),
        Scope::Networks { networks } => {
            format!("limits by address, for {}", networks.join(", "))
        }
    };
    format!(
        "rate limits suspended: {scope}; {remaining}s left; by {}; reason: {}",
        record.by, record.reason
    )
}

pub async fn limits(config: &AspenConfig, command: LimitsCommand) -> Result<()> {
    let client = async_nats::connect_with_options(
        &config.nats_url,
        async_nats::ConnectOptions::new().token(config.nats_auth_token.clone()),
    )
    .await
    .context("could not connect to NATS")?;
    let store = suspension::bucket(client).await.map_err(|e| anyhow!(e))?;
    let max = Duration::from_secs(config.rate_limits.max_suspension_seconds);
    match command {
        LimitsCommand::Suspend {
            duration,
            scope,
            networks,
            reason,
        } => {
            let duration = parse_duration(&duration)?;
            let started_at = suspension::now_ms();
            let record = Suspension {
                started_at,
                until: started_at + u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
                scope: match scope {
                    ScopeArg::All => Scope::All,
                    ScopeArg::Networks => Scope::Networks { networks },
                },
                reason,
                by: operator(),
            };
            record.validate(max).map_err(|e| anyhow!(e))?;
            suspension::write(&store, &record)
                .await
                .map_err(|e| anyhow!(e))?;
            println!("{}", describe(&record, max));
        }
        LimitsCommand::Resume => {
            suspension::clear(&store).await.map_err(|e| anyhow!(e))?;
            println!("rate limits are in force");
        }
        LimitsCommand::Status => {
            match suspension::read(&store)
                .await
                .map_err(|e| anyhow!(e))?
                .filter(|record| suspension::now_ms() < record.effective_until(max))
            {
                Some(record) => println!("{}", describe(&record, max)),
                None => println!("rate limits are in force"),
            }
        }
    }
    Ok(())
}

pub async fn bench(config: &AspenConfig, command: BenchCommand) -> Result<()> {
    use diesel_async::AsyncConnection;
    let mut conn = diesel_async::AsyncPgConnection::establish(&config.database_url)
        .await
        .context("could not connect to the database")?;
    match command {
        BenchCommand::Seed { plan, out } => {
            let plan: aspen_bench_protocol::SeedPlan = serde_json::from_slice(
                &std::fs::read(&plan)
                    .with_context(|| format!("could not read {}", plan.display()))?,
            )
            .context("the plan is not a seed plan")?;
            let started = std::time::Instant::now();
            let manifest = crate::app::benchmark::seed(
                &mut conn,
                &plan,
                config.limits.max_communities_per_user,
            )
            .await
            .map_err(|e| anyhow!("{e}"))?;
            std::fs::write(&out, serde_json::to_vec(&manifest)?)
                .with_context(|| format!("could not write {}", out.display()))?;
            println!(
                "seeded run {}: {} users, {} communities in {:.1}s; manifest in {}",
                manifest.run,
                manifest.users.len(),
                manifest.communities.len(),
                started.elapsed().as_secs_f64(),
                out.display()
            );
        }
        BenchCommand::Purge { run } => {
            let media = crate::app::media_store::MediaStore::new(config)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            let report = crate::app::benchmark::purge(&mut conn, &media, &run)
                .await
                .map_err(|e| match e {
                    crate::app::Error::Diesel(diesel::result::Error::NotFound) => {
                        anyhow!("there is no benchmark run {run:?}")
                    }
                    other => anyhow!("{other}"),
                })?;
            let rows: usize = report.rows.values().sum();
            println!("purged run {run}: {rows} rows, {} files", report.objects);
            for (table, count) in &report.rows {
                println!("  {table}: {count}");
            }
            if !report.failed_objects.is_empty() {
                println!("files that could not be deleted:");
                for key in &report.failed_objects {
                    println!("  {key}");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_duration;
    use std::time::Duration;

    #[test]
    fn durations_parse_with_units() {
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86_400));
        assert_eq!(parse_duration("45").unwrap(), Duration::from_secs(45));
        assert!(parse_duration("2 hours").is_err());
        assert!(parse_duration("h").is_err());
    }
}
