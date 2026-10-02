//! A community's own emoji (`app::custom_emoji`): listing them, and adding, renaming, and
//! removing them, which take Manage custom emoji. Community reads sideload them with
//! `include=emoji`.

use crate::api::auth::SessionUser;
use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::{Created, Json, NoContent, Path};
use crate::api::message_enum::CustomEmoji;
use crate::api::message_enum::request::{CustomEmojiCreateRequest, CustomEmojiUpdateRequest};
use crate::api::{API_PREFIX, GlobalServerContext, TAG_CUSTOM_EMOJI};
use crate::app;
use crate::app::{CommunityId, CustomEmojiId};
use axum::extract::State;
use diesel::result::DatabaseErrorKind;

/// A community's emoji, by name.
#[utoipa::path(
    get,
    path = "/communities/{community}/emoji",
    tag = TAG_CUSTOM_EMOJI,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<CustomEmoji>),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_emoji(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
) -> ApiResult<Json<Vec<CustomEmoji>>> {
    Ok(Json(
        app::custom_emoji::read_emoji(&state, user.id, community).await?,
    ))
}

/// Adds an emoji from an icon uploaded first (`POST /icons`, a PNG, JPEG, WebP, or GIF of at
/// most 256 KiB and 128 by 128), named uniquely within the community. Takes Manage custom
/// emoji; the community may hold at most `[communities] custom_emoji_limit`.
#[utoipa::path(
    post,
    path = "/communities/{community}/emoji",
    tag = TAG_CUSTOM_EMOJI,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = CustomEmoji, headers(("Location" = String, description = "URL of the new emoji"))),
        (status = BAD_REQUEST, description = "`validation`: the name, the picture, or the community's limit", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: lacks Manage custom emoji", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`customEmojiNameTaken`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_emoji(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Json(request): Json<CustomEmojiCreateRequest>,
) -> ApiResult<Created<CustomEmoji>> {
    let emoji =
        app::custom_emoji::create_emoji(&state, user.id, community, &request.name, request.icon)
            .await
            .map_err(name_taken)?;
    Ok(Created::new(
        format!("{API_PREFIX}/emoji/{}", emoji.id.0),
        emoji,
    ))
}

/// Renames an emoji. Takes Manage custom emoji in its community.
#[utoipa::path(
    patch,
    path = "/emoji/{emoji}",
    tag = TAG_CUSTOM_EMOJI,
    params(("emoji" = CustomEmojiId, Path)),
    request_body = CustomEmojiUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = CustomEmoji),
        (status = BAD_REQUEST, description = "`validation`: the name", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: lacks Manage custom emoji", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`customEmojiNameTaken`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_emoji(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(emoji): Path<CustomEmojiId>,
    Json(request): Json<CustomEmojiUpdateRequest>,
) -> ApiResult<Json<CustomEmoji>> {
    Ok(Json(
        app::custom_emoji::update_emoji(&state, user.id, emoji, &request)
            .await
            .map_err(name_taken)?,
    ))
}

/// Removes an emoji, and with it its reactions and its picture. Takes Manage custom emoji in
/// its community.
#[utoipa::path(
    delete,
    path = "/emoji/{emoji}",
    tag = TAG_CUSTOM_EMOJI,
    params(("emoji" = CustomEmojiId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: lacks Manage custom emoji", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_emoji(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(emoji): Path<CustomEmojiId>,
) -> ApiResult<NoContent> {
    app::custom_emoji::delete_emoji(&state, user.id, emoji).await?;
    Ok(NoContent)
}

/// A unique violation here can only be the name, the one thing the table keeps unique.
fn name_taken(e: app::Error) -> ApiError {
    match e {
        app::Error::Diesel(diesel::result::Error::DatabaseError(
            DatabaseErrorKind::UniqueViolation,
            _,
        )) => ApiError::new(ProblemCode::CustomEmojiNameTaken),
        other => other.into(),
    }
}
