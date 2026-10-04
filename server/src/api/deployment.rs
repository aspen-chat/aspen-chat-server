//! The deployment's own roles (`app::deployment_role`), under `/admin`: what they allow, who
//! holds them, and the moderation log (`app::moderation_log`). Managing roles takes Manage
//! deployment roles and reaches only roles and people ranked below the caller's highest role.

use crate::api::admin::AdminUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Created, Json, NoContent, Path, Query, double_option};
use crate::api::{API_PREFIX, TAG_ADMIN};
use crate::app::context::GlobalServerContext;
use crate::app::deployment::{DeploymentPermission, from_names, to_names};
use crate::app::deployment_role::DeploymentRoleRow;
use crate::app::file_transfer::{FileTransferMode, FileTransferOutcome};
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
    /// The hue, 0 to 359, that holders' names are drawn in everywhere, over any community
    /// role's, when this is the highest role they hold that has one.
    pub hue: Option<i16>,
}

impl From<DeploymentRoleRow> for DeploymentRole {
    fn from(row: DeploymentRoleRow) -> Self {
        DeploymentRole {
            id: row.id,
            name: row.name,
            position: row.position,
            permissions: to_names(row.permissions),
            hue: row.hue,
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
        app::deployment_role::read_roles(&state)
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
    /// 0 to 359; absent for none.
    #[serde(default)]
    pub hue: Option<i16>,
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
    let role = app::deployment_role::create_role(
        &state,
        session.user.id,
        &request.name,
        from_names(&request.permissions),
        request.hue,
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
    /// 0 to 359, or `null` to take the hue away.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(nullable)]
    pub hue: Option<Option<i16>>,
}

/// Renames a deployment role below the caller's highest, or changes its permissions or hue;
/// every permission given or taken must be one the caller holds. A hue changed reaches its
/// holders' `nameHue`, announced as an update of each.
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
    let role = app::deployment_role::update_role(
        &state,
        session.user.id,
        role,
        request.name.as_deref(),
        request.permissions.as_deref().map(from_names),
        request.hue,
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
    app::deployment_role::delete_role(&state, session.user.id, role).await?;
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
        app::deployment_role::reorder_roles(&state, session.user.id, &request.roles)
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
    let added =
        app::deployment_role::set_user_role(&state, session.user.id, user, role, true).await?;
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
    app::deployment_role::set_user_role(&state, session.user.id, user, role, false).await?;
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
    /// What the ids above name, as they stand now.
    pub details: app::moderation_log::ModerationDetails,
}

impl From<app::moderation_log::ModerationEntry> for ModerationEntry {
    fn from(e: app::moderation_log::ModerationEntry) -> Self {
        Self {
            id: e.id,
            actor: e.actor,
            action: e.action,
            community: e.community,
            channel: e.channel,
            subject: e.subject,
            at: e.at,
            details: e.details,
        }
    }
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
    let entries = app::moderation_log::read_moderation_log(
        &state,
        query.before,
        i64::from(query.limit.unwrap_or(50).clamp(1, 100)),
    )
    .await?;
    Ok(Json(
        entries.into_iter().map(ModerationEntry::from).collect(),
    ))
}

/// A file offered in a call, as the deployment's record of transfers keeps it: never its
/// contents, which never reach the server.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileOfferEntry {
    pub id: uuid::Uuid,
    /// The call's channel; `null` once it is gone.
    pub channel: Option<ChannelId>,
    /// Who offered it; `null` once their account is gone.
    pub sender: Option<UserId>,
    pub file_name: String,
    /// In bytes, as the sender stated it.
    pub file_size: i64,
    /// Whether the sender let receivers connect to them directly.
    pub allow_direct: bool,
    pub valid_for_seconds: i32,
    pub offered_at: DateTime<Utc>,
    /// Everyone who accepted it, in the order they did; someone who accepted again after a
    /// transfer stopped appears again.
    pub transfers: Vec<FileTransferEntry>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileTransferEntry {
    /// `null` once their account is gone.
    pub receiver: Option<UserId>,
    pub mode: FileTransferMode,
    pub started_at: DateTime<Utc>,
    /// `null` while it is under way, or when its end was never reported.
    pub ended_at: Option<DateTime<Utc>>,
    pub outcome: Option<FileTransferOutcome>,
    /// Which side ended it.
    pub ended_by: Option<UserId>,
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct FileTransferLogQuery {
    /// Continue before this offer, the last of the previous page.
    pub before: Option<uuid::Uuid>,
    /// Only offers this user made or received.
    #[serde(rename = "filter[user]")]
    #[param(rename = "filter[user]")]
    pub user: Option<UserId>,
    /// How many offers to return, at most 100; 50 when absent.
    pub limit: Option<u32>,
}

/// The record of files offered in calls and who received them, newest offer first, a page at a
/// time. Anyone who may view the dashboard may read it.
#[utoipa::path(
    get,
    path = "/admin/file-transfers",
    tag = TAG_ADMIN,
    params(FileTransferLogQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<FileOfferEntry>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired` or `forbidden`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_file_transfer_log(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Query(query): Query<FileTransferLogQuery>,
) -> ApiResult<Json<Vec<FileOfferEntry>>> {
    access.require(DeploymentPermission::ViewDashboard)?;
    let entries = app::file_transfer::read_log(
        &state,
        query.before,
        query.user,
        i64::from(query.limit.unwrap_or(50).clamp(1, 100)),
    )
    .await?;
    Ok(Json(
        entries
            .into_iter()
            .map(|(offer, transfers)| FileOfferEntry {
                id: offer.id,
                channel: offer.channel,
                sender: offer.sender,
                file_name: offer.file_name,
                file_size: offer.file_size,
                allow_direct: offer.allow_direct,
                valid_for_seconds: offer.valid_for_seconds,
                offered_at: offer.offered_at,
                transfers: transfers
                    .into_iter()
                    .map(|t| FileTransferEntry {
                        receiver: t.receiver,
                        mode: t.mode,
                        started_at: t.started_at,
                        ended_at: t.ended_at,
                        outcome: t.outcome,
                        ended_by: t.ended_by,
                    })
                    .collect(),
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
