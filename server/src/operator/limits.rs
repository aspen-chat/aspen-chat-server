//! `limits suspend`, `resume`, and `status`: rate limit suspensions (`aspen_limits::suspension`).

use super::operator;
use crate::aspen_config::AspenConfig;
use anyhow::{Context, Result, anyhow};
use aspen_limits::suspension::{self, Scope, Suspension};
use clap::{Subcommand, ValueEnum};
use std::time::Duration;

#[derive(Subcommand, Debug)]
pub enum LimitsCommand {
    /// Suspend rate limits until `--for` has passed. They come back by themselves.
    Suspend {
        /// How long, as `90s`, `30m`, `2h`, `1d`, or `1h 30m`; at most `max_suspension_seconds`.
        #[clap(long = "for", default_value = "2h")]
        duration: humantime::Duration,
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

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeArg {
    Networks,
    All,
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
            let duration = Duration::from(duration);
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
