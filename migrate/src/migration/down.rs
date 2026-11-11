//! `down [--steps N]` — roll back the N applied migrations registered
//! last, the last of them first.

use anyhow::{Context, Result, anyhow, bail};
use diesel::sql_query;
use diesel::sql_types::Text;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

use super::{Migration, ensure_migrations_table, list_applied};

/// The `steps` applied migrations that come last in the registry, the
/// last first. The registry's order is the order `up` applies in and
/// the only order migrations have; when each was applied is not
/// consulted, since a clock set back between two runs, or a history
/// imported with another's timestamps, would put a later migration
/// before an earlier one it builds on. An applied migration the
/// registry does not hold has no place in that order, so nothing is
/// chosen while one exists.
fn last_applied<'a>(
    migrations: &'a [&'a dyn Migration],
    applied: &[String],
    steps: usize,
) -> Result<Vec<&'a dyn Migration>> {
    if let Some(unknown) = applied
        .iter()
        .find(|version| !migrations.iter().any(|m| m.id() == version.as_str()))
    {
        return Err(anyhow!(
            "applied migration {unknown} has no matching entry in the registry; \
             refusing to silently skip a corrupt-state rollback"
        ));
    }
    Ok(migrations
        .iter()
        .rev()
        .filter(|m| applied.iter().any(|version| version == m.id()))
        .take(steps)
        .copied()
        .collect())
}

pub async fn run_down(
    conn: &mut AsyncPgConnection,
    migrations: &[&dyn Migration],
    steps: usize,
) -> Result<()> {
    if steps == 0 {
        bail!("--steps must be at least 1");
    }
    ensure_migrations_table(conn).await?;
    let applied: Vec<String> = list_applied(conn)
        .await?
        .into_iter()
        .map(|row| row.version)
        .collect();
    let to_revert = last_applied(migrations, &applied, steps)?;

    if to_revert.is_empty() {
        tracing::info!("nothing to roll back");
        return Ok(());
    }

    for migration in to_revert {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SqlMigration;

    static FIRST: SqlMigration = SqlMigration {
        id: "20260102_000000_first",
        up: "",
        down: "",
    };
    static SECOND: SqlMigration = SqlMigration {
        id: "20260101_000000_second",
        up: "",
        down: "",
    };
    static THIRD: SqlMigration = SqlMigration {
        id: "20260103_000000_third",
        up: "",
        down: "",
    };

    fn chosen(applied: &[&str], steps: usize) -> Result<Vec<&'static str>> {
        let registry: [&dyn Migration; 3] = [&FIRST, &SECOND, &THIRD];
        let applied: Vec<String> = applied.iter().map(|id| id.to_string()).collect();
        Ok(last_applied(&registry, &applied, steps)?
            .into_iter()
            .map(|m| m.id())
            .collect())
    }

    #[test]
    fn down_takes_the_applied_migrations_registered_last() {
        // Whatever order the rows come in, which is by when each was applied.
        let all = [THIRD.id, FIRST.id, SECOND.id];
        assert_eq!(chosen(&all, 1).unwrap(), [THIRD.id]);
        assert_eq!(chosen(&all, 2).unwrap(), [THIRD.id, SECOND.id]);
        assert_eq!(chosen(&all, 9).unwrap(), [THIRD.id, SECOND.id, FIRST.id]);
        // One not applied is passed over, not counted.
        assert_eq!(chosen(&[SECOND.id, FIRST.id], 1).unwrap(), [SECOND.id]);
        assert_eq!(
            chosen(&[THIRD.id, FIRST.id], 2).unwrap(),
            [THIRD.id, FIRST.id]
        );
        assert!(chosen(&[], 1).unwrap().is_empty());
    }

    #[test]
    fn down_refuses_while_an_applied_migration_is_not_registered() {
        assert!(chosen(&[FIRST.id, "20260104_000000_elsewhere"], 1).is_err());
    }
}
