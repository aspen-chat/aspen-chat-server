use crate::api::GlobalServerContext;
use crate::api::login::SessionUser;
use crate::api::message_enum::command::{
    CommunityCategoriesReadCommand, CommunityCategoriesReadCommandResponse,
    CommunityChannelsReadCommand, CommunityChannelsReadCommandResponse, CommunityCreateCommand,
    CommunityCreateCommandResponse, CommunityDeleteCommand, CommunityDeleteCommandResponse,
    CommunityReadCommand, CommunityReadCommandResponse, CommunityUpdateCommand,
    CommunityUpdateCommandResponse, CommunityUsersReadCommand, CommunityUsersReadCommandResponse,
};
use crate::{api, app};
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

#[utoipa::path(get, path = "/community/users", responses((status = OK, body=CommunityUsersReadCommandResponse)))]
pub async fn read_community_users(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityUsersReadCommand>,
) -> (StatusCode, Json<CommunityUsersReadCommandResponse>) {
    todo!()
}

#[utoipa::path(get, path = "/community/categories", responses((status = OK, body=CommunityCategoriesReadCommandResponse)))]
pub async fn read_community_categories(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityCategoriesReadCommand>,
) -> (StatusCode, Json<CommunityCategoriesReadCommandResponse>) {
    todo!()
}

#[utoipa::path(get, path = "/community/channels", responses((status = OK, body=CommunityChannelsReadCommandResponse)))]
pub async fn read_community_channels(
    State(state): State<GlobalServerContext>,
    Json(command): Json<CommunityChannelsReadCommand>,
) -> (StatusCode, Json<CommunityChannelsReadCommandResponse>) {
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
