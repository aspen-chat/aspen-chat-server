//! The Administration Dashboard's API, under `/admin`: the deployment's totals, its users and
//! communities, registration invites, and the health of its servers. Only administrators may
//! call it (`AdminUser`); who is one is decided from the terminal (`app::admin`). Powers too
//! strong to expose even here, such as suspending rate limits, stay terminal commands.

use crate::auth::SessionUser;
use crate::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::extract::{Created, Json, NoContent, Path, Query};
use crate::{API_PREFIX, TAG_ADMIN};
use aspen_app::context::GlobalServerContext;
use aspen_app::deployment::{DeploymentAccess, DeploymentPermission, DeploymentPermissions};
use aspen_app::{self as app, CommunityId, DeploymentRoleId, IconId, UserId, VoiceServerId};
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// A caller who holds some deployment permission, with what they may do; anyone else is
/// refused with `adminRequired`. Each handler requires the permission it needs, refusing with
/// `forbidden` without it.
pub struct AdminUser(pub SessionUser, pub DeploymentAccess);

impl FromRequestParts<GlobalServerContext> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &GlobalServerContext,
    ) -> Result<Self, Self::Rejection> {
        let session = <SessionUser as FromRequestParts<GlobalServerContext>>::from_request_parts(
            parts, state,
        )
        .await?;
        let access = app::deployment::access_of(state, session.user.id).await?;
        if access.permissions == DeploymentPermissions::empty() {
            Err(ApiError::new(ProblemCode::AdminRequired))
        } else {
            Ok(AdminUser(session, access))
        }
    }
}

/// Refuses a caller who may not browse the user and community directories
/// (`DeploymentPermissions::DIRECTORIES`).
fn require_directories(access: &DeploymentAccess) -> app::Result<()> {
    if access.has_any(DeploymentPermissions::DIRECTORIES) {
        Ok(())
    } else {
        access.require(DeploymentPermission::ViewDashboard)
    }
}

/// What the caller may do across the deployment.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminAccess {
    /// What they hold, including what each of their permissions includes.
    pub permissions: Vec<DeploymentPermission>,
    /// The deployment roles they hold, lowest first.
    pub roles: Vec<DeploymentRoleId>,
    /// Each permission that includes others, and those it includes, the same for everyone,
    /// so a role editor shows a role's included permissions as held.
    pub inclusions: Vec<DeploymentPermissionInclusion>,
}

/// A permission, and the permissions holding it gives too.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentPermissionInclusion {
    pub permission: DeploymentPermission,
    pub includes: Vec<DeploymentPermission>,
}

/// What the caller may do across the deployment, so a client knows what to offer, such as the
/// Administration Dashboard. It says nothing about anyone else.
#[utoipa::path(
    get,
    path = "/users/@me/admin",
    tag = TAG_ADMIN,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = AdminAccess),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_admin_access(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
) -> ApiResult<Json<AdminAccess>> {
    let (access, roles) = app::deployment::access_and_roles(&state, user.id).await?;
    Ok(Json(AdminAccess {
        permissions: app::deployment::to_names(access.permissions),
        roles,
        inclusions: DeploymentPermission::ALL
            .iter()
            .filter(|p| !p.includes().is_empty())
            .map(|&permission| DeploymentPermissionInclusion {
                permission,
                includes: permission.includes().to_vec(),
            })
            .collect(),
    }))
}

/// The deployment's totals.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminOverview {
    pub users: i64,
    /// Accounts made in the last seven days.
    pub new_users_this_week: i64,
    pub communities: i64,
    /// Whether creating an account takes a registration invite (the deployment setting
    /// `registrationInviteRequired`).
    pub registration_invite_required: bool,
}

#[utoipa::path(
    get,
    path = "/admin/overview",
    tag = TAG_ADMIN,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = AdminOverview),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without the permission this needs", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_overview(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<AdminOverview>> {
    access.require(DeploymentPermission::ViewDashboard)?;
    let overview = app::admin::overview(&state).await?;
    Ok(Json(AdminOverview {
        users: overview.users,
        new_users_this_week: overview.new_users_this_week,
        communities: overview.communities,
        registration_invite_required: state.settings().registration_invite_required,
    }))
}

/// How many rows a page of a dashboard list holds when the request does not say.
const DEFAULT_PAGE: i64 = 15;

