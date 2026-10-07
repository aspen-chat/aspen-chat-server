//! Roles and permissions in a community (`app::permissions`, `app::role`): the roles, which
//! member holds which, the per-channel and per-category overrides, removing members, and
//! handing a community to a new owner.

use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Created, Json, NoContent, Path};
use crate::message_enum::request::{
    CategoryOverrideCreateRequest, ChannelOverrideCreateRequest, RoleCreateRequest,
    RoleUpdateRequest,
};
use crate::message_enum::{self, CategoryOverride, ChannelOverride, Role, UserCommunity};
use crate::{API_PREFIX, TAG_ROLES};
use aspen_app as app;
use aspen_app::context::GlobalServerContext;
use aspen_app::permissions::from_names;
use aspen_app::role::OverrideTarget;
use aspen_app::{CategoryId, ChannelId, CommunityId, RoleId, UserId};
use axum::extract::State;
use axum::http::StatusCode;
use serde::Deserialize;
use utoipa::ToSchema;

/// A community's roles, lowest first; everyone's is first, at position 0.
#[utoipa::path(
    get,
    path = "/communities/{community}/roles",
    tag = TAG_ROLES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<Role>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such community, or the caller is not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_roles(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
) -> ApiResult<Json<Vec<Role>>> {
    Ok(Json(
        app::role::read_roles(&state, user.id, community).await?,
    ))
}

/// Makes a role, placed just above everyone's. Takes Manage roles, and only permissions the
/// caller holds may be given.
#[utoipa::path(
    post,
    path = "/communities/{community}/roles",
    tag = TAG_ROLES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = Role, headers(("Location" = String, description = "URL of the new role"))),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: lacks Manage roles, or a permission given", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_role(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Json(request): Json<RoleCreateRequest>,
) -> ApiResult<Created<Role>> {
    let role = app::role::create_role(&state, user.id, community, &request).await?;
    Ok(Created::new(
        format!("{API_PREFIX}/roles/{}", role.id.0),
        role,
    ))
}

