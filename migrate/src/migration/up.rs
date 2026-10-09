//! `up` — apply every registered migration that is not yet recorded
//! in `__aspen_migrations`, in registry order.

use anyhow::{Context, Result};
use diesel::sql_query;
use diesel::sql_types::Text;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

use super::{Migration, ensure_migrations_table, list_applied};

pub async fn run_up(conn: &mut AsyncPgConnection, migrations: &[&dyn Migration]) -> Result<()> {
    ensure_migrations_table(conn).await?;
    let applied: std::collections::HashSet<String> = list_applied(conn)
        .await?
        .into_iter()
        .map(|r| r.version)
        .collect();

    let mut applied_count = 0usize;
    for migration in migrations {
        let id = migration.id();
        if applied.contains(id) {
            continue;
        }
        tracing::info!(migration = id, "applying");
        conn.transaction::<_, anyhow::Error, _>(|conn| {
            async move {
                migration.up(conn).await?;
                sql_query("INSERT INTO __aspen_migrations (version) VALUES ($1)")
                    .bind::<Text, _>(id)
                    .execute(conn)
                    .await
                    .with_context(|| format!("failed to record migration {id}"))?;
                Ok(())
            }
            .scope_boxed()
        })
        .await
        .with_context(|| format!("migration {id} failed; transaction rolled back"))?;
        applied_count += 1;
    }

    if applied_count == 0 {
        tracing::info!("no pending migrations");
    } else {
        tracing::info!(count = applied_count, "applied migrations");
    }
    Ok(())
}
