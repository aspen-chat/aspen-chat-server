use crate::api::GlobalServerContext;
use crate::api::link_preview::LinkPreview;
use crate::api::login::SessionUser;
use crate::api::message_enum::Message;
use crate::api::message_enum::command::{
    MessageCreateCommand, MessageCreateCommandResponse, MessageDeleteCommand,
    MessageDeleteCommandResponse, MessageReadCommand, MessageReadCommandResponse,
    MessageUpdateCommand, MessageUpdateCommandResponse,
};
use crate::app;
use crate::app::{AttachmentId, Error};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use tracing::error;

#[utoipa::path(post, path = "/message", responses((status = OK, body=MessageCreateCommandResponse)))]

pub async fn create_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(command): Json<MessageCreateCommand>,
) -> (StatusCode, Json<MessageCreateCommandResponse>) {
    let r = app::message::create_message(
        &state,
        user.id,
        command.channel_id,
        command.content,
        command.attachments.clone(),
    )
    .await;
    match r {
        Ok(msg) => (
            StatusCode::OK,
            // Freshly-created messages always ship with an empty preview
            // list; the async fetcher's `MessageLinkPreviewsReady` event
            // will populate the final set shortly.
            MessageCreateCommandResponse::CreateOk(message_to_api(
                msg,
                command.attachments,
                Vec::new(),
            ))
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "message create command error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                MessageCreateCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

pub fn message_to_api(
    msg: app::message::Message,
    attachments: Vec<AttachmentId>,
    link_previews: Vec<LinkPreview>,
) -> Message {
    Message {
        id: msg.id,
        author: *msg.author.id(),
        timestamp: msg.timestamp,
        content: msg.content,
        attachments,
        channel_id: *msg.channel.id(),
        link_previews,
    }
}

#[utoipa::path(get, path = "/message", responses((status = OK, body=MessageReadCommandResponse)))]

pub async fn read_message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageReadCommand>,
) -> (StatusCode, Json<MessageReadCommandResponse>) {
    match app::message::read_message(&state, command.id).await {
        Ok(m) => (
            StatusCode::OK,
            MessageReadCommandResponse::Message(message_to_api(
                m.message,
                m.attachments,
                m.link_previews,
            ))
            .into(),
        ),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                MessageReadCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error reading message");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    MessageReadCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}

#[utoipa::path(patch, path = "/message", responses((status = OK, body=MessageUpdateCommandResponse)))]
pub async fn update_message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageUpdateCommand>,
) -> (StatusCode, Json<MessageUpdateCommandResponse>) {
    match app::message::update_message(&state, command).await {
        Ok(_) => (
            StatusCode::OK,
            MessageUpdateCommandResponse::UpdateOk.into(),
        ),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                MessageUpdateCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error updating message");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    MessageUpdateCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}

#[utoipa::path(delete, path = "/message", responses((status = OK, body=MessageDeleteCommandResponse)))]
pub async fn delete_message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageDeleteCommand>,
) -> (StatusCode, Json<MessageDeleteCommandResponse>) {
    match app::message::delete_message(&state, command.id).await {
        Ok(()) => (
            StatusCode::OK,
            MessageDeleteCommandResponse::DeleteOk.into(),
        ),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                MessageDeleteCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error deleting message");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    MessageDeleteCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}
