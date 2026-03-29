use crate::api::message_enum::Channel;
use crate::api::message_enum::command::{
    CategoryCreateCommand, CategoryCreateCommandResponse, CategoryDeleteCommand,
    CategoryDeleteCommandResponse, CategoryReadCommand, CategoryReadCommandResponse,
    CategoryUpdateCommand, CategoryUpdateCommandResponse,
};
use crate::api::{GlobalServerContext, message_enum};
use crate::app::{CategoryId, ChannelId};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CategoryChannelsReadCommand {
    category: CategoryId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum CategoryChannelsReadCommandResponse {
    Channels { data: Vec<Channel> },
    NotAllowed { reason: Option<String> },
    Error { cause: Option<String> },
}

#[utoipa::path(get, path = "/category/channels", responses((status = OK, body=CategoryChannelsReadCommandResponse)))]
pub async fn read_category_channels(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryChannelsReadCommand>,
) -> (StatusCode, Json<CategoryChannelsReadCommandResponse>) {
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

pub fn category_to_api(c: crate::app::category::Category) -> message_enum::Category {
    message_enum::Category {
        id: c.id,
        name: c.name,
        sort_index: c.sort_index,
        community: c.community.id().clone(),
    }
}
