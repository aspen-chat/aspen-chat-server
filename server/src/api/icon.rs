//! REST surface for the two-phase icon upload flow. Mirrors [`crate::api::attachment`] except
//! that icons carry no file name.

use crate::api::auth::SessionUser;
use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::{Created, Json, NoContent, Path};
use crate::api::{API_PREFIX, TAG_ICONS};
use crate::app::context::GlobalServerContext;
use crate::app::{self, IconId};
use axum::extract::State;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Icon {
    pub id: IconId,
    pub mime_type: String,
    pub download_url: String,
}

pub(crate) fn icon_to_api(state: &GlobalServerContext, row: app::icon::Icon) -> Icon {
    let download_url = state.media_store.public_url(&row.storage_key);
    Icon {
        id: row.id,
        mime_type: row.mime_type,
        download_url,
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IconUploadInitRequest {
    /// `image/png`, `image/jpeg`, `image/webp`, or `image/gif`.
    pub mime_type: String,
}

/// A reserved icon slot. `PUT` the image bytes to `uploadUrl` before `expiresAt`, then confirm
/// the upload.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IconUploadHandle {
    pub id: IconId,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
}

#[utoipa::path(
    post,
    path = "/icons",
    tag = TAG_ICONS,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = IconUploadHandle, headers(("Location" = String, description = "URL of the icon once confirmed"))),
        (status = BAD_REQUEST, description = "`validation`: `mimeType` is not PNG, JPEG, WebP, or GIF", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn init_icon_upload(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(request): Json<IconUploadInitRequest>,
) -> ApiResult<Created<IconUploadHandle>> {
    let upload = app::icon::init_upload(&state, user.id, request.mime_type).await?;
    Ok(Created::new(
        format!("{API_PREFIX}/icons/{}", upload.id.0),
        IconUploadHandle {
            id: upload.id,
            upload_url: upload.upload_url,
            expires_at: upload.expires_at,
        },
    ))
}

#[utoipa::path(
    post,
    path = "/icons/{icon}/confirm",
    tag = TAG_ICONS,
    params(("icon" = IconId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Icon),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (object not found in storage)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such upload pending, or one the caller did not start", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn confirm_icon_upload(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(icon): Path<IconId>,
) -> ApiResult<Json<Icon>> {
    let row = app::icon::confirm_upload(&state, user.id, icon)
        .await
        .map_err(|e| match e {
            app::Error::Validation(reason) => {
                ApiError::new(ProblemCode::Validation).with_detail(reason)
            }
            other => other.into(),
        })?;
    Ok(Json(icon_to_api(&state, row)))
}

#[utoipa::path(
    get,
    path = "/icons/{icon}",
    tag = TAG_ICONS,
    params(("icon" = IconId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Icon),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_icon(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
    Path(icon): Path<IconId>,
) -> ApiResult<Json<Icon>> {
    let row = app::icon::read_icon(&state, icon).await?;
    Ok(Json(icon_to_api(&state, row)))
}

/// Deletes an icon the caller uploaded that nothing uses: no profile, community, custom emoji,
/// the deployment's profile, or a profile a report or warning keeps. Anyone else's icon is not
/// found.
#[utoipa::path(
    delete,
    path = "/icons/{icon}",
    tag = TAG_ICONS,
    params(("icon" = IconId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such icon, or one the caller did not upload", body = Problem),
        (status = CONFLICT, description = "`conflict`: something uses the icon", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_icon(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(icon): Path<IconId>,
) -> ApiResult<NoContent> {
    app::icon::delete_own_icon(&state, user.id, icon).await?;
    Ok(NoContent)
}
