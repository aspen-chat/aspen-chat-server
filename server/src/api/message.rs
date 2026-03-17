use crate::api::GlobalServerContext;
use crate::api::login::SessionUser;
use crate::api::message_enum::Message;
use crate::api::message_enum::command::{
    MessageCreateCommand, MessageCreateCommandResponse, MessageDeleteCommand,
    MessageDeleteCommandResponse, MessageReadCommand, MessageReadCommandResponse,
    MessageUpdateCommand, MessageUpdateCommandResponse,
};
use crate::app;
use crate::app::Error;
use crate::database::schema::user::dsl;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rust_i18n::t;

#[utoipa::path(post, path = "/message", responses((status = OK, body=MessageCreateCommandResponse)))]

pub async fn create_message(
    State(state): State<GlobalServerContext>,
    SessionUser(user): SessionUser,
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
            MessageCreateCommandResponse::CreateOk(Message {
                id: msg.id,
                author: *msg.author.id(),
                timestamp: msg.timestamp,
                content: msg.content,
                attachments: command.attachments,
                channel_id: *msg.channel.id(),
            })
            .into(),
        ),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            MessageCreateCommandResponse::Error {
                cause: t!("internalServerError").into(),
            }
            .into(),
        ),
    }
}

#[utoipa::path(get, path = "/message", responses((status = OK, body=MessageReadCommandResponse)))]

pub async fn read_message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageReadCommand>,
) -> (StatusCode, Json<MessageReadCommandResponse>) {
    todo!()
}

#[utoipa::path(patch, path = "/message", responses((status = OK, body=MessageUpdateCommandResponse)))]

pub async fn update_message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageUpdateCommand>,
) -> (StatusCode, Json<MessageUpdateCommandResponse>) {
    todo!()
}

#[utoipa::path(delete, path = "/message", responses((status = OK, body=MessageDeleteCommandResponse)))]
pub async fn delete_message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageDeleteCommand>,
) -> (StatusCode, Json<MessageDeleteCommandResponse>) {
    todo!()
}
