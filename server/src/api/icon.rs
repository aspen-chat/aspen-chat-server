use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{
    IconCreateCommand, IconCreateCommandResponse, IconDeleteCommand, IconDeleteCommandResponse,
    IconReadCommand, IconReadCommandResponse,
};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

#[utoipa::path(post, path = "/icon", responses((status = OK, body=IconCreateCommandResponse)))]
pub async fn create_icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconCreateCommand>,
) -> (StatusCode, Json<IconCreateCommandResponse>) {
    todo!()
}

#[utoipa::path(get, path = "/icon", responses((status = OK, body=IconReadCommandResponse)))]
pub async fn read_icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconReadCommand>,
) -> (StatusCode, Json<IconReadCommandResponse>) {
    todo!()
}

#[utoipa::path(delete, path = "/icon", responses((status = OK, body=IconDeleteCommandResponse)))]
pub async fn delete_icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconDeleteCommand>,
) -> (StatusCode, Json<IconDeleteCommandResponse>) {
    todo!()
}
