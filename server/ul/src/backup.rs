//! `backup` and `restore`: the whole deployment in a directory of its own, and back.
//!
//! A backup is a directory holding `database.dump` (`pg_dump`'s custom format, without owners or
//! privileges, so it restores as any user), `media/` (the files, when they are in the data
//! directory), `aspen-ul.toml`, `acme/` (the certificate and its account), and `backup.json`,
//! written last, so a directory without it is not a backup.
//!
//! A backup is made while the deployment runs. The files and the database are taken at one moment:
//! the database's snapshot is taken in a transaction kept open for the dump, and the files are
//! hard-linked into a staging directory beside them while removals of files are held back
//! (`media_store::directory::REMOVALS_LOCK`), so every file the snapshot names is among them.
//! Linking takes seconds; the dump and copying the files out take as long as they need, with
//! nothing held back. A file written after the snapshot may be among them too, and is then one no
//! row names.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::postgres::{self, Postgres};

/// What a backup's `backup.json` says of the backup's own shape; one this does not know is
/// refused.
const FORMAT: u32 = 1;
/// How many dumps made before applying migrations are kept, newest first.
const KEEP_BEFORE_MIGRATING: usize = 3;

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub format: u32,
    pub created_at: String,
    pub public_url: String,
    pub aspen_ul_version: String,
    pub postgres_major: String,
    /// The newest migration the database had applied.
    pub schema: Option<String>,
    /// Whether the files are in the backup (`directory`) or in object storage (`s3`), which a
    /// backup does not reach.
    pub files: Files,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Files {
    Directory,
    S3,
}

fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("a time formats")
}

/// A name for a file or directory from the present moment, sortable and safe in a path.
fn stamp() -> String {
    now().replace(':', "-")
}

/// Hard-links every file under `from` into `to`, or copies it where links cannot reach, keeping
/// the tree's shape.
fn link_tree(from: &Path, to: &Path, copy: bool) -> Result<u64> {
    let mut count = 0;
    if !from.exists() {
        return Ok(0);
    }
    let mut pending = vec![from.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let relative = directory.strip_prefix(from).expect("under from");
        std::fs::create_dir_all(to.join(relative))?;
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            let target = to.join(relative).join(entry.file_name());
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                if copy {
                    std::fs::copy(entry.path(), &target)?;
                } else {
                    std::fs::hard_link(entry.path(), &target).with_context(|| {
                        format!("could not link {} into the backup", entry.path().display())
                    })?;
                }
                count += 1;
            }
        }
    }
    Ok(count)
}

/// Moves `from` to `to`, copying when they are on different filesystems.
fn move_tree(from: &Path, to: &Path) -> Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    link_tree(from, to, true)?;
    std::fs::remove_dir_all(from)?;
    Ok(())
}

fn newest_migration(applied: &[aspen_migrate::migration::AppliedMigration]) -> Option<String> {
    applied.iter().map(|m| m.version.clone()).max()
}

