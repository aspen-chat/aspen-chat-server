//! REST surface for the two-phase attachment upload flow.
//!
//! - `POST /attachments` — caller sends `{fileName, mimeType}`, server reserves an id and
//!   returns `{id, uploadUrl, expiresAt}`.
//! - `POST /attachments/{id}/confirm` — the uploader calls this after the direct-to-S3 PUT;
//!   the server HEADs the object and flips the row to ready. Returns the final [`Attachment`]
//!   with a public `downloadUrl`.
//! - `GET /attachments/{id}` — metadata and `downloadUrl` of a ready attachment, for its
//!   uploader or anyone who may view a message it is in.
//! - `DELETE /attachments/{id}` — the uploader drops an attachment in no message, row and
//!   object.
//!
//! Anyone else is answered as if the attachment did not exist.
//!
//! Bytes do not flow through these handlers in either direction.

use crate::api::auth::SessionUser;
use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::{Created, Json, NoContent, Path};
use crate::api::{API_PREFIX, TAG_ATTACHMENTS};
use crate::app::context::GlobalServerContext;
use crate::app::{self, AttachmentId};
use axum::extract::State;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Wire-level representation of an attachment. Clients fetch the bytes themselves from the
/// anonymous-read endpoint behind `downloadUrl`.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: AttachmentId,
    pub file_name: String,
    pub mime_type: String,
    pub download_url: String,
    /// A picture's size in pixels as its uploader measured it, both or neither, so a reader
    /// can make room for it before it loads.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

pub(crate) fn attachment_to_api(
    state: &GlobalServerContext,
    row: app::attachment::Attachment,
) -> Attachment {
    let download_url = state.media_store.public_url(&row.storage_key);
    Attachment {
        id: row.id,
        file_name: row.file_name,
        mime_type: row.mime_type,
        download_url,
        width: row.width.and_then(|w| u32::try_from(w).ok()),
        height: row.height.and_then(|h| u32::try_from(h).ok()),
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUploadInitRequest {
    pub file_name: String,
    pub mime_type: String,
    /// A picture's size in pixels, both or neither, each at most
    /// `app::attachment::MAX_PICTURE_SIDE`; a client that measures pictures before sending them
    /// gives it, so readers can make room for the picture before it loads.
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

/// A reserved attachment slot. `PUT` the file bytes to `uploadUrl` before `expiresAt`, then
/// confirm the upload.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUploadHandle {
    pub id: AttachmentId,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
}

#[utoipa::path(
    post,
    path = "/attachments",
    tag = TAG_ATTACHMENTS,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = AttachmentUploadHandle, headers(("Location" = String, description = "URL of the attachment once confirmed"))),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn init_attachment_upload(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(request): Json<AttachmentUploadInitRequest>,
) -> ApiResult<Created<AttachmentUploadHandle>> {
    let size = app::attachment::picture_size(request.width, request.height)?;
    let upload =
        app::attachment::init_upload(&state, user.id, request.file_name, request.mime_type, size)
            .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/attachments/{}", upload.id.0),
        AttachmentUploadHandle {
            id: upload.id,
            upload_url: upload.upload_url,
            expires_at: upload.expires_at,
        },
    ))
}

#[utoipa::path(
    post,
    path = "/attachments/{attachment}/confirm",
    tag = TAG_ATTACHMENTS,
    params(("attachment" = AttachmentId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Attachment),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (object not found in storage)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn confirm_attachment_upload(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(attachment): Path<AttachmentId>,
) -> ApiResult<Json<Attachment>> {
    let row = app::attachment::confirm_upload(&state, user.id, attachment)
        .await
        .map_err(|e| match e {
            app::Error::Validation(reason) => {
                ApiError::new(ProblemCode::Validation).with_detail(reason)
            }
            other => other.into(),
        })?;
    Ok(Json(attachment_to_api(&state, row)))
}

#[utoipa::path(
    get,
    path = "/attachments/{attachment}",
    tag = TAG_ATTACHMENTS,
    params(("attachment" = AttachmentId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Attachment),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_attachment(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(attachment): Path<AttachmentId>,
) -> ApiResult<Json<Attachment>> {
    let row = app::attachment::read_attachment(&state, user.id, attachment).await?;
    Ok(Json(attachment_to_api(&state, row)))
}

#[utoipa::path(
    delete,
    path = "/attachments/{attachment}",
    tag = TAG_ATTACHMENTS,
    params(("attachment" = AttachmentId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_attachment(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(attachment): Path<AttachmentId>,
) -> ApiResult<NoContent> {
    app::attachment::delete_attachment(&state, user.id, attachment).await?;
    Ok(NoContent)
}
