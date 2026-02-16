use axum::extract::State;
use axum::Json;
use axum::http::StatusCode;
use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{ReactCreateCommand, ReactCreateCommandResponse, UserDeleteCommand, UserDeleteCommandResponse, UserReadCommand, UserReadCommandResponse, UserUpdateCommand, UserUpdateCommandResponse};

pub async fn create_react(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ReactCreateCommand>,
) -> (StatusCode, Json<ReactCreateCommandResponse>) {
    todo!()
}

pub async fn delete_react(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserDeleteCommand>,
) -> (StatusCode, Json<UserDeleteCommandResponse>) {
    todo!()
}