use crate::api::GlobalServerContext;
use crate::api::message_enum::command::{
    IconCreateCommand, IconCreateCommandResponse, IconDeleteCommand, IconDeleteCommandResponse,
    IconReadCommand, IconReadCommandResponse,
};
use crate::app::{self, Error};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use tracing::error;

#[utoipa::path(post, path = "/icon", responses((status = OK, body=IconCreateCommandResponse)))]
pub async fn create_icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconCreateCommand>,
) -> (StatusCode, Json<IconCreateCommandResponse>) {
    let data = command.data.clone();
    let mime_type = command.mime_type.clone();
    match app::icon::create_icon(&state, command.data, command.mime_type).await {
        Ok(icon) => (
            StatusCode::OK,
            IconCreateCommandResponse::CreateOk(crate::api::message_enum::Icon {
                id: icon.id,
                data,
                mime_type,
            })
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error creating icon");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                IconCreateCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

#[utoipa::path(get, path = "/icon", responses((status = OK, body=IconReadCommandResponse)))]
pub async fn read_icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconReadCommand>,
) -> (StatusCode, Json<IconReadCommandResponse>) {
    match app::icon::read_icon(&state, command.id).await {
        Ok((icon, data)) => (
            StatusCode::OK,
            IconReadCommandResponse::Icon(crate::api::message_enum::Icon {
                id: icon.id,
                data,
                mime_type: icon.mime_type,
            })
            .into(),
        ),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                IconReadCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error reading icon");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    IconReadCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}

#[utoipa::path(delete, path = "/icon", responses((status = OK, body=IconDeleteCommandResponse)))]
pub async fn delete_icon(
    State(state): State<GlobalServerContext>,
    Json(command): Json<IconDeleteCommand>,
) -> (StatusCode, Json<IconDeleteCommandResponse>) {
    match app::icon::delete_icon(&state, command.id).await {
        Ok(()) => (StatusCode::OK, IconDeleteCommandResponse::DeleteOk.into()),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                IconDeleteCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error deleting icon");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    IconDeleteCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}
