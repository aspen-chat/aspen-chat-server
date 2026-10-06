//! Thin wrapper over the AWS S3 SDK that the rest of the app uses to talk to
//! object storage.
//!
//! Two different access paths flow through this module:
//!
//! 1. **Authenticated, server-side I/O.** [`put_bytes`] uploads bytes the
//!    server has fetched on its own (link-preview thumbnails today). [`delete`]
//!    removes objects when their owning DB row goes away.
//! 2. **Client-driven uploads via presigned URLs.** [`presign_upload`] mints
//!    a short-lived URL the client PUTs raw bytes to without ever touching the
//!    Aspen API process. The URL names the S3 API as clients reach it
//!    (`public_endpoint`), which need not be the address the server uses.
//!    It names the object's staging key ([`upload_key`], under
//!    [`UPLOAD_PREFIX`]), never the key readers fetch: the confirm endpoints
//!    [`promote`] what arrived there, copying it within the store to its own
//!    key and deleting the staging object, so the bytes readers see are
//!    written by the server alone, and the URL, which stays valid until it
//!    expires, can write only to a key nothing reads. What a URL writes after
//!    its upload was promoted, and what was uploaded and never confirmed, the
//!    sweeper deletes once the URL has expired ([`spawn_upload_sweeper`]).
//!
//! The server also reads objects back and writes what it makes of them: the
//! previews of pictures and videos (`app::attachment::preview`) through
//! [`copy_object_to`] and [`put_bytes`].
//!
//! Downloads do not pass through Aspen at all; clients hit the
//! `public_base_url` (an anonymous-read endpoint operated alongside the S3
//! API) directly with the storage key the server has stamped onto a
//! `downloadUrl`/`imageUrl` field. [`MediaStore::public_url`] is the one
//! place that templates those URLs so a misconfigured base or a stray
//! trailing slash can't desync between callers.

use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::Client;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::head_object::HeadObjectError;
use aws_sdk_s3::presigning::PresigningConfig;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use aws_types::region::Region;
use chrono::{DateTime, Duration, Utc};
use std::time::Duration as StdDuration;

use crate::app;
use crate::aspen_config::{AspenConfig, MediaS3Config};

/// Where clients' uploads land until they are confirmed (see the module docs).
pub const UPLOAD_PREFIX: &str = "uploads/";

/// The largest object one `CopyObject` copies; [`MediaStore::promote`] copies larger ones in
/// parts of [`COPY_PART_BYTES`].
const MAX_SINGLE_COPY_BYTES: u64 = 5 * 1024 * 1024 * 1024;
const COPY_PART_BYTES: u64 = 1024 * 1024 * 1024;

/// How long after its URL expires a staging object is kept, for a confirm already under way.
const STAGING_GRACE: StdDuration = StdDuration::from_secs(3600);
/// How often each server sweeps staging objects.
const SWEEP_EVERY: StdDuration = StdDuration::from_secs(3600);

/// The staging key a client's upload of what becomes `key` is written to.
pub fn upload_key(key: &str) -> String {
    format!("{UPLOAD_PREFIX}{key}")
}

fn request_error(e: impl std::error::Error + Send + Sync + 'static) -> app::Error {
    app::Error::S3Request(Box::new(e))
}

/// Result of a successful presign request.
#[derive(Debug, Clone)]
pub struct PresignedUpload {
    /// Pre-signed `PUT` URL the client uploads the raw bytes to.
    pub url: String,
    /// Wall-clock time at which the URL stops being accepted by S3.
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone)]
pub struct MediaStore {
    client: Client,
    /// Signs upload URLs for clients: the same credentials, addressed to the S3
    /// API as clients reach it. Signing is local, so this client never needs to
    /// reach that address itself.
    presign_client: Client,
    bucket: String,
    public_base_url: String,
    upload_url_ttl: StdDuration,
}

impl MediaStore {
    pub async fn new(config: &AspenConfig) -> app::error::Result<Self> {
        Self::from_s3(&config.media.s3).await
    }

