use axum::extract::State;
use axum::Json;
use axum::http::StatusCode;
use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{CategoryCreateCommand, CategoryCreateCommandResponse, CategoryDeleteCommand, CategoryDeleteCommandResponse, CategoryReadCommand, CategoryReadCommandResponse, CategoryUpdateCommand, CategoryUpdateCommandResponse, UserDeleteCommand, UserDeleteCommandResponse, UserReadCommand, UserReadCommandResponse, UserUpdateCommand, UserUpdateCommandResponse};

pub async fn create_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryCreateCommand>,
) -> (StatusCode, Json<CategoryCreateCommandResponse>) {
    todo!()
}

pub async fn read_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryReadCommand>,
) -> (StatusCode, Json<CategoryReadCommandResponse>) {
    todo!()
}

pub async fn update_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryUpdateCommand>,
) -> (StatusCode, Json<CategoryUpdateCommandResponse>) {
    todo!()
}
pub async fn delete_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryDeleteCommand>,
) -> (StatusCode, Json<CategoryDeleteCommandResponse>) {
    todo!()
}