//! Blocking other users, for the caller alone (`app::block`). Only the caller's own list is
//! reachable: nobody may read who someone else has blocked, or learn that they are blocked.

use crate::api::TAG_USERS;
use crate::api::auth::SessionUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Json, NoContent, Path, Query};
use crate::api::include::{IncludeSet, Included, SideloadedList};
use crate::api::message_enum::User;
use crate::app::context::GlobalServerContext;
use crate::app::{self, UserId};
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Someone the caller has blocked: their messages are collapsed for the caller, their reactions
/// left out, and they are silenced and hidden in the caller's calls. No one-to-one DM between
/// them may be started or written in.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserBlock {
    pub user: UserId,
    pub created_at: DateTime<Utc>,
}

impl From<app::block::UserBlock> for UserBlock {
    fn from(block: app::block::UserBlock) -> Self {
        UserBlock {
            user: block.blocked,
            created_at: block.created_at,
        }
    }
}

/// Relationships a block list read can sideload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum BlockInclude {
    /// The blocked users, as `included.users`, since the caller may share no community with
    /// them any more.
    Users,
}

/// Body of a block list read; a named alias for the same reason as
/// `api::community::CommunityRead`.
pub type BlockList = SideloadedList<UserBlock>;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BlockListQuery {
    /// Related records to return alongside the blocks, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<BlockInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<BlockInclude>,
}

/// Everyone the caller has blocked, the most recent first. `include=users` sideloads them.
#[utoipa::path(
    get,
    path = "/users/@me/blocks",
    tag = TAG_USERS,
    params(BlockListQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = BlockList),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_blocks(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Query(query): Query<BlockListQuery>,
) -> ApiResult<Json<BlockList>> {
    let blocks = app::block::read_blocks(&state, user.id).await?;
    let users = if query.include.contains(BlockInclude::Users) {
        let ids: Vec<UserId> = blocks.iter().map(|b| b.blocked).collect();
        Some(
            app::user::read_users(&state, user.id, &ids)
                .await?
                .into_iter()
                .map(User::from)
                .collect(),
        )
    } else {
        None
    };
    Ok(Json(BlockList::new(
        blocks.into_iter().map(UserBlock::from).collect(),
        Included {
            users,
            ..Included::default()
        },
    )))
}

/// Blocks someone for the caller. They are not told. The change reaches the caller's devices as
/// `userBlockChanged`.
#[utoipa::path(
    put,
    path = "/users/@me/blocks/{user}",
    tag = TAG_USERS,
    params(("user" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Blocked", body = UserBlock),
        (status = OK, description = "Was blocked already", body = UserBlock),
        (status = BAD_REQUEST, description = "`validation`: the caller themself", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn block_user(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(blocked): Path<UserId>,
) -> ApiResult<(StatusCode, Json<UserBlock>)> {
    let (block, existed) = app::block::block(&state, user.id, blocked).await?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(UserBlock::from(block))))
}

/// Lifts the caller's block of someone. Nothing to lift is not an error.
#[utoipa::path(
    delete,
    path = "/users/@me/blocks/{user}",
    tag = TAG_USERS,
    params(("user" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Not blocked"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn unblock_user(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(blocked): Path<UserId>,
) -> ApiResult<NoContent> {
    app::block::unblock(&state, user.id, blocked).await?;
    Ok(NoContent)
}