/// How the user list is ordered: a field, `-` prefixed for descending.
#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
pub enum UserSort {
    #[serde(rename = "name")]
    Name,
    #[serde(rename = "-name")]
    NameDescending,
    #[serde(rename = "createdAt")]
    CreatedAt,
    #[default]
    #[serde(rename = "-createdAt")]
    CreatedAtDescending,
}

/// How the community list is ordered: a field, `-` prefixed for descending.
#[derive(Debug, Clone, Copy, Default, Deserialize, ToSchema)]
pub enum CommunitySort {
    #[serde(rename = "name")]
    Name,
    #[serde(rename = "-name")]
    NameDescending,
    #[serde(rename = "members")]
    Members,
    #[serde(rename = "-members")]
    MembersDescending,
    #[serde(rename = "createdAt")]
    CreatedAt,
    #[default]
    #[serde(rename = "-createdAt")]
    CreatedAtDescending,
}

/// A page of the user list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct UserListQuery {
    /// Only those whose username or display name contains this, ignoring case.
    #[serde(rename = "filter[name]")]
    #[param(rename = "filter[name]")]
    pub name: Option<String>,
    /// With `true`, only those banned from the deployment now.
    #[serde(rename = "filter[banned]", default)]
    #[param(rename = "filter[banned]")]
    pub banned: bool,
    /// The order; newest first when absent. `name` sorts by display name, or username where
    /// there is none.
    #[serde(default)]
    #[param(inline)]
    pub sort: UserSort,
    /// How many rows to skip, at most 100,000.
    pub offset: Option<i64>,
    /// How many to return, at most 100; 15 when absent.
    pub limit: Option<i64>,
}

/// A page of the community list.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct CommunityListQuery {
    /// Only those whose name contains this, ignoring case.
    #[serde(rename = "filter[name]")]
    #[param(rename = "filter[name]")]
    pub name: Option<String>,
    /// The order; newest first when absent.
    #[serde(default)]
    #[param(inline)]
    pub sort: CommunitySort,
    /// How many rows to skip, at most 100,000.
    pub offset: Option<i64>,
    /// How many to return, at most 100; 15 when absent.
    pub limit: Option<i64>,
}

/// A user as the dashboard lists them.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserEntry {
    pub id: UserId,
    pub name: String,
    pub display_name: Option<String>,
    pub icon: Option<IconId>,
    pub created_at: DateTime<Utc>,
    /// The deployment roles they hold, lowest first.
    pub roles: Vec<DeploymentRoleId>,
    /// The registration invite the account was made with, if one was.
    pub registered_with: Option<String>,
    /// Whether this is a bot, and who owns it: `null` for a bot whose owner deleted their
    /// account, which a holder of Manage deployment settings may delete.
    pub bot: bool,
    pub bot_owner: Option<UserId>,
    /// For a user of another deployment, that deployment's domain.
    pub home_domain: Option<String>,
    /// Whether this is the deployment's own account, which sends notices and is never banned.
    pub system: bool,
    /// Whether a ban from the deployment stands now.
    pub banned: bool,
    /// The ban standing now, if one does.
    pub ban: Option<AdminUserBan>,
}

/// A ban from the deployment as the dashboard lists it.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminUserBan {
    pub banned_at: DateTime<Utc>,
    /// What they are told when they try to sign in.
    pub reason: Option<String>,
    /// When it ends; `null` until it is lifted.
    pub until: Option<DateTime<Utc>>,
}

