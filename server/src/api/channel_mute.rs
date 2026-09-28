//! Muting channels and DMs, for the caller alone (`app::channel_mute`). Community and DM list
//! reads sideload the caller's mutes with `include=mutes`.

use crate::api::auth::SessionUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Json, NoContent, Path};
use crate::api::{GlobalServerContext, TAG_CHANNELS};
use crate::app::{self, ChannelId};
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A channel the caller has muted: dimmed in their lists and never shown there as unread.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMute {
    pub channel: ChannelId,
    /// When the mute ends by itself; `null` for a mute that lasts until it is lifted.
    pub until: Option<DateTime<Utc>>,
}

pub fn mute_to_api(mute: app::channel_mute::ChannelMute) -> ChannelMute {
    ChannelMute {
        channel: mute.channel,
        until: mute.until,
    }
}

/// How long to mute a channel for.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelMuteRequest {
    /// Seconds from now, at most `app::channel_mute::MAX_MUTE_SECONDS` (a year); absent or
    /// `null` to mute until the caller unmutes it.
    #[serde(default)]
    pub duration_seconds: Option<u32>,
}

/// Mutes a text channel or DM for the caller, replacing any mute already on it. The change
/// reaches the caller's devices as `channelMuteChanged`.
#[utoipa::path(
    put,
    path = "/channels/{channel}/mutes/@me",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    request_body = ChannelMuteRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Muted", body = ChannelMute),
        (status = OK, description = "Was muted already; the mute now ends as asked", body = ChannelMute),
        (status = BAD_REQUEST, description = "`validation`: not a text channel or DM, or too long", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn mute_channel(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Json(request): Json<ChannelMuteRequest>,
) -> ApiResult<(StatusCode, Json<ChannelMute>)> {
    let (mute, existed) =
        app::channel_mute::mute(&state, user.id, channel, request.duration_seconds).await?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(mute_to_api(mute))))
}

/// Lifts the caller's mute of a channel. Nothing to lift is not an error.
#[utoipa::path(
    delete,
    path = "/channels/{channel}/mutes/@me",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Not muted"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn unmute_channel(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<NoContent> {
    app::channel_mute::unmute(&state, user.id, channel).await?;
    Ok(NoContent)
}
