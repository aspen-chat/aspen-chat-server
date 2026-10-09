//! `import-diesel` — one-shot: copy `__diesel_schema_migrations` row
//! history into `__aspen_migrations` (mapping each Diesel `version` to
//! the registry id with the matching `YYYYMMDDHHMMSS` prefix) and drop
//! the Diesel bookkeeping table. Refuses to run if `__aspen_migrations`
//! is already populated, or if any Diesel row can't be mapped to a
//! registry entry.

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use diesel::sql_types::{BigInt, Nullable, Text, Timestamptz};
use diesel::{QueryableByName, sql_query};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl, SimpleAsyncConnection};

use super::{Migration, ensure_migrations_table};

/// One-shot: copy each `__diesel_schema_migrations` row into
/// `__aspen_migrations` (per-row, by name) and drop the Diesel
/// bookkeeping table. Exists so that a deployment which tracked its
/// history under Diesel's migration runner can adopt the in-house
/// migrator without re-running every migration.
///
/// Mapping: Diesel's `version` column stores a bare `YYYYMMDDHHMMSS`
/// timestamp (14 digits, no separator). Our registry IDs are
/// `YYYYMMDD_HHMMSS_slug`. We bridge the two by stripping the
/// underscore between the date and time portion of each registry ID
/// and using that 14-digit string as the lookup key — see
/// [`aspen_id_to_diesel_version`]. Every Diesel row must resolve to
/// exactly one registry entry; a Diesel row we can't map is refused.
/// The original `run_on` timestamp is preserved in `applied_at`.
///
/// Guardrails (all enforced inside one transaction for atomicity):
/// * `__diesel_schema_migrations` must exist; otherwise there is
///   nothing to import and we bail loudly rather than silently doing
///   nothing.
/// * `__aspen_migrations` must be empty. If the in-house runner has
///   already recorded work, importing Diesel state on top would
///   conflate two histories and we refuse.
/// * Every Diesel `version` must match some registry entry's
///   14-digit timestamp prefix. An unmappable row signals either a
///   dropped-from-registry migration or a corrupt Diesel row; we
///   won't drop history we can't reconstruct.
///
/// Up-front registry validation (outside the transaction, so bad
/// registry state is reported without touching the DB):
/// * Every registry ID must be convertible to a Diesel `version`
///   via the mechanical `YYYYMMDD_HHMMSS` prefix rule.
/// * Two registry IDs must not share the same 14-digit prefix; a
///   collision would make the mapping ambiguous.
pub async fn run_import_diesel(
    conn: &mut AsyncPgConnection,
    migrations: &[&dyn Migration],
) -> Result<()> {
    ensure_migrations_table(conn).await?;

    let by_diesel_version = build_diesel_version_index(migrations)?;

    conn.transaction::<_, anyhow::Error, _>(|conn| {
        async move {
            // `to_regclass` returns NULL instead of erroring when the
            // relation is missing, which is exactly the "does this
            // table exist?" probe we want and avoids catalog-schema
            // guesswork.
            let diesel_table: Option<RelationName> = sql_query(
                "SELECT to_regclass('__diesel_schema_migrations')::text AS name",
            )
            .get_result(conn)
            .await
            .context("failed to probe for __diesel_schema_migrations")?;
            if diesel_table
                .as_ref()
                .and_then(|r| r.name.as_deref())
                .is_none()
            {
                bail!(
                    "__diesel_schema_migrations does not exist; nothing to import"
                );
            }

            let aspen_count: RowCount =
                sql_query("SELECT COUNT(*) AS count FROM __aspen_migrations")
                    .get_result(conn)
                    .await
                    .context("failed to count existing __aspen_migrations rows")?;
            if aspen_count.count != 0 {
                bail!(
                    "__aspen_migrations already has {} row(s); refusing to import on top of existing history",
                    aspen_count.count,
                );
            }

            // `run_on` is a naive TIMESTAMP in Diesel's schema; cast to
            // TIMESTAMPTZ server-side (treating the stored value as
            // UTC, which is how Postgres hands out CURRENT_TIMESTAMP to
            // timestamp-without-tz columns) so we can bind directly to
            // chrono::DateTime<Utc>.
            let diesel_rows: Vec<DieselMigrationRow> = sql_query(
                "SELECT version, (run_on AT TIME ZONE 'UTC') AS applied_at \
                 FROM __diesel_schema_migrations \
                 ORDER BY run_on ASC, version ASC",
            )
            .load(conn)
            .await
            .context("failed to read __diesel_schema_migrations")?;

            // Resolve every Diesel row to a registry ID *before* we
            // write anything. An unmappable row aborts the transaction
            // with a clear error rather than a half-imported history.
            let mut resolved: Vec<(&'static str, DateTime<Utc>)> =
                Vec::with_capacity(diesel_rows.len());
            for row in &diesel_rows {
                let aspen_id = by_diesel_version
                    .get(row.version.as_str())
                    .copied()
                    .ok_or_else(|| {
                        anyhow!(
                            "__diesel_schema_migrations row {:?} has no matching registry entry; \
                             refusing to drop history we can't reconstruct",
                            row.version,
                        )
                    })?;
                resolved.push((aspen_id, row.applied_at));
            }

            for (aspen_id, applied_at) in &resolved {
                sql_query(
                    "INSERT INTO __aspen_migrations (version, applied_at) VALUES ($1, $2)",
                )
                .bind::<Text, _>(*aspen_id)
                .bind::<Timestamptz, _>(*applied_at)
                .execute(conn)
                .await
                .with_context(|| format!("failed to insert bookkeeping row for {aspen_id}"))?;
            }

            conn.batch_execute("DROP TABLE __diesel_schema_migrations")
                .await
                .context("failed to drop __diesel_schema_migrations")?;

            tracing::info!(
                imported = resolved.len(),
                "imported Diesel migration history and dropped __diesel_schema_migrations",
            );
            Ok(())
        }
        .scope_boxed()
    })
    .await
    .context("import-diesel failed; transaction rolled back")?;

    Ok(())
}

/// Map each registry entry's 14-digit Diesel-style timestamp prefix to
/// its registry ID. Errors on malformed IDs or prefix collisions — both
/// are registry integrity problems the caller must fix before a safe
/// import is possible.
fn build_diesel_version_index(
    migrations: &[&dyn Migration],
) -> Result<std::collections::HashMap<String, &'static str>> {
    let mut by_diesel_version: std::collections::HashMap<String, &'static str> =
        std::collections::HashMap::with_capacity(migrations.len());
    for m in migrations {
        let id = m.id();
        let diesel_version = aspen_id_to_diesel_version(id).ok_or_else(|| {
            anyhow!(
                "registry id {id:?} does not start with YYYYMMDD_HHMMSS; \
                 import-diesel requires the mechanical prefix mapping to succeed",
            )
        })?;
        if let Some(prev) = by_diesel_version.insert(diesel_version.clone(), id) {
            bail!(
                "registry has two migrations mapping to Diesel version {diesel_version}: \
                 {prev} and {id}",
            );
        }
    }
    Ok(by_diesel_version)
}

