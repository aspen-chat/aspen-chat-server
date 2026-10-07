//! REST surface for the two-phase attachment upload flow.
//!
//! - `POST /attachments` — caller sends `{fileName, mimeType, byteSize}`, server reserves an id
//!   and returns `{id, uploadUrl, expiresAt, contentType}`.
//! - `POST /attachments/{id}/confirm` — the uploader calls this after the direct-to-S3 PUT;
//!   the server moves the object into place, refusing one over the deployment's limit, and
//!   flips the row to ready. Returns the final [`Attachment`]
//!   with a public `downloadUrl`.
//! - `GET /attachments/{id}` — metadata and `downloadUrl` of a ready attachment, for its
//!   uploader or anyone who may view a message it is in.
//! - `PATCH /attachments/{id}` — the uploader sets or clears its description until it is sent.
//! - `DELETE /attachments/{id}` — the uploader drops an attachment in no message, row and
//!   object.
//!
//! Anyone else is answered as if the attachment did not exist.
//!
//! Bytes do not flow through these handlers in either direction.

use crate::auth::SessionUser;
use crate::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::extract::{Created, Json, NoContent, Path, double_option};
use crate::{API_PREFIX, TAG_ATTACHMENTS};
use aspen_app::context::GlobalServerContext;
use aspen_app::{self as app, AttachmentId};
pub use aspen_wire::attachment::AttachmentPreview;
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
    /// What a picture or video shows, in its uploader's words, for readers who cannot see it;
    /// absent when it has none. Apps give it as the attachment's text alternative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// A smaller copy of a picture or video, made by the server for showing it inline, where
    /// one was made and is worth having (`app::attachment::preview`); absent otherwise, and
    /// until it is made, which `attachmentPreviewed` announces. The original, at `downloadUrl`,
    /// is what is shown at full size and saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<AttachmentPreview>,
}

pub fn attachment_to_api(
    state: &GlobalServerContext,
    row: app::attachment::Attachment,
) -> Attachment {
    let download_url = state.media_store.public_url(&row.storage_key);
    let preview = app::attachment::preview::of(state, &row);
    Attachment {
        id: row.id,
        file_name: row.file_name,
        mime_type: row.mime_type,
        download_url,
        width: row.width.and_then(|w| u32::try_from(w).ok()),
        height: row.height.and_then(|h| u32::try_from(h).ok()),
        description: row.description,
        preview,
    }
}

/// An attachment as a review reads it: one kept as evidence (`app::attachment::evidence`) at
/// URLs signed for `app::attachment::evidence::URL_LIFETIME`, which the anonymous read path does
/// not serve, and any other as everyone reads it.
pub async fn attachment_for_review(
    state: &GlobalServerContext,
    row: app::attachment::Attachment,
) -> ApiResult<Attachment> {
    if row.evidence_at.is_none() {
        return Ok(attachment_to_api(state, row));
    }
    let (original, preview_url) = app::attachment::evidence::signed_urls(state, &row).await?;
    let mut attachment = attachment_to_api(state, row);
    attachment.download_url = original;
    if let (Some(preview), Some(url)) = (attachment.preview.as_mut(), preview_url) {
        preview.url = url;
    }
    Ok(attachment)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUploadInitRequest {
    /// At most 255 characters (`app::attachment::MAX_FILE_NAME_CHARS`).
    pub file_name: String,
    /// What the file is, as the uploader's system names it; any type, at most 255 bytes
    /// (`app::attachment::MAX_MIME_TYPE_BYTES`) of printable ASCII with no commas or quotes.
    /// Kept on the record and shown by apps; the file is uploaded and served as `contentType`
    /// on the handle.
    pub mime_type: String,
    /// The file's size in bytes, at most the deployment's `[media] max_attachment_bytes`; the
    /// upload URL accepts exactly this many bytes. Required: a request without it is refused
    /// with `validation`.
    #[serde(default)]
    #[schema(required = true, nullable = false, value_type = u64)]
    pub byte_size: Option<u64>,
    /// A picture's size in pixels, both or neither, each at most
    /// `app::attachment::MAX_PICTURE_SIDE`; a client that measures pictures before sending them
    /// gives it, so readers can make room for the picture before it loads.
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// What the picture or video shows, at most `app::attachment::DESCRIPTION_MAX_CHARS`
    /// characters; blank is none.
    #[serde(default)]
    pub description: Option<String>,
}

/// A change to an attachment not yet sent, as a merge patch.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUpdateRequest {
    /// What the picture or video shows, at most `app::attachment::DESCRIPTION_MAX_CHARS`
    /// characters; `null` or blank clears it.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(nullable)]
    pub description: Option<Option<String>>,
}

/// A reserved attachment slot. `PUT` the file bytes to `uploadUrl` before `expiresAt`, with
/// `Content-Type: {contentType}`, then confirm the upload.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentUploadHandle {
    pub id: AttachmentId,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
    /// The `Content-Type` the upload must be sent with, which the URL is signed for: the
    /// declared `mimeType` without its parameters (but a plain text's `charset`) for a kind apps
    /// show in place (`app::attachment::INLINE_TYPES`), otherwise `application/octet-stream`,
    /// for a file to be saved. Absent from a deployment
    /// that predates it, whose URL is signed for the declared `mimeType`.
    #[schema(required = false)]
    pub content_type: String,
}

#[utoipa::path(
    post,
    path = "/attachments",
    tag = TAG_ATTACHMENTS,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = AttachmentUploadHandle, headers(("Location" = String, description = "URL of the attachment once confirmed"))),
        (status = BAD_REQUEST, description = "`validation`, as when `byteSize` is missing or over the deployment's limit, `fileName` is too long, or `mimeType` is too long or holds a comma, a quote, or a control character", body = Problem),
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
    let description = app::attachment::description(request.description)?;
    let upload = app::attachment::init_upload(
        &state,
        user.id,
        request.file_name,
        request.mime_type,
        request.byte_size,
        size,
        description,
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/attachments/{}", upload.id.0),
        AttachmentUploadHandle {
            id: upload.id,
            upload_url: upload.upload_url,
            expires_at: upload.expires_at,
            content_type: upload.content_type,
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
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (object not found in storage, or larger than the deployment allows, and deleted)", body = Problem),
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
    patch,
    path = "/attachments/{attachment}",
    tag = TAG_ATTACHMENTS,
    params(("attachment" = AttachmentId, Path)),
    request_body = AttachmentUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Attachment),
        (status = BAD_REQUEST, description = "`validation`: the description is too long", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No attachment of the caller's that is not yet sent", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_attachment(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(attachment): Path<AttachmentId>,
    Json(request): Json<AttachmentUpdateRequest>,
) -> ApiResult<Json<Attachment>> {
    let row =
        app::attachment::describe_attachment(&state, user.id, attachment, request.description)
            .await?;
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