/// A page of the deployment's users, searched by name and sorted.
#[utoipa::path(
    get,
    path = "/admin/users",
    tag = TAG_ADMIN,
    params(UserListQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<AdminUserEntry>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without the permission this needs", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_users(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Query(query): Query<UserListQuery>,
) -> ApiResult<Json<Vec<AdminUserEntry>>> {
    require_directories(&access)?;
    use app::admin::{Sort, UserColumn};
    let sort = match query.sort {
        UserSort::Name => Sort {
            column: UserColumn::Name,
            descending: false,
        },
        UserSort::NameDescending => Sort {
            column: UserColumn::Name,
            descending: true,
        },
        UserSort::CreatedAt => Sort {
            column: UserColumn::Joined,
            descending: false,
        },
        UserSort::CreatedAtDescending => Sort {
            column: UserColumn::Joined,
            descending: true,
        },
    };
    let users = app::admin::search_users(
        &state,
        query.name.as_deref(),
        query.banned,
        sort,
        query.offset.unwrap_or(0),
        query.limit.unwrap_or(DEFAULT_PAGE),
    )
    .await?;
    let mut roles =
        app::deployment::roles_of(&state, &users.iter().map(|u| u.id).collect::<Vec<_>>()).await?;
    Ok(Json(
        users
            .into_iter()
            .map(|u| AdminUserEntry {
                roles: roles.remove(&u.id).unwrap_or_default(),
                id: u.id,
                name: u.name,
                display_name: u.display_name,
                icon: u.icon,
                created_at: u.created_at,
                registered_with: u.registered_with,
                bot: u.bot,
                bot_owner: u.bot_owner,
                home_domain: u.home_domain,
                system: u.system,
                banned: u.banned_at.is_some(),
                ban: u.banned_at.map(|banned_at| AdminUserBan {
                    banned_at,
                    reason: u.ban_reason,
                    until: u.banned_until,
                }),
            })
            .collect(),
    ))
}

/// A community as the dashboard lists it.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminCommunityEntry {
    pub id: CommunityId,
    pub name: String,
    pub icon: Option<IconId>,
    pub members: i64,
    pub created_at: DateTime<Utc>,
}

/// A page of the deployment's communities, searched by name and sorted.
#[utoipa::path(
    get,
    path = "/admin/communities",
    tag = TAG_ADMIN,
    params(CommunityListQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<AdminCommunityEntry>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without the permission this needs", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_communities(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Query(query): Query<CommunityListQuery>,
) -> ApiResult<Json<Vec<AdminCommunityEntry>>> {
    require_directories(&access)?;
    use app::admin::{CommunityColumn, Sort};
    let sort = |column, descending| Sort { column, descending };
    let sort = match query.sort {
        CommunitySort::Name => sort(CommunityColumn::Name, false),
        CommunitySort::NameDescending => sort(CommunityColumn::Name, true),
        CommunitySort::Members => sort(CommunityColumn::Members, false),
        CommunitySort::MembersDescending => sort(CommunityColumn::Members, true),
        CommunitySort::CreatedAt => sort(CommunityColumn::Created, false),
        CommunitySort::CreatedAtDescending => sort(CommunityColumn::Created, true),
    };
    let communities = app::admin::search_communities(
        &state,
        query.name.as_deref(),
        sort,
        query.offset.unwrap_or(0),
        query.limit.unwrap_or(DEFAULT_PAGE),
    )
    .await?;
    Ok(Json(
        communities
            .into_iter()
            .map(|c| AdminCommunityEntry {
                id: c.id,
                name: c.name,
                icon: c.icon,
                members: c.members,
                created_at: c.created_at,
            })
            .collect(),
    ))
}

/// How far back the growth charts reach.
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum GrowthRange {
    ThreeMonths,
    SixMonths,
    OneYear,
    FiveYears,
    AllTime,
}

#[derive(Debug, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct GrowthQuery {
    /// How far back to reach.
    #[param(inline)]
    pub range: GrowthRange,
}

/// The size of each step of a growth series.
#[derive(Debug, Clone, Copy, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum GrowthUnit {
    Day,
    Week,
    Month,
}

/// How many users and communities there were at the end of one step.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GrowthPoint {
    /// When the step began.
    pub at: DateTime<Utc>,
    pub users: i64,
    pub communities: i64,
}

/// How the deployment grew: its users and communities at each step of the range.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Growth {
    pub unit: GrowthUnit,
    /// Oldest first; the last step runs to now.
    pub points: Vec<GrowthPoint>,
}