    async fn from_s3(s3: &MediaS3Config) -> app::error::Result<Self> {
        let sdk_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(s3.region.clone()))
            .credentials_provider(Credentials::new(
                s3.access_key.clone(),
                s3.secret_key.clone(),
                None,
                None,
                "aspen-media-config",
            ))
            .load()
            .await;
        let client_for = |endpoint: &str| {
            Client::from_conf(
                aws_sdk_s3::config::Builder::from(&sdk_config)
                    .endpoint_url(endpoint)
                    .force_path_style(true)
                    .build(),
            )
        };
        Ok(Self {
            client: client_for(&s3.endpoint),
            presign_client: client_for(s3.public_endpoint.as_deref().unwrap_or(&s3.endpoint)),
            bucket: s3.bucket.clone(),
            public_base_url: s3.public_base_url.trim_end_matches('/').to_string(),
            upload_url_ttl: StdDuration::from_secs(s3.upload_url_ttl_seconds),
        })
    }

    /// Server-side upload.
    ///
    /// Used by [`crate::app::link_preview`] for OG-image thumbnails the server
    /// fetches itself; client-driven uploads go through [`presign_upload`]
    /// instead so the bytes never touch this process.
    pub async fn put_bytes(
        &self,
        key: &str,
        bytes: Vec<u8>,
        content_type: &str,
    ) -> app::error::Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .body(ByteStream::from(bytes))
            .send()
            .await
            .map_err(Box::new)?;
        Ok(())
    }

    /// Mint a short-lived presigned `PUT` URL for what becomes `key` once [`promote`]d: it
    /// writes the staging key ([`upload_key`]), never `key` itself.
    ///
    /// The client is expected to upload with `Content-Type: <content_type>`;
    /// S3 enforces the value because it was part of the canonical request
    /// that produced the signature.
    pub async fn presign_upload(
        &self,
        key: &str,
        content_type: &str,
    ) -> app::error::Result<PresignedUpload> {
        let presigning = PresigningConfig::expires_in(self.upload_url_ttl)?;
        let presigned = self
            .presign_client
            .put_object()
            .bucket(&self.bucket)
            .key(upload_key(key))
            .content_type(content_type)
            .presigned(presigning)
            .await
            .map_err(Box::new)?;
        let expires_at =
            Utc::now() + Duration::from_std(self.upload_url_ttl).unwrap_or(Duration::seconds(900));
        Ok(PresignedUpload {
            url: presigned.uri().to_string(),
            expires_at,
        })
    }

    /// The size in bytes of the object at `key`, or `None` when there is none.
    ///
    /// Used by [`promote`] to find a client's upload, and by
    /// `app::attachment::preview` to weigh a preview against its original. A
    /// missing object is a normal control-flow case (caller surfaces a
    /// validation error to the client), so 404s are folded into `Ok(None)`;
    /// every other transport / auth failure is propagated as `S3HeadObject`.
    pub async fn head_object(&self, key: &str) -> app::error::Result<Option<u64>> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(head) => Ok(Some(
                head.content_length()
                    .and_then(|length| u64::try_from(length).ok())
                    .unwrap_or_default(),
            )),
            Err(SdkError::ServiceError(svc))
                if matches!(svc.err(), HeadObjectError::NotFound(_)) =>
            {
                Ok(None)
            }
            Err(e) => Err(app::Error::S3HeadObject(Box::new(e))),
        }
    }

    /// Streams the object at `key` into `sink`, answering how many bytes it wrote, or `None`
    /// without writing them all when the object holds more than `limit`.
    pub async fn copy_object_to(
        &self,
        key: &str,
        limit: u64,
        sink: &mut (impl tokio::io::AsyncWrite + Unpin),
    ) -> app::error::Result<Option<u64>> {
        let object = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(Box::new)?;
        if object
            .content_length()
            .and_then(|length| u64::try_from(length).ok())
            .is_some_and(|length| length > limit)
        {
            return Ok(None);
        }
        let mut body =
            tokio::io::AsyncReadExt::take(object.body.into_async_read(), limit.saturating_add(1));
        let written = tokio::io::copy(&mut body, sink).await?;
        Ok((written <= limit).then_some(written))
    }

    /// Moves a client's upload of `key` from its staging key to `key`, answering its size in
    /// bytes, or `None` when nothing was uploaded. The copy is the store's own, so no byte passes
    /// through this process, and the staging object is deleted after it. An upload promoted
    /// already, by a confirm that then failed, is answered as it is, since only the server
    /// writes `key`.
    pub async fn promote(&self, key: &str) -> app::error::Result<Option<u64>> {
        let staging = upload_key(key);
        let Some(size) = self.head_object(&staging).await? else {
            return self.head_object(key).await;
        };
        let source = format!("{}/{}", self.bucket, staging);
        if size <= MAX_SINGLE_COPY_BYTES {
            self.client
                .copy_object()
                .bucket(&self.bucket)
                .key(key)
                .copy_source(&source)
                .send()
                .await
                .map_err(request_error)?;
        } else {
            self.copy_in_parts(&source, key, size).await?;
        }
        if let Err(e) = self.delete(&staging).await {
            tracing::warn!(error = %e, key = staging, "could not delete a promoted upload");
        }
        Ok(Some(size))
    }

    /// Copies `size` bytes of `source` to `key` in parts, as `CopyObject` copies at most
    /// [`MAX_SINGLE_COPY_BYTES`].
    async fn copy_in_parts(&self, source: &str, key: &str, size: u64) -> app::error::Result<()> {
        let upload = self
            .client
            .create_multipart_upload()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(request_error)?;
        let upload_id = upload.upload_id().unwrap_or_default().to_string();
        let copied = async {
            let mut parts = Vec::new();
            let mut start = 0u64;
            let mut number = 1i32;
            while start < size {
                let end = (start + COPY_PART_BYTES).min(size) - 1;
                let part = self
                    .client
                    .upload_part_copy()
                    .bucket(&self.bucket)
                    .key(key)
                    .upload_id(&upload_id)
                    .part_number(number)
                    .copy_source(source)
                    .copy_source_range(format!("bytes={start}-{end}"))
                    .send()
                    .await
                    .map_err(request_error)?;
                parts.push(
                    CompletedPart::builder()
                        .part_number(number)
                        .set_e_tag(
                            part.copy_part_result()
                                .and_then(|r| r.e_tag().map(String::from)),
                        )
                        .build(),
                );
                start = end + 1;
                number += 1;
            }
            self.client
                .complete_multipart_upload()
                .bucket(&self.bucket)
                .key(key)
                .upload_id(&upload_id)
                .multipart_upload(
                    CompletedMultipartUpload::builder()
                        .set_parts(Some(parts))
                        .build(),
                )
                .send()
                .await
                .map_err(request_error)?;
            app::error::Result::Ok(())
        };
        let result = copied.await;
        if result.is_err() {
            let _ = self
                .client
                .abort_multipart_upload()
                .bucket(&self.bucket)
                .key(key)
                .upload_id(&upload_id)
                .send()
                .await;
        }
        result
    }

    /// The time before which a staging object was written by a URL that expired more than
    /// [`STAGING_GRACE`] ago, and is no use to anyone.
    pub fn staging_cutoff(&self) -> DateTime<Utc> {
        Utc::now()
            - Duration::from_std(self.upload_url_ttl + STAGING_GRACE).unwrap_or(Duration::hours(2))
    }

    /// Deletes the staging objects last written before `older_than` ([`staging_cutoff`]): what a
    /// URL wrote after its upload was promoted, and uploads never confirmed. Answers how many it
    /// deleted.
    pub async fn sweep_uploads(&self, older_than: DateTime<Utc>) -> app::error::Result<usize> {
        let mut pages = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(UPLOAD_PREFIX)
            .into_paginator()
            .send();
        let mut swept = 0;
        while let Some(page) = pages.next().await {
            let page = page.map_err(request_error)?;
            for object in page.contents() {
                let stale = object
                    .last_modified()
                    .and_then(|at| DateTime::<Utc>::from_timestamp(at.secs(), 0))
                    .is_some_and(|at| at < older_than);
                if let (true, Some(key)) = (stale, object.key()) {
                    self.delete(key).await?;
                    swept += 1;
                }
            }
        }
        Ok(swept)
    }

    /// Reads the object at `key` and its content type, as the server serves a copy of it
    /// itself (`app::federation::abroad`).
    pub async fn get_bytes(&self, key: &str) -> app::error::Result<(Vec<u8>, Option<String>)> {
        let object = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(Box::new)?;
        let content_type = object.content_type().map(str::to_string);
        let bytes = object
            .body
            .collect()
            .await
            .map_err(std::io::Error::other)?
            .into_bytes()
            .to_vec();
        Ok((bytes, content_type))
    }

    pub async fn delete(&self, key: &str) -> app::error::Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(Box::new)?;
        Ok(())
    }

    /// Deletes a client's upload of `key`: the object, and its staging key, where one never
    /// confirmed still is.
    pub async fn delete_upload(&self, key: &str) -> app::error::Result<()> {
        self.delete(&upload_key(key)).await?;
        self.delete(key).await
    }

    /// Build the anonymous-read URL clients fetch the bytes from.
    ///
    /// The base URL is normalised once at construction time, so this is a
    /// straight `format!`; the caller is responsible for never passing a
    /// leading `/` in `key`.
    pub fn public_url(&self, key: &str) -> String {
        format!("{}/{}", self.public_base_url, key)
    }
}

