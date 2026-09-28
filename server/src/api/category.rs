use crate::api::auth::SessionUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Created, Json, NoContent, Path};
use crate::api::message_enum::Channel;
use crate::api::message_enum::request::{CategoryCreateRequest, CategoryUpdateRequest};
use crate::api::{API_PREFIX, GlobalServerContext, TAG_CATEGORIES, message_enum};
use crate::app::{CategoryId, CommunityId};
use crate::{api, app};
use axum::extract::State;

pub fn category_to_api(c: crate::app::category::Category) -> message_enum::Category {
    message_enum::Category {
        id: c.id,
        name: c.name,
        sort_index: c.sort_index,
        community: *c.community.id(),
    }
}

#[utoipa::path(
    post,
    path = "/communities/{community}/categories",
    tag = TAG_CATEGORIES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = message_enum::Category, headers(("Location" = String, description = "URL of the new category"))),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, description = "No such community, or the caller is not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_category(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Json(request): Json<CategoryCreateRequest>,
) -> ApiResult<Created<message_enum::Category>> {
    let c = app::category::create_category(
        &state,
        user.id,
        request.name,
        request.sort_index,
        community,
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/categories/{}", c.id.0),
        category_to_api(c),
    ))
}

#[utoipa::path(
    get,
    path = "/communities/{community}/categories",
    tag = TAG_CATEGORIES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<message_enum::Category>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such community, or the caller is not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_community_categories(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
) -> ApiResult<Json<Vec<message_enum::Category>>> {
    let categories = app::category::read_community_categories(&state, user.id, community).await?;
    Ok(Json(categories.into_iter().map(category_to_api).collect()))
}

#[utoipa::path(
    get,
    path = "/categories/{category}",
    tag = TAG_CATEGORIES,
    params(("category" = CategoryId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = message_enum::Category),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_category(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(category): Path<CategoryId>,
) -> ApiResult<Json<message_enum::Category>> {
    let c = app::category::read_category(&state, user.id, category).await?;
    Ok(Json(category_to_api(c)))
}

/// Channels filed under the category, in sort order.
#[utoipa::path(
    get,
    path = "/categories/{category}/channels",
    tag = TAG_CATEGORIES,
    params(("category" = CategoryId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<Channel>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_category_channels(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(category): Path<CategoryId>,
) -> ApiResult<Json<Vec<Channel>>> {
    let channels = app::category::read_category_channels(&state, user.id, category).await?;
    Ok(Json(
        channels
            .into_iter()
            .map(api::channel::channel_to_api)
            .collect(),
    ))
}

#[utoipa::path(
    patch,
    path = "/categories/{category}",
    tag = TAG_CATEGORIES,
    params(("category" = CategoryId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = message_enum::Category),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_category(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(category): Path<CategoryId>,
    Json(request): Json<CategoryUpdateRequest>,
) -> ApiResult<Json<message_enum::Category>> {
    let c = app::category::update_category(&state, user.id, category, request).await?;
    Ok(Json(category_to_api(c)))
}

#[utoipa::path(
    delete,
    path = "/categories/{category}",
    tag = TAG_CATEGORIES,
    params(("category" = CategoryId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_category(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(category): Path<CategoryId>,
) -> ApiResult<NoContent> {
    app::category::delete_category(&state, user.id, category).await?;
    Ok(NoContent)
}
