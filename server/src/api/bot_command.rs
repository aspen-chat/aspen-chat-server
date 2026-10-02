//! The commands bots advertise (`app::bot_command`): publishing a bot's list, reading it, and
//! reading what a channel offers, which clients complete from.

use crate::api::auth::SessionUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Created, Json, Path};
use crate::api::message_enum::Message;
use crate::api::{API_PREFIX, TAG_USERS};
use crate::app::bot_command::{BotCommands, CommandList, Invocation};
use crate::app::context::GlobalServerContext;
use crate::app::{self, ChannelId, UserId};
use axum::extract::State;

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
        (status = OK, body = Vec<BotCommands>),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such channel, or one the caller may not view", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn channel_commands(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<Json<Vec<BotCommands>>> {
    Ok(Json(
        app::bot_command::for_channel(&state, user.id, channel).await?,
    ))
}

/// Invokes a bot's command in a channel, as its person sends a message there. The server
/// checks that the bot can see the channel and answers the command, and each argument against
/// its parameter's type, then shows the command in the channel as the caller's message of kind
/// `command` and tells the bot alone (`botCommandInvoked`). Files an `attachmentId` argument
/// names are uploaded first and listed in `attachments`.
#[utoipa::path(
    post,
    path = "/channels/{channel}/commands",
    tag = TAG_USERS,
    params(("channel" = ChannelId, Path)),
    request_body = Invocation,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = Message, headers(("Location" = String, description = "The message showing the command"))),
        (status = BAD_REQUEST, description = "`badRequest`: no such command, the wrong number of arguments, or one that is not what its parameter takes, which the detail names", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` without Send messages (or Send in threads), or `blocked`", body = Problem),
        (status = NOT_FOUND, description = "No such channel, or one the caller may not view", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn invoke_command(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Json(invocation): Json<Invocation>,
) -> ApiResult<Created<Message>> {
    let attachments = invocation.attachments.clone();
    let message = app::message::create_message(
        &state,
        user.id,
        channel,
        String::new(),
        attachments.clone(),
        false,
        Some(invocation),
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/messages/{}", message.id.0),
        app::message::record(&message, attachments, Vec::new()),
    ))
}
