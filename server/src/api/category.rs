use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{
    CategoryCreateCommand, CategoryCreateCommandResponse, CategoryDeleteCommand,
    CategoryDeleteCommandResponse, CategoryReadCommand, CategoryReadCommandResponse,
    CategoryUpdateCommand, CategoryUpdateCommandResponse,
};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

#[utoipa::path(post, path = "/category", responses((status = OK, body=CategoryCreateCommandResponse)))]

pub async fn create_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryCreateCommand>,
) -> (StatusCode, Json<CategoryCreateCommandResponse>) {
    todo!()
}

#[utoipa::path(get, path = "/category", responses((status = OK, body=CategoryReadCommandResponse)))]
pub async fn read_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryReadCommand>,
) -> (StatusCode, Json<CategoryReadCommandResponse>) {
    todo!()
}

#[utoipa::path(patch, path = "/category", responses((status = OK, body=CategoryUpdateCommandResponse)))]
pub async fn update_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryUpdateCommand>,
) -> (StatusCode, Json<CategoryUpdateCommandResponse>) {
    todo!()
}

#[utoipa::path(delete, path = "/category", responses((status = OK, body=CategoryDeleteCommandResponse)))]
pub async fn delete_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryDeleteCommand>,
) -> (StatusCode, Json<CategoryDeleteCommandResponse>) {
    todo!()
}
