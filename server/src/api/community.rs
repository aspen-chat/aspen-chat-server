use crate::api::login::SessionUser;
use crate::api::message_enum::command::{
    CommunityCreateCommand, CommunityCreateCommandResponse, CommunityDeleteCommand,
    CommunityDeleteCommandResponse, CommunityReadCommand, CommunityReadCommandResponse,
    CommunityUpdateCommand, CommunityUpdateCommandResponse, UserCommunityCreateCommand,
    UserCommunityCreateCommandResponse, UserCommunityDeleteCommand,
    UserCommunityDeleteCommandResponse,
};
use crate::api::message_enum::{Category, Channel, User};
use crate::api::{GlobalServerContext, message_enum};
use crate::app::{CategoryId, CommunityId};
use crate::{api, app};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use tracing::error;
use utoipa::ToSchema;

#[utoipa::path(post, path = "/community", responses((status = OK, body=CommunityCreateCommandResponse)))]
pub async fn create_community(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
    Json(command): Json<CommunityCreateCommand>,
) -> (StatusCode, Json<CommunityCreateCommandResponse>) {
    let new_community = match app::community::create_community(state, &command).await {
        Ok(value) => value,
        Err(e) => {
            return {
                error!("Error creating community {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    CommunityCreateCommandResponse::Error {
                        cause: Some(t!("tryAgainLater")),
                    }
                    .into(),
                )
            };
        }
    };
    (
        StatusCode::OK,
        CommunityCreateCommandResponse::CreateOk(api::message_enum::Community {
            id: new_community.id,
            name: new_community.name,
            icon: new_community.icon.map(|i| i.id().clone()),
        })
        .into(),
    )
}

#[utoipa::path(get, path = "/community", responses((status = OK, body=CommunityReadCommandResponse)))]
pub async fn read_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityReadCommand>,
) -> (StatusCode, Json<CommunityReadCommandResponse>) {
    todo!()
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommunityUsersReadCommand {
    community: CommunityId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum CommunityUsersReadCommandResponse {
    Users { data: Vec<User> },
    NotAllowed { reason: Option<String> },
    Error { cause: Option<String> },
}

#[utoipa::path(get, path = "/community/users", responses((status = OK, body=CommunityUsersReadCommandResponse)))]
pub async fn read_community_users(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityUsersReadCommand>,
) -> (StatusCode, Json<CommunityUsersReadCommandResponse>) {
    todo!()
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommunityCategoriesReadCommand {
    community: CommunityId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum CommunityCategoriesReadCommandResponse {
    Categories { data: Vec<Category> },
    NotAllowed { reason: Option<String> },
    Error { cause: Option<String> },
}

#[utoipa::path(get, path = "/community/categories", responses((status = OK, body=CommunityCategoriesReadCommandResponse)))]
pub async fn read_community_categories(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityCategoriesReadCommand>,
) -> (StatusCode, Json<CommunityCategoriesReadCommandResponse>) {
    match app::category::read_community_categories(&state, command.community).await {
        Ok(categories) => (
            StatusCode::OK,
            CommunityCategoriesReadCommandResponse::Categories {
                data: categories
                    .into_iter()
                    .map(api::category::category_to_api)
                    .collect(),
            }
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error reading community channels");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                CommunityCategoriesReadCommandResponse::Error {
                    cause: None,
                }
                .into(),
            )
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommunityChannelsReadCommand {
    community: CommunityId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum CommunityChannelsReadCommandResponse {
    Channels { data: Vec<Channel> },
    NotAllowed { reason: Option<String> },
    Error { cause: Option<String> },
}

#[utoipa::path(get, path = "/community/channels", responses((status = OK, body=CommunityChannelsReadCommandResponse)))]
pub async fn read_community_channels(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityChannelsReadCommand>,
) -> (StatusCode, Json<CommunityChannelsReadCommandResponse>) {
    match app::community::read_community_channels(&state, command.community).await {
        Ok(channels) => (
            StatusCode::OK,
            CommunityChannelsReadCommandResponse::Channels {
                data: channels
                    .into_iter()
                    .map(api::channel::channel_to_api)
                    .collect(),
            }
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error reading community channels");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                CommunityChannelsReadCommandResponse::Error {
                    cause: None,
                }
                .into(),
            )
        }
    }
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

#[utoipa::path(post, path = "/community/join", responses((status = OK, body=CommunityUpdateCommandResponse)))]
pub async fn join_community(
    State(state): State<GlobalServerContext>,
    SessionUser(user): SessionUser,
    Json(command): Json<UserCommunityCreateCommand>,
) -> (StatusCode, Json<UserCommunityCreateCommandResponse>) {
    match app::community::join_community(&state, user.id, command.community).await {
        Ok(_) => (
            StatusCode::OK,
            UserCommunityCreateCommandResponse::CreateOk(message_enum::UserCommunity {
                user: user.id,
                community: command.community,
            })
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error joining community");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                UserCommunityCreateCommandResponse::Error {
                    cause: None,
                }
                .into(),
            )
        }
    }
}

#[utoipa::path(delete, path = "/community/leave", responses((status = OK, body=CommunityDeleteCommandResponse)))]
pub async fn leave_community(
    State(state): State<GlobalServerContext>,
    SessionUser(user): SessionUser,
    Json(command): Json<UserCommunityDeleteCommand>,
) -> (StatusCode, Json<UserCommunityDeleteCommandResponse>) {
    match app::community::leave_community(&state, user.id, command.community).await {
        Ok(_) => (
            StatusCode::OK,
            UserCommunityDeleteCommandResponse::DeleteOk.into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error leaving community");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                UserCommunityDeleteCommandResponse::Error {
                    cause: None,
                }
                .into(),
            )
        }
    }
}
