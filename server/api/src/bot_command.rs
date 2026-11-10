//! The commands bots advertise (`app::bot_command`): publishing a bot's list, reading it, and
//! reading what a channel offers, which clients complete from.

use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Created, Json, Path};
use crate::include::Listed;
use crate::message_enum::Message;
use crate::{API_PREFIX, TAG_USERS};
use aspen_app::bot_command::{BotCommands, CommandList, Invocation};
use aspen_app::context::GlobalServerContext;
use aspen_app::{self as app, ChannelId, UserId};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use utoipa::ToSchema;

pub type BotCommandsList = Listed<BotCommands>;

/// Publishes the commands a bot answers, replacing any it published before. The bot itself
/// may, or its owner. The whole list is checked first, and a refusal says what is wrong
/// where.
#[utoipa::path(
    put,
    path = "/bots/{bot}/commands",
    tag = TAG_USERS,
    params(("bot" = UserId, Path)),
    request_body = CommandList,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = CommandList),
        (status = BAD_REQUEST, description = "`badRequest`: the list breaks a rule, which the detail names", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: neither the bot nor its owner", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn publish_bot_commands(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(bot): Path<UserId>,
    Json(list): Json<CommandList>,
) -> ApiResult<Json<CommandList>> {
    Ok(Json(
        app::bot_command::publish(&state, user.id, bot, list).await?,
    ))
}

/// The commands a bot advertises; an empty list when it has published none.
#[utoipa::path(
    get,
    path = "/bots/{bot}/commands",
    tag = TAG_USERS,
    params(("bot" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = CommandList),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_bot_commands(
    State(state): State<GlobalServerContext>,
    SessionUser { .. }: SessionUser,
    Path(bot): Path<UserId>,
) -> ApiResult<Json<CommandList>> {
    Ok(Json(app::bot_command::read(&state, bot).await?))
}

/// The commands on offer in a channel: those of each bot that can see it, in a community
/// channel the community's and in a DM its own, a thread counting as its parent.
#[utoipa::path(
    get,
    path = "/channels/{channel}/commands",
    tag = TAG_USERS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = BotCommandsList),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such channel, or one the caller may not view", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn channel_commands(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<Json<BotCommandsList>> {
    Ok(Json(
        app::bot_command::for_channel(&state, user.id, channel)
            .await?
            .into(),
    ))
}

/// A command to invoke, and how it may be held.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InvocationRequest {
    #[serde(flatten)]
    pub invocation: Invocation,
    /// The invoking client shows the command not yet sent while its files upload, so
    /// `attachments` may name uploads of the caller's not yet confirmed: the command is then
    /// held, answered `202 Accepted`, and invoked once each has been confirmed. Without it an
    /// attachment not yet confirmed is refused.
    pub await_uploads: Option<bool>,
}

/// Invokes a bot's command in a channel, as its person sends a message there. The server
/// checks that the bot can see the channel and answers the command, and each argument against
/// its parameter's type, then shows the command in the channel as the caller's message of kind
/// `command` and tells the bot alone (`botCommandInvoked`). Files an `attachmentId` argument
/// names are uploaded first and listed in `attachments`. Plugins that decide messages there
/// decide the command as the text it shows, with its files.
#[utoipa::path(
    post,
    path = "/channels/{channel}/commands",
    tag = TAG_USERS,
    params(("channel" = ChannelId, Path)),
    request_body = InvocationRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = Message, headers(("Location" = String, description = "The message showing the command"))),
        (status = ACCEPTED, description = "Waiting: with `awaitUploads`, for its files, still being uploaded; or, the channel being busy, its turn for the channel's mana (`waiting`)", body = crate::message::HeldMessage),
        (status = TOO_MANY_REQUESTS, description = "`channelBusy`: the channel cannot take more waiting for its mana now; try again after `Retry-After`", body = Problem),
        (status = BAD_REQUEST, description = "`badRequest`: no such command, the wrong number of arguments, or one that is not what its parameter takes, which the detail names", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` without Send messages (or Send in threads), or `blocked`", body = Problem),
        (status = NOT_FOUND, description = "No such channel, or one the caller may not view", body = Problem),
        (status = UNPROCESSABLE_ENTITY, description = "`pluginRefused`: a plugin refused the command as shown, or would have changed its text", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
        (status = SERVICE_UNAVAILABLE, description = "`pluginUnavailable`: a plugin that must decide what is posted here could not", body = Problem),
    )
)]
pub async fn invoke_command(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Json(request): Json<InvocationRequest>,
) -> ApiResult<Response> {
    let InvocationRequest {
        invocation,
        await_uploads,
    } = request;
    let attachments = invocation.attachments.clone();
    let invoked = app::message::invoke_command(
        &state,
        user.id,
        channel,
        invocation,
        await_uploads.unwrap_or(false),
    )
    .await?;
    let message = match invoked {
        app::message::held::Posted::Sent(message) => *message,
        app::message::held::Posted::Held(held) => {
            return Ok((
                StatusCode::ACCEPTED,
                Json(crate::message::HeldMessage::from(*held)),
            )
                .into_response());
        }
    };
    Ok(Created::new(
        format!("{API_PREFIX}/messages/{}", message.id.0),
        app::message::record(&message, attachments, Vec::new()),
    )
    .into_response())
}
