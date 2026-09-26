//! Thin wrapper over the AWS S3 SDK that the rest of the app uses to talk to
//! object storage.
//!
//! Two different access paths flow through this module:
//!
//! 1. **Authenticated, server-side I/O.** [`put_bytes`] uploads bytes the
//!    server has fetched on its own (link-preview thumbnails today). [`delete`]
//!    removes objects when their owning DB row goes away.
//! 2. **Client-driven uploads via presigned URLs.** [`presign_put`] mints a
//!    short-lived URL the client PUTs raw bytes to without ever touching the
//!    Aspen API process. [`head_object`] is how the confirm endpoint verifies
//!    the bytes actually landed before flipping the DB row to "ready".
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
use aws_types::region::Region;
use chrono::{DateTime, Duration, Utc};
use std::time::Duration as StdDuration;

use crate::app;
use crate::aspen_config::AspenConfig;

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
    bucket: String,
    public_base_url: String,
    upload_url_ttl: StdDuration,
}

impl MediaStore {
    pub async fn new(config: &AspenConfig) -> app::error::Result<Self> {
        let s3 = &config.media.s3;
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
        let s3_config = aws_sdk_s3::config::Builder::from(&sdk_config)
            .endpoint_url(s3.endpoint.clone())
            .force_path_style(true)
            .build();
        let client = Client::from_conf(s3_config);
        Ok(Self {
            client,
            bucket: s3.bucket.clone(),
            public_base_url: s3.public_base_url.trim_end_matches('/').to_string(),
            upload_url_ttl: StdDuration::from_secs(s3.upload_url_ttl_seconds),
        })
    }

    /// Server-side upload.
    ///
    /// Used by [`crate::app::link_preview`] for OG-image thumbnails the server
    /// fetches itself; client-driven uploads go through [`presign_put`]
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

    /// Mint a short-lived presigned `PUT` URL for `key`.
    ///
    /// The client is expected to upload with `Content-Type: <content_type>`;
    /// S3 enforces the value because it was part of the canonical request
    /// that produced the signature.
    pub async fn presign_put(
        &self,
        key: &str,
        content_type: &str,
    ) -> app::error::Result<PresignedUpload> {
        let presigning = PresigningConfig::expires_in(self.upload_url_ttl)?;
        let presigned = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
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

    /// Returns `true` when an object exists at `key`.
    ///
    /// Used by the confirm endpoints to verify the client's direct-to-S3
    /// upload succeeded before the row is flipped to `ready`. A missing
    /// object is a normal control-flow case (caller surfaces a validation
    /// error to the client), so 404s are folded into `Ok(false)`; every
    /// other transport / auth failure is propagated as `S3HeadObject`.
    pub async fn head_object(&self, key: &str) -> app::error::Result<bool> {
        match self
            .client
            .head_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(SdkError::ServiceError(svc))
                if matches!(svc.err(), HeadObjectError::NotFound(_)) =>
            {
                Ok(false)
            }
            Err(e) => Err(app::Error::S3HeadObject(Box::new(e))),
        }
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

    /// Build the anonymous-read URL clients fetch the bytes from.
    ///
    /// The base URL is normalised once at construction time, so this is a
    /// straight `format!`; the caller is responsible for never passing a
    /// leading `/` in `key`.
    pub fn public_url(&self, key: &str) -> String {
        format!("{}/{}", self.public_base_url, key)
    }
}