/// Runs a PostgreSQL program of the runtime's, failing with what it wrote when it fails.
async fn run_tool(database: &Postgres, program: &str, args: &[&str]) -> Result<()> {
    let output = database
        .tool(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .with_context(|| format!("could not run the bundled {program}"))?;
    if !output.status.success() {
        bail!(
            "{program} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Makes a backup of the deployment at `data` in `out`, which must be empty or absent.
/// `database` is running, the deployment's or the command's own.
pub async fn backup(
    config: &crate::config::Config,
    data: &Path,
    database: &Postgres,
    out: &Path,
) -> Result<()> {
    if out.exists() && std::fs::read_dir(out)?.next().is_some() {
        bail!(
            "{} is not empty; give a directory of its own to each backup",
            out.display()
        );
    }
    std::fs::create_dir_all(out)?;
    // The dump holds the deployment's secrets (its federation key, its signing secrets), so the
    // backup is this user's alone.
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o700))?;
    }
    let out = std::fs::canonicalize(out)?;
    let url = database.url(postgres::DATABASE);
    let mut conn = aspen_app::database::connect(&url).await?;
    let schema = newest_migration(&aspen_migrate::migration::list_applied(&mut conn).await?);
    conn.batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .await?;
    #[derive(diesel::QueryableByName)]
    struct Snapshot {
        #[diesel(sql_type = diesel::sql_types::Text)]
        id: String,
    }
    let Snapshot { id: snapshot } = diesel::sql_query("SELECT pg_export_snapshot() AS id")
        .get_result(&mut conn)
        .await?;

    let files = if config.files_in_s3() {
        Files::S3
    } else {
        Files::Directory
    };
    let staged = match files {
        Files::S3 => None,
        Files::Directory => Some(stage_files(data).await?),
    };
    info!(backup = %out.display(), "dumping the database");
    let dumped = run_tool(
        database,
        "pg_dump",
        &[
            "--format=custom",
            "--no-owner",
            "--no-privileges",
            &format!("--snapshot={snapshot}"),
            &format!("--file={}", out.join("database.dump").display()),
            &format!("--dbname={url}"),
        ],
    )
    .await;
    // The snapshot is needed only until the dump has read it.
    let _ = conn.batch_execute("ROLLBACK").await;
    drop(conn);
    if let Err(e) = dumped {
        if let Some(staged) = &staged {
            let _ = std::fs::remove_dir_all(staged);
        }
        return Err(e);
    }
    if let Some(staged) = staged {
        info!("copying the files out");
        move_tree(&staged, &out.join("media"))?;
    }
    std::fs::copy(data.join("aspen-ul.toml"), out.join("aspen-ul.toml"))?;
    if data.join("acme").is_dir() {
        link_tree(&data.join("acme"), &out.join("acme"), true)?;
    }
    let manifest = Manifest {
        format: FORMAT,
        created_at: now(),
        public_url: config.public_url.clone(),
        aspen_ul_version: env!("CARGO_PKG_VERSION").to_string(),
        postgres_major: database.major().to_string(),
        schema,
        files,
    };
    let written = out.join("backup.json.partial");
    std::fs::write(&written, serde_json::to_vec_pretty(&manifest)?)?;
    std::fs::rename(&written, out.join("backup.json"))?;
    match files {
        Files::Directory => println!("backed up to {}", out.display()),
        Files::S3 => println!(
            "backed up to {}; the files are in object storage, which this backup does not \
             hold: keep them as the storage allows (versioning, or a copy of the bucket)",
            out.display()
        ),
    }
    Ok(())
}

/// Links the files into a staging directory beside them while removals are held back, returning
/// it.
async fn stage_files(data: &Path) -> Result<PathBuf> {
    let media = data.join("media");
    let backups = data.join("backups");
    std::fs::create_dir_all(&backups)?;
    for entry in std::fs::read_dir(&backups)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with(".staging-") {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    let staging = backups.join(format!(".staging-{}", std::process::id()));
    let lock_path = media.join(aspen_app::media_store::directory::REMOVALS_LOCK);
    let staged = staging.clone();
    tokio::task::spawn_blocking(move || -> Result<u64> {
        std::fs::create_dir_all(&media)?;
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)?;
        lock.lock()?;
        let mut linked = 0;
        for part in ["objects", "meta"] {
            linked += link_tree(&media.join(part), &staged.join(part), false)?;
        }
        drop(lock);
        Ok(linked)
    })
    .await??;
    Ok(staging)
}

/// Restores the backup at `from` into `data`, moving what the data directory held into
/// `replaced-<time>/` within it. The deployment must be stopped.
pub async fn restore(runtime: &Path, data: &Path, from: &Path) -> Result<()> {
    let manifest_path = from.join("backup.json");
    let manifest: Manifest = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .with_context(|| format!("{} holds no backup (no backup.json)", from.display()))?,
    )
    .with_context(|| format!("{} is not a backup's manifest", manifest_path.display()))?;
    if manifest.format != FORMAT {
        bail!(
            "this backup is of format {}, which this aspen-ul does not know; restore it with the \
             aspen-ul that made it ({})",
            manifest.format,
            manifest.aspen_ul_version
        );
    }
    let major = postgres::major_of(&runtime.join("pgsql").join("bin"))?;
    if manifest.postgres_major.parse::<u32>().ok() > Some(major) {
        bail!(
            "this backup's database is from PostgreSQL {}, newer than this aspen-ul's {major}; \
             restore it with an aspen-ul as new as the one that made it ({})",
            manifest.postgres_major,
            manifest.aspen_ul_version
        );
    }
    let aside = data.join(format!("replaced-{}", stamp()));
    for part in ["aspen-ul.toml", "postgres", "media", "acme"] {
        if data.join(part).exists() {
            std::fs::create_dir_all(&aside)?;
            std::fs::rename(data.join(part), aside.join(part))?;
        }
    }
    if aside.exists() {
        println!(
            "moved what {} held into {}",
            data.display(),
            aside.display()
        );
    }
    std::fs::copy(from.join("aspen-ul.toml"), data.join("aspen-ul.toml"))?;
    if from.join("acme").is_dir() {
        link_tree(&from.join("acme"), &data.join("acme"), true)?;
    }
    if manifest.files == Files::Directory {
        info!("copying the files in");
        let media = data.join("media");
        for part in ["objects", "meta"] {
            link_tree(&from.join("media").join(part), &media.join(part), true)?;
        }
    }
    let config = crate::config::Config::read(&data.join("aspen-ul.toml"))?;
    let sizes = postgres::Sizes::for_memory(crate::memory_mib(config.memory_mib)?);
    let mut database = Postgres::new(runtime, data)?;
    database.start(sizes).await?;
    info!("restoring the database");
    let restored = run_tool(
        &database,
        "pg_restore",
        &[
            "--no-owner",
            "--no-privileges",
            "--exit-on-error",
            "--single-transaction",
            &format!("--dbname={}", database.url(postgres::DATABASE)),
            &from.join("database.dump").display().to_string(),
        ],
    )
    .await;
    if let Some(running) = database.running() {
        running.stop()?;
    }
    restored?;
    println!(
        "restored the backup of {} made {}; start the deployment (sudo systemctl start aspen-ul)",
        manifest.public_url, manifest.created_at
    );
    Ok(())
}