/// Convert `"20250503_010148_setup"` → `"20250503010148"`. Returns
/// `None` if the id doesn't start with the expected
/// `YYYYMMDD_HHMMSS` shape — validated at the byte level so a stray
/// non-digit produces a clean error instead of a silent mis-mapping.
pub(super) fn aspen_id_to_diesel_version(id: &str) -> Option<String> {
    let bytes = id.as_bytes();
    if bytes.len() < 15 {
        return None;
    }
    if !bytes[0..8].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if bytes[8] != b'_' {
        return None;
    }
    if !bytes[9..15].iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut out = String::with_capacity(14);
    out.push_str(&id[0..8]);
    out.push_str(&id[9..15]);
    Some(out)
}

#[derive(QueryableByName)]
struct RelationName {
    #[diesel(sql_type = Nullable<Text>)]
    name: Option<String>,
}

#[derive(QueryableByName)]
struct RowCount {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct DieselMigrationRow {
    #[diesel(sql_type = Text)]
    version: String,
    #[diesel(sql_type = Timestamptz)]
    applied_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    #[test]
    fn aspen_id_to_diesel_version_strips_underscore() {
        assert_eq!(
            aspen_id_to_diesel_version("20250503_010148_setup").as_deref(),
            Some("20250503010148"),
        );
        assert_eq!(
            aspen_id_to_diesel_version("20260420_000000_message_link_preview").as_deref(),
            Some("20260420000000"),
        );
    }

