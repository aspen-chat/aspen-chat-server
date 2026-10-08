//! Threads the caller follows (`app::thread_follow`), each of whose replies tells them: their
//! list, and following and unfollowing one. Each change reaches the caller's devices as
//! `threadFollowChanged`.

use crate::TAG_CHANNELS;
use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Json, NoContent, Path};
use aspen_app::context::GlobalServerContext;
use aspen_app::{self as app, ChannelId};
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Serialize;
use utoipa::ToSchema;

/// A thread the caller follows.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ThreadFollow {
    pub thread: ChannelId,
    /// When the caller last followed it, by hand or by taking part. Past
    /// `app::thread_follow::MAX_THREAD_FOLLOWS` follows, those followed least recently are let
    /// go.
    pub followed_at: DateTime<Utc>,
}

impl From<app::thread_follow::ThreadFollow> for ThreadFollow {
    fn from(follow: app::thread_follow::ThreadFollow) -> Self {
        ThreadFollow {
            thread: follow.thread,
            followed_at: follow.followed_at,
        }
    }
}

/// The threads the caller follows that they may read now, most recently followed first.
#[utoipa::path(
    get,
    path = "/users/@me/thread-follows",
    tag = TAG_CHANNELS,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<ThreadFollow>),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_follows(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
) -> ApiResult<Json<Vec<ThreadFollow>>> {
    let follows = app::thread_follow::read_follows(&state, user.id).await?;
    Ok(Json(follows.into_iter().map(ThreadFollow::from).collect()))
}

/// Follows a thread for the caller.
#[utoipa::path(
    put,
    path = "/channels/{channel}/follows/@me",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Followed", body = ThreadFollow),
        (status = OK, description = "Was followed already", body = ThreadFollow),
        (status = BAD_REQUEST, description = "`validation`: not a thread", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn follow_thread(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<(StatusCode, Json<ThreadFollow>)> {
    let (follow, existed) = app::thread_follow::follow(&state, user.id, channel).await?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(ThreadFollow::from(follow))))
}

/// Stops following a thread for the caller. Nothing to stop is not an error.
#[utoipa::path(
    delete,
    path = "/channels/{channel}/follows/@me",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Not followed"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn unfollow_thread(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<NoContent> {
    app::thread_follow::unfollow(&state, user.id, channel).await?;
    Ok(NoContent)
}
