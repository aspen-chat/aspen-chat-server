//! `aspen-migrate` — Aspen's database migration CLI.
//!
//! Connection-string resolution, in order: `--database-url` CLI flag,
//! the `DATABASE_URL` environment variable (same convention
//! `diesel print-schema` uses), and finally the `database_url` field of
//! `aspen.toml` in the current working directory (same file the server
//! reads).

mod commands;

use std::path::Path;

use anyhow::{Context, Result, bail};
use aspen_migrate::{
    MIGRATIONS, run_apply, run_down, run_import_diesel, run_redo, run_revert, run_status, run_up,
};
use clap::{Parser, Subcommand};
use diesel_async::{AsyncConnection, AsyncPgConnection};
use serde::Deserialize;
use tracing_subscriber::EnvFilter;

const ASPEN_CONFIG_FILENAME: &str = "aspen.toml";

#[derive(Parser, Debug)]
#[command(name = "aspen-migrate", about = "Aspen database migrations")]
struct Cli {
    /// PostgreSQL connection string. Falls back to the DATABASE_URL
    /// environment variable, then to the `database_url` field of
    /// `aspen.toml` in the current working directory.
    #[arg(long, global = true)]
    database_url: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Apply every pending migration in registry order.
    Up,
    /// Apply only the named migrations (in registry order), skipping
    /// any that are already recorded. Unknown ids are refused before
    /// any DB work, so a typo can't leave a half-applied subset behind.
    Apply {
        /// Registry ids to apply, e.g. `20260409_000004_invite_table`.
        #[arg(required = true, num_args = 1..)]
        ids: Vec<String>,
    },
    /// Roll back the most recently applied migrations, newest first.
    Down {
        #[arg(long, default_value_t = 1)]
        steps: usize,
    },
    /// Roll back only the named migrations (in reverse registry order),
    /// skipping any that aren't currently applied. Unknown ids are
    /// refused before any DB work, so a typo can't leave a half-reverted
    /// subset behind.
    Revert {
        /// Registry ids to revert, e.g. `20260409_000004_invite_table`.
        #[arg(required = true, num_args = 1..)]
        ids: Vec<String>,
    },
    /// Roll back the most recently applied migration and reapply
    /// everything that's pending. Useful while authoring.
    Redo,
    /// Print applied + pending migrations.
    Status,
    /// One-shot: import `__diesel_schema_migrations` history into
    /// `__aspen_migrations` (positionally, by row count) and drop the
    /// Diesel bookkeeping table. Refuses to run if `__aspen_migrations`
    /// is already populated.
    ImportDiesel,
    /// Scaffold a new migration directory (does NOT edit
    /// migrations/mod.rs or registry.rs — those edits stay manual so
    /// they show up in code review).
    New { slug: String },
    /// Delete a migration: remove its `migrate/src/migrations/m<id>/`
    /// directory, its `pub mod` declaration in `migrations/mod.rs`, and
    /// its entry in `registry.rs`. Intended for discarding migrations
    /// that were authored locally but never shipped. Requires
    /// confirmation at the terminal (bypass with `-y`); the command
    /// never touches the database, on purpose — deleting a migration
    /// that has already run anywhere would orphan its
    /// `__aspen_migrations` row and leave that deployment unable to
    /// roll the change back.
    Delete {
        /// Migration id (e.g. `20260423_063751_asd`) or full module
        /// name (e.g. `m20260423_063751_asd`). Both forms resolve to
        /// the same directory; the `m`-prefixed form is what appears
        /// in `registry.rs` and on disk.
        name: String,
        /// Skip the interactive confirmation prompt. Use only when
        /// you're driving this command from a script you trust.
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,aspen_migrate=info")),
        )
        .init();

    let cli = Cli::parse();

    if let Cmd::New { slug } = &cli.cmd {
        return commands::new::scaffold_new(slug);
    }
    if let Cmd::Delete { name, yes } = &cli.cmd {
        return commands::delete::delete_migration(name, *yes);
    }

    let url = resolve_database_url(cli.database_url.as_deref())?;

    let mut conn = AsyncPgConnection::establish(&url)
        .await
        .with_context(|| format!("failed to connect to {url}"))?;

    match cli.cmd {
        Cmd::Up => run_up(&mut conn, MIGRATIONS).await?,
        Cmd::Apply { ids } => run_apply(&mut conn, MIGRATIONS, &ids).await?,
        Cmd::Down { steps } => run_down(&mut conn, MIGRATIONS, steps).await?,
        Cmd::Revert { ids } => run_revert(&mut conn, MIGRATIONS, &ids).await?,
        Cmd::Redo => run_redo(&mut conn, MIGRATIONS).await?,
        Cmd::Status => run_status(&mut conn, MIGRATIONS).await?,
        Cmd::ImportDiesel => run_import_diesel(&mut conn, MIGRATIONS).await?,
        Cmd::New { .. } | Cmd::Delete { .. } => unreachable!("handled above"),
    }

