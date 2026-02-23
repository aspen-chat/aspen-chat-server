use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{
    CommunityCreateCommand, CommunityCreateCommandResponse, CommunityDeleteCommand,
    CommunityDeleteCommandResponse, CommunityReadCommand, CommunityReadCommandResponse,
    CommunityUpdateCommand, CommunityUpdateCommandResponse,
};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

#[utoipa::path(post, path = "/community", responses((status = OK, body=CommunityCreateCommandResponse)))]
pub async fn create_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityCreateCommand>,
) -> (StatusCode, Json<CommunityCreateCommandResponse>) {
    todo!()
}

#[utoipa::path(get, path = "/community", responses((status = OK, body=CommunityReadCommandResponse)))]
pub async fn read_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityReadCommand>,
) -> (StatusCode, Json<CommunityReadCommandResponse>) {
    todo!()
}

#[utoipa::path(patch, path = "/community", responses((status = OK, body=CommunityUpdateCommandResponse)))]
pub async fn update_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityUpdateCommand>,
) -> (StatusCode, Json<CommunityUpdateCommandResponse>) {
    todo!()
}

#[utoipa::path(delete, path = "/community", responses((status = OK, body=CommunityDeleteCommandResponse)))]
pub async fn delete_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityDeleteCommand>,
) -> (StatusCode, Json<CommunityDeleteCommandResponse>) {
    todo!()
}
