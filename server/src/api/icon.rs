//! REST surface for the two-phase icon upload flow.
//!
//! Mirrors the attachment flow in [`crate::api::attachment`] except that
//! the wire DTO has no `fileName`. Icons today are unauthenticated for
//! create/read/delete (the previous CRUD shape was the same); the review's
//! P1-2 follow-up is the right place to tighten that.

use crate::api::GlobalServerContext;
use crate::app::{self, IconId};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use tracing::error;
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Icon {
    pub id: IconId,
    pub mime_type: String,
    pub download_url: String,
}

fn icon_to_api(state: &GlobalServerContext, row: app::icon::Icon) -> Icon {
    let download_url = state.media_store.public_url(&row.storage_key);
    Icon {
        id: row.id,
        mime_type: row.mime_type,
        download_url,
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IconUploadInitCommand {
    pub mime_type: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IconUploadHandle {
    pub id: IconId,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum IconUploadInitCommandResponse {
    Upload(IconUploadHandle),
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(
    post,
    path = "/icon/upload-init",
    responses((status = OK, body = IconUploadInitCommandResponse))
)]
pub async fn init_icon_upload(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconUploadInitCommand>,
) -> (StatusCode, Json<IconUploadInitCommandResponse>) {
    match app::icon::init_upload(&state, command.mime_type).await {
        Ok(upload) => (
            StatusCode::OK,
            IconUploadInitCommandResponse::Upload(IconUploadHandle {
                id: upload.id,
                upload_url: upload.upload_url,
                expires_at: upload.expires_at,
            })
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error initiating icon upload");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                IconUploadInitCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IconUploadConfirmCommand {
    pub id: IconId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum IconUploadConfirmCommandResponse {
    Icon(Icon),
    NotAllowed { reason: Option<Cow<'static, str>> },
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(
    post,
    path = "/icon/upload-confirm",
    responses((status = OK, body = IconUploadConfirmCommandResponse))
)]
pub async fn confirm_icon_upload(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconUploadConfirmCommand>,
) -> (StatusCode, Json<IconUploadConfirmCommandResponse>) {
    match app::icon::confirm_upload(&state, command.id).await {
        Ok(row) => (
            StatusCode::OK,
            IconUploadConfirmCommandResponse::Icon(icon_to_api(&state, row)).into(),
        ),
        Err(app::Error::Validation(reason)) => (
            StatusCode::BAD_REQUEST,
            IconUploadConfirmCommandResponse::NotAllowed {
                reason: Some(reason),
            }
            .into(),
        ),
        Err(app::Error::Diesel(diesel::result::Error::NotFound)) => (
            StatusCode::NOT_FOUND,
            IconUploadConfirmCommandResponse::Error { cause: None }.into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error confirming icon upload");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                IconUploadConfirmCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IconReadCommand {
    pub id: IconId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum IconReadCommandResponse {
    Icon(Icon),
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(
    get,
    path = "/icon",
    responses((status = OK, body = IconReadCommandResponse))
)]
pub async fn read_icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconReadCommand>,
) -> (StatusCode, Json<IconReadCommandResponse>) {
    match app::icon::read_icon(&state, command.id).await {
        Ok(row) => (
            StatusCode::OK,
            IconReadCommandResponse::Icon(icon_to_api(&state, row)).into(),
        ),
        Err(app::Error::Diesel(diesel::result::Error::NotFound)) => (
            StatusCode::NOT_FOUND,
            IconReadCommandResponse::Error { cause: None }.into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error reading icon");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                IconReadCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IconDeleteCommand {
    pub id: IconId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum IconDeleteCommandResponse {
    DeleteOk,
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(
    delete,
    path = "/icon",
    responses((status = OK, body = IconDeleteCommandResponse))
)]
pub async fn delete_icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconDeleteCommand>,
) -> (StatusCode, Json<IconDeleteCommandResponse>) {
    match app::icon::delete_icon(&state, command.id).await {
        Ok(()) => (StatusCode::OK, IconDeleteCommandResponse::DeleteOk.into()),
        Err(app::Error::Diesel(diesel::result::Error::NotFound)) => (
            StatusCode::NOT_FOUND,
            IconDeleteCommandResponse::Error { cause: None }.into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error deleting icon");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                IconDeleteCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}