    #[test]
    fn aspen_id_to_diesel_version_rejects_malformed() {
        // Too short.
        assert!(aspen_id_to_diesel_version("20250503_01014").is_none());
        // Missing underscore separator between date and time.
        assert!(aspen_id_to_diesel_version("202505030101480setup").is_none());
        // Non-digit in the date portion.
        assert!(aspen_id_to_diesel_version("2025050A_010148_x").is_none());
        // Non-digit in the time portion.
        assert!(aspen_id_to_diesel_version("20250503_01014Z_x").is_none());
        assert!(aspen_id_to_diesel_version("").is_none());
    }

    /// Registry-time validation: every real migration ID must round-trip
    /// through the mapping cleanly. If this test ever fails, a new
    /// migration was added with an ID that `import-diesel` can't
    /// mechanically translate, which breaks the upgrade path for
    /// anyone still on Diesel's runner.
    #[test]
    fn every_registry_id_is_mappable() {
        for m in crate::MIGRATIONS {
            let id = m.id();
            assert!(
                aspen_id_to_diesel_version(id).is_some(),
                "registry id {id:?} is not convertible to a Diesel version; \
                 import-diesel would reject the whole registry",
            );
        }
    }

    /// Registry-time validation: no two IDs may collapse to the same
    /// Diesel prefix. A collision would make the mapping ambiguous and
    /// `build_diesel_version_index` would rightly refuse to proceed.
    #[test]
    fn registry_has_no_diesel_prefix_collisions() {
        let idx = build_diesel_version_index(crate::MIGRATIONS)
            .expect("registry must have a clean Diesel-version index");
        assert_eq!(idx.len(), crate::MIGRATIONS.len());
    }

    #[test]
    fn build_index_rejects_malformed_registry_id() {
        struct Bad;
        #[async_trait]
        impl Migration for Bad {
            fn id(&self) -> &'static str {
                "not-a-timestamp"
            }
            async fn up(&self, _: &mut AsyncPgConnection) -> Result<()> {
                unreachable!()
            }
            async fn down(&self, _: &mut AsyncPgConnection) -> Result<()> {
                unreachable!()
            }
        }
        let bad: &dyn Migration = &Bad;
        let err = build_diesel_version_index(&[bad]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("YYYYMMDD_HHMMSS"), "unexpected error: {msg}");
    }

    #[test]
    fn build_index_rejects_duplicate_prefix() {
        struct A;
        struct B;
        #[async_trait]
        impl Migration for A {
            fn id(&self) -> &'static str {
                "20250101_000000_first"
            }
            async fn up(&self, _: &mut AsyncPgConnection) -> Result<()> {
                unreachable!()
            }
            async fn down(&self, _: &mut AsyncPgConnection) -> Result<()> {
                unreachable!()
            }
        }
        #[async_trait]
        impl Migration for B {
            fn id(&self) -> &'static str {
                "20250101_000000_second"
            }
            async fn up(&self, _: &mut AsyncPgConnection) -> Result<()> {
                unreachable!()
            }
            async fn down(&self, _: &mut AsyncPgConnection) -> Result<()> {
                unreachable!()
            }
        }
        let a: &dyn Migration = &A;
        let b: &dyn Migration = &B;
        let err = build_diesel_version_index(&[a, b]).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("20250101000000"), "unexpected error: {msg}",);
    }
}
