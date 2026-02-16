use axum::extract::State;
use axum::Json;
use axum::http::StatusCode;
use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{MessageCreateCommand, MessageCreateCommandResponse, MessageDeleteCommand, MessageDeleteCommandResponse, MessageReadCommand, MessageReadCommandResponse, MessageUpdateCommand, MessageUpdateCommandResponse, UserDeleteCommand, UserDeleteCommandResponse, UserReadCommand, UserReadCommandResponse, UserUpdateCommand, UserUpdateCommandResponse};

pub async fn create_message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageCreateCommand>,
) -> (StatusCode, Json<MessageCreateCommandResponse>) {
    todo!()
}

pub async fn read_message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageReadCommand>,
) -> (StatusCode, Json<MessageReadCommandResponse>) {
    todo!()
}

pub async fn update_message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageUpdateCommand>,
) -> (StatusCode, Json<MessageUpdateCommandResponse>) {
    todo!()
}
pub async fn delete_message(
    State(state): State<GlobalServerContext>,
    Json(command): Json<MessageDeleteCommand>,
) -> (StatusCode, Json<MessageDeleteCommandResponse>) {
    todo!()
}