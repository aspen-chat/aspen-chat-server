//! Muting categories, for the caller alone (`app::category_mute`): every channel in one is muted
//! for them unless the channel's own setting says otherwise. Community reads sideload the
//! caller's category mutes with `include=mutes`, as `included.categoryMutes`.

use crate::TAG_CATEGORIES;
use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Json, NoContent, Path};
use aspen_app::context::GlobalServerContext;
use aspen_app::{self as app, CategoryId};
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A category the caller has muted, and so every channel in it without a setting of its own.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoryMute {
    pub category: CategoryId,
    /// When the mute ends by itself; `null` for a mute that lasts until it is lifted.
    pub until: Option<DateTime<Utc>>,
}

impl From<app::category_mute::CategoryMute> for CategoryMute {
    fn from(mute: app::category_mute::CategoryMute) -> Self {
        CategoryMute {
            category: mute.category,
            until: mute.until,
        }
    }
}

/// How long to mute a category for.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoryMuteRequest {
    /// Seconds from now, at most `app::channel_mute::MAX_MUTE_SECONDS` (a year); absent or
    /// `null` to mute until the caller unmutes it.
    #[serde(default)]
    pub duration_seconds: Option<u32>,
}

/// Mutes a category for the caller, replacing any mute already on it. The change reaches the
/// caller's devices as `categoryMuteChanged`.
#[utoipa::path(
    put,
    path = "/categories/{category}/mutes/@me",
    tag = TAG_CATEGORIES,
    params(("category" = CategoryId, Path)),
    request_body = CategoryMuteRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Muted", body = CategoryMute),
        (status = OK, description = "Was muted already; the mute now ends as asked", body = CategoryMute),
        (status = BAD_REQUEST, description = "`validation`: too long", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn mute_category(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(category): Path<CategoryId>,
    Json(request): Json<CategoryMuteRequest>,
) -> ApiResult<(StatusCode, Json<CategoryMute>)> {
    let (mute, existed) =
        app::category_mute::mute(&state, user.id, category, request.duration_seconds).await?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(CategoryMute::from(mute))))
}

/// Lifts the caller's mute of a category. Nothing to lift is not an error.
#[utoipa::path(
    delete,
    path = "/categories/{category}/mutes/@me",
    tag = TAG_CATEGORIES,
    params(("category" = CategoryId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Not muted"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn unmute_category(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(category): Path<CategoryId>,
) -> ApiResult<NoContent> {
    app::category_mute::unmute(&state, user.id, category).await?;
    Ok(NoContent)
}
