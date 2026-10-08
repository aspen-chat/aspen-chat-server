//! `jobs list`, `retry`, and `cancel`: the background jobs (`app::jobs`) from the terminal.

use super::{database, operator};
use anyhow::{Result, bail};
use aspen_app::aspen_config::AspenConfig;
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum JobsCommand {
    /// List what the jobs are doing: those running, the next waiting, and the latest given up,
    /// at most 100, with how many of each there are.
    List,
    /// Try a job that was given up again, from where it stood.
    Retry {
        /// The job's id, as `jobs list` shows it.
        job: uuid::Uuid,
    },
    /// Delete a job, waiting or given up, so it never runs. A job running now finishes its
    /// step first.
    Cancel {
        /// The job's id, as `jobs list` shows it.
        job: uuid::Uuid,
    },
}

pub async fn jobs(config: &AspenConfig, command: JobsCommand) -> Result<()> {
    let mut conn = database(config).await?;
    match command {
        JobsCommand::List => {
            let overview = aspen_app::jobs::read_overview(&mut conn)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            let shown = |count: i64| {
                if count >= aspen_app::jobs::MAX_COUNTED {
                    format!("{}+", aspen_app::jobs::MAX_COUNTED)
                } else {
                    count.to_string()
                }
            };
            println!("running: {}", shown(overview.running_count));
            for job in &overview.running {
                println!(
                    "  {}  {:<24} {:<12} since {}",
                    job.id,
                    job.kind,
                    job.class().to_string(),
                    job.running_since.unwrap_or(job.due).to_rfc3339()
                );
            }
            for (class, count) in &overview.waiting_counts {
                println!("waiting, {class}: {}", shown(*count));
            }
            for job in &overview.waiting {
                println!(
                    "  {}  {:<24} {:<12} due {}  attempts {}",
                    job.id,
                    job.kind,
                    job.class().to_string(),
                    job.due.to_rfc3339(),
                    job.attempts
                );
            }
            println!("given up: {}", shown(overview.failed_count));
            for job in &overview.failed {
                println!(
                    "  {}  {:<24} at {}: {}",
                    job.id,
                    job.kind,
                    job.failed_at.unwrap_or(job.due).to_rfc3339(),
                    job.error.as_deref().unwrap_or("")
                );
            }
        }
        JobsCommand::Retry { job } => {
            if !aspen_app::jobs::retry(&mut conn, job)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?
            {
                bail!("no job given up has the id {job}");
            }
            tracing::info!(%job, operator = operator(), "retried a job");
            println!("job {job} will be tried again");
        }
        JobsCommand::Cancel { job } => {
            if !aspen_app::jobs::cancel(&mut conn, job)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?
            {
                bail!("no job has the id {job}");
            }
            tracing::info!(%job, operator = operator(), "cancelled a job");
            println!("job {job} is cancelled");
        }
    }
    Ok(())
}
