//! `bench seed` and `bench purge`: benchmark populations (`app::benchmark`).

use anyhow::{Context, Result, anyhow};
use aspen_app::aspen_config::AspenConfig;
use clap::Subcommand;

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

pub async fn bench(config: &AspenConfig, command: BenchCommand) -> Result<()> {
    let mut conn = aspen_app::database::connect(&config.database_url)
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
            let manifest = aspen_app::benchmark::seed(
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
            // Drawn for the run unless the plan gave one; the manifest holds it too.
            println!("password of every user of the run: {}", manifest.password);
        }
        BenchCommand::Purge { run } => {
            let media = aspen_app::media_store::MediaStore::new(config)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            let report = aspen_app::benchmark::purge(&mut conn, &media, &run)
                .await
                .map_err(|e| match e {
                    aspen_app::Error::Diesel(diesel::result::Error::NotFound) => {
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
