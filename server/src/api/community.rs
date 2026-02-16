use axum::extract::State;
use axum::Json;
use axum::http::StatusCode;
use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{CommunityCreateCommand, CommunityCreateCommandResponse, CommunityDeleteCommand, CommunityDeleteCommandResponse, CommunityReadCommand, CommunityReadCommandResponse, CommunityUpdateCommand, CommunityUpdateCommandResponse, UserDeleteCommand, UserDeleteCommandResponse, UserReadCommand, UserReadCommandResponse, UserUpdateCommand, UserUpdateCommandResponse};

pub async fn create_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityCreateCommand>,
) -> (StatusCode, Json<CommunityCreateCommandResponse>) {
    todo!()
}

pub async fn read_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityReadCommand>,
) -> (StatusCode, Json<CommunityReadCommandResponse>) {
    todo!()
}

pub async fn update_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityUpdateCommand>,
) -> (StatusCode, Json<CommunityUpdateCommandResponse>) {
    todo!()
}
pub async fn delete_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityDeleteCommand>,
) -> (StatusCode, Json<CommunityDeleteCommandResponse>) {
    todo!()
}