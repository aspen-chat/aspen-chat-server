use crate::api::auth::SessionUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Created, Json, NoContent, Path};
use crate::api::message_enum::Pin;
use crate::api::message_enum::request::{ChannelCreateRequest, ChannelUpdateRequest};
use crate::api::{API_PREFIX, GlobalServerContext, TAG_CHANNELS, message_enum};
use crate::app;
use crate::app::{ChannelId, MaybeLoaded};
use axum::extract::State;

pub fn channel_to_api(c: app::channel::Channel) -> message_enum::Channel {
    message_enum::Channel {
        id: c.id,
        parent_category: c.parent_category.as_ref().map(MaybeLoaded::id).cloned(),
        community: c.community.as_ref().map(MaybeLoaded::id).cloned(),
        name: c.name,
        sort_index: c.sort_index,
        ty: c.ty,
    }
}

/// Creates a channel. A channel may belong to a community and optionally to a category within it;
/// both references are set through the request body because either may be absent.
#[utoipa::path(
    post,
    path = "/channels",
    tag = TAG_CHANNELS,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = message_enum::Channel, headers(("Location" = String, description = "URL of the new channel"))),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_channel(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
    Json(request): Json<ChannelCreateRequest>,
) -> ApiResult<Created<message_enum::Channel>> {
    let c = app::channel::create_channel(
        &state,
        request.name,
        request.sort_index,
        request.ty,
        request.community,
        request.parent_category,
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/channels/{}", c.id.0),
        channel_to_api(c),
    ))
}

#[utoipa::path(
    get,
    path = "/channels/{channel}",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = message_enum::Channel),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_channel(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<Json<message_enum::Channel>> {
    let c = app::channel::read_channel(&state, channel).await?;
    Ok(Json(channel_to_api(c)))
}

/// Pinned messages in the channel, in pin order.
#[utoipa::path(
    get,
    path = "/channels/{channel}/pins",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<Pin>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_channel_pins(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<Json<Vec<Pin>>> {
    let pins = app::channel::read_channel_pins(&state, channel).await?;
    Ok(Json(
        pins.into_iter()
            .map(|p| Pin {
                message_id: p.message_id,
                timestamp: p.timestamp,
                sort_index: p.sort_index,
            })
            .collect(),
    ))
}

#[utoipa::path(
    patch,
    path = "/channels/{channel}",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = message_enum::Channel),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_channel(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
    Path(channel): Path<ChannelId>,
    Json(request): Json<ChannelUpdateRequest>,
) -> ApiResult<Json<message_enum::Channel>> {
    let c = app::channel::update_channel(&state, channel, request).await?;
    Ok(Json(channel_to_api(c)))
}

#[utoipa::path(
    delete,
    path = "/channels/{channel}",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_channel(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<NoContent> {
    app::channel::delete_channel(&state, channel).await?;
    Ok(NoContent)
}
