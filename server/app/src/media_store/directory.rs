//! [`MediaStore::Directory`](super::MediaStore::Directory): the deployment's files in a directory
//! of the machine its one server runs on (`[media.directory]`), served by that server at an origin
//! of their own, `public_base_url`, never the web client's (`api::media`).
//!
//! An object is a file under `objects/` named by its key, beside a record under `meta/` of the
//! type and disposition it is served with and its ETag, which a move to S3 gives each object it
//! copies. Each is written to `tmp/` and renamed into place, so a reader never meets half a file,
//! and a key is read whole and refused unless each segment is one the server makes
//! ([`Directory::path_of`]), so no key reaches outside the directory.
//!
//! Upload URLs and the signed URLs of evidence are signed with a key this process makes when it
//! opens the store and holds in memory alone: only this server answers them, and a restart ends
//! only those still in use, within their few minutes.

use std::path::{Path, PathBuf};
use std::time::{Duration as StdDuration, SystemTime};

use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::{PresignedUpload, Promotion, STAGING_GRACE, Served, UPLOAD_PREFIX, upload_key};
use crate::CHACHA_RNG;

/// The type an object is served with when its record does not say.
const UNKNOWN_TYPE: &str = "application/octet-stream";

/// A file in the store's directory that every removal of an object (deleting it, or moving it to
/// another key) holds a shared lock on while it removes, and that `aspen-ul backup` locks
/// exclusively while it links the objects its database snapshot may name, so none of them is
/// removed between the snapshot and the links. Links take seconds where copying could take
/// hours, so removals wait no longer than that.
pub const REMOVALS_LOCK: &str = "removals.lock";

/// What is kept beside an object.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub content_type: String,
    pub disposition: Option<String>,
    /// Changes each time the object is written, so a client's cached copy is told apart.
    pub e_tag: String,
}

/// Why a signed request is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Its signature is not this server's, or it was changed.
    Unsigned,
    /// Its time has passed.
    Expired,
    /// It names no key this store makes.
    BadKey,
    /// Its body is not the type, or not the length, it was signed for.
    NotAsSigned,
}

#[derive(Clone)]
pub struct Directory {
    root: PathBuf,
    public_base_url: String,
    upload_url_ttl: StdDuration,
    signing_key: [u8; 32],
}

fn io(e: std::io::Error) -> crate::Error {
    crate::Error::Io(e)
}

