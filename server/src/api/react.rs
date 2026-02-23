use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{
    ReactCreateCommand, ReactCreateCommandResponse, ReactDeleteCommand, ReactDeleteCommandResponse,
    UserDeleteCommand, UserDeleteCommandResponse,
};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

#[utoipa::path(post, path = "/react", responses((status = OK, body=ReactCreateCommandResponse)))]

pub async fn create_react(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ReactCreateCommand>,
) -> (StatusCode, Json<ReactCreateCommandResponse>) {
    todo!()
}

#[utoipa::path(delete, path = "/react", responses((status = OK, body=ReactDeleteCommandResponse)))]
pub async fn delete_react(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ReactDeleteCommand>,
) -> (StatusCode, Json<ReactDeleteCommandResponse>) {
    todo!()
}