/// Renames a role, or changes its permissions, its hue, or whether it is shown apart. The role
/// must rank below the caller's highest, and every permission given or taken must be one the
/// caller holds. Everyone's role keeps its name and has no hue and is never shown apart.
#[utoipa::path(
    patch,
    path = "/roles/{role}",
    tag = TAG_ROLES,
    params(("role" = RoleId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Role),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_role(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(role): Path<RoleId>,
    Json(request): Json<RoleUpdateRequest>,
) -> ApiResult<Json<Role>> {
    Ok(Json(
        app::role::update_role(&state, user.id, role, &request).await?,
    ))
}

/// Deletes a role ranked below the caller's highest. Everyone's cannot be deleted.
#[utoipa::path(
    delete,
    path = "/roles/{role}",
    tag = TAG_ROLES,
    params(("role" = RoleId, Path)),
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
pub async fn delete_role(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(role): Path<RoleId>,
) -> ApiResult<NoContent> {
    app::role::delete_role(&state, user.id, role).await?;
    Ok(NoContent)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoleOrderRequest {
    /// Every role ranked below the caller's highest, everyone's excepted, lowest first.
    pub roles: Vec<RoleId>,
}

/// Reorders the roles ranked below the caller's highest, which takes Manage roles. Returns
/// every role of the community, lowest first.
#[utoipa::path(
    put,
    path = "/communities/{community}/role-order",
    tag = TAG_ROLES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<Role>),
        (status = BAD_REQUEST, description = "`validation`: the list is not exactly the roles below the caller", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn reorder_roles(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Json(request): Json<RoleOrderRequest>,
) -> ApiResult<Json<Vec<Role>>> {
    Ok(Json(
        app::role::reorder_roles(&state, user.id, community, &request.roles).await?,
    ))
}

/// Gives a member a role. Takes Assign roles and every permission the role allows; the role, and
/// the member unless it is the caller, must rank below the caller's highest. Returns the membership, `201` when the role was new to
/// them and `200` when they held it already.
#[utoipa::path(
    put,
    path = "/communities/{community}/members/{user}/roles/{role}",
    tag = TAG_ROLES,
    params(
        ("community" = CommunityId, Path),
        ("user" = UserId, Path),
        ("role" = RoleId, Path),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Given", body = UserCommunity),
        (status = OK, description = "Already held", body = UserCommunity),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn add_member_role(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((community, member, role)): Path<(CommunityId, UserId, RoleId)>,
) -> ApiResult<(StatusCode, Json<UserCommunity>)> {
    let added = app::role::set_member_role(&state, user.id, community, member, role, true).await?;
    let membership = app::community::read_membership(&state, member, community).await?;
    let status = if added {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(membership)))
}

/// Takes a role from a member, on the same terms as giving it but for its permissions, which the
/// caller need not hold.
#[utoipa::path(
    delete,
    path = "/communities/{community}/members/{user}/roles/{role}",
    tag = TAG_ROLES,
    params(
        ("community" = CommunityId, Path),
        ("user" = UserId, Path),
        ("role" = RoleId, Path),
    ),
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
pub async fn remove_member_role(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((community, member, role)): Path<(CommunityId, UserId, RoleId)>,
) -> ApiResult<NoContent> {
    app::role::set_member_role(&state, user.id, community, member, role, false).await?;
    Ok(NoContent)
}

/// Removes someone from the community. Takes Remove members, and they must rank below the
/// caller's highest role; the owner cannot be removed. They may join again with an invite.
#[utoipa::path(
    delete,
    path = "/communities/{community}/members/{user}",
    tag = TAG_ROLES,
    params(("community" = CommunityId, Path), ("user" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, description = "No such community, or either person is not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_member(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((community, member)): Path<(CommunityId, UserId)>,
) -> ApiResult<NoContent> {
    app::role::remove_member(&state, user.id, community, member).await?;
    Ok(NoContent)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OwnerRequest {
    /// The member who becomes the owner.
    pub user: UserId,
}

/// Hands the community to another member. Only its owner may; they stay a member, holding
/// their roles.
#[utoipa::path(
    put,
    path = "/communities/{community}/owner",
    tag = TAG_ROLES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = message_enum::Community),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn transfer_ownership(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Json(request): Json<OwnerRequest>,
) -> ApiResult<Json<message_enum::Community>> {
    app::role::transfer_ownership(&state, user.id, community, request.user).await?;
    let c = app::community::read_community(&state, user.id, community).await?;
    Ok(Json(message_enum::Community::from(c)))
}

/// Sets a role's override in a channel, which takes Manage channels and viewing the channel
/// (`404` for one the caller may not view): channel permissions it allows or denies there over
/// what the role grants across the community. The role must rank
/// below the caller's highest, and every permission named must be one the caller holds.
/// Threads follow their parent channel and take no overrides of their own.
#[utoipa::path(
    put,
    path = "/channels/{channel}/overrides/{role}",
    tag = TAG_ROLES,
    params(("channel" = ChannelId, Path), ("role" = RoleId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Set where there was none", body = ChannelOverride),
        (status = OK, description = "Replaced", body = ChannelOverride),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn set_channel_override(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((channel, role)): Path<(ChannelId, RoleId)>,
    Json(request): Json<ChannelOverrideCreateRequest>,
) -> ApiResult<(StatusCode, Json<ChannelOverride>)> {
    let outcome = app::role::set_override(
        &state,
        user.id,
        OverrideTarget::Channel(channel),
        role,
        Some((from_names(&request.allow), from_names(&request.deny))),
    )
    .await?;
    Ok((
        created_or_ok(outcome.changed_presence),
        Json(ChannelOverride {
            channel,
            role,
            allow: app::permissions::to_names(outcome.allow),
            deny: app::permissions::to_names(outcome.deny),
        }),
    ))
}

/// Clears a role's override in a channel, on the same terms as setting it.
#[utoipa::path(
    delete,
    path = "/channels/{channel}/overrides/{role}",
    tag = TAG_ROLES,
    params(("channel" = ChannelId, Path), ("role" = RoleId, Path)),
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
pub async fn clear_channel_override(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((channel, role)): Path<(ChannelId, RoleId)>,
) -> ApiResult<NoContent> {
    app::role::set_override(
        &state,
        user.id,
        OverrideTarget::Channel(channel),
        role,
        None,
    )
    .await?;
    Ok(NoContent)
}

/// Sets a role's override for every channel of a category, which takes Manage categories and
/// viewing what the category's overrides let the caller view (`403` naming View channel
/// otherwise). A channel's own override for the role applies after its category's.
#[utoipa::path(
    put,
    path = "/categories/{category}/overrides/{role}",
    tag = TAG_ROLES,
    params(("category" = CategoryId, Path), ("role" = RoleId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Set where there was none", body = CategoryOverride),
        (status = OK, description = "Replaced", body = CategoryOverride),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn set_category_override(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((category, role)): Path<(CategoryId, RoleId)>,
    Json(request): Json<CategoryOverrideCreateRequest>,
) -> ApiResult<(StatusCode, Json<CategoryOverride>)> {
    let outcome = app::role::set_override(
        &state,
        user.id,
        OverrideTarget::Category(category),
        role,
        Some((from_names(&request.allow), from_names(&request.deny))),
    )
    .await?;
    Ok((
        created_or_ok(outcome.changed_presence),
        Json(CategoryOverride {
            category,
            role,
            allow: app::permissions::to_names(outcome.allow),
            deny: app::permissions::to_names(outcome.deny),
        }),
    ))
}

/// Clears a role's override for a category, on the same terms as setting it.
#[utoipa::path(
    delete,
    path = "/categories/{category}/overrides/{role}",
    tag = TAG_ROLES,
    params(("category" = CategoryId, Path), ("role" = RoleId, Path)),
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
pub async fn clear_category_override(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((category, role)): Path<(CategoryId, RoleId)>,
) -> ApiResult<NoContent> {
    app::role::set_override(
        &state,
        user.id,
        OverrideTarget::Category(category),
        role,
        None,
    )
    .await?;
    Ok(NoContent)
}

fn created_or_ok(created: bool) -> StatusCode {
    if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    }
}
