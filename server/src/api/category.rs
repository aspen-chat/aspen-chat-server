use crate::api::message_enum::Channel;
use crate::api::message_enum::command::{
    CategoryCreateCommand, CategoryCreateCommandResponse, CategoryDeleteCommand,
    CategoryDeleteCommandResponse, CategoryReadCommand, CategoryReadCommandResponse,
    CategoryUpdateCommand, CategoryUpdateCommandResponse,
};
use crate::api::{GlobalServerContext, message_enum};
use crate::app::{CategoryId, Error};
use crate::{api, app};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use tracing::error;
use utoipa::ToSchema;

#[utoipa::path(post, path = "/category", responses((status = OK, body=CategoryCreateCommandResponse)))]

pub async fn create_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryCreateCommand>,
) -> (StatusCode, Json<CategoryCreateCommandResponse>) {
    match app::category::create_category(
        &state,
        command.name,
        command.sort_index,
        command.community,
    )
    .await
    {
        Ok(c) => (
            StatusCode::OK,
            CategoryCreateCommandResponse::CreateOk(category_to_api(c)).into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "category create command error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                CategoryCreateCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

#[utoipa::path(get, path = "/category", responses((status = OK, body=CategoryReadCommandResponse)))]
pub async fn read_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryReadCommand>,
) -> (StatusCode, Json<CategoryReadCommandResponse>) {
    match app::category::read_category(&state, command.id).await {
        Ok(c) => (
            StatusCode::OK,
            CategoryReadCommandResponse::Category(category_to_api(c)).into(),
        ),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                CategoryReadCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error reading category");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    CategoryReadCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
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
    NotAllowed { reason: Option<Cow<'static, str>> },
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(get, path = "/category/channels", responses((status = OK, body=CategoryChannelsReadCommandResponse)))]
pub async fn read_category_channels(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryChannelsReadCommand>,
) -> (StatusCode, Json<CategoryChannelsReadCommandResponse>) {
    match app::category::read_category_channels(&state, command.category).await {
        Ok(channels) => (
            StatusCode::OK,
            CategoryChannelsReadCommandResponse::Channels {
                data: channels
                    .into_iter()
                    .map(api::channel::channel_to_api)
                    .collect(),
            }
            .into(),
        ),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                CategoryChannelsReadCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error reading category channels");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    CategoryChannelsReadCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}

#[utoipa::path(patch, path = "/category", responses((status = OK, body=CategoryUpdateCommandResponse)))]
pub async fn update_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryUpdateCommand>,
) -> (StatusCode, Json<CategoryUpdateCommandResponse>) {
    match app::category::update_category(&state, command).await {
        Ok(_) => (
            StatusCode::OK,
            CategoryUpdateCommandResponse::UpdateOk.into(),
        ),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                CategoryUpdateCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error updating category");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    CategoryUpdateCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}

#[utoipa::path(delete, path = "/category", responses((status = OK, body=CategoryDeleteCommandResponse)))]
pub async fn delete_category(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CategoryDeleteCommand>,
) -> (StatusCode, Json<CategoryDeleteCommandResponse>) {
    match app::category::delete_category(&state, command.id).await {
        Ok(()) => (
            StatusCode::OK,
            CategoryDeleteCommandResponse::DeleteOk.into(),
        ),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                CategoryDeleteCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error deleting category");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    CategoryDeleteCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}

pub fn category_to_api(c: crate::app::category::Category) -> message_enum::Category {
    message_enum::Category {
        id: c.id,
        name: c.name,
        sort_index: c.sort_index,
        community: *c.community.id(),
    }
}
