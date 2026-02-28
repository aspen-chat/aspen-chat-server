use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{ChannelCreateCommand, ChannelCreateCommandResponse, ChannelDeleteCommand, ChannelDeleteCommandResponse, ChannelMessagesReadCommand, ChannelMessagesReadCommandResponse, ChannelPinsReadCommand, ChannelPinsReadCommandResponse, ChannelReadCommand, ChannelReadCommandResponse, ChannelUpdateCommand, ChannelUpdateCommandResponse};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;

#[utoipa::path(post, path = "/channel", responses((status = OK, body=ChannelCreateCommandResponse)))]
pub async fn create_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelCreateCommand>,
) -> (StatusCode, Json<ChannelCreateCommandResponse>) {
    todo!()
}

#[utoipa::path(get, path = "/channel", responses((status = OK, body=ChannelReadCommandResponse)))]
pub async fn read_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelReadCommand>,
) -> (StatusCode, Json<ChannelReadCommandResponse>) {
    todo!()
}

#[utoipa::path(get, path = "/channel/messages", responses((status = OK, body=ChannelMessagesReadCommandResponse)))]
pub async fn read_channel_messages(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelMessagesReadCommand>,
) -> (StatusCode, Json<ChannelMessagesReadCommandResponse>) {
    todo!()
}

#[utoipa::path(get, path = "/channel/pins", responses((status = OK, body=ChannelPinsReadCommandResponse)))]
pub async fn read_channel_pins(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelPinsReadCommand>,
) -> (StatusCode, Json<ChannelPinsReadCommandResponse>) {
    todo!()
}

#[utoipa::path(patch, path = "/channel", responses((status = OK, body=ChannelUpdateCommandResponse)))]
pub async fn update_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelUpdateCommand>,
) -> (StatusCode, Json<ChannelUpdateCommandResponse>) {
    todo!()
}

#[utoipa::path(delete, path = "/channel", responses((status = OK, body=ChannelDeleteCommandResponse)))]
pub async fn delete_channel(
    State(state): State<GlobalServerContext>,
    Json(command): Json<ChannelDeleteCommand>,
) -> (StatusCode, Json<ChannelDeleteCommandResponse>) {
    todo!()
}
