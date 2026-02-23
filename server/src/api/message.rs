use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{
    MessageCreateCommand, MessageCreateCommandResponse, MessageDeleteCommand,
    MessageDeleteCommandResponse, MessageReadCommand, MessageReadCommandResponse,
    MessageUpdateCommand, MessageUpdateCommandResponse,
};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

#[utoipa::path(post, path = "/message", responses((status = OK, body=MessageCreateCommandResponse)))]

pub async fn create_message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageCreateCommand>,
) -> (StatusCode, Json<MessageCreateCommandResponse>) {
    todo!()
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
