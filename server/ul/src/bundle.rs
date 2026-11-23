//! The runtime UL carries: the bundled PostgreSQL (`pgsql/`) and the web client (`web-client/`),
//! a zstd-compressed tar appended to the binary by `scripts/package_ul.py`, which `cargo build`
//! never needs. It is unpacked once into the data directory's `runtime/<hash>/`, where every later
//! start finds it.
//!
//! The binary ends with a trailer: the archive's SHA-256, its length as a little-endian `u64`, and
//! [`MAGIC`]. The archive lies just before them. A runtime directory exists only once its archive
//! unpacked whole and hashed right, since it is unpacked elsewhere and renamed into place.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

/// Ends a binary carrying a runtime; `scripts/package_ul.py` writes the same bytes.
const MAGIC: &[u8; 8] = b"ASPENUL\x01";
const TRAILER: u64 = 32 + 8 + 8;
/// How much of the hash names a runtime's directory.
const NAME_BYTES: usize = 12;
/// Names the runtime started last, whose directory is kept when another replaces it, so that a
/// PostgreSQL major-version upgrade can still run the previous one's binaries.
const CURRENT: &str = "current";

struct Appended {
    offset: u64,
    length: u64,
    hash: [u8; 32],
}

/// The runtime `exe` carries, if it carries one.
fn appended(exe: &mut File) -> Result<Option<Appended>> {
    let size = exe.metadata()?.len();
    if size < TRAILER {
        return Ok(None);
    }
    exe.seek(SeekFrom::Start(size - TRAILER))?;
    let mut trailer = [0u8; TRAILER as usize];
    exe.read_exact(&mut trailer)?;
    let (hash, rest) = trailer.split_at(32);
    let (length, magic) = rest.split_at(8);
    if magic != MAGIC {
        return Ok(None);
    }
    let length = u64::from_le_bytes(length.try_into().expect("eight bytes"));
    let Some(offset) = (size - TRAILER).checked_sub(length) else {
        bail!("this binary's runtime is cut short: its trailer names more than the file holds");
    };
    Ok(Some(Appended {
        offset,
        length,
        hash: hash.try_into().expect("32 bytes"),
    }))
}

/// Hashes what is read through it.
struct Hashing<R> {
    inner: R,
    hasher: Sha256,
}

impl<R: Read> Read for Hashing<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buf)?;
        self.hasher.update(&buf[..read]);
        Ok(read)
    }
}

/// The directory of the runtime this binary carries, under `data`'s `runtime/`, unpacked there
/// first if it is not yet. Of the runtimes unpacked before, only the one started last is kept.
pub fn unpacked(data: &Path) -> Result<PathBuf> {
    let exe_path = std::env::current_exe().context("could not find this program's own file")?;
    let mut exe =
        File::open(&exe_path).with_context(|| format!("could not read {}", exe_path.display()))?;
    let Some(appended) = appended(&mut exe)? else {
        bail!(
            "this aspen-ul carries no runtime (the bundled PostgreSQL and web client): give one with \
             --runtime, or use a build scripts/package_ul.py made"
        );
    };
    let runtimes = data.join("runtime");
    std::fs::create_dir_all(&runtimes)?;
    let name = hex::encode(&appended.hash[..NAME_BYTES]);
    let target = runtimes.join(&name);
    if !target.is_dir() {
        unpack(&mut exe, &appended, &runtimes, &target)?;
    }
    keep_only(&runtimes, &name)?;
    Ok(target)
}

