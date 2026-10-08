//! Messages the caller saved for themself (`app::saved_message`): their list, the messages in
//! it, and saving and unsaving one. Each change reaches the caller's devices as
//! `savedMessageChanged`.

use crate::TAG_MESSAGES;
use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Json, NoContent, Path, Query};
use crate::include::IncludeSet;
use crate::message::{MessageInclude, MessageList, sideload_messages};
use crate::message_enum::Message;
use aspen_app::context::GlobalServerContext;
use aspen_app::{self as app, MessageId, SavedMessageId};
use axum::extract::State;
use axum::http::StatusCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// A message the caller saved. Saves are listed newest first, by `id`, a UUIDv7 of the saving.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SavedMessage {
    pub id: SavedMessageId,
    pub message: MessageId,
}

impl From<app::saved_message::SavedMessage> for SavedMessage {
    fn from(saved: app::saved_message::SavedMessage) -> Self {
        SavedMessage {
            id: saved.id,
            message: saved.message,
        }
    }
}

/// Every save of the caller's that they may read now, newest first: at most
/// `app::saved_message::MAX_SAVED_MESSAGES`. A save of a message in a channel they can no
/// longer read is left out until they may read it again.
#[utoipa::path(
    get,
    path = "/users/@me/saved-messages",
    tag = TAG_MESSAGES,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<SavedMessage>),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_saves(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
) -> ApiResult<Json<Vec<SavedMessage>>> {
    let saves = app::saved_message::read_saves(&state, user.id).await?;
    Ok(Json(saves.into_iter().map(SavedMessage::from).collect()))
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SavedMessagesQuery {
    /// Only messages saved before this one: the last of the previous page.
    pub before: Option<MessageId>,
    /// How many to return, at most `app::saved_message::MAX_PAGE` (100); 50 when absent.
    pub limit: Option<u32>,
    /// Related records to return alongside the messages, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<MessageInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<MessageInclude>,
}

/// How many saved messages a page holds when it does not say.
const DEFAULT_SAVED_PAGE: u32 = 50;

/// The messages the caller saved and may read now, newest save first.
#[utoipa::path(
    get,
    path = "/users/@me/saved-messages/messages",
    tag = TAG_MESSAGES,
    params(SavedMessagesQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = MessageList),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_saved_messages(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Query(query): Query<SavedMessagesQuery>,
) -> ApiResult<Json<MessageList>> {
    let messages: Vec<Message> = app::saved_message::read_saved_messages(
        &state,
        user.id,
        query.before,
        query.limit.unwrap_or(DEFAULT_SAVED_PAGE),
    )
    .await?
    .into_iter()
    .map(Message::from)
    .collect();
    let included = sideload_messages(&state, user.id, &messages, &query.include).await?;
    Ok(Json(MessageList::new(messages, included)))
}

/// Saves a message for the caller, one of a channel or DM they may read.
#[utoipa::path(
    put,
    path = "/users/@me/saved-messages/{message}",
    tag = TAG_MESSAGES,
    params(("message" = MessageId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Saved", body = SavedMessage),
        (status = OK, description = "Was saved already", body = SavedMessage),
        (status = BAD_REQUEST, description = "`validation`: the caller keeps as many saved messages as they may", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such message, or one the caller may not read", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn save_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
) -> ApiResult<(StatusCode, Json<SavedMessage>)> {
    let (saved, existed) = app::saved_message::save(&state, user.id, message).await?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(SavedMessage::from(saved))))
}

/// Stops saving a message for the caller. Nothing to remove is not an error.
#[utoipa::path(
    delete,
    path = "/users/@me/saved-messages/{message}",
    tag = TAG_MESSAGES,
    params(("message" = MessageId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Not saved"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn unsave_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
) -> ApiResult<NoContent> {
    app::saved_message::unsave(&state, user.id, message).await?;
    Ok(NoContent)
}