fn unix_seconds(at: SystemTime) -> u64 {
    at.duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Whether `segment` is one a key the server makes may hold: letters, digits, `.`, `_`, and `-`,
/// beginning with a letter or digit, so never `.` or `..`.
fn segment_allowed(segment: &str) -> bool {
    segment
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

impl Directory {
    /// Opens the store at `root`, making its directories, readable by this user alone, when they
    /// are not there.
    pub async fn open(
        root: &Path,
        public_base_url: &str,
        upload_url_ttl: StdDuration,
    ) -> crate::Result<Self> {
        for directory in ["objects", "meta", "tmp"] {
            let path = root.join(directory);
            tokio::fs::create_dir_all(&path).await.map_err(io)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                    .await
                    .map_err(io)?;
            }
        }
        Ok(Self {
            root: root.to_path_buf(),
            public_base_url: public_base_url.trim_end_matches('/').to_string(),
            upload_url_ttl,
            signing_key: CHACHA_RNG.with(|rng| rng.borrow_mut().random()),
        })
    }

    /// Where `key`'s object and record are, or a refusal for a key the server does not make.
    fn path_of(&self, key: &str) -> Result<(PathBuf, PathBuf), Refusal> {
        let segments: Vec<&str> = key.split('/').collect();
        if segments.is_empty() || !segments.iter().all(|segment| segment_allowed(segment)) {
            return Err(Refusal::BadKey);
        }
        let mut object = self.root.join("objects");
        let mut record = self.root.join("meta");
        for segment in &segments {
            object.push(segment);
            record.push(segment);
        }
        record.set_extension("json");
        Ok((object, record))
    }

    fn paths(&self, key: &str) -> crate::Result<(PathBuf, PathBuf)> {
        self.path_of(key).map_err(|_| {
            crate::Error::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{key} is not a key the media store makes"),
            ))
        })
    }

    /// A file in `tmp/` no other write uses.
    fn scratch(&self) -> PathBuf {
        let name: u128 = CHACHA_RNG.with(|rng| rng.borrow_mut().random());
        self.root.join("tmp").join(format!("{name:032x}"))
    }

    async fn write_record(&self, path: &Path, record: &Record) -> crate::Result<()> {
        let scratch = self.scratch();
        tokio::fs::write(&scratch, serde_json::to_vec(record)?)
            .await
            .map_err(io)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        tokio::fs::rename(&scratch, path).await.map_err(io)
    }

    /// Moves the file at `scratch` into place as `key`, served as `served`.
    async fn install(&self, scratch: &Path, key: &str, served: &Served) -> crate::Result<()> {
        let (object, record) = self.paths(key)?;
        let e_tag: u128 = CHACHA_RNG.with(|rng| rng.borrow_mut().random());
        self.write_record(
            &record,
            &Record {
                content_type: served.content_type.clone(),
                disposition: served.disposition.clone(),
                e_tag: format!("{e_tag:032x}"),
            },
        )
        .await?;
        if let Some(parent) = object.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        tokio::fs::rename(scratch, &object).await.map_err(io)
    }

    /// What is kept beside `key`'s object; `None` when there is no record.
    pub async fn record(&self, key: &str) -> crate::Result<Option<Record>> {
        let (_, record) = self.paths(key)?;
        match tokio::fs::read(&record).await {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(e)),
        }
    }

    /// Where `key`'s object is and what it is served as, for serving it; `None` when there is none.
    pub async fn object(&self, key: &str) -> crate::Result<Option<(PathBuf, Record)>> {
        let (object, _) = self.paths(key)?;
        match tokio::fs::metadata(&object).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io(e)),
        }
        let record = self.record(key).await?.unwrap_or_else(|| Record {
            content_type: UNKNOWN_TYPE.to_string(),
            disposition: None,
            e_tag: String::new(),
        });
        Ok(Some((object, record)))
    }

    pub(crate) async fn put_bytes(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> crate::Result<()> {
        let scratch = self.scratch();
        tokio::fs::write(&scratch, bytes).await.map_err(io)?;
        self.install(
            &scratch,
            key,
            &Served {
                content_type: content_type.to_string(),
                disposition: None,
            },
        )
        .await
    }

    fn sign(&self, purpose: &str, key: &str, parts: &[&str], expires: u64) -> String {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.signing_key).expect("HMAC takes any key length");
        for part in [purpose, key]
            .into_iter()
            .chain(parts.iter().copied())
            .chain([expires.to_string().as_str()])
        {
            mac.update(&(part.len() as u64).to_be_bytes());
            mac.update(part.as_bytes());
        }
        data_encoding::HEXLOWER.encode(&mac.finalize().into_bytes())
    }

    fn verify(
        &self,
        purpose: &str,
        key: &str,
        parts: &[&str],
        expires: u64,
        signature: &str,
    ) -> Result<(), Refusal> {
        use subtle::ConstantTimeEq;
        self.path_of(key)?;
        let expected = self.sign(purpose, key, parts, expires);
        if !bool::from(expected.as_bytes().ct_eq(signature.as_bytes())) {
            return Err(Refusal::Unsigned);
        }
        if unix_seconds(SystemTime::now()) >= expires {
            return Err(Refusal::Expired);
        }
        Ok(())
    }

    fn url(&self, path: &str, query: &[(&str, &str)]) -> String {
        let mut url = url::Url::parse(&format!("{}/{path}", self.public_base_url))
            .expect("public_base_url is a URL, checked when the configuration is read");
        url.query_pairs_mut().extend_pairs(query);
        url.to_string()
    }

    pub(crate) async fn presign_upload(
        &self,
        key: &str,
        content_type: &str,
        length: u64,
    ) -> crate::Result<PresignedUpload> {
        let staging = upload_key(key);
        self.paths(&staging)?;
        let expires = unix_seconds(SystemTime::now() + self.upload_url_ttl);
        let length = length.to_string();
        let signature = self.sign("upload", &staging, &[content_type, &length], expires);
        Ok(PresignedUpload {
            url: self.url(
                &format!("upload/{staging}"),
                &[("expires", &expires.to_string()), ("signature", &signature)],
            ),
            expires_at: DateTime::from_timestamp(expires as i64, 0).unwrap_or_else(Utc::now),
        })
    }

    /// Takes the body of an upload to `staging` (an [`upload_key`]) that a URL of
    /// [`presign_upload`](Self::presign_upload) signed, refusing one whose type or length is
    /// not the one signed, or that holds more or less than its length.
    pub async fn receive_upload<B>(
        &self,
        staging: &str,
        expires: u64,
        signature: &str,
        content_type: &str,
        length: u64,
        mut body: B,
    ) -> crate::Result<Result<(), Refusal>>
    where
        B: futures_util::Stream<Item = std::io::Result<bytes::Bytes>> + Unpin,
    {
        use futures_util::StreamExt;
        if !staging.starts_with(UPLOAD_PREFIX) {
            return Ok(Err(Refusal::BadKey));
        }
        if let Err(refusal) = self.verify(
            "upload",
            staging,
            &[content_type, &length.to_string()],
            expires,
            signature,
        ) {
            return Ok(Err(refusal));
        }
        let scratch = self.scratch();
        let mut file = tokio::fs::File::create(&scratch).await.map_err(io)?;
        let mut written: u64 = 0;
        let taken = async {
            while let Some(chunk) = body.next().await {
                let chunk = chunk?;
                written = written.saturating_add(chunk.len() as u64);
                if written > length {
                    return Ok(false);
                }
                file.write_all(&chunk).await?;
            }
            file.flush().await?;
            Ok::<_, std::io::Error>(written == length)
        }
        .await;
        drop(file);
        match taken {
            Ok(true) => {}
            Ok(false) => {
                let _ = tokio::fs::remove_file(&scratch).await;
                return Ok(Err(Refusal::NotAsSigned));
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&scratch).await;
                return Err(io(e));
            }
        }
        self.install(
            &scratch,
            staging,
            &Served {
                content_type: content_type.to_string(),
                disposition: None,
            },
        )
        .await?;
        Ok(Ok(()))
    }

    pub(crate) async fn head_object(&self, key: &str) -> crate::Result<Option<u64>> {
        let (object, _) = self.paths(key)?;
        match tokio::fs::metadata(&object).await {
            Ok(metadata) => Ok(Some(metadata.len())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(e)),
        }
    }

    pub(crate) async fn copy_object_to(
        &self,
        key: &str,
        limit: u64,
        sink: &mut (impl AsyncWrite + Unpin),
    ) -> crate::Result<Option<u64>> {
        let (object, _) = self.paths(key)?;
        let file = tokio::fs::File::open(&object).await.map_err(io)?;
        if file.metadata().await.map_err(io)?.len() > limit {
            return Ok(None);
        }
        let mut body = file.take(limit.saturating_add(1));
        let written = tokio::io::copy(&mut body, sink).await.map_err(io)?;
        Ok((written <= limit).then_some(written))
    }

    pub(crate) async fn promote(
        &self,
        key: &str,
        max_bytes: u64,
        served: &Served,
    ) -> crate::Result<Promotion> {
        let (staging, staging_record) = self.paths(&upload_key(key))?;
        // Taken out of the staging key first, so what is weighed is what is installed, whatever
        // a second upload to the same URL writes meanwhile.
        let claimed = self.scratch();
        match tokio::fs::rename(&staging, &claimed).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(match self.head_object(key).await? {
                    Some(size) => Promotion::Promoted(size),
                    None => Promotion::NotUploaded,
                });
            }
            Err(e) => return Err(io(e)),
        }
        let _ = tokio::fs::remove_file(&staging_record).await;
        let size = tokio::fs::metadata(&claimed).await.map_err(io)?.len();
        if size > max_bytes {
            tokio::fs::remove_file(&claimed).await.map_err(io)?;
            return Ok(Promotion::TooLarge);
        }
        self.install(&claimed, key, served).await?;
        Ok(Promotion::Promoted(size))
    }

    pub(crate) fn staging_cutoff(&self) -> DateTime<Utc> {
        Utc::now()
            - Duration::from_std(self.upload_url_ttl + STAGING_GRACE).unwrap_or(Duration::hours(2))
    }

    pub(crate) async fn sweep_uploads(&self, older_than: DateTime<Utc>) -> crate::Result<usize> {
        let (uploads, _) = self.paths(UPLOAD_PREFIX.trim_end_matches('/'))?;
        let mut swept = 0;
        let mut directories = vec![uploads];
        while let Some(directory) = directories.pop() {
            let mut entries = match tokio::fs::read_dir(&directory).await {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(io(e)),
            };
            while let Some(entry) = entries.next_entry().await.map_err(io)? {
                let metadata = entry.metadata().await.map_err(io)?;
                if metadata.is_dir() {
                    directories.push(entry.path());
                    continue;
                }
                let modified: DateTime<Utc> = metadata.modified().map_err(io)?.into();
                if modified < older_than {
                    let key = entry
                        .path()
                        .strip_prefix(self.root.join("objects"))
                        .ok()
                        .and_then(|relative| relative.to_str())
                        .map(|relative| relative.replace(std::path::MAIN_SEPARATOR, "/"));
                    if let Some(key) = key {
                        self.delete(&key).await?;
                        swept += 1;
                    }
                }
            }
        }
        Ok(swept)
    }

    pub(crate) async fn get_bytes(&self, key: &str, limit: u64) -> crate::Result<Option<Vec<u8>>> {
        let mut bytes = Vec::new();
        Ok(self
            .copy_object_to(key, limit, &mut bytes)
            .await?
            .map(|_| bytes))
    }

    /// A shared lock on [`REMOVALS_LOCK`], held until it is dropped.
    async fn removing(&self) -> crate::Result<std::fs::File> {
        let path = self.root.join(REMOVALS_LOCK);
        tokio::task::spawn_blocking(move || {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path)?;
            file.lock_shared()?;
            Ok::<_, std::io::Error>(file)
        })
        .await
        .map_err(|e| crate::Error::Io(std::io::Error::other(e)))?
        .map_err(io)
    }

    pub(crate) async fn delete(&self, key: &str) -> crate::Result<()> {
        let (object, record) = self.paths(key)?;
        let _removing = self.removing().await?;
        for path in [object, record] {
            match tokio::fs::remove_file(&path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io(e)),
            }
        }
        Ok(())
    }

    pub(crate) async fn delete_upload(&self, key: &str) -> crate::Result<()> {
        self.delete(&upload_key(key)).await
    }

    pub(crate) async fn move_object(&self, from: &str, to: &str) -> crate::Result<bool> {
        let (from_object, from_record) = self.paths(from)?;
        let (to_object, to_record) = self.paths(to)?;
        let _removing = self.removing().await?;
        for path in [&to_object, &to_record] {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(io)?;
            }
        }
        match tokio::fs::rename(&from_record, &to_record).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io(e)),
        }
        match tokio::fs::rename(&from_object, &to_object).await {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io(e)),
        }
    }

    pub(crate) async fn presign_get(&self, key: &str, ttl: StdDuration) -> crate::Result<String> {
        self.paths(key)?;
        let expires = unix_seconds(SystemTime::now() + ttl);
        let signature = self.sign("read", key, &[], expires);
        Ok(self.url(
            &format!("signed/{key}"),
            &[("expires", &expires.to_string()), ("signature", &signature)],
        ))
    }

    /// Whether a URL of [`presign_get`](Self::presign_get) for `key` is this server's and still
    /// in time.
    pub fn verify_read(&self, key: &str, expires: u64, signature: &str) -> Result<(), Refusal> {
        self.verify("read", key, &[], expires, signature)
    }

    pub(crate) fn public_url(&self, key: &str) -> String {
        format!("{}/{key}", self.public_base_url)
    }

    /// The origin objects are served from, as `public_base_url` names it.
    pub fn public_base_url(&self) -> &str {
        &self.public_base_url
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store(ttl: StdDuration) -> (tempfile::TempDir, Directory) {
        let root = tempfile::tempdir().unwrap();
        let store = Directory::open(root.path(), "https://media.example.org/", ttl)
            .await
            .unwrap();
        (root, store)
    }

    fn query(url: &str) -> (String, u64, String) {
        let url = url::Url::parse(url).unwrap();
        let pairs: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        (
            url.path().trim_start_matches('/').to_string(),
            pairs["expires"].parse().unwrap(),
            pairs["signature"].clone(),
        )
    }

    fn served(kind: &str) -> Served {
        Served {
            content_type: kind.to_string(),
            disposition: Some("inline".to_string()),
        }
    }

    #[tokio::test]
    async fn removals_wait_while_a_backup_holds_the_lock() {
        let (root, store) = store(StdDuration::from_secs(60)).await;
        for key in ["attachments/a", "attachments/b"] {
            store.put_bytes(key, vec![1], "text/plain").await.unwrap();
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.path().join(REMOVALS_LOCK))
            .unwrap();
        lock.lock().unwrap();
        let removing = tokio::spawn({
            let store = store.clone();
            async move {
                store.delete("attachments/a").await.unwrap();
                store.move_object("attachments/b", "evidence/b").await.unwrap()
            }
        });
        tokio::time::sleep(StdDuration::from_millis(300)).await;
        assert!(!removing.is_finished());
        assert_eq!(store.head_object("attachments/a").await.unwrap(), Some(1));
        drop(lock);
        assert!(removing.await.unwrap());
        assert_eq!(store.head_object("attachments/a").await.unwrap(), None);
        assert_eq!(store.head_object("evidence/b").await.unwrap(), Some(1));
    }

    #[tokio::test]
    async fn keys_the_server_does_not_make_are_refused() {
        let (_root, store) = store(StdDuration::from_secs(60)).await;
        for key in [
            "../x", "a/../b", "", "a//b", ".hidden", "a/.", "a b", "a\\b", "/abs",
        ] {
            assert_eq!(store.path_of(key), Err(Refusal::BadKey), "{key:?}");
        }
        assert!(store.path_of("attachments/0193-abc_d.e").is_ok());
        assert!(
            store
                .put_bytes("../escape", vec![1], "text/plain")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn an_upload_is_taken_only_as_signed_and_promoted_once() {
        let (_root, store) = store(StdDuration::from_secs(60)).await;
        let signed = store
            .presign_upload("attachments/a", "image/png", 3)
            .await
            .unwrap();
        assert!(
            signed
                .url
                .starts_with("https://media.example.org/upload/uploads/attachments/a?")
        );
        let (path, expires, signature) = query(&signed.url);
        let staging = path.trim_start_matches("upload/").to_string();
        let take = async |kind: &str, length: u64, body: &'static [u8], signature: &str| {
            let chunks = futures_util::stream::iter(
                body.chunks(2)
                    .map(|chunk| Ok(bytes::Bytes::from_static(chunk))),
            );
            store
                .receive_upload(&staging, expires, signature, kind, length, chunks)
                .await
                .unwrap()
        };
        assert_eq!(
            take("text/html", 3, b"abc", &signature).await,
            Err(Refusal::Unsigned)
        );
        assert_eq!(
            take("image/png", 4, b"abcd", &signature).await,
            Err(Refusal::Unsigned)
        );
        assert_eq!(
            take("image/png", 3, b"abc", "00").await,
            Err(Refusal::Unsigned)
        );
        assert_eq!(
            take("image/png", 3, b"ab", &signature).await,
            Err(Refusal::NotAsSigned)
        );
        assert_eq!(
            take("image/png", 3, b"abcd", &signature).await,
            Err(Refusal::NotAsSigned)
        );
        assert_eq!(
            store.head_object("uploads/attachments/a").await.unwrap(),
            None
        );
        assert_eq!(take("image/png", 3, b"abc", &signature).await, Ok(()));
        // Readers never meet the staging key's object at the key.
        assert_eq!(store.head_object("attachments/a").await.unwrap(), None);
        assert_eq!(
            store
                .promote("attachments/a", 2, &served("image/png"))
                .await
                .unwrap(),
            Promotion::TooLarge
        );
        assert_eq!(
            store
                .promote("attachments/a", 3, &served("image/png"))
                .await
                .unwrap(),
            Promotion::NotUploaded
        );
        assert_eq!(take("image/png", 3, b"xyz", &signature).await, Ok(()));
        assert_eq!(
            store
                .promote("attachments/a", 3, &served("image/png"))
                .await
                .unwrap(),
            Promotion::Promoted(3)
        );
        // Promoting again finds it in place.
        assert_eq!(
            store
                .promote("attachments/a", 3, &served("image/png"))
                .await
                .unwrap(),
            Promotion::Promoted(3)
        );
        assert_eq!(
            store.get_bytes("attachments/a", 10).await.unwrap(),
            Some(b"xyz".to_vec())
        );
        let record = store.record("attachments/a").await.unwrap().unwrap();
        assert_eq!(
            (record.content_type.as_str(), record.disposition.as_deref()),
            ("image/png", Some("inline"))
        );
        assert_eq!(
            store.head_object("uploads/attachments/a").await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn signatures_end_when_their_time_does() {
        let (_root, store) = store(StdDuration::from_secs(0)).await;
        let signed = store
            .presign_upload("icons/a", "image/webp", 1)
            .await
            .unwrap();
        let (path, expires, signature) = query(&signed.url);
        let staging = path.trim_start_matches("upload/").to_string();
        assert_eq!(
            store
                .receive_upload(
                    &staging,
                    expires,
                    &signature,
                    "image/webp",
                    1,
                    futures_util::stream::iter([Ok(bytes::Bytes::from_static(b"x"))]),
                )
                .await
                .unwrap(),
            Err(Refusal::Expired)
        );
        let read = store
            .presign_get("evidence/a", StdDuration::from_secs(60))
            .await
            .unwrap();
        let (path, expires, signature) = query(&read);
        assert_eq!(path, "signed/evidence/a");
        assert_eq!(store.verify_read("evidence/a", expires, &signature), Ok(()));
        assert_eq!(
            store.verify_read("evidence/b", expires, &signature),
            Err(Refusal::Unsigned)
        );
        assert_eq!(
            store.verify_read("evidence/a", expires + 1, &signature),
            Err(Refusal::Unsigned)
        );
        assert_eq!(
            store.verify_read("evidence/a", 1, &store.sign("read", "evidence/a", &[], 1)),
            Err(Refusal::Expired)
        );
        // Another process's key signs nothing this one takes.
        let (_other_root, other) = super::tests::store(StdDuration::from_secs(60)).await;
        assert_eq!(
            other.verify_read("evidence/a", expires, &signature),
            Err(Refusal::Unsigned)
        );
    }

    #[tokio::test]
    async fn objects_move_copy_and_go() {
        let (_root, store) = store(StdDuration::from_secs(60)).await;
        store
            .put_bytes("attachments/m", b"hello".to_vec(), "text/plain")
            .await
            .unwrap();
        assert_eq!(
            store.public_url("attachments/m"),
            "https://media.example.org/attachments/m"
        );
        let mut sink = Vec::new();
        assert_eq!(
            store
                .copy_object_to("attachments/m", 4, &mut sink)
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .copy_object_to("attachments/m", 5, &mut sink)
                .await
                .unwrap(),
            Some(5)
        );
        assert!(
            store
                .move_object("attachments/m", "evidence/attachments/m")
                .await
                .unwrap()
        );
        assert!(
            !store
                .move_object("attachments/m", "evidence/attachments/m")
                .await
                .unwrap()
        );
        assert_eq!(store.head_object("attachments/m").await.unwrap(), None);
        assert_eq!(
            store
                .record("evidence/attachments/m")
                .await
                .unwrap()
                .unwrap()
                .content_type,
            "text/plain"
        );
        store.delete("evidence/attachments/m").await.unwrap();
        store.delete("evidence/attachments/m").await.unwrap();
        assert_eq!(store.object("evidence/attachments/m").await.unwrap(), None);
    }

    #[tokio::test]
    async fn uploads_past_their_time_are_swept() {
        let (_root, store) = store(StdDuration::from_secs(60)).await;
        store
            .put_bytes("uploads/attachments/old", vec![1], "text/plain")
            .await
            .unwrap();
        store
            .put_bytes("attachments/kept", vec![1], "text/plain")
            .await
            .unwrap();
        assert_eq!(
            store
                .sweep_uploads(Utc::now() - Duration::hours(1))
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .sweep_uploads(Utc::now() + Duration::seconds(1))
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            store.head_object("uploads/attachments/old").await.unwrap(),
            None
        );
        assert_eq!(
            store.head_object("attachments/kept").await.unwrap(),
            Some(1)
        );
    }
}
