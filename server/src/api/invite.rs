use crate::api::GlobalServerContext;
use crate::api::login::SessionUser;
use crate::api::message_enum;
use crate::app::{self, CommunityId};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use tracing::error;
use utoipa::ToSchema;

fn invite_to_api(invite: &app::invite::Invite) -> message_enum::Invite {
    message_enum::Invite {
        code: invite.code.clone(),
        created_by: invite.created_by,
        created_at: invite.created_at,
        community: invite.community,
        expires_at: invite.expires_at,
    }
}

// --- Create Invite ---

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InviteCreateCommand {
    pub community: CommunityId,
    pub custom_code: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum InviteCreateCommandResponse {
    Invite(message_enum::Invite),
    CodeAlreadyTaken,
    NotAllowed { reason: Option<Cow<'static, str>> },
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(post, path = "/invite", security(("loginKey" = [])), responses((status = OK, body = InviteCreateCommandResponse)))]
pub async fn create_invite(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(command): Json<InviteCreateCommand>,
) -> (StatusCode, Json<InviteCreateCommandResponse>) {
    match app::invite::create_invite(
        &state,
        user.id,
        command.community,
        command.custom_code,
        command.expires_at,
    )
    .await
    {
        Ok(invite) => (
            StatusCode::OK,
            InviteCreateCommandResponse::Invite(invite_to_api(&invite)).into(),
        ),
        Err(app::Error::Validation(reason)) => (
            StatusCode::BAD_REQUEST,
            InviteCreateCommandResponse::NotAllowed {
                reason: Some(reason),
            }
            .into(),
        ),
        Err(app::Error::Diesel(diesel::result::Error::DatabaseError(
            diesel::result::DatabaseErrorKind::UniqueViolation,
            _,
        ))) => (
            StatusCode::CONFLICT,
            InviteCreateCommandResponse::CodeAlreadyTaken.into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error creating invite");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                InviteCreateCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

// --- Read Community Invites ---

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommunityInvitesReadCommand {
    pub community: CommunityId,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum CommunityInvitesReadCommandResponse {
    Invites { data: Vec<message_enum::Invite> },
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(get, path = "/invite", security(("loginKey" = [])), responses((status = OK, body = CommunityInvitesReadCommandResponse)))]
pub async fn read_community_invites(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
    Json(command): Json<CommunityInvitesReadCommand>,
) -> (StatusCode, Json<CommunityInvitesReadCommandResponse>) {
    match app::invite::read_community_invites(&state, command.community).await {
        Ok(invites) => (
            StatusCode::OK,
            CommunityInvitesReadCommandResponse::Invites {
                data: invites.iter().map(invite_to_api).collect(),
            }
            .into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error reading community invites");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                CommunityInvitesReadCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

// --- Update Invite ---

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InviteUpdateCommand {
    pub code: String,
    pub expires_at: Option<Option<DateTime<Utc>>>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum InviteUpdateCommandResponse {
    Invite(message_enum::Invite),
    NotAllowed { reason: Option<Cow<'static, str>> },
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(patch, path = "/invite", security(("loginKey" = [])), responses((status = OK, body = InviteUpdateCommandResponse)))]
pub async fn update_invite(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(command): Json<InviteUpdateCommand>,
) -> (StatusCode, Json<InviteUpdateCommandResponse>) {
    match app::invite::update_invite(&state, user.id, command.code, command.expires_at).await {
        Ok(invite) => (
            StatusCode::OK,
            InviteUpdateCommandResponse::Invite(invite_to_api(&invite)).into(),
        ),
        Err(app::Error::Validation(reason)) => (
            StatusCode::FORBIDDEN,
            InviteUpdateCommandResponse::NotAllowed {
                reason: Some(reason),
            }
            .into(),
        ),
        Err(app::Error::Diesel(diesel::result::Error::NotFound)) => (
            StatusCode::NOT_FOUND,
            InviteUpdateCommandResponse::Error { cause: None }.into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error updating invite");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                InviteUpdateCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}

// --- Revoke Invite ---

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InviteRevokeCommand {
    pub code: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum InviteRevokeCommandResponse {
    RevokeOk,
    NotAllowed { reason: Option<Cow<'static, str>> },
    Error { cause: Option<Cow<'static, str>> },
}

#[utoipa::path(delete, path = "/invite", security(("loginKey" = [])), responses((status = OK, body = InviteRevokeCommandResponse)))]
pub async fn revoke_invite(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(command): Json<InviteRevokeCommand>,
) -> (StatusCode, Json<InviteRevokeCommandResponse>) {
    match app::invite::revoke_invite(&state, user.id, command.code).await {
        Ok(()) => (StatusCode::OK, InviteRevokeCommandResponse::RevokeOk.into()),
        Err(app::Error::Validation(reason)) => (
            StatusCode::FORBIDDEN,
            InviteRevokeCommandResponse::NotAllowed {
                reason: Some(reason),
            }
            .into(),
        ),
        Err(app::Error::Diesel(diesel::result::Error::NotFound)) => (
            StatusCode::NOT_FOUND,
            InviteRevokeCommandResponse::Error { cause: None }.into(),
        ),
        Err(e) => {
            error!(error = e.to_string(), "error revoking invite");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                InviteRevokeCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}
