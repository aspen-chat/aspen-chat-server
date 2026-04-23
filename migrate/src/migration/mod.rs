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
/// `up.sql` / `down.sql` files. Future migrations that need real Rust
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
