//! Reactions are modelled as a set keyed by (message, emoji, user), so adding one is an
//! idempotent `PUT` on `/messages/{message}/reactions/{emoji}/@me` and removing one is a
//! `DELETE` on the same URL. Only the calling user's own reaction can be addressed.

use crate::api::auth::SessionUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Json, NoContent, Path};
use crate::api::message_enum::React;
use crate::api::{GlobalServerContext, TAG_REACTIONS};
use crate::app;
use crate::app::MessageId;
use axum::extract::State;
use axum::http::StatusCode;
use diesel::result::DatabaseErrorKind;

#[utoipa::path(
    put,
    path = "/messages/{message}/reactions/{emoji}/@me",
    tag = TAG_REACTIONS,
    params(
        ("message" = MessageId, Path),
        ("emoji" = String, Path, description = "A single Unicode emoji, percent-encoded"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Reaction added", body = React),
        (status = OK, description = "Reaction already present", body = React),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (not a single emoji)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn add_reaction(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((message, emoji)): Path<(MessageId, String)>,
) -> ApiResult<(StatusCode, Json<React>)> {
    let record = React {
        message_id: message,
        emoji: emoji.clone(),
        user_id: user.id,
    };
    match app::react::create_react(&state, user.id, message, emoji).await {
        Ok(_) => Ok((StatusCode::CREATED, Json(record))),
        Err(app::Error::Diesel(diesel::result::Error::DatabaseError(
            DatabaseErrorKind::UniqueViolation,
            _,
        ))) => Ok((StatusCode::OK, Json(record))),
        Err(e) => Err(e.into()),
    }
}

#[utoipa::path(
    delete,
    path = "/messages/{message}/reactions/{emoji}/@me",
    tag = TAG_REACTIONS,
    params(
        ("message" = MessageId, Path),
        ("emoji" = String, Path, description = "A single Unicode emoji, percent-encoded"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_reaction(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((message, emoji)): Path<(MessageId, String)>,
) -> ApiResult<NoContent> {
    app::react::delete_react(&state, user.id, message, emoji).await?;
    Ok(NoContent)
}
