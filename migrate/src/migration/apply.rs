//! `apply <ids...>` — apply a selected subset of migrations by id,
//! in registry order, refusing unknown ids up-front so a typo can't
//! leave a half-applied subset behind.

use anyhow::{Context, Result, bail};
use diesel::sql_query;
use diesel::sql_types::Text;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

use super::{Migration, ensure_migrations_table, list_applied};

/// Apply a specific subset of registered migrations selected by id, in
/// registry order. Unknown ids are refused up-front — before any DB
/// work — so a typo can't leave a half-applied subset behind. Ids that
/// are already recorded in `__aspen_migrations` are silently skipped,
/// matching `run_up`'s "already applied" semantics; this makes `apply`
/// safely re-runnable and useful for cherry-picking a single pending
/// migration out of a larger pending set.
///
/// The requested set may be given in any order; we always iterate the
/// registry to preserve the chronological invariant the rest of the
/// runner depends on.
pub async fn run_apply(
    conn: &mut AsyncPgConnection,
    migrations: &[&dyn Migration],
    ids: &[String],
) -> Result<()> {
    if ids.is_empty() {
        bail!("apply requires at least one migration id");
    }

    let registry_set: std::collections::HashSet<&str> = migrations.iter().map(|m| m.id()).collect();
    let mut unknown: Vec<&str> = ids
        .iter()
        .map(String::as_str)
        .filter(|id| !registry_set.contains(id))
        .collect();
    if !unknown.is_empty() {
        unknown.sort();
        unknown.dedup();
        bail!(
            "unknown migration id(s): {}; refusing to apply any migrations",
            unknown.join(", "),
        );
    }

    let requested: std::collections::HashSet<&str> = ids.iter().map(String::as_str).collect();

    ensure_migrations_table(conn).await?;
    let applied: std::collections::HashSet<String> = list_applied(conn)
        .await?
        .into_iter()
        .map(|r| r.version)
        .collect();

    let mut applied_count = 0usize;
    let mut skipped_count = 0usize;
    for migration in migrations {
        let id = migration.id();
        if !requested.contains(id) {
            continue;
        }
        if applied.contains(id) {
            tracing::info!(migration = id, "already applied, skipping");
            skipped_count += 1;
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

    tracing::info!(
        applied = applied_count,
        already_applied = skipped_count,
        "apply finished",
    );
    Ok(())
}