/// When the database lacks migrations this binary holds and already has some, dumps it to
/// `backups/before-migrating-<time>.dump` first, keeping the newest few, so an upgrade whose
/// migrations go wrong can be undone with the binary before it.
pub async fn dump_before_migrating(data: &Path, database: &Postgres) -> Result<()> {
    let url = database.url(postgres::DATABASE);
    let mut conn = aspen_app::database::connect(&url).await?;
    aspen_migrate::migration::ensure_migrations_table(&mut conn).await?;
    let applied = aspen_migrate::migration::list_applied(&mut conn).await?;
    drop(conn);
    if applied.is_empty() {
        return Ok(());
    }
    let pending = aspen_migrate::registry::MIGRATIONS
        .iter()
        .filter(|migration| !applied.iter().any(|a| a.version == migration.id()))
        .count();
    if pending == 0 {
        return Ok(());
    }
    let backups = data.join("backups");
    std::fs::create_dir_all(&backups)?;
    let path = backups.join(format!("before-migrating-{}.dump", stamp()));
    info!(pending, dump = %path.display(), "dumping the database before migrating it");
    run_tool(
        database,
        "pg_dump",
        &[
            "--format=custom",
            "--no-owner",
            "--no-privileges",
            &format!("--file={}", path.display()),
            &format!("--dbname={url}"),
        ],
    )
    .await?;
    let mut dumps: Vec<PathBuf> = std::fs::read_dir(&backups)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| {
                    name.starts_with("before-migrating-") && name.ends_with(".dump")
                })
        })
        .collect();
    dumps.sort();
    let excess = dumps.len().saturating_sub(KEEP_BEFORE_MIGRATING);
    for old in &dumps[..excess] {
        std::fs::remove_file(old)?;
    }
    Ok(())
}
