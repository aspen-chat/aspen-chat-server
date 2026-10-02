//! Bans from a community (`app::ban`): listing them, banning, and lifting, which take Ban
//! members. Banning may also delete the person's recent messages, which takes Manage messages
//! besides.

use crate::api::auth::SessionUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Json, NoContent, Path};
use crate::api::message_enum::CommunityBan;
use crate::api::{GlobalServerContext, TAG_BANS};
use crate::app;
use crate::app::ban::BanRequest;
use crate::app::{CommunityId, UserId};
use axum::extract::State;
use axum::http::StatusCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What a ban asks for.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommunityBanRequest {
    /// What the banned person is told when they try to come back; at most
    /// `app::ban::REASON_MAX_CHARS` characters.
    #[serde(default)]
    pub reason: Option<String>,
    /// How long the ban lasts, in seconds, at least a minute; absent or `null` until lifted.
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    /// How far back the person's messages in the community are deleted with the ban: 3600
    /// (the last hour) or 86400 (the last day); absent or `null` to leave them. Takes Manage
    /// messages.
    #[serde(default)]
    pub delete_messages_seconds: Option<u32>,
}

/// A ban as made, with what it did.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommunityBanOutcome {
    pub ban: CommunityBan,
    /// How many of the person's messages the ban deleted.
    pub deleted_messages: u32,
}

/// The community's standing bans, newest first. Takes Ban members.
#[utoipa::path(
    get,
    path = "/communities/{community}/bans",
    tag = TAG_BANS,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<CommunityBan>),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: lacks Ban members", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_community_bans(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
) -> ApiResult<Json<Vec<CommunityBan>>> {
    Ok(Json(app::ban::read_bans(&state, user.id, community).await?))
}

/// Bans a person from the community: their membership ends, every way back in refuses them
/// with `banned` and the reason until the ban ends or is lifted, and, asked to, their messages
/// in the community from the last hour or day are deleted. Takes Ban members, over someone
/// below the caller's highest role and never the owner; deleting messages takes Manage
/// messages too. Someone who has already left may be banned by their id. A ban standing
/// already is replaced (`200`).
#[utoipa::path(
    put,
    path = "/communities/{community}/bans/{user}",
    tag = TAG_BANS,
    params(("community" = CommunityId, Path), ("user" = UserId, Path)),
    request_body = CommunityBanRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Banned", body = CommunityBanOutcome),
        (status = OK, description = "A ban stood already and is replaced", body = CommunityBanOutcome),
        (status = BAD_REQUEST, description = "`validation`: the caller themself, the reason, the duration, or the deletion window", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: lacks Ban members (or Manage messages, to delete), outranked, or the owner", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn ban_community_member(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((community, member)): Path<(CommunityId, UserId)>,
    Json(request): Json<CommunityBanRequest>,
) -> ApiResult<(StatusCode, Json<CommunityBanOutcome>)> {
    let banned = app::ban::ban_member(
        &state,
        user.id,
        community,
        member,
        &BanRequest {
            reason: request.reason,
            duration_seconds: request.duration_seconds,
            delete_messages_seconds: request.delete_messages_seconds,
        },
    )
    .await?;
    let status = if banned.replaced {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(CommunityBanOutcome {
            ban: banned.ban,
            deleted_messages: u32::try_from(banned.deleted_messages).unwrap_or(u32::MAX),
        }),
    ))
}

/// Lifts a ban, after which the person may come back with an invite. Nothing standing is not
/// an error. Takes Ban members.
#[utoipa::path(
    delete,
    path = "/communities/{community}/bans/{user}",
    tag = TAG_BANS,
    params(("community" = CommunityId, Path), ("user" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: lacks Ban members", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn lift_community_ban(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((community, member)): Path<(CommunityId, UserId)>,
) -> ApiResult<NoContent> {
    app::ban::lift_ban(&state, user.id, community, member).await?;
    Ok(NoContent)
}
