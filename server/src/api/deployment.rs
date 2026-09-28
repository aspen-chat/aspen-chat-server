//! The deployment's own roles (`app::deployment`), under `/admin`: what they allow, who holds
//! them, and the moderation log. Managing roles takes Manage deployment roles and reaches only
//! roles and people ranked below the caller's highest role.

use crate::api::admin::AdminUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Created, Json, NoContent, Path, Query};
use crate::api::{API_PREFIX, GlobalServerContext, TAG_ADMIN};
use crate::app::deployment::{DeploymentPermission, DeploymentRoleRow, from_names, to_names};
use crate::app::{self, ChannelId, CommunityId, DeploymentRoleId, UserId};
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// A deployment role.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRole {
    pub id: DeploymentRoleId,
    pub name: String,
    /// Its rank: higher outranks lower, from 1.
    pub position: i32,
    pub permissions: Vec<DeploymentPermission>,
}

impl From<DeploymentRoleRow> for DeploymentRole {
    fn from(row: DeploymentRoleRow) -> Self {
        DeploymentRole {
            id: row.id,
            name: row.name,
            position: row.position,
            permissions: to_names(row.permissions),
        }
    }
}

/// The deployment's roles, lowest first. Anyone with a deployment permission may read them.
#[utoipa::path(
    get,
    path = "/admin/roles",
    tag = TAG_ADMIN,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<DeploymentRole>),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_deployment_roles(
    State(state): State<GlobalServerContext>,
    _admin: AdminUser,
) -> ApiResult<Json<Vec<DeploymentRole>>> {
    Ok(Json(
        app::deployment::read_roles(&state)
            .await?
            .into_iter()
            .map(DeploymentRole::from)
            .collect(),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRoleCreateRequest {
    pub name: String,
    pub permissions: Vec<DeploymentPermission>,
}

/// Makes a deployment role, placed lowest. Takes Manage deployment roles; only permissions the
/// caller holds may be given.
#[utoipa::path(
    post,
    path = "/admin/roles",
    tag = TAG_ADMIN,
    request_body = DeploymentRoleCreateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = DeploymentRole, headers(("Location" = String, description = "URL of the new role"))),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired` or `forbidden`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_deployment_role(
    State(state): State<GlobalServerContext>,
    AdminUser(session, _access): AdminUser,
    Json(request): Json<DeploymentRoleCreateRequest>,
) -> ApiResult<Created<DeploymentRole>> {
    let role = app::deployment::create_role(
        &state,
        session.user.id,
        &request.name,
        from_names(&request.permissions),
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/admin/roles/{}", role.id.0),
        DeploymentRole::from(role),
    ))
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRoleUpdateRequest {
    pub name: Option<String>,
    pub permissions: Option<Vec<DeploymentPermission>>,
}

/// Renames a deployment role below the caller's highest, or changes its permissions; every
/// permission given or taken must be one the caller holds.
#[utoipa::path(
    patch,
    path = "/admin/roles/{role}",
    tag = TAG_ADMIN,
    params(("role" = DeploymentRoleId, Path)),
    request_body = DeploymentRoleUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = DeploymentRole),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired` or `forbidden`", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_deployment_role(
    State(state): State<GlobalServerContext>,
    AdminUser(session, _access): AdminUser,
    Path(role): Path<DeploymentRoleId>,
    Json(request): Json<DeploymentRoleUpdateRequest>,
) -> ApiResult<Json<DeploymentRole>> {
    let role = app::deployment::update_role(
        &state,
        session.user.id,
        role,
        request.name.as_deref(),
        request.permissions.as_deref().map(from_names),
    )
    .await?;
    Ok(Json(DeploymentRole::from(role)))
}

/// Deletes a deployment role below the caller's highest; its holders lose what it allowed.
#[utoipa::path(
    delete,
    path = "/admin/roles/{role}",
    tag = TAG_ADMIN,
    params(("role" = DeploymentRoleId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired` or `forbidden`", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_deployment_role(
    State(state): State<GlobalServerContext>,
    AdminUser(session, _access): AdminUser,
    Path(role): Path<DeploymentRoleId>,
) -> ApiResult<NoContent> {
    app::deployment::delete_role(&state, session.user.id, role).await?;
    Ok(NoContent)
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentRoleOrderRequest {
    /// Every role ranked below the caller's highest, lowest first.
    pub roles: Vec<DeploymentRoleId>,
}

/// Reorders the deployment roles below the caller's highest. Returns every role, lowest first.
#[utoipa::path(
    put,
    path = "/admin/role-order",
    tag = TAG_ADMIN,
    request_body = DeploymentRoleOrderRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<DeploymentRole>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired` or `forbidden`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn reorder_deployment_roles(
    State(state): State<GlobalServerContext>,
    AdminUser(session, _access): AdminUser,
    Json(request): Json<DeploymentRoleOrderRequest>,
) -> ApiResult<Json<Vec<DeploymentRole>>> {
    Ok(Json(
        app::deployment::reorder_roles(&state, session.user.id, &request.roles)
            .await?
            .into_iter()
            .map(DeploymentRole::from)
            .collect(),
    ))
}

/// Gives someone a deployment role below the caller's highest: `201` when it is new to them,
/// `200` when they held it. They must rank below the caller, unless they are the caller.
#[utoipa::path(
    put,
    path = "/admin/users/{user}/roles/{role}",
    tag = TAG_ADMIN,
    params(("user" = UserId, Path), ("role" = DeploymentRoleId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Given"),
        (status = OK, description = "Already held"),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired` or `forbidden`", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn add_user_deployment_role(
    State(state): State<GlobalServerContext>,
    AdminUser(session, _access): AdminUser,
    Path((user, role)): Path<(UserId, DeploymentRoleId)>,
) -> ApiResult<StatusCode> {
    let added = app::deployment::set_user_role(&state, session.user.id, user, role, true).await?;
    Ok(if added {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    })
}

/// Takes a deployment role from someone, on the same terms as giving it.
#[utoipa::path(
    delete,
    path = "/admin/users/{user}/roles/{role}",
    tag = TAG_ADMIN,
    params(("user" = UserId, Path), ("role" = DeploymentRoleId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired` or `forbidden`", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_user_deployment_role(
    State(state): State<GlobalServerContext>,
    AdminUser(session, _access): AdminUser,
    Path((user, role)): Path<(UserId, DeploymentRoleId)>,
) -> ApiResult<NoContent> {
    app::deployment::set_user_role(&state, session.user.id, user, role, false).await?;
    Ok(NoContent)
}

/// One use of Moderate any community.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModerationEntry {
    pub id: uuid::Uuid,
    /// Who did it; `null` once their account is gone.
    pub actor: Option<UserId>,
    /// What they did: `readDm`, `deleteMessage`, `removeAttachment`,
    /// `removeReaction`, `removeMember`, `renameChannel`, `deleteChannel`, `renameCommunity`,
    /// `deleteCommunity`, or `removeWriteIn`.
    pub action: String,
    pub community: Option<CommunityId>,
    pub channel: Option<ChannelId>,
    /// What else it was done to: a message, a user, an attachment, a reaction.
    pub subject: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct ModerationLogQuery {
    /// Continue before this entry, the last of the previous page.
    pub before: Option<uuid::Uuid>,
    /// How many to return, at most 100; 50 when absent.
    pub limit: Option<u32>,
}

/// The moderation log, newest first, a page at a time. Anyone who may view the dashboard may
/// read it, so moderators answer to the deployment's other administrators.
#[utoipa::path(
    get,
    path = "/admin/moderation-log",
    tag = TAG_ADMIN,
    params(ModerationLogQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<ModerationEntry>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired` or `forbidden`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_moderation_log(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Query(query): Query<ModerationLogQuery>,
) -> ApiResult<Json<Vec<ModerationEntry>>> {
    access.require(DeploymentPermission::ViewDashboard)?;
    let entries = app::deployment::read_moderation_log(
        &state,
        query.before,
        i64::from(query.limit.unwrap_or(50).clamp(1, 100)),
    )
    .await?;
    Ok(Json(
        entries
            .into_iter()
            .map(|e| ModerationEntry {
                id: e.id,
                actor: e.actor,
                action: e.action,
                community: e.community,
                channel: e.channel,
                subject: e.subject,
                at: e.at,
            })
            .collect(),
    ))
}

/// Someone's DMs and group DMs, the most recently active first, for a deployment moderator to
/// open. Takes Moderate any community; reading any of them is written to the moderation log.
#[utoipa::path(
    get,
    path = "/admin/users/{user}/dms",
    tag = TAG_ADMIN,
    params(("user" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<crate::api::message_enum::Channel>),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired` or `forbidden`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_user_dms(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(user): Path<UserId>,
) -> ApiResult<Json<Vec<crate::api::message_enum::Channel>>> {
    access.require(DeploymentPermission::ModerateCommunities)?;
    Ok(Json(
        app::dm::list_dms(&state, user)
            .await?
            .into_iter()
            .map(|(dm, recipients)| app::channel::record(&dm, recipients))
            .collect(),
    ))
}
