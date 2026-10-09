//! `revert <ids...>` — roll back a selected subset of migrations by id.

use anyhow::{Context, Result, bail};
use diesel::sql_query;
use diesel::sql_types::Text;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

use super::{Migration, ensure_migrations_table, list_applied};

/// Roll back a specific subset of registered migrations selected by id,
/// in reverse registry order (newest first). Unknown ids are refused
/// up-front — before any DB work — so a typo can't leave a half-reverted
/// subset behind. Ids that are not currently recorded in
/// `__aspen_migrations` are silently skipped, mirroring `run_apply`'s
/// "already in the desired state" semantics; this makes `revert` safely
/// re-runnable.
///
/// The requested set may be given in any order; we always iterate the
/// registry in reverse so that if the set contains migrations with
/// dependency-like ordering, the newer one comes down before the older
/// one it was stacked on — the same invariant `run_down` relies on.
pub async fn run_revert(
    conn: &mut AsyncPgConnection,
    migrations: &[&dyn Migration],
    ids: &[String],
) -> Result<()> {
    if ids.is_empty() {
        bail!("revert requires at least one migration id");
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
            "unknown migration id(s): {}; refusing to revert any migrations",
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

    let mut reverted_count = 0usize;
    let mut skipped_count = 0usize;
    for migration in migrations.iter().rev() {
        let id = migration.id();
        if !requested.contains(id) {
            continue;
        }
        if !applied.contains(id) {
            tracing::info!(migration = id, "not applied, skipping");
            skipped_count += 1;
            continue;
        }
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
        reverted_count += 1;
    }

    tracing::info!(
        reverted = reverted_count,
        not_applied = skipped_count,
        "revert finished",
    );
    Ok(())
}
