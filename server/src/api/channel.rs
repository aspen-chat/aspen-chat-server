use axum::extract::State;
use axum::Json;
use axum::http::StatusCode;
use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{ChannelCreateCommand, ChannelCreateCommandResponse, ChannelDeleteCommand, ChannelDeleteCommandResponse, ChannelReadCommand, ChannelReadCommandResponse, ChannelUpdateCommand, ChannelUpdateCommandResponse, UserDeleteCommand, UserDeleteCommandResponse, UserReadCommand, UserReadCommandResponse, UserUpdateCommand, UserUpdateCommandResponse};

pub async fn create_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelCreateCommand>,
) -> (StatusCode, Json<ChannelCreateCommandResponse>) {
    todo!()
}

pub async fn read_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelReadCommand>,
) -> (StatusCode, Json<ChannelReadCommandResponse>) {
    todo!()
}

pub async fn update_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelUpdateCommand>,
) -> (StatusCode, Json<ChannelUpdateCommandResponse>) {
    todo!()
}
pub async fn delete_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelDeleteCommand>,
) -> (StatusCode, Json<ChannelDeleteCommandResponse>) {
    todo!()
}