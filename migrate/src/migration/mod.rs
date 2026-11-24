//! Migration trait, the SQL-only helper, and the runner that drives the
//! `__aspen_migrations` bookkeeping table.
//!
//! Each migration runs in its own DB transaction. We deliberately do not
//! wrap the whole `up` command in a single transaction: some PostgreSQL
//! statements (`CREATE TYPE`, `CREATE INDEX CONCURRENTLY`, etc.) interact
//! awkwardly with surrounding transaction state, and per-migration
//! transactions are also what Diesel's own runner does, so the migration
//! authoring expectations stay the same.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use diesel::sql_types::{Text, Timestamptz};
use diesel::{QueryableByName, sql_query};
use diesel_async::{AsyncPgConnection, RunQueryDsl, SimpleAsyncConnection};

pub mod apply;
pub mod down;
pub mod import_diesel;
pub mod redo;
pub mod revert;
pub mod status;
pub mod up;

pub use apply::run_apply;
pub use down::run_down;
pub use import_diesel::run_import_diesel;
pub use redo::run_redo;
pub use revert::run_revert;
pub use status::run_status;
pub use up::run_up;

/// One migration step. Identified by a stable, human-readable string that
/// is also the row key in `__aspen_migrations`.
#[async_trait]
pub trait Migration: Send + Sync {
    fn id(&self) -> &'static str;
    async fn up(&self, conn: &mut AsyncPgConnection) -> Result<()>;
    async fn down(&self, conn: &mut AsyncPgConnection) -> Result<()>;
}

/// Helper for pure-DDL migrations whose bodies live in adjacent
/// `up.sql` / `down.sql` files. Migrations that need real Rust
/// work skip this helper and `impl Migration` directly.
pub struct SqlMigration {
    pub id: &'static str,
    pub up: &'static str,
    pub down: &'static str,
}

#[async_trait]
impl Migration for SqlMigration {
    fn id(&self) -> &'static str {
        self.id
    }

    async fn up(&self, conn: &mut AsyncPgConnection) -> Result<()> {
        // `batch_execute` accepts multi-statement SQL with no parameter
        // bindings, which is what every existing migration's up.sql /
        // down.sql is.
        conn.batch_execute(self.up)
            .await
            .with_context(|| format!("up.sql failed for migration {}", self.id))?;
        Ok(())
    }

    async fn down(&self, conn: &mut AsyncPgConnection) -> Result<()> {
        conn.batch_execute(self.down)
            .await
            .with_context(|| format!("down.sql failed for migration {}", self.id))?;
        Ok(())
    }
}

/// One row in `__aspen_migrations`.
#[derive(QueryableByName, Debug, Clone)]
pub struct AppliedMigration {
    #[diesel(sql_type = Text)]
    pub version: String,
    #[diesel(sql_type = Timestamptz)]
    pub applied_at: DateTime<Utc>,
}

/// The advisory lock every command that changes the schema holds, so that two runs against one
/// database, as when several servers start at once and each migrates first, take turns: the
/// second finds the first's migrations applied. Its value is arbitrary, and fixed for good.
const MIGRATION_LOCK: i64 = 0x4153_5045_4e4d_4947;

#[derive(QueryableByName)]
struct Locked {
    #[diesel(sql_type = diesel::sql_types::Bool)]
    locked: bool,
}

/// Takes the migration lock, waiting while another run holds it. It is held for as long as this
/// connection is open, whatever transactions come and go on it, and let go when it closes,
/// however the run ends.
///
/// That needs a session of PostgreSQL's own, so a connection through a pooler is refused before
/// the lock is asked for: the pooler would keep the lock on a server connection after this run
/// exits, where nothing lets it go, and every later run would wait for it without end.
pub async fn lock(conn: &mut AsyncPgConnection) -> Result<()> {
    match aspen_database::session(conn)
        .await
        .context("failed to tell whether the connection reaches PostgreSQL through a pooler")?
    {
        aspen_database::Session::Own => {}
        aspen_database::Session::Pooled { announced, serving } => anyhow::bail!(
            "the database connection goes through a connection pooler such as PgBouncer (it \
             announced process {announced}, and statements run in process {serving}). \
             aspen-migrate holds a lock for as long as its session lasts, which a pooler keeps \
             on its server connection after aspen-migrate exits, so that every later run waits \
             for it without end. Give aspen-migrate a direct connection to PostgreSQL with \
             --database-url or DATABASE_URL: the same address with the port PostgreSQL itself \
             listens on (often 5432, where PgBouncer's is often 6432)"
        ),
    }
    let taken = sql_query("SELECT pg_try_advisory_lock($1) AS locked")
        .bind::<diesel::sql_types::BigInt, _>(MIGRATION_LOCK)
        .get_result::<Locked>(conn)
        .await
        .context("failed to ask for the migration lock")?;
    if taken.locked {
        return Ok(());
    }
    tracing::info!(
        "another aspen-migrate is running against this database; waiting for it to finish"
    );
    sql_query("SELECT pg_advisory_lock($1)")
        .bind::<diesel::sql_types::BigInt, _>(MIGRATION_LOCK)
        .execute(conn)
        .await
        .context("failed to take the migration lock")?;
    Ok(())
}

/// Create the bookkeeping table on first run. Idempotent.
pub async fn ensure_migrations_table(conn: &mut AsyncPgConnection) -> Result<()> {
    conn.batch_execute(
        "CREATE TABLE IF NOT EXISTS __aspen_migrations (\
            version TEXT PRIMARY KEY, \
            applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\
        )",
    )
    .await
    .context("failed to create __aspen_migrations table")?;
    Ok(())
}

/// All applied migrations, oldest first.
pub async fn list_applied(conn: &mut AsyncPgConnection) -> Result<Vec<AppliedMigration>> {
    let rows: Vec<AppliedMigration> = sql_query(
        "SELECT version, applied_at FROM __aspen_migrations ORDER BY applied_at ASC, version ASC",
    )
    .load(conn)
    .await
    .context("failed to read __aspen_migrations")?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use crate::MIGRATIONS;
    use std::collections::HashSet;

    #[test]
    fn every_registered_migration_has_an_id_of_its_own() {
        let mut seen = HashSet::new();
        for migration in MIGRATIONS {
            assert!(seen.insert(migration.id()), "{} twice", migration.id());
        }
    }

    /// A hand-written `mod.rs` copied from another keeps the other's id unless it is changed,
    /// which is otherwise found only when its row meets the primary key after its `up` ran.
    #[test]
    fn every_registered_migration_is_named_as_its_module() {
        let modules: Vec<&str> = include_str!("../registry.rs")
            .lines()
            .filter_map(|line| {
                line.trim()
                    .strip_prefix("&migrations::m")?
                    .strip_suffix("::M,")
            })
            .collect();
        let ids: Vec<&str> = MIGRATIONS.iter().map(|migration| migration.id()).collect();
        assert_eq!(ids, modules);
    }
}
