//! `status` — print applied + pending migrations and flag orphaned
//! applied rows (i.e. bookkeeping rows whose id is no longer in the
//! registry).

use anyhow::{Result, bail};
use diesel_async::AsyncPgConnection;

use super::{AppliedMigration, Migration, ensure_migrations_table, list_applied};

/// Print applied + pending status. Non-mutating beyond table bootstrap.
pub async fn run_status(conn: &mut AsyncPgConnection, migrations: &[&dyn Migration]) -> Result<()> {
    ensure_migrations_table(conn).await?;
    let applied = list_applied(conn).await?;
    let applied_set: std::collections::HashSet<&str> =
        applied.iter().map(|r| r.version.as_str()).collect();

    println!("applied ({}):", applied.len());
    if applied.is_empty() {
        println!("  (none)");
    } else {
        for row in &applied {
            println!("  {}  {}", row.applied_at.to_rfc3339(), row.version);
        }
    }

    let pending: Vec<&&dyn Migration> = migrations
        .iter()
        .filter(|m| !applied_set.contains(m.id()))
        .collect();
    println!("pending ({}):", pending.len());
    if pending.is_empty() {
        println!("  (none)");
    } else {
        for m in &pending {
            println!("  {}", m.id());
        }
    }

    let registry_set: std::collections::HashSet<&str> = migrations.iter().map(|m| m.id()).collect();
    let orphans: Vec<&AppliedMigration> = applied
        .iter()
        .filter(|r| !registry_set.contains(r.version.as_str()))
        .collect();
    if !orphans.is_empty() {
        println!("orphan applied rows (in DB but not in registry):");
        for row in orphans {
            println!("  {}  {}", row.applied_at.to_rfc3339(), row.version);
        }
        bail!("registry is missing entries for applied migrations; cannot proceed safely");
    }

    Ok(())
}
