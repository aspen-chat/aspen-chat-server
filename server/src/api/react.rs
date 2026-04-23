use crate::api::GlobalServerContext;
use crate::api::login::SessionUser;
use crate::api::message_enum;
use crate::api::message_enum::command::{
    ReactCreateCommand, ReactCreateCommandResponse, ReactDeleteCommand, ReactDeleteCommandResponse,
};
use crate::app;
use crate::app::Error;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use tracing::error;

#[utoipa::path(post, path = "/react", responses((status = OK, body=ReactCreateCommandResponse)))]

pub async fn create_react(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(command): Json<ReactCreateCommand>,
) -> (StatusCode, Json<ReactCreateCommandResponse>) {
    match app::react::create_react(&state, user.id, command.message_id, command.emoji.clone()).await
    {
        Ok(_) => (
            StatusCode::OK,
            ReactCreateCommandResponse::CreateOk(message_enum::React {
                message_id: command.message_id,
                emoji: command.emoji,
                user_id: user.id,
            })
            .into(),
        ),
        Err(Error::Validation(reason)) => (
            StatusCode::BAD_REQUEST,
            ReactCreateCommandResponse::Error {
                cause: Some(reason),
            }
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error creating react");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ReactCreateCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

#[utoipa::path(delete, path = "/react", responses((status = OK, body=ReactDeleteCommandResponse)))]
pub async fn delete_react(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(command): Json<ReactDeleteCommand>,
) -> (StatusCode, Json<ReactDeleteCommandResponse>) {
    match app::react::delete_react(&state, user.id, command.message_id, command.emoji).await {
        Ok(()) => (StatusCode::OK, ReactDeleteCommandResponse::DeleteOk.into()),
        Err(e) => {
            error!(error = e.to_string(), "error deleting react");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ReactDeleteCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}
