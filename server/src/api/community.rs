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
use crate::app::CommunityId;
use crate::{api, app};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
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
            icon: new_community.icon.map(|i| *i.id()),
        })
        .into(),
    )
}

#[utoipa::path(get, path = "/community", responses((status = OK, body=CommunityReadCommandResponse)))]
pub async fn read_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityReadCommand>,
) -> (StatusCode, Json<CommunityReadCommandResponse>) {
    match app::community::read_community(&state, command.id).await {
        Ok(community) => (
            StatusCode::OK,
            CommunityReadCommandResponse::Community(message_enum::Community {
                id: community.id,
                name: community.name,
                icon: community.icon.map(|i| *i.id()),
            })
            .into(),
        ),
        Err(e) => match e {
            app::Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                CommunityReadCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error reading community");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    CommunityReadCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
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
    NotAllowed { reason: Option<Cow<'static, str>> },
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(get, path = "/community/users", responses((status = OK, body=CommunityUsersReadCommandResponse)))]
pub async fn read_community_users(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityUsersReadCommand>,
) -> (StatusCode, Json<CommunityUsersReadCommandResponse>) {
    match app::community::read_community_users(&state, command.community).await {
        Ok(users) => (
            StatusCode::OK,
            CommunityUsersReadCommandResponse::Users {
                data: users
                    .into_iter()
                    .map(|u| User {
                        id: u.id,
                        name: u.name,
                        icon: u.icon.map(|i| *i.id()),
                    })
                    .collect(),
            }
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error reading community users");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                CommunityUsersReadCommandResponse::Error { cause: None }.into(),
            )
        }
    }
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
    NotAllowed { reason: Option<Cow<'static, str>> },
    Error { cause: Option<Cow<'static, str>> },
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
                CommunityCategoriesReadCommandResponse::Error { cause: None }.into(),
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
    NotAllowed { reason: Option<Cow<'static, str>> },
    Error { cause: Option<Cow<'static, str>> },
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
                CommunityChannelsReadCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

#[utoipa::path(patch, path = "/community", responses((status = OK, body=CommunityUpdateCommandResponse)))]
pub async fn update_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityUpdateCommand>,
) -> (StatusCode, Json<CommunityUpdateCommandResponse>) {
    match app::community::update_community(&state, command).await {
        Ok(_) => (
            StatusCode::OK,
            CommunityUpdateCommandResponse::UpdateOk.into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error updating community");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                CommunityUpdateCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

#[utoipa::path(delete, path = "/community", responses((status = OK, body=CommunityDeleteCommandResponse)))]
pub async fn delete_community(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityDeleteCommand>,
) -> (StatusCode, Json<CommunityDeleteCommandResponse>) {
    match app::community::delete_community(&state, command.id).await {
        Ok(()) => (
            StatusCode::OK,
            CommunityDeleteCommandResponse::DeleteOk.into(),
        ),
        Err(e) => match e {
            app::Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                CommunityDeleteCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error deleting community");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    CommunityDeleteCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}

#[utoipa::path(post, path = "/community/join", responses((status = OK, body=CommunityUpdateCommandResponse)))]
pub async fn join_community(
    State(state): State<GlobalServerContext>,
    SessionUser(user): SessionUser,
    Json(command): Json<UserCommunityCreateCommand>,
) -> (StatusCode, Json<UserCommunityCreateCommandResponse>) {
    match app::community::join_community(&state, user.id, command.community, command.invite_code)
        .await
    {
        Ok(_) => (
            StatusCode::OK,
            UserCommunityCreateCommandResponse::CreateOk(message_enum::UserCommunity {
                user: user.id,
                community: command.community,
            })
            .into(),
        ),
        Err(app::Error::Validation(reason)) => (
            StatusCode::BAD_REQUEST,
            UserCommunityCreateCommandResponse::Error {
                cause: Some(reason),
            }
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error joining community");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                UserCommunityCreateCommandResponse::Error { cause: None }.into(),
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
                UserCommunityDeleteCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}