    Ok(())
}

/// Minimal projection of `aspen.toml`. Only `database_url` is needed
/// here; other fields are ignored. This intentionally does NOT share a
/// struct with `server::aspen_config::AspenConfig` because the migrator
/// must stay usable even when `aspen.toml` is missing mandatory
/// server-only fields (`nats_url`, `valkey_url`, etc.) that the server
/// requires but the migrator has no use for.
#[derive(Deserialize)]
struct MigrateConfig {
    database_url: Option<String>,
}

/// Resolve the connection string using CLI arg → `DATABASE_URL` env var
/// → `aspen.toml` in the current working directory. Returns an error
/// only if none of the three sources produces a value, or if
/// `aspen.toml` exists but cannot be parsed.
fn resolve_database_url(cli_arg: Option<&str>) -> Result<String> {
    if let Some(url) = cli_arg {
        return Ok(url.to_owned());
    }
    if let Ok(url) = std::env::var("DATABASE_URL")
        && !url.is_empty()
    {
        return Ok(url);
    }
    if let Some(url) = load_database_url_from_aspen_toml(Path::new(ASPEN_CONFIG_FILENAME))? {
        return Ok(url);
    }
    bail!(
        "no database URL found; pass --database-url, set DATABASE_URL, or add `database_url = \"...\"` to {ASPEN_CONFIG_FILENAME} in the current directory",
    );
}

/// Read `database_url` from the given `aspen.toml`. `Ok(None)` if the
/// file is absent or the key is missing. `Err` only if the file exists
/// but cannot be read or parsed — a malformed config file is a real
/// problem the user should see, not a silent fall-through to an
/// equally-confusing "no database URL found" message.
fn load_database_url_from_aspen_toml(path: &Path) -> Result<Option<String>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(e).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let parsed: MigrateConfig = toml::from_str(&contents)
        .with_context(|| format!("failed to parse {} as TOML", path.display()))?;
    Ok(parsed.database_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;

    #[test]
    fn reads_database_url_from_aspen_toml() {
        let dir = tempdir();
        let path = dir.join("aspen.toml");
        write_file(&path, "database_url = \"postgres://example/db\"\n");
        let url = load_database_url_from_aspen_toml(&path).unwrap();
        assert_eq!(url.as_deref(), Some("postgres://example/db"));
    }

    #[test]
    fn absent_file_yields_none() {
        let dir = tempdir();
        let path = dir.join("does-not-exist.toml");
        let url = load_database_url_from_aspen_toml(&path).unwrap();
        assert!(url.is_none());
    }

    #[test]
    fn missing_key_yields_none() {
        let dir = tempdir();
        let path = dir.join("aspen.toml");
        write_file(&path, "nats_url = \"nats://x\"\n");
        let url = load_database_url_from_aspen_toml(&path).unwrap();
        assert!(url.is_none());
    }

    #[test]
    fn ignores_unknown_fields() {
        // aspen.toml has plenty of fields the migrator doesn't care
        // about (nats_url, valkey_url, [media.s3]); reading it must not
        // fail on their presence.
        let dir = tempdir();
        let path = dir.join("aspen.toml");
        write_file(
            &path,
            "database_url = \"postgres://ok\"\n\
             nats_url = \"nats://x\"\n\
             valkey_url = \"redis://x\"\n\
             [media.s3]\n\
             endpoint = \"http://x\"\n",
        );
        let url = load_database_url_from_aspen_toml(&path).unwrap();
        assert_eq!(url.as_deref(), Some("postgres://ok"));
    }

    #[test]
    fn malformed_file_errors() {
        let dir = tempdir();
        let path = dir.join("aspen.toml");
        write_file(&path, "database_url = \n");
        let err = load_database_url_from_aspen_toml(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("failed to parse"), "unexpected error: {msg}");
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!("aspen-migrate-test-{}", std::process::id()));
        let unique = base.join(format!(
            "{}-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            rand_suffix(),
        ));
        std::fs::create_dir_all(&unique).unwrap();
        unique
    }

    fn write_file(path: &Path, contents: &str) {
        let mut f = std::fs::File::create(path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
    }

    fn rand_suffix() -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        std::thread::current().id().hash(&mut h);
        std::time::Instant::now().elapsed().as_nanos().hash(&mut h);
        h.finish()
    }
}