/// Sweeps staging objects every [`SWEEP_EVERY`], on every server, for as long as it runs: deleting
/// one twice is harmless, and a deployment of one server needs no other to do it.
pub fn spawn_upload_sweeper(state: app::context::GlobalServerContext) {
    tokio::spawn(async move {
        loop {
            // Spread around the hour, so servers started together do not sweep together.
            let jitter = StdDuration::from_secs(rand::random_range(0..SWEEP_EVERY.as_secs() / 2));
            tokio::time::sleep(SWEEP_EVERY * 3 / 4 + jitter).await;
            let cutoff = state.media_store.staging_cutoff();
            match state.media_store.sweep_uploads(cutoff).await {
                Ok(0) => {}
                Ok(swept) => tracing::info!(swept, "deleted staging uploads past their URLs"),
                Err(e) => tracing::warn!(error = %e, "could not sweep staging uploads"),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upload_urls_name_storage_as_clients_reach_it() {
        let internal = MediaS3Config {
            endpoint: "http://127.0.0.1:8333".to_string(),
            ..MediaS3Config::default()
        };
        let store = MediaStore::from_s3(&internal).await.unwrap();
        let url = store
            .presign_upload("attachments/a", "text/plain")
            .await
            .unwrap()
            .url;
        assert!(url.starts_with("http://127.0.0.1:8333/"), "{url}");
        // The staging key, never the one readers fetch.
        assert!(url.contains("/aspen-media/uploads/attachments/a?"), "{url}");

        let public = MediaS3Config {
            public_endpoint: Some("https://media.example.org".to_string()),
            ..internal
        };
        let store = MediaStore::from_s3(&public).await.unwrap();
        let url = store
            .presign_upload("attachments/a", "text/plain")
            .await
            .unwrap()
            .url;
        assert!(url.starts_with("https://media.example.org/"), "{url}");
    }
}
