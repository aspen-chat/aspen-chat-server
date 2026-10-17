//! Read positions: how far the caller has read each channel (`app::read_state`). Community and
//! DM list reads sideload them with `include=readStates`, as message reads do those of the
//! channels and threads their messages were posted in; these endpoints read one and move it.

use crate::TAG_CHANNELS;
use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Json, NoContent, Path};
use crate::t;
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

/// Where the caller has read a channel up to, or where it is unread from. Exactly one of the
/// fields is given.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadStateUpdate {
    /// A message of the channel, deleted or not, that the caller has now seen. The position
    /// moves forward to it, never back.
    #[serde(default)]
    pub last_read: Option<MessageId>,
    /// A message of the channel, not deleted, to mark unread with everything after it: the
    /// position moves to just before it, back or forward.
    #[serde(default)]
    pub unread_from: Option<MessageId>,
    /// `true` marks the channel unread from its newest message by neither the caller nor anyone
    /// they blocked, among its newest `app::read_state::NEWEST_LOOKED_AT` (100).
    #[serde(default)]
    pub unread_newest: bool,
}

/// The caller's read position in a channel or thread they belong to.
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
        (status = NOT_FOUND, description = "Not a channel the caller belongs to", body = Problem),
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

/// Moves the caller's read position in a channel forward to a message they have seen
/// (`lastRead`), or marks the channel unread from a message on (`unreadFrom`, `unreadNewest`).
/// Reading leaves a position already past the message where it is, so devices reporting out of
/// order never move it back; marking unread is the one way back. A read is announced to the
/// caller's devices as `channelRead`, marking unread as `channelMarkedUnread`.
#[utoipa::path(
    put,
    path = "/channels/{channel}/read-states/@me",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    request_body = ReadStateUpdate,
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Recorded"),
        (status = BAD_REQUEST, description = "`validation`: not exactly one field given, or nothing by anyone else to mark unread among the channel's newest messages", body = Problem),
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
    use app::read_state::UnreadFrom;
    match (update.last_read, update.unread_from, update.unread_newest) {
        (Some(last_read), None, false) => {
            app::read_state::mark_read(&state, user.id, channel, last_read).await?;
        }
        (None, Some(message), false) => {
            app::read_state::mark_unread(&state, user.id, channel, UnreadFrom::Message(message))
                .await?;
        }
        (None, None, true) => {
            app::read_state::mark_unread(&state, user.id, channel, UnreadFrom::Newest).await?;
        }
        _ => {
            return Err(app::Error::Validation(t!("readStateUpdateOneField")).into());
        }
    }
    Ok(NoContent)
}

/// A channel, DM, or thread, and where the caller has read it up to.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadPosition {
    pub channel: ChannelId,
    /// A message of the channel, deleted or not, that the caller has now seen.
    pub last_read: MessageId,
}

/// Where the caller has read several channels up to.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReadStatesUpdate {
    /// At most `app::read_state::MAX_MARKED` (100) positions.
    pub positions: Vec<ReadPosition>,
}

/// Moves the caller's read positions in several channels, DMs, and threads forward at once, each
/// as `PUT /channels/{channel}/read-states/@me` moves one, and each move announced to the
/// caller's devices as `channelRead`. A position is passed over, not refused, when its channel
/// is not one the caller may read or its message is not of that channel.
#[utoipa::path(
    put,
    path = "/users/@me/read-states",
    tag = TAG_CHANNELS,
    request_body = ReadStatesUpdate,
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Recorded"),
        (status = BAD_REQUEST, description = "`validation`: more positions than one request may move", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn put_read_states(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(update): Json<ReadStatesUpdate>,
) -> ApiResult<NoContent> {
    let positions: Vec<app::read_state::Position> = update
        .positions
        .into_iter()
        .map(|p| app::read_state::Position {
            channel: p.channel,
            message: p.last_read,
        })
        .collect();
    app::read_state::mark_many_read(&state, user.id, &positions).await?;
    Ok(NoContent)
}
