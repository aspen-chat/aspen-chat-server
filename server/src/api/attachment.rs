//! REST surface for the two-phase attachment upload flow.
//!
//! - `POST /attachment/upload-init` — caller sends `{fileName, mimeType}`,
//!   server reserves an id and returns `{id, uploadUrl, expiresAt}`.
//! - `POST /attachment/upload-confirm` — caller sends `{id}` after the
//!   direct-to-S3 PUT, server HEADs the object and flips the row to
//!   ready. Returns the final [`Attachment`] DTO with a public
//!   `downloadUrl`.
//! - `GET /attachment` — looks up an already-ready attachment by id and
//!   returns its metadata + `downloadUrl`.
//! - `DELETE /attachment` — drops the row and the object.
//!
//! Bytes do not flow through these handlers in either direction. Auth
//! mirrors the previous CRUD shape: writes require `SessionUser`, the
//! read does not.

use crate::api::GlobalServerContext;
use crate::api::login::SessionUser;
use crate::app::{self, AttachmentId};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use tracing::error;
use utoipa::ToSchema;

/// Wire-level representation of an attachment.
///
/// `data` no longer rides on this DTO; clients fetch the bytes themselves
/// from the anonymous-read endpoint behind `downloadUrl`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: AttachmentId,
    pub file_name: String,
    pub mime_type: String,
    pub download_url: String,
}

fn attachment_to_api(state: &GlobalServerContext, row: app::attachment::Attachment) -> Attachment {
    let download_url = state.media_store.public_url(&row.storage_key);
    Attachment {
        id: row.id,
        file_name: row.file_name,
        mime_type: row.mime_type,
        download_url,
    }
}

// --- Upload init ---

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUploadInitCommand {
    pub file_name: String,
    pub mime_type: String,
}

/// Body of the `Upload` variant on [`AttachmentUploadInitCommandResponse`].
///
/// Pulled out into its own struct so its fields are camelCased on the wire;
/// `#[serde(rename_all = ...)]` on an enum variant only renames the variant
/// itself, not the fields of an inline struct payload.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUploadHandle {
    pub id: AttachmentId,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentUploadInitCommandResponse {
    Upload(AttachmentUploadHandle),
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(
    post,
    path = "/attachment/upload-init",
    security(("loginKey" = [])),
    responses((status = OK, body = AttachmentUploadInitCommandResponse))
)]
pub async fn init_attachment_upload(
    State(state): State<GlobalServerContext>,
    SessionUser { .. }: SessionUser,
    Json(command): Json<AttachmentUploadInitCommand>,
) -> (StatusCode, Json<AttachmentUploadInitCommandResponse>) {
    match app::attachment::init_upload(&state, command.file_name, command.mime_type).await {
        Ok(upload) => (
            StatusCode::OK,
            AttachmentUploadInitCommandResponse::Upload(AttachmentUploadHandle {
                id: upload.id,
                upload_url: upload.upload_url,
                expires_at: upload.expires_at,
            })
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error initiating attachment upload");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AttachmentUploadInitCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

// --- Upload confirm ---

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUploadConfirmCommand {
    pub id: AttachmentId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentUploadConfirmCommandResponse {
    Attachment(Attachment),
    NotAllowed { reason: Option<Cow<'static, str>> },
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(
    post,
    path = "/attachment/upload-confirm",
    security(("loginKey" = [])),
    responses((status = OK, body = AttachmentUploadConfirmCommandResponse))
)]
pub async fn confirm_attachment_upload(
    State(state): State<GlobalServerContext>,
    SessionUser { .. }: SessionUser,
    Json(command): Json<AttachmentUploadConfirmCommand>,
) -> (StatusCode, Json<AttachmentUploadConfirmCommandResponse>) {
    match app::attachment::confirm_upload(&state, command.id).await {
        Ok(row) => (
            StatusCode::OK,
            AttachmentUploadConfirmCommandResponse::Attachment(attachment_to_api(&state, row))
                .into(),
        ),
        Err(app::Error::Validation(reason)) => (
            StatusCode::BAD_REQUEST,
            AttachmentUploadConfirmCommandResponse::NotAllowed {
                reason: Some(reason),
            }
            .into(),
        ),
        Err(app::Error::Diesel(diesel::result::Error::NotFound)) => (
            StatusCode::NOT_FOUND,
            AttachmentUploadConfirmCommandResponse::Error { cause: None }.into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error confirming attachment upload");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AttachmentUploadConfirmCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

// --- Read ---

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentReadCommand {
    pub id: AttachmentId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentReadCommandResponse {
    Attachment(Attachment),
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(
    get,
    path = "/attachment",
    responses((status = OK, body = AttachmentReadCommandResponse))
)]
pub async fn read_attachment(
    State(state): State<GlobalServerContext>,
    Json(command): Json<AttachmentReadCommand>,
) -> (StatusCode, Json<AttachmentReadCommandResponse>) {
    match app::attachment::read_attachment(&state, command.id).await {
        Ok(row) => (
            StatusCode::OK,
            AttachmentReadCommandResponse::Attachment(attachment_to_api(&state, row)).into(),
        ),
        Err(app::Error::Diesel(diesel::result::Error::NotFound)) => (
            StatusCode::NOT_FOUND,
            AttachmentReadCommandResponse::Error { cause: None }.into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error reading attachment");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AttachmentReadCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

// --- Delete ---

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentDeleteCommand {
    pub id: AttachmentId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentDeleteCommandResponse {
    DeleteOk,
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(
    delete,
    path = "/attachment",
    security(("loginKey" = [])),
    responses((status = OK, body = AttachmentDeleteCommandResponse))
)]
pub async fn delete_attachment(
    State(state): State<GlobalServerContext>,
    SessionUser { .. }: SessionUser,
    Json(command): Json<AttachmentDeleteCommand>,
) -> (StatusCode, Json<AttachmentDeleteCommandResponse>) {
    match app::attachment::delete_attachment(&state, command.id).await {
        Ok(()) => (
            StatusCode::OK,
            AttachmentDeleteCommandResponse::DeleteOk.into(),
        ),
        Err(app::Error::Diesel(diesel::result::Error::NotFound)) => (
            StatusCode::NOT_FOUND,
            AttachmentDeleteCommandResponse::Error { cause: None }.into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error deleting attachment");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AttachmentDeleteCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}
