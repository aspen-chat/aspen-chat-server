//! Read positions: how far the caller has read each channel (`app::read_state`). Community and
//! DM list reads sideload them with `include=readStates`; these endpoints read one and move it.

use crate::TAG_CHANNELS;
use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Json, NoContent, Path};
use aspen_app::context::GlobalServerContext;
use aspen_app::{self as app, ChannelId, MessageId};
use axum::extract::State;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// How far the caller has read a channel. The channel is unread while `lastMessage` is after
/// `lastRead`.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadState {
    pub channel: ChannelId,
    /// A position among the channel's message ids, which are ordered by time: every message
    /// after it is unread. It may name a message since deleted, or no message at all when it is
    /// the moment the caller joined, before which nothing is unread to them.
    pub last_read: MessageId,
    /// The channel's newest message by neither the caller nor anyone they blocked, if any.
    pub last_message: Option<MessageId>,
    /// How many of the unread messages tag the caller: directly, through a role they hold, or
    /// as everyone. Tags count in muted channels too.
    pub mentions: u32,
}

impl From<app::read_state::ReadState> for ReadState {
    fn from(state: app::read_state::ReadState) -> Self {
        ReadState {
            channel: state.channel,
            last_read: state.last_read,
            last_message: state.last_message,
            mentions: state.mentions,
        }
    }
}

/// Where the caller has read a channel up to.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadStateUpdate {
    /// A message of the channel, deleted or not, that the caller has now seen.
    pub last_read: MessageId,
}

/// The caller's read position in a channel they belong to. Threads keep none.
#[utoipa::path(
    get,
    path = "/channels/{channel}/read-states/@me",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ReadState),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "Not a channel the caller belongs to, or a thread", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_read_state(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<Json<ReadState>> {
    let read = app::read_state::read_read_state(&state, user.id, channel).await?;
    Ok(Json(ReadState::from(read)))
}

/// Moves the caller's read position in a channel forward to a message they have seen. A
/// position already past it is left where it is, so devices reporting out of order never move
/// it back. A move is announced to the caller's devices as `channelRead`.
#[utoipa::path(
    put,
    path = "/channels/{channel}/read-states/@me",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    request_body = ReadStateUpdate,
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Recorded"),
        (status = BAD_REQUEST, description = "`validation` for a thread", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such channel, or no such message in it", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn put_read_state(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Json(update): Json<ReadStateUpdate>,
) -> ApiResult<NoContent> {
    app::read_state::mark_read(&state, user.id, channel, update.last_read).await?;
    Ok(NoContent)
}
