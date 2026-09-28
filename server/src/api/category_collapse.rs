//! Collapsing categories in the caller's own channel list (`app::category_collapse`). Community
//! reads sideload the caller's collapsed categories with `include=collapses`.

use crate::api::auth::SessionUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Json, NoContent, Path};
use crate::api::{GlobalServerContext, TAG_CATEGORIES};
use crate::app::{self, CategoryId};
use axum::extract::State;
use axum::http::StatusCode;
use schemars::JsonSchema;
use serde::Serialize;
use utoipa::ToSchema;

/// A category the caller has collapsed in their channel list.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoryCollapse {
    pub category: CategoryId,
}

/// Collapses a category in the caller's channel list. The change reaches the caller's devices
/// as `categoryCollapseChanged`.
#[utoipa::path(
    put,
    path = "/categories/{category}/collapses/@me",
    tag = TAG_CATEGORIES,
    params(("category" = CategoryId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Collapsed", body = CategoryCollapse),
        (status = OK, description = "Was collapsed already", body = CategoryCollapse),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn collapse_category(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(category): Path<CategoryId>,
) -> ApiResult<(StatusCode, Json<CategoryCollapse>)> {
    let already = app::category_collapse::collapse(&state, user.id, category).await?;
    let status = if already {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(CategoryCollapse { category })))
}

/// Expands a category in the caller's channel list. Expanding one that is not collapsed is not
/// an error.
#[utoipa::path(
    delete,
    path = "/categories/{category}/collapses/@me",
    tag = TAG_CATEGORIES,
    params(("category" = CategoryId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Expanded"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn expand_category(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(category): Path<CategoryId>,
) -> ApiResult<NoContent> {
    app::category_collapse::expand(&state, user.id, category).await?;
    Ok(NoContent)
}
