//! `redo` — down one then up: useful for iterating on the migration
//! registered last.

use anyhow::Result;
use diesel_async::AsyncPgConnection;

use super::{Migration, run_down, run_up};

pub async fn run_redo(conn: &mut AsyncPgConnection, migrations: &[&dyn Migration]) -> Result<()> {
    run_down(conn, migrations, 1).await?;
    run_up(conn, migrations).await?;
    Ok(())
}
