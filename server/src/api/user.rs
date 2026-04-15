use crate::api::GlobalServerContext;
use crate::api::login::SessionUser;
use crate::api::message_enum::Community;
use crate::api::message_enum::command::{
    UserCreateCommand, UserCreateCommandResponse, UserDeleteCommand, UserDeleteCommandResponse,
    UserReadCommand, UserReadCommandResponse, UserUpdateCommand, UserUpdateCommandResponse,
};
use crate::app::Error;
use crate::{api, app};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use diesel::result::DatabaseErrorKind;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use tracing::error;
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UserOnlineStatus {
    Online,
    Offline,
    Away,
}

#[utoipa::path(post, path = "/user", responses((status = OK, body=UserCreateCommandResponse)))]
pub async fn create_user(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserCreateCommand>,
) -> (StatusCode, Json<UserCreateCommandResponse>) {
    let new_user_id = match app::user::create_user(state, &command).await {
        Ok(value) => value,
        Err(err) => {
            return {
                if let Error::Diesel(diesel::result::Error::DatabaseError(
                    DatabaseErrorKind::UniqueViolation,
                    _,
                )) = err
                {
                    (
                        StatusCode::BAD_REQUEST,
                        UserCreateCommandResponse::Error {
                            cause: Some(t!("usernameAlreadyTaken")),
                        }
                        .into(),
                    )
                } else {
                    error!("error inserting new user into database {err}");
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        UserCreateCommandResponse::Error { cause: None }.into(),
                    )
                }
            };
        }
    };
    (
        StatusCode::OK,
        UserCreateCommandResponse::CreateOk(api::message_enum::User {
            id: new_user_id,
            name: command.name,
            icon: command.icon,
        })
        .into(),
    )
}

#[utoipa::path(get, path = "/user", responses((status = OK, body=UserReadCommandResponse)))]
pub async fn read_user(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
    Json(command): Json<UserReadCommand>,
) -> (StatusCode, Json<UserReadCommandResponse>) {
    match app::user::read_user(state, command.id).await {
        Ok(user) => (
            StatusCode::OK,
            UserReadCommandResponse::User(api::message_enum::User {
                id: user.id,
                name: user.name,
                icon: user.icon.map(|i| *i.id()),
            })
            .into(),
        ),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                UserReadCommandResponse::Error { cause: None }.into(),
            ),
            _ => (
                StatusCode::INTERNAL_SERVER_ERROR,
                UserReadCommandResponse::Error { cause: None }.into(),
            ),
        },
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum UserCommunitiesReadCommandResponse {
    Communities { data: Vec<Community> },
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(
    get,
    path = "/user/communities",
    security(("loginKey" = [])),
    responses((status = OK, body=UserCommunitiesReadCommandResponse))
)]
pub async fn read_user_communities(
    State(state): State<GlobalServerContext>,
    SessionUser(user): SessionUser,
) -> (StatusCode, Json<UserCommunitiesReadCommandResponse>) {
    match app::user::read_user_communities(state, user.id).await {
        Ok(communities) => (
            StatusCode::OK,
            UserCommunitiesReadCommandResponse::Communities {
                data: communities
                    .into_iter()
                    .map(|community| Community {
                        id: community.id,
                        name: community.name,
                        icon: community.icon.map(|i| *i.id()),
                    })
                    .collect(),
            }
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error reading user communities");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                UserCommunitiesReadCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

#[utoipa::path(patch, path = "/user", responses((status = OK, body=UserUpdateCommandResponse)))]

pub async fn update_user(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserUpdateCommand>,
) -> (StatusCode, Json<UserUpdateCommandResponse>) {
    match app::user::update_user(state, command).await {
        Ok(_) => (StatusCode::OK, UserUpdateCommandResponse::UpdateOk.into()),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                UserUpdateCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "user update command error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    UserUpdateCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}

#[utoipa::path(delete, path = "/user", responses((status = OK, body=UserDeleteCommandResponse)))]
pub async fn delete_user(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserDeleteCommand>,
) -> (StatusCode, Json<UserDeleteCommandResponse>) {
    match app::user::delete_user(state, command.id).await {
        Ok(()) => (StatusCode::OK, UserDeleteCommandResponse::DeleteOk.into()),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                UserDeleteCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "user delete command error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    UserDeleteCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}