/// How many users and communities the deployment had over a range, by day for ranges up to
/// six months, by week up to two years, and by month beyond; a user or community counts from
/// its creation until its deletion.
#[utoipa::path(
    get,
    path = "/admin/growth",
    tag = TAG_ADMIN,
    params(GrowthQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Growth),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without the permission this needs", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_growth(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Query(query): Query<GrowthQuery>,
) -> ApiResult<Json<Growth>> {
    access.require(DeploymentPermission::ViewDashboard)?;
    use app::admin::GrowthRange as Range;
    let range = match query.range {
        GrowthRange::ThreeMonths => Range::ThreeMonths,
        GrowthRange::SixMonths => Range::SixMonths,
        GrowthRange::OneYear => Range::OneYear,
        GrowthRange::FiveYears => Range::FiveYears,
        GrowthRange::AllTime => Range::AllTime,
    };
    let (unit, points) = app::admin::growth(&state, range).await?;
    Ok(Json(Growth {
        unit: match unit {
            app::admin::GrowthUnit::Day => GrowthUnit::Day,
            app::admin::GrowthUnit::Week => GrowthUnit::Week,
            app::admin::GrowthUnit::Month => GrowthUnit::Month,
        },
        points: points
            .into_iter()
            .map(|p| GrowthPoint {
                at: p.at,
                users: p.users,
                communities: p.communities,
            })
            .collect(),
    }))
}

/// An invite to create an account.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RegistrationInvite {
    pub code: String,
    /// The administrator who made it; `null` for one made from the terminal.
    pub created_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub max_uses: i32,
    pub uses: i32,
    pub revoked_at: Option<DateTime<Utc>>,
    pub note: Option<String>,
    /// Whether it would create an account now: not revoked, expired, or used up.
    pub usable: bool,
    /// For a dual invite, the community each account it makes joins; `null` for a plain one,
    /// or when its community invite is gone.
    pub community: Option<InvitedCommunity>,
}

/// The community a dual invite's accounts join.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InvitedCommunity {
    pub id: CommunityId,
    pub name: String,
    /// The code of the community invite they join with.
    pub invite: String,
    /// Whether the community invite still works; revoked or expired, the dual invite makes
    /// accounts that join nothing.
    pub usable: bool,
}

impl From<app::registration_invite::InvitedCommunity> for InvitedCommunity {
    fn from(community: app::registration_invite::InvitedCommunity) -> Self {
        InvitedCommunity {
            id: community.id,
            name: community.name,
            invite: community.invite,
            usable: community.usable,
        }
    }
}

impl RegistrationInvite {
    /// The wire record, with `usable` as of the moment it is made.
    fn new(
        invite: app::registration_invite::RegistrationInvite,
        community: Option<app::registration_invite::InvitedCommunity>,
    ) -> Self {
        RegistrationInvite {
            usable: invite.usable(Utc::now()),
            code: invite.code,
            created_by: invite.created_by,
            created_at: invite.created_at,
            expires_at: invite.expires_at,
            max_uses: invite.max_uses,
            uses: invite.uses,
            revoked_at: invite.revoked_at,
            note: invite.note,
            community: community.map(InvitedCommunity::from),
        }
    }
}

/// A new registration invite.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegistrationInviteRequest {
    /// How many accounts it may create; 1 when absent.
    #[serde(default)]
    pub max_uses: Option<i32>,
    /// How long it lasts; for good when absent.
    #[serde(default)]
    pub expires_in_seconds: Option<u32>,
    /// What it is for, to remember it by.
    #[serde(default)]
    pub note: Option<String>,
    /// Makes a dual invite: each account it makes joins this community, through a community
    /// invite the caller makes, which needs Create invites there. It expires with the
    /// registration invite.
    #[serde(default)]
    pub community: Option<CommunityId>,
}

impl From<&RegistrationInviteRequest> for app::registration_invite::Terms {
    fn from(request: &RegistrationInviteRequest) -> Self {
        Self {
            max_uses: request.max_uses.unwrap_or(1),
            expires_in: request
                .expires_in_seconds
                .map(|s| chrono::Duration::seconds(i64::from(s))),
            note: request.note.clone(),
        }
    }
}

