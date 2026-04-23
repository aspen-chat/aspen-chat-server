//! `down [--steps N]` — roll back the N most recently applied
//! migrations, newest first.

use anyhow::{Context, Result, anyhow, bail};
use diesel::sql_query;
use diesel::sql_types::Text;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

use super::{AppliedMigration, Migration, ensure_migrations_table, list_applied};

/// Roll back the most recently applied `steps` migrations, newest first.
pub async fn run_down(
    conn: &mut AsyncPgConnection,
    migrations: &[&dyn Migration],
    steps: usize,
) -> Result<()> {
    if steps == 0 {
        bail!("--steps must be at least 1");
    }
    ensure_migrations_table(conn).await?;
    let applied = list_applied(conn).await?;
    let to_revert: Vec<&AppliedMigration> = applied.iter().rev().take(steps).collect();

    if to_revert.is_empty() {
        tracing::info!("nothing to roll back");
        return Ok(());
    }

    for row in to_revert {
        let migration = migrations
            .iter()
            .find(|m| m.id() == row.version)
            .ok_or_else(|| {
                anyhow!(
                    "applied migration {} has no matching entry in the registry; \
                     refusing to silently skip a corrupt-state rollback",
                    row.version
                )
            })?;
        let id = migration.id();
        tracing::info!(migration = id, "reverting");
        conn.transaction::<_, anyhow::Error, _>(|conn| {
            async move {
                migration.down(conn).await?;
                sql_query("DELETE FROM __aspen_migrations WHERE version = $1")
                    .bind::<Text, _>(id)
                    .execute(conn)
                    .await
                    .with_context(|| format!("failed to delete bookkeeping row for {id}"))?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .with_context(|| format!("rollback of {id} failed; transaction rolled back"))?;
    }
    Ok(())
}
