use axum::extract::State;
use axum::Json;
use axum::http::StatusCode;
use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{IconCreateCommand, IconCreateCommandResponse, IconDeleteCommand, IconDeleteCommandResponse, IconReadCommand, IconReadCommandResponse, MessageDeleteCommand, MessageDeleteCommandResponse, MessageReadCommand, MessageReadCommandResponse, MessageUpdateCommand, MessageUpdateCommandResponse};

pub async fn create_icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconCreateCommand>,
) -> (StatusCode, Json<IconCreateCommandResponse>) {
    todo!()
}

pub async fn read_icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconReadCommand>,
) -> (StatusCode, Json<IconReadCommandResponse>) {
    todo!()
}
pub async fn delete_icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconDeleteCommand>,
) -> (StatusCode, Json<IconDeleteCommandResponse>) {
    todo!()
}