/// The newest 500 registration invites: every usable one, and those that no longer work while
/// they are under a week old.
#[utoipa::path(
    get,
    path = "/admin/registration-invites",
    tag = TAG_ADMIN,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<RegistrationInvite>),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without the permission this needs", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_registration_invites(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<Vec<RegistrationInvite>>> {
    access.require(DeploymentPermission::ManageRegistrationInvites)?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let invites = app::registration_invite::list(conn.as_mut(), false).await?;
    let mut communities =
        app::registration_invite::invited_communities(conn.as_mut(), &invites).await?;
    Ok(Json(
        invites
            .into_iter()
            .map(|invite| {
                let community = communities.remove(&invite.code);
                RegistrationInvite::new(invite, community)
            })
            .collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/admin/registration-invites",
    tag = TAG_ADMIN,
    request_body = RegistrationInviteRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = RegistrationInvite, headers(("Location" = String, description = "URL of the new invite"))),
        (status = BAD_REQUEST, description = "`validation`: uses, expiry, or note out of bounds", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without the permission this needs, Create invites in the community included", body = Problem),
        (status = NOT_FOUND, description = "The community does not exist, or the caller is not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_registration_invite(
    State(state): State<GlobalServerContext>,
    AdminUser(session, access): AdminUser,
    Json(request): Json<RegistrationInviteRequest>,
) -> ApiResult<Created<RegistrationInvite>> {
    access.require(DeploymentPermission::ManageRegistrationInvites)?;
    let terms = app::registration_invite::Terms::from(&request);
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let invite = match request.community {
        Some(community) => {
            app::registration_invite::create_dual(&state, session.user.id, community, terms).await?
        }
        None => {
            app::registration_invite::create(conn.as_mut(), Some(session.user.id), terms).await?
        }
    };
    let communities =
        app::registration_invite::invited_communities(conn.as_mut(), std::slice::from_ref(&invite))
            .await?;
    tracing::info!(code = app::registration_invite::logged_code(&invite.code), admin = %session.user.id.0, community = ?request.community, "made a registration invite");
    let community = communities.into_values().next();
    Ok(Created::new(
        format!("{API_PREFIX}/admin/registration-invites/{}", invite.code),
        RegistrationInvite::new(invite, community),
    ))
}

/// Revokes a registration invite, so it makes no more accounts, and a dual invite's community
/// invite with it; it stays listed, and the accounts it made are kept.
#[utoipa::path(
    delete,
    path = "/admin/registration-invites/{code}",
    tag = TAG_ADMIN,
    params(("code" = String, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Revoked"),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without the permission this needs", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn revoke_registration_invite(
    State(state): State<GlobalServerContext>,
    AdminUser(session, access): AdminUser,
    Path(code): Path<String>,
) -> ApiResult<NoContent> {
    access.require(DeploymentPermission::ManageRegistrationInvites)?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    app::registration_invite::revoke(&state, conn.as_mut(), &code).await?;
    tracing::info!(code = app::registration_invite::logged_code(&code), admin = %session.user.id.0, "revoked a registration invite");
    Ok(NoContent)
}

/// An API server's last heartbeat.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApiServerHealth {
    pub instance: String,
    pub host: String,
    pub version: String,
    pub started_at: DateTime<Utc>,
    pub reported_at: DateTime<Utc>,
    pub event_streams: i64,
    pub requests_per_minute: f64,
    pub server_errors_per_minute: f64,
    pub resident_bytes: Option<u64>,
    pub db_connections: u32,
    pub db_connections_idle: u32,
}

/// A voice server's standing.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceServerHealth {
    pub id: VoiceServerId,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub capacity: i32,
    pub participants: i32,
    pub last_report_at: Option<DateTime<Utc>>,
    /// Reporting recently enough to be offered to new callers.
    pub reporting: bool,
}

/// The deployment's servers.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Fleet {
    /// Every API server that has written a heartbeat in the last half minute.
    pub api_servers: Vec<ApiServerHealth>,
    /// Every registered voice server.
    pub voice_servers: Vec<VoiceServerHealth>,
}

#[utoipa::path(
    get,
    path = "/admin/fleet",
    tag = TAG_ADMIN,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Fleet),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without the permission this needs", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_fleet(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<Fleet>> {
    access.require(DeploymentPermission::ViewDashboard)?;
    let (api_servers, voice_servers) = tokio::try_join!(
        app::fleet::read_api_servers(&state),
        app::fleet::read_voice_servers(&state),
    )?;
    Ok(Json(Fleet {
        api_servers: api_servers
            .into_iter()
            .map(|h| ApiServerHealth {
                instance: h.instance,
                host: h.host,
                version: h.version,
                started_at: h.started_at,
                reported_at: h.reported_at,
                event_streams: h.event_streams,
                requests_per_minute: h.requests_per_minute,
                server_errors_per_minute: h.server_errors_per_minute,
                resident_bytes: h.resident_bytes,
                db_connections: h.db_connections,
                db_connections_idle: h.db_connections_idle,
            })
            .collect(),
        voice_servers: voice_servers
            .into_iter()
            .map(|v| VoiceServerHealth {
                id: v.id,
                name: v.name,
                url: v.url,
                enabled: v.enabled,
                capacity: v.capacity,
                participants: v.participants,
                last_report_at: v.last_report_at,
                reporting: v.reporting,
            })
            .collect(),
    }))
}

