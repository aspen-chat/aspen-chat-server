use crate::api::auth::SessionUser;
use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::double_option;
use crate::api::extract::{Created, Json, NoContent, Path, Query};
use crate::api::include::{IncludeSet, Included, Sideloaded};
use crate::api::message_enum;
use crate::api::{API_PREFIX, TAG_INVITES};
use crate::app::context::GlobalServerContext;
use crate::app::{self, CommunityId};
use axum::extract::State;
use chrono::{DateTime, Utc};
use diesel::result::DatabaseErrorKind;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InviteCreateRequest {
    /// Alphanumeric, 1 to 16 characters. Omit to have the server generate a code.
    pub custom_code: Option<String>,
    /// Omit for an invite that never expires.
    pub expires_at: Option<DateTime<Utc>>,
}

#[utoipa::path(
    post,
    path = "/communities/{community}/invites",
    tag = TAG_INVITES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = message_enum::Invite, headers(("Location" = String, description = "URL of the new invite"))),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (bad custom code, not a member)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = CONFLICT, description = "`inviteCodeTaken`", body = Problem),
        (status = NOT_FOUND, description = "No such community, or the caller is not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_invite(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Json(request): Json<InviteCreateRequest>,
) -> ApiResult<Created<message_enum::Invite>> {
    let invite = app::invite::create_invite(
        &state,
        user.id,
        community,
        request.custom_code,
        request.expires_at,
    )
    .await
    .map_err(|e| match e {
        app::Error::Diesel(diesel::result::Error::DatabaseError(
            DatabaseErrorKind::UniqueViolation,
            _,
        )) => ApiError::new(ProblemCode::InviteCodeTaken),
        other => other.into(),
    })?;
    Ok(Created::new(
        format!("{API_PREFIX}/invites/{}", invite.code),
        message_enum::Invite::from(&invite),
    ))
}

#[utoipa::path(
    get,
    path = "/communities/{community}/invites",
    tag = TAG_INVITES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<message_enum::Invite>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such community, or the caller is not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_community_invites(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
) -> ApiResult<Json<Vec<message_enum::Invite>>> {
    let invites = app::invite::read_community_invites(&state, user.id, community).await?;
    Ok(Json(
        invites.iter().map(message_enum::Invite::from).collect(),
    ))
}

/// Relationships an invite read can sideload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum InviteInclude {
    /// The community the invite opens, as `included.communities`.
    Community,
}

/// Body of an invite read; a named alias for the same reason as `api::community::CommunityRead`.
pub type InviteRead = Sideloaded<message_enum::Invite>;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InviteReadQuery {
    /// Related records to return alongside the invite, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<InviteInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<InviteInclude>,
}

