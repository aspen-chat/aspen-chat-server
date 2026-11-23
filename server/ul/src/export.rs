//! Leaving Ultra Lite: `export`, what a full deployment takes from this one, and `move-to-s3`,
//! the files moved into object storage while staying on Ultra Lite.
//!
//! An export is a backup (`backup.rs`) made with the deployment stopped, so nothing changes after
//! it, with `export.json` beside it: what the full deployment needs to know of this one that the
//! database does not say. Everything else that must survive the move (the federation key, every
//! secret and session) is in the database.

use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use diesel_async::RunQueryDsl;
use serde::Serialize;

use crate::config::Config;
use crate::postgres::{self, Postgres};

/// What `export.json` says of its own shape.
const FORMAT: u32 = 1;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceServer {
    id: String,
    name: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: u32,
    created_at: String,
    /// The full deployment's `public_url`, exactly: passkeys, the federation domain other
    /// deployments pinned, identity providers' redirects, and mailed links all depend on it.
    public_url: String,
    /// The voice server Ultra Lite ran in itself, which the full deployment removes by its name
    /// (`voice-servers remove aspen-ul`) before registering its own.
    voice_server: Option<VoiceServer>,
    /// Whether the files are in the export (`directory`, its `media/`) or already in object
    /// storage (`s3`), whose bucket the full deployment then uses as it is.
    files: crate::backup::Files,
    /// The newest migration applied; the full deployment's `aspen-migrate up` applies the rest.
    schema: Option<String>,
    postgres_major: u32,
}

/// Writes an export of the stopped deployment at `data` into `out`, which must be empty or absent.
pub async fn export(config: &Config, data: &Path, database: &Postgres, out: &Path) -> Result<()> {
    let files = crate::backup::backup(config, data, database, out).await?;
    #[derive(diesel::QueryableByName)]
    struct Voice {
        #[diesel(sql_type = diesel::sql_types::Uuid)]
        id: uuid::Uuid,
    }
    let mut conn = aspen_app::database::connect(&database.url(postgres::DATABASE)).await?;
    let voice: Option<Voice> = diesel::sql_query("SELECT id FROM voice_server WHERE name = $1")
        .bind::<diesel::sql_types::Text, _>(crate::VOICE_NAME)
        .get_result(&mut conn)
        .await
        .map(Some)
        .or_else(|e| match e {
            diesel::result::Error::NotFound => Ok(None),
            e => Err(e),
        })?;
    let schema = aspen_migrate::migration::list_applied(&mut conn)
        .await?
        .into_iter()
        .map(|applied| applied.version)
        .max();
    let manifest = Manifest {
        format: FORMAT,
        created_at: crate::backup::now(),
        public_url: config.public_url.clone(),
        voice_server: voice.map(|voice| VoiceServer {
            id: voice.id.to_string(),
            name: crate::VOICE_NAME,
        }),
        files,
        schema,
        postgres_major: database.major(),
    };
    let out = std::fs::canonicalize(out)?;
    std::fs::write(
        out.join("export.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    println!(
        "exported to {}; docs/operators/ultra-lite/leaving.md says how a full deployment takes it",
        out.display()
    );
    Ok(())
}

/// Whether the data directory still holds files, which a deployment configured to keep them in
/// object storage would not serve.
pub fn files_left(data: &Path) -> Result<bool> {
    let objects = data.join("media").join("objects");
    let Ok(entries) = std::fs::read_dir(&objects) else {
        return Ok(false);
    };
    let mut pending: Vec<_> = entries.collect::<std::io::Result<_>>()?;
    while let Some(entry) = pending.pop() {
        if entry.file_type()?.is_dir() {
            pending.extend(std::fs::read_dir(entry.path())?.collect::<std::io::Result<Vec<_>>>()?);
        } else {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Uploads the data directory's files into the object storage `[server.media.s3]` names, then
/// moves `media/` aside to `media.moved-<time>/`. The deployment must be stopped.
pub async fn move_to_s3(
    config: &Config,
    server_config: &aspen_app::aspen_config::AspenConfig,
    data: &Path,
    concurrency: usize,
) -> Result<()> {
    if !config.files_in_s3() {
        bail!(
            "aspen-ul.toml names no object storage: add [server.media.s3] (docs/operators/\
             ultra-lite/configuration.md), then run this again"
        );
    }
    let media = data.join("media");
    if !files_left(data)? {
        println!("{} holds no files; nothing to move", media.display());
        return Ok(());
    }
    let aspen_app::media_store::MediaStore::S3(to) =
        aspen_app::media_store::MediaStore::new(server_config)
            .await
            .map_err(|e| anyhow!("{e}"))?
    else {
        bail!("[server.media.s3] was not taken as object storage");
    };
    let from = aspen_app::media_store::directory::Directory::open(
        &media,
        "https://move.invalid",
        std::time::Duration::from_secs(60),
    )
    .await
    .map_err(|e| anyhow!("{e}"))?;
    let done = aspen_app::media_store::import_directory(&from, &to, concurrency, |_, done| {
        let seen = done.copied + done.present;
        if seen % 1000 == 0 {
            eprintln!("{seen} files, {} MiB uploaded", done.bytes >> 20);
        }
    })
    .await
    .map_err(|e| {
        anyhow!(
            "the move stopped, and the files are all still here; run it again to finish it: {e}"
        )
    })?;
    let aside = data.join(format!("media.moved-{}", crate::backup::stamp()));
    std::fs::rename(&media, &aside)
        .with_context(|| format!("could not move {} aside", media.display()))?;
    println!(
        "uploaded {} files ({} MiB; {} were already there). The files the data directory held \
         are in {}: remove it once the deployment serves them from object storage",
        done.copied,
        done.bytes >> 20,
        done.present,
        aside.display()
    );
    Ok(())
}
