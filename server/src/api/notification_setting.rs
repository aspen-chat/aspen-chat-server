//! What the caller wants to be told of, for a community or a channel (`app::notification_setting`).
//! Community and DM list reads sideload the caller's settings with `include=notifications`.

use crate::api::auth::SessionUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Json, NoContent, Path};
use crate::api::{TAG_CHANNELS, TAG_COMMUNITIES};
use crate::app::context::GlobalServerContext;
use crate::app::notification_setting::{NotificationLevel, NotificationTarget};
use crate::app::{self, ChannelId, CommunityId};
use axum::extract::State;
use axum::http::StatusCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The caller's setting for a community (with `community`) or a channel (with `channel`).
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSetting {
    pub community: Option<CommunityId>,
    pub channel: Option<ChannelId>,
    pub level: NotificationLevel,
}

impl From<app::notification_setting::NotificationSetting> for NotificationSetting {
    fn from(setting: app::notification_setting::NotificationSetting) -> Self {
        NotificationSetting {
            community: setting.community,
            channel: setting.channel,
            level: setting.level,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSettingRequest {
    pub level: NotificationLevel,
}

fn created(existed: bool) -> StatusCode {
    if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    }
}

/// Sets what the caller is told of a community's channels that have no setting of their own.
/// The change reaches their devices as `notificationSettingChanged`.
#[utoipa::path(
    put,
    path = "/communities/{community}/notification-settings/@me",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path)),
    request_body = NotificationSettingRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Set", body = NotificationSetting),
        (status = OK, description = "Replaced the caller's setting", body = NotificationSetting),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such community, or the caller is not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn set_community_notifications(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Json(request): Json<NotificationSettingRequest>,
) -> ApiResult<(StatusCode, Json<NotificationSetting>)> {
    let existed = app::notification_setting::set(
        &state,
        user.id,
        NotificationTarget::Community(community),
        request.level,
    )
    .await?;
    Ok((
        created(existed),
        Json(NotificationSetting {
            community: Some(community),
            channel: None,
            level: request.level,
        }),
    ))
}

/// Returns a community to the default: tags only.
#[utoipa::path(
    delete,
    path = "/communities/{community}/notification-settings/@me",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn reset_community_notifications(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
) -> ApiResult<NoContent> {
    app::notification_setting::reset(&state, user.id, NotificationTarget::Community(community))
        .await?;
    Ok(NoContent)
}

/// Sets what the caller is told of a text channel or DM, whatever its community's setting. A
/// thread follows its channel. The change reaches their devices as `notificationSettingChanged`.
#[utoipa::path(
    put,
    path = "/channels/{channel}/notification-settings/@me",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    request_body = NotificationSettingRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Set", body = NotificationSetting),
        (status = OK, description = "Replaced the caller's setting", body = NotificationSetting),
        (status = BAD_REQUEST, description = "`validation`: not a text channel or DM", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn set_channel_notifications(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Json(request): Json<NotificationSettingRequest>,
) -> ApiResult<(StatusCode, Json<NotificationSetting>)> {
    let existed = app::notification_setting::set(
        &state,
        user.id,
        NotificationTarget::Channel(channel),
        request.level,
    )
    .await?;
    Ok((
        created(existed),
        Json(NotificationSetting {
            community: None,
            channel: Some(channel),
            level: request.level,
        }),
    ))
}

/// Returns a channel to its community's setting, or for a DM to every message.
#[utoipa::path(
    delete,
    path = "/channels/{channel}/notification-settings/@me",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn reset_channel_notifications(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<NoContent> {
    app::notification_setting::reset(&state, user.id, NotificationTarget::Channel(channel)).await?;
    Ok(NoContent)
}
