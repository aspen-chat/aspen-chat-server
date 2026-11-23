//! `storage import`: a directory of files kept the way `[media.directory]` keeps them (an
//! `aspen-ul export`'s `media/`, or a directory deployment's own) uploaded into this deployment's
//! object storage, each with the type and disposition it is served with
//! (`app::media_store::import_directory`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::operator;
use anyhow::{Result, anyhow, bail};
use aspen_app::aspen_config::AspenConfig;
use aspen_app::media_store::{self, MediaStore};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum StorageCommand {
    /// Upload a directory's files into this deployment's object storage (`[media.s3]`), each
    /// with the type and disposition it is served with, checking each by its size once there.
    /// Files already in the bucket at their size are left as they are, so an import that stopped
    /// is finished by running it again.
    Import {
        /// An `aspen-ul export`, or a directory holding `objects/` and `meta/`.
        dir: PathBuf,
        /// How many files are uploaded at once.
        #[clap(long, default_value_t = 8)]
        concurrency: usize,
    },
}

/// The directory holding `objects/` and `meta/`: `dir` itself, or an export's `media/`.
fn files_in(dir: &Path) -> Result<PathBuf> {
    for candidate in [dir.join("media"), dir.to_path_buf()] {
        if candidate.join("objects").is_dir() {
            return Ok(candidate);
        }
    }
    bail!(
        "{} holds no files: give an aspen-ul export, or a directory holding objects/ and meta/",
        dir.display()
    )
}

pub async fn storage(config: &AspenConfig, command: StorageCommand) -> Result<()> {
    match command {
        StorageCommand::Import { dir, concurrency } => {
            let MediaStore::S3(to) = MediaStore::new(config).await.map_err(|e| anyhow!("{e}"))?
            else {
                bail!(
                    "this deployment keeps its files in a directory ([media.directory]); storage \
                     import uploads into object storage, [media.s3]"
                );
            };
            let files = files_in(&dir)?;
            let from = media_store::directory::Directory::open(
                &files,
                "https://import.invalid",
                Duration::from_secs(60),
            )
            .await
            .map_err(|e| anyhow!("could not read {}: {e}", files.display()))?;
            let started = std::time::Instant::now();
            let done = media_store::import_directory(&from, &to, concurrency, |_, done| {
                let seen = done.copied + done.present;
                if seen % 1000 == 0 {
                    eprintln!("{seen} files, {} MiB uploaded", done.bytes >> 20);
                }
            })
            .await
            .map_err(|e| anyhow!("the import stopped; run it again to finish it: {e}"))?;
            tracing::info!(
                copied = done.copied,
                present = done.present,
                bytes = done.bytes,
                from = %files.display(),
                operator = operator(),
                "imported files into object storage"
            );
            println!(
                "uploaded {} files ({} MiB) in {} s; {} were already there",
                done.copied,
                done.bytes >> 20,
                started.elapsed().as_secs(),
                done.present
            );
        }
    }
    Ok(())
}