/// Reads an invite by its code, so a client holding only a link can show which community it
/// opens before joining. An expired invite is still returned, with its `expiresAt` in the past;
/// a revoked or unknown code is `404`.
#[utoipa::path(
    get,
    path = "/invites/{code}",
    tag = TAG_INVITES,
    params(("code" = String, Path), InviteReadQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = InviteRead),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_invite(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
    Path(code): Path<String>,
    Query(query): Query<InviteReadQuery>,
) -> ApiResult<Json<InviteRead>> {
    let invite = app::invite::read_invite(&state, &code).await?;
    let mut included = Included::default();
    if query.include.contains(InviteInclude::Community) {
        let community = app::community::read_invited_community(&state, invite.community).await?;
        included.communities = Some(vec![message_enum::Community::from(community)]);
    }
    Ok(Json(InviteRead::new(
        message_enum::Invite::from(&invite),
        included,
    )))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InviteUpdateRequest {
    /// Omit to leave the expiry unchanged; send `null` to make the invite permanent.
    #[serde(default, deserialize_with = "double_option")]
    pub expires_at: Option<Option<DateTime<Utc>>>,
}

#[utoipa::path(
    patch,
    path = "/invites/{code}",
    tag = TAG_INVITES,
    params(("code" = String, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = message_enum::Invite),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_invite(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(code): Path<String>,
    Json(request): Json<InviteUpdateRequest>,
) -> ApiResult<Json<message_enum::Invite>> {
    let invite = app::invite::update_invite(&state, user.id, code, request.expires_at)
        .await
        .map_err(membership_required)?;
    Ok(Json(message_enum::Invite::from(&invite)))
}

#[utoipa::path(
    delete,
    path = "/invites/{code}",
    tag = TAG_INVITES,
    params(("code" = String, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn revoke_invite(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(code): Path<String>,
) -> ApiResult<NoContent> {
    app::invite::revoke_invite(&state, user.id, code)
        .await
        .map_err(membership_required)?;
    Ok(NoContent)
}

/// The app layer reports "not a member of this community" as a validation failure; on the wire
/// that is an authorization problem, so it is served as `403`.
fn membership_required(e: app::Error) -> ApiError {
    match e {
        app::Error::Validation(reason) => ApiError::new(ProblemCode::Forbidden).with_detail(reason),
        other => other.into(),
    }
}

/// A usable registration invite, as someone about to register with it sees it.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationInvitePreview {
    pub code: String,
    pub expires_at: Option<DateTime<Utc>>,
    /// For a dual invite whose community invite still works, that invite's code: the account
    /// joins its community as it is made, and someone who already has an account joins with
    /// it instead. `null` for a plain registration invite.
    pub community_invite: Option<String>,
    /// The community `communityInvite` opens; `null` with it.
    pub community: Option<CommunityId>,
}

/// Relationships a registration invite read can sideload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum RegistrationInviteInclude {
    /// The community a dual invite joins, as `included.communities`.
    Community,
}

/// Body of a registration invite read; a named alias for the same reason as
/// `api::community::CommunityRead`.
pub type RegistrationInviteRead = Sideloaded<RegistrationInvitePreview>;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RegistrationInviteReadQuery {
    /// Related records to return alongside the invite, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<RegistrationInviteInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<RegistrationInviteInclude>,
}

/// Reads a registration invite by its code, so the page a registration link opens can say which
/// community the account will join, and send someone already signed in to join it instead.
/// Unauthenticated, like registering. An invite that is unknown, revoked, expired, or used up
/// is `404`.
#[utoipa::path(
    get,
    path = "/registration-invites/{code}",
    tag = TAG_INVITES,
    params(("code" = String, Path), RegistrationInviteReadQuery),
    responses(
        (status = OK, body = RegistrationInviteRead),
        (status = BAD_REQUEST, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_registration_invite(
    State(state): State<GlobalServerContext>,
    Path(code): Path<String>,
    Query(query): Query<RegistrationInviteReadQuery>,
) -> ApiResult<Json<RegistrationInviteRead>> {
    let invite = app::registration_invite::read_usable(&state, &code).await?;
    let community = {
        let mut conn = state
            .connection_pool
            .get()
            .await
            .map_err(app::Error::from)?;
        app::registration_invite::invited_communities(conn.as_mut(), std::slice::from_ref(&invite))
            .await?
            .into_values()
            .next()
            .filter(|community| community.usable)
    };
    let mut included = Included::default();
    if let Some(community) = &community
        && query.include.contains(RegistrationInviteInclude::Community)
    {
        let record = app::community::read_invited_community(&state, community.id).await?;
        included.communities = Some(vec![message_enum::Community::from(record)]);
    } else if query.include.contains(RegistrationInviteInclude::Community) {
        included.communities = Some(Vec::new());
    }
    Ok(Json(RegistrationInviteRead::new(
        RegistrationInvitePreview {
            code: invite.code,
            expires_at: invite.expires_at,
            community_invite: community.as_ref().map(|c| c.invite.clone()),
            community: community.map(|c| c.id),
        },
        included,
    )))
}
