use axum::extract::State;
use axum::Json;
use axum::http::StatusCode;
use tracing::error;
use diesel::result::DatabaseErrorKind;
use diesel::ExpressionMethods;
use diesel_async::RunQueryDsl;
use crate::api::{GlobalServerContext, UserId};
use crate::app::login::hash_password;
use crate::api::message_enum::command::{UserCreateCommand, UserCreateCommandResponse, UserDeleteCommand, UserDeleteCommandResponse, UserReadCommand, UserReadCommandResponse, UserUpdateCommand, UserUpdateCommandResponse};
use crate::database::schema;

pub async fn create_user(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserCreateCommand>,
) -> (StatusCode, Json<UserCreateCommandResponse>) {
    let mut conn = match state.connection_pool.get().await {
        Ok(conn) => conn,
        Err(e) => {
            error!("unable to get a database connection from pool {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                UserCreateCommandResponse::Error { cause: None }.into(),
            );
        }
    };
    let password_hash_result = hash_password(&command.password);
    let password_hash = match password_hash_result {
        Ok(s) => s,
        Err(e) => {
            error!("error generating password hash at user creation {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                UserCreateCommandResponse::Error { cause: None }.into(),
            );
        }
    };
    let new_user_id = UserId::new();
    let r = diesel::insert_into(schema::user::table)
        .values((
            schema::user::columns::id.eq(new_user_id.0),
            schema::user::columns::password_hash.eq(password_hash),
            schema::user::columns::name.eq(&command.name),
        ))
        .execute(&mut conn)
        .await;
    match r {
        Ok(_) => (
            StatusCode::OK,
            UserCreateCommandResponse::CreateOk { id: new_user_id, name: command.name, icon: command.icon }.into(),
        ),
        Err(e) => {
            if let diesel::result::Error::DatabaseError(
                DatabaseErrorKind::UniqueViolation,
                e,
            ) = e
            {
                (
                    StatusCode::BAD_REQUEST,
                    UserCreateCommandResponse::Error {
                        cause: Some("usernameAlreadyTaken".to_string()),
                    }
                        .into(),
                )
            } else {
                error!("error inserting new user into database {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    UserCreateCommandResponse::Error { cause: None }.into(),
                )
            }
        }
    }
}

pub async fn read_user(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserReadCommand>,
) -> (StatusCode, Json<UserReadCommandResponse>) {
    todo!()
}

pub async fn update_user(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserUpdateCommand>,
) -> (StatusCode, Json<UserUpdateCommandResponse>) {
    todo!()
}
pub async fn delete_user(
    State(state): State<GlobalServerContext>,
    Json(command): Json<UserDeleteCommand>,
) -> (StatusCode, Json<UserDeleteCommandResponse>) {
    todo!()
}
