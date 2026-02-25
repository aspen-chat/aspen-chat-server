use crate::api::GlobalServerContext;
use crate::api::login::SessionUser;
use crate::api::message_enum::command::{
    CommunityCreateCommand, CommunityCreateCommandResponse, CommunityDeleteCommand,
    CommunityDeleteCommandResponse, CommunityReadCommand, CommunityReadCommandResponse,
    CommunityUpdateCommand, CommunityUpdateCommandResponse,
};
use crate::app;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use rust_i18n::t;
use tracing::error;

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
        CommunityCreateCommandResponse::CreateOk {
            id: new_community.id,
            name: new_community.name,
            icon: new_community.icon.map(|i| i.id().clone()),
        }
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
