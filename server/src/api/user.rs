use crate::api::login::SessionUser;
use crate::api::message_enum::command::{
    UserCreateCommand, UserCreateCommandResponse, UserDeleteCommand, UserDeleteCommandResponse,
    UserReadCommand, UserReadCommandResponse, UserUpdateCommand, UserUpdateCommandResponse,
};
use crate::api::{GlobalServerContext, UserId};
use crate::app;
use crate::app::Error;
use crate::app::login::hash_password;
use crate::app::user::User;
use crate::database::schema;
use axum::extract::State;
use axum::http::StatusCode;
use axum::{Extension, Json};
use diesel::result::DatabaseErrorKind;
use diesel::{ExpressionMethods, QueryResult};
use diesel_async::RunQueryDsl;
use rust_i18n::t;
use tracing::error;

#[utoipa::path(post, path = "/user", responses((status = OK, body=UserCreateCommandResponse)))]
pub async fn create_user(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserCreateCommand>,
) -> (StatusCode, Json<UserCreateCommandResponse>) {
    let new_user_id = match app::user::create_user(state, &command).await {
        Ok(value) => value,
        Err(err) => {
            return {
                if let app::Error::Diesel(diesel::result::Error::DatabaseError(
                    DatabaseErrorKind::UniqueViolation,
                    e,
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
        UserCreateCommandResponse::CreateOk {
            id: new_user_id,
            name: command.name,
            icon: command.icon,
        }
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
            UserReadCommandResponse::User {
                name: user.name,
                icon: user.icon,
            }
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

#[utoipa::path(patch, path = "/user", responses((status = OK, body=UserUpdateCommandResponse)))]

pub async fn update_user(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserUpdateCommand>,
) -> (StatusCode, Json<UserUpdateCommandResponse>) {
    todo!()
}

#[utoipa::path(delete, path = "/user", responses((status = OK, body=UserDeleteCommandResponse)))]
pub async fn delete_user(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserDeleteCommand>,
) -> (StatusCode, Json<UserDeleteCommandResponse>) {
    todo!()
}