fn unpack(exe: &mut File, appended: &Appended, runtimes: &Path, target: &Path) -> Result<()> {
    tracing::info!(runtime = %target.display(), "unpacking the runtime");
    let partial = runtimes.join(format!(".unpacking-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&partial);
    std::fs::create_dir(&partial)?;
    let unpacked = (|| -> Result<()> {
        exe.seek(SeekFrom::Start(appended.offset))?;
        let mut source = Hashing {
            inner: (&mut *exe).take(appended.length),
            hasher: Sha256::new(),
        };
        {
            let decoder = ruzstd::decoding::StreamingDecoder::new(&mut source)
                .map_err(|e| anyhow::anyhow!("the runtime is not zstd: {e}"))?;
            let mut archive = tar::Archive::new(decoder);
            archive.set_preserve_permissions(true);
            archive.set_overwrite(false);
            // `unpack` refuses entries that would land outside `partial`.
            archive
                .unpack(&partial)
                .context("could not unpack the runtime")?;
        }
        std::io::copy(&mut source, &mut std::io::sink())?;
        if source.hasher.finalize().as_slice() != appended.hash {
            bail!("this binary's runtime does not match its hash: the file is damaged");
        }
        Ok(())
    })();
    if let Err(e) = unpacked {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(e);
    }
    match std::fs::rename(&partial, target) {
        Ok(()) => Ok(()),
        // Another aspen-ul (a command beside the server) unpacked the same runtime first.
        Err(_) if target.is_dir() => {
            let _ = std::fs::remove_dir_all(&partial);
            Ok(())
        }
        Err(e) => {
            Err(e).with_context(|| format!("could not move the runtime to {}", target.display()))
        }
    }
}

/// Removes every runtime but `name` and the one [`CURRENT`] named before it, then names `name`.
fn keep_only(runtimes: &Path, name: &str) -> Result<()> {
    let current = runtimes.join(CURRENT);
    let previous = std::fs::read_to_string(&current)
        .map(|text| text.trim().to_string())
        .ok();
    for entry in std::fs::read_dir(runtimes)? {
        let entry = entry?;
        let entry_name = entry.file_name();
        let Some(entry_name) = entry_name.to_str() else {
            continue;
        };
        let kept = entry_name == name
            || entry_name == CURRENT
            || Some(entry_name) == previous.as_deref()
            // Another aspen-ul unpacking now.
            || entry_name.starts_with(".unpacking-");
        if !kept && entry.file_type()?.is_dir() {
            tracing::info!(runtime = entry_name, "removing a runtime no longer used");
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    if previous.as_deref() != Some(name) {
        let written = runtimes.join(format!(".{CURRENT}-{}", std::process::id()));
        std::fs::write(&written, format!("{name}\n"))?;
        std::fs::rename(&written, &current)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_trailer(archive: &[u8], magic: &[u8; 8]) -> Vec<u8> {
        let mut file = b"\x7fELF binary".to_vec();
        file.extend_from_slice(archive);
        file.extend_from_slice(&Sha256::digest(archive));
        file.extend_from_slice(&(archive.len() as u64).to_le_bytes());
        file.extend_from_slice(magic);
        file
    }

    fn file_of(bytes: &[u8]) -> File {
        let path = std::env::temp_dir().join(format!(
            "aspen-ul-bundle-{}-{}",
            std::process::id(),
            uuid::Uuid::now_v7()
        ));
        std::fs::write(&path, bytes).unwrap();
        let file = File::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        file
    }

    #[test]
    fn the_trailer_names_the_archive_before_it() {
        let found = appended(&mut file_of(&with_trailer(b"archive", MAGIC)))
            .unwrap()
            .unwrap();
        assert_eq!((found.offset, found.length), (11, 7));
        assert_eq!(found.hash.as_slice(), Sha256::digest(b"archive").as_slice());
    }

    #[test]
    fn a_binary_without_the_magic_carries_nothing() {
        assert!(
            appended(&mut file_of(&with_trailer(b"archive", b"NOTASPEN")))
                .unwrap()
                .is_none()
        );
        assert!(appended(&mut file_of(b"short")).unwrap().is_none());
    }

    #[test]
    fn a_length_past_the_file_is_refused() {
        let mut bytes = with_trailer(b"archive", MAGIC);
        let at = bytes.len() - 16;
        bytes[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(appended(&mut file_of(&bytes)).is_err());
    }
}