/// What a ban from the deployment asks for.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserBanRequest {
    /// What the banned person is told when they try to sign in; at most
    /// `app::ban::REASON_MAX_CHARS` characters.
    #[serde(default)]
    pub reason: Option<String>,
    /// How long the ban lasts, in seconds, at least a minute; absent or `null` until lifted.
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    /// How far back the person's messages anywhere on the deployment, DMs included, are
    /// deleted with the ban: 3600 (the last hour) or 86400 (the last day); absent or `null` to
    /// leave them. Takes Remove content.
    #[serde(default)]
    pub delete_messages_seconds: Option<u32>,
    /// For a bot, ban its owner too, with the same reason, end, and deletion.
    #[serde(default)]
    pub with_owner: bool,
}

impl From<UserBanRequest> for app::user_ban::UserBanRequest {
    fn from(request: UserBanRequest) -> Self {
        Self {
            ban: app::ban::BanRequest {
                reason: request.reason,
                duration_seconds: request.duration_seconds,
                delete_messages_seconds: request.delete_messages_seconds,
            },
            with_owner: request.with_owner,
        }
    }
}

/// A ban from the deployment as made, with what it did.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserBanOutcome {
    /// Who was banned: the person named, and a bot's owner when asked.
    pub banned: Vec<UserId>,
    /// How many of their messages the ban deleted.
    pub deleted_messages: u32,
}

impl From<app::user_ban::UserBanned> for UserBanOutcome {
    fn from(outcome: app::user_ban::UserBanned) -> Self {
        Self {
            banned: outcome.banned,
            deleted_messages: u32::try_from(outcome.deleted_messages).unwrap_or(u32::MAX),
        }
    }
}

/// Bans someone from the deployment: every sign-in of theirs ends, their event streams close,
/// they leave every call, and signing in is refused with `deploymentBanned` and the reason until
/// the ban ends or is lifted. A bot's token is refused while it stands. Asked to, their messages
/// from the last hour or day are deleted everywhere. Takes Ban users, over someone whose highest
/// deployment role is below the caller's, never the caller or the system account; deleting
/// messages takes Remove content too. A ban standing already is replaced (`200`).
/// Written to the moderation log.
#[utoipa::path(
    put,
    path = "/admin/users/{user}/ban",
    tag = TAG_ADMIN,
    params(("user" = UserId, Path)),
    request_body = UserBanRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Banned", body = UserBanOutcome),
        (status = OK, description = "A ban that stood was replaced", body = UserBanOutcome),
        (status = BAD_REQUEST, description = "`validation`: the reason, duration, or deletion window, banning oneself or the system account, or `withOwner` for someone who is not a bot", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Ban users (and Remove content to delete messages) or over someone of the caller's rank or above", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn ban_user(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(user): Path<UserId>,
    Json(request): Json<UserBanRequest>,
) -> ApiResult<(axum::http::StatusCode, Json<UserBanOutcome>)> {
    let outcome = app::user_ban::ban_user(&state, &access, user, &request.into()).await?;
    let status = if outcome.replaced {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::CREATED
    };
    Ok((status, Json(outcome.into())))
}

/// Lifts a ban from the deployment. Nothing standing is not an error. Takes Ban users and
/// ranking above the person banned, as banning does; written to the moderation log.
#[utoipa::path(
    delete,
    path = "/admin/users/{user}/ban",
    tag = TAG_ADMIN,
    params(("user" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Not banned"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Ban users or when the person banned does not rank below the caller", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn lift_ban(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(user): Path<UserId>,
) -> ApiResult<NoContent> {
    app::user_ban::lift_ban(&state, &access, user).await?;
    Ok(NoContent)
}
