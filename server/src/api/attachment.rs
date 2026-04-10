use crate::api::GlobalServerContext;
use crate::api::login::SessionUser;
use crate::api::message_enum::command::{
    AttachmentCreateCommand, AttachmentCreateCommandResponse, AttachmentDeleteCommand,
    AttachmentDeleteCommandResponse, AttachmentReadCommand, AttachmentReadCommandResponse,
};
use crate::app::{self, Error};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use tracing::error;

#[utoipa::path(post, path = "/attachment", responses((status = OK, body=AttachmentCreateCommandResponse)))]
pub async fn create_attachment(
    State(state): State<GlobalServerContext>,
    SessionUser(_user): SessionUser,
    Json(command): Json<AttachmentCreateCommand>,
) -> (StatusCode, Json<AttachmentCreateCommandResponse>) {
    match app::attachment::create_attachment(
        &state,
        command.file_name.clone(),
        command.data.clone(),
        command.mime_type.clone(),
    )
    .await
    {
        Ok(attachment) => (
            StatusCode::OK,
            AttachmentCreateCommandResponse::CreateOk(crate::api::message_enum::Attachment {
                id: attachment.id,
                file_name: command.file_name,
                data: command.data,
                mime_type: command.mime_type,
            })
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error creating attachment");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                AttachmentCreateCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

#[utoipa::path(get, path = "/attachment", responses((status = OK, body=AttachmentReadCommandResponse)))]
pub async fn read_attachment(
    State(state): State<GlobalServerContext>,
    Json(command): Json<AttachmentReadCommand>,
) -> (StatusCode, Json<AttachmentReadCommandResponse>) {
    match app::attachment::read_attachment(&state, command.id).await {
        Ok((attachment, data)) => (
            StatusCode::OK,
            AttachmentReadCommandResponse::Attachment(crate::api::message_enum::Attachment {
                id: attachment.id,
                file_name: attachment.file_name,
                data,
                mime_type: attachment.mime_type,
            })
            .into(),
        ),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                AttachmentReadCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error reading attachment");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    AttachmentReadCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}

#[utoipa::path(delete, path = "/attachment", responses((status = OK, body=AttachmentDeleteCommandResponse)))]
pub async fn delete_attachment(
    State(state): State<GlobalServerContext>,
    SessionUser(_user): SessionUser,
    Json(command): Json<AttachmentDeleteCommand>,
) -> (StatusCode, Json<AttachmentDeleteCommandResponse>) {
    match app::attachment::delete_attachment(&state, command.id).await {
        Ok(()) => (
            StatusCode::OK,
            AttachmentDeleteCommandResponse::DeleteOk.into(),
        ),
        Err(e) => match e {
            Error::Diesel(diesel::result::Error::NotFound) => (
                StatusCode::NOT_FOUND,
                AttachmentDeleteCommandResponse::Error { cause: None }.into(),
            ),
            _ => {
                error!(error = e.to_string(), "error deleting attachment");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    AttachmentDeleteCommandResponse::Error { cause: None }.into(),
                )
            }
        },
    }
}
