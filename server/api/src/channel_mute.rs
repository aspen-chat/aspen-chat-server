//! Muting channels and DMs, for the caller alone (`app::channel_mute`), and keeping a channel
//! unmuted while its category is muted. Community and DM list reads sideload the caller's
//! settings with `include=mutes`.

use crate::TAG_CHANNELS;
use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Json, NoContent, Path};
use crate::t;
use aspen_app::channel_mute::ChannelMuteChoice;
use aspen_app::context::GlobalServerContext;
use aspen_app::{self as app, ChannelId};
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The caller's own setting for a channel: muted, dimmed in their lists and never shown there as
/// unread; or kept unmuted while its category is muted.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMute {
    pub channel: ChannelId,
    /// When the mute ends by itself; `null` for a mute that lasts until it is lifted, and for a
    /// channel kept unmuted.
    pub until: Option<DateTime<Utc>>,
    /// `true` for a mute, `false` for a channel kept unmuted while its category is muted.
    pub muted: bool,
}

impl From<app::channel_mute::ChannelMute> for ChannelMute {
    fn from(mute: app::channel_mute::ChannelMute) -> Self {
        ChannelMute {
            channel: mute.channel,
            until: mute.until,
            muted: mute.muted,
        }
    }
}

fn mutes() -> bool {
    true
}

/// How long to mute a channel for, or to keep it unmuted.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelMuteRequest {
    /// Seconds from now, at most `app::channel_mute::MAX_MUTE_SECONDS` (a year); absent or
    /// `null` to mute until the caller unmutes it.
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    /// `false` keeps the channel unmuted while its category is muted, for as long as the setting
    /// stands, so it takes no duration. `true` (the default) mutes it.
    #[serde(default = "mutes")]
    pub muted: bool,
}

/// Mutes a text channel or DM for the caller, or keeps it unmuted while its category is muted,
/// replacing any setting already on it. The change reaches the caller's devices as
/// `channelMuteChanged`.
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
        (status = BAD_REQUEST, description = "`validation`: not a text channel or DM, too long, or kept unmuted for a while", body = Problem),
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
    let choice = match (request.muted, request.duration_seconds) {
        (true, duration_seconds) => ChannelMuteChoice::Muted { duration_seconds },
        (false, None) => ChannelMuteChoice::KeptUnmuted,
        (false, Some(_)) => {
            return Err(app::Error::Validation(t!("keptUnmutedLasts")).into());
        }
    };
    let (mute, existed) = app::channel_mute::mute(&state, user.id, channel, choice).await?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(ChannelMute::from(mute))))
}

/// Removes the caller's own setting for a channel, mute or kept unmuted, so it follows its
/// category. Nothing to remove is not an error.
#[utoipa::path(
    delete,
    path = "/channels/{channel}/mutes/@me",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "No setting of its own"),
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
