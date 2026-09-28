//! What people may do across the whole deployment, rather than in one community: open the
//! Administration Dashboard, manage registration invites, voice servers, and the deployment's
//! own roles, and moderate any community.
//!
//! Deployment roles are ranked by `position`, like a community's. A holder of Manage deployment
//! roles may create, edit, reorder, delete, give, and take away only roles below their own
//! highest, and give a role only permissions they hold. Nothing ranks above the top role but the
//! terminal (`aspen-chat-server admin`), which is how a deployment gets its first
//! administrator and how the top role itself changes hands.
//!
//! Moderate any community is the deployment's power over what its users post: it reads every
//! community, channel, and DM, and may delete messages, attachments, and reactions, remove
//! members (never an owner), rename and delete channels and communities. Each use that the
//! community's own permissions would not have allowed, and every reading of a DM by someone not
//! in it, is written to the moderation log (`moderation_log`). A change to what someone may do
//! is published to them as `deploymentAccessChanged`, which their event stream follows.

use crate::api::GlobalServerContext;
use crate::api::message_enum::server_event::ServerEvent;
use crate::app::{
    self, ChannelId, CommunityId, DeploymentRoleId, EventScope, UserId, publish_event,
};
use crate::database::schema::{deployment_role, moderation_log, user, user_deployment_role};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use rust_i18n::t;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ops::BitOr;
use utoipa::ToSchema;

/// A set of deployment permissions, as the bits the database stores. The values are fixed:
/// migrations write them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DeploymentPermissions(pub i64);

impl DeploymentPermissions {
    pub const NONE: Self = Self(0);
    pub const VIEW_DASHBOARD: Self = Self(1 << 0);
    pub const MANAGE_REGISTRATION_INVITES: Self = Self(1 << 1);
    pub const MANAGE_VOICE_SERVERS: Self = Self(1 << 2);
    pub const MANAGE_DEPLOYMENT_ROLES: Self = Self(1 << 3);
    pub const MODERATE_COMMUNITIES: Self = Self(1 << 4);
    pub const ALL: Self = Self((1 << 5) - 1);
    /// What the terminal's `admin grant` gives: everything but moderation, which is given
    /// deliberately.
    pub const ADMINISTRATOR: Self = Self(Self::ALL.0 & !Self::MODERATE_COMMUNITIES.0);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn valid(self) -> Self {
        Self(self.0 & Self::ALL.0)
    }
}

impl BitOr for DeploymentPermissions {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// Something a deployment role may allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DeploymentPermission {
    ViewDashboard,
    ManageRegistrationInvites,
    ManageVoiceServers,
    ManageDeploymentRoles,
    ModerateCommunities,
}

impl DeploymentPermission {
    pub const ALL: [DeploymentPermission; 5] = [
        DeploymentPermission::ViewDashboard,
        DeploymentPermission::ManageRegistrationInvites,
        DeploymentPermission::ManageVoiceServers,
        DeploymentPermission::ManageDeploymentRoles,
        DeploymentPermission::ModerateCommunities,
    ];

    pub fn bits(self) -> DeploymentPermissions {
        match self {
            Self::ViewDashboard => DeploymentPermissions::VIEW_DASHBOARD,
            Self::ManageRegistrationInvites => DeploymentPermissions::MANAGE_REGISTRATION_INVITES,
            Self::ManageVoiceServers => DeploymentPermissions::MANAGE_VOICE_SERVERS,
            Self::ManageDeploymentRoles => DeploymentPermissions::MANAGE_DEPLOYMENT_ROLES,
            Self::ModerateCommunities => DeploymentPermissions::MODERATE_COMMUNITIES,
        }
    }

    fn describe(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::ViewDashboard => t!("deploymentViewDashboard"),
            Self::ManageRegistrationInvites => t!("deploymentManageRegistrationInvites"),
            Self::ManageVoiceServers => t!("deploymentManageVoiceServers"),
            Self::ManageDeploymentRoles => t!("deploymentManageDeploymentRoles"),
            Self::ModerateCommunities => t!("deploymentModerateCommunities"),
        }
    }
}

pub fn to_names(permissions: DeploymentPermissions) -> Vec<DeploymentPermission> {
    DeploymentPermission::ALL
        .into_iter()
        .filter(|p| permissions.contains(p.bits()))
        .collect()
}

pub fn from_names(names: &[DeploymentPermission]) -> DeploymentPermissions {
    names
        .iter()
        .fold(DeploymentPermissions::NONE, |set, p| set | p.bits())
}

/// What someone may do across the deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentAccess {
    pub user: UserId,
    /// The positions of the roles they hold.
    pub positions: Vec<i32>,
    pub permissions: DeploymentPermissions,
}

impl DeploymentAccess {
    pub fn has(&self, permission: DeploymentPermission) -> bool {
        self.permissions.contains(permission.bits())
    }

    pub fn require(&self, permission: DeploymentPermission) -> app::Result<()> {
        if self.has(permission) {
            Ok(())
        } else {
            Err(app::Error::Forbidden(t!(
                "permissionMissing",
                permission = permission.describe()
            )))
        }
    }

    /// Their highest role's position, 0 with none.
    pub fn rank(&self) -> i32 {
        self.positions.iter().copied().max().unwrap_or(0)
    }

    pub fn require_above(&self, position: i32) -> app::Result<()> {
        if position < self.rank() {
            Ok(())
        } else {
            Err(app::Error::Forbidden(t!("permissionRank")))
        }
    }

    pub fn require_holds(&self, permissions: DeploymentPermissions) -> app::Result<()> {
        if self.permissions.contains(permissions) {
            Ok(())
        } else {
            Err(app::Error::Forbidden(t!("permissionNotHeld")))
        }
    }
}

/// What `user` may do across the deployment.
pub async fn deployment_access(
    conn: &mut AsyncPgConnection,
    user: UserId,
) -> app::Result<DeploymentAccess> {
    let rows: Vec<(i32, i64)> = user_deployment_role::table
        .inner_join(deployment_role::table)
        .select((deployment_role::position, deployment_role::permissions))
        .filter(user_deployment_role::user.eq(user))
        .load(conn)
        .await?;
    Ok(DeploymentAccess {
        user,
        positions: rows.iter().map(|(position, _)| *position).collect(),
        permissions: rows
            .iter()
            .fold(DeploymentPermissions::NONE, |held, (_, p)| {
                held | DeploymentPermissions(*p)
            })
            .valid(),
    })
}

/// What `user` may do across the deployment, on a connection of its own.
pub async fn access_of(state: &GlobalServerContext, user: UserId) -> app::Result<DeploymentAccess> {
    deployment_access(state.connection_pool.get().await?.as_mut(), user).await
}

/// What `user` may do across the deployment, and the roles that give it, lowest first.
pub async fn access_and_roles(
    state: &GlobalServerContext,
    user: UserId,
) -> app::Result<(DeploymentAccess, Vec<DeploymentRoleId>)> {
    let mut conn = state.connection_pool.get().await?;
    let access = deployment_access(conn.as_mut(), user).await?;
    let roles = roles_of_users(conn.as_mut(), &[user])
        .await?
        .remove(&user)
        .unwrap_or_default();
    Ok((access, roles))
}

/// The deployment roles each of `users` holds, lowest first.
pub async fn roles_of(
    state: &GlobalServerContext,
    users: &[UserId],
) -> app::Result<HashMap<UserId, Vec<DeploymentRoleId>>> {
    roles_of_users(state.connection_pool.get().await?.as_mut(), users).await
}

/// Whether `user` may moderate any community.
pub async fn is_moderator(conn: &mut AsyncPgConnection, user: UserId) -> app::Result<bool> {
    Ok(deployment_access(conn, user)
        .await?
        .has(DeploymentPermission::ModerateCommunities))
}

/// One action for the moderation log.
#[derive(Debug, Clone, Copy)]
pub enum ModerationAction {
    ReadDm,
    DeleteMessage,
    RemoveAttachment,
    RemoveReaction,
    RemoveMember,
    RenameChannel,
    DeleteChannel,
    RenameCommunity,
    DeleteCommunity,
    RemoveWriteIn,
}

impl ModerationAction {
    fn name(self) -> &'static str {
        match self {
            Self::ReadDm => "readDm",
            Self::DeleteMessage => "deleteMessage",
            Self::RemoveAttachment => "removeAttachment",
            Self::RemoveReaction => "removeReaction",
            Self::RemoveMember => "removeMember",
            Self::RenameChannel => "renameChannel",
            Self::DeleteChannel => "deleteChannel",
            Self::RenameCommunity => "renameCommunity",
            Self::DeleteCommunity => "deleteCommunity",
            Self::RemoveWriteIn => "removeWriteIn",
        }
    }
}

/// Writes a use of Moderate any community to the moderation log, and to the server's own log.
/// `subject` names what was acted on beyond the community and channel: a message, a user.
pub async fn log_moderation(
    conn: &mut AsyncPgConnection,
    actor: UserId,
    action: ModerationAction,
    community: Option<CommunityId>,
    channel: Option<ChannelId>,
    subject: Option<String>,
) -> app::Result<()> {
    tracing::info!(
        actor = %actor.0,
        action = action.name(),
        community = ?community.map(|c| c.0),
        channel = ?channel.map(|c| c.0),
        subject = ?subject,
        "deployment moderation"
    );
    diesel::insert_into(moderation_log::table)
        .values((
            moderation_log::id.eq(uuid::Uuid::now_v7()),
            moderation_log::actor.eq(Some(actor)),
            moderation_log::action.eq(action.name()),
            moderation_log::community.eq(community),
            moderation_log::channel.eq(channel),
            moderation_log::subject.eq(subject),
        ))
        .execute(conn)
        .await?;
    Ok(())
}

/// One entry of the moderation log.
#[derive(Debug, Clone, Queryable)]
pub struct ModerationEntry {
    pub id: uuid::Uuid,
    pub actor: Option<UserId>,
    pub action: String,
    pub community: Option<CommunityId>,
    pub channel: Option<ChannelId>,
    pub subject: Option<String>,
    pub at: chrono::DateTime<chrono::Utc>,
}

/// The newest entries of the moderation log, before `before` when given.
pub async fn read_moderation_log(
    state: &GlobalServerContext,
    before: Option<uuid::Uuid>,
    limit: i64,
) -> app::Result<Vec<ModerationEntry>> {
    let mut conn = state.connection_pool.get().await?;
    let mut query = moderation_log::table
        .select((
            moderation_log::id,
            moderation_log::actor,
            moderation_log::action,
            moderation_log::community,
            moderation_log::channel,
            moderation_log::subject,
            moderation_log::at,
        ))
        .order(moderation_log::id.desc())
        .limit(limit)
        .into_boxed();
    if let Some(before) = before {
        query = query.filter(moderation_log::id.lt(before));
    }
    Ok(query.load(conn.as_mut()).await?)
}

// ---------------------------------------------------------------------------------------------
// Roles

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = deployment_role)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DeploymentRoleRow {
    pub id: DeploymentRoleId,
    pub name: String,
    pub position: i32,
    pub permissions: i64,
}

/// The longest a deployment role's name may be, in characters.
pub const MAX_ROLE_NAME_CHARS: usize = 64;

async fn load_roles(conn: &mut AsyncPgConnection) -> app::Result<Vec<DeploymentRoleRow>> {
    Ok(deployment_role::table
        .select(DeploymentRoleRow::as_select())
        .order((deployment_role::position, deployment_role::id))
        .load(conn)
        .await?)
}

/// The deployment's roles, lowest first.
pub async fn read_roles(state: &GlobalServerContext) -> app::Result<Vec<DeploymentRoleRow>> {
    load_roles(state.connection_pool.get().await?.as_mut()).await
}

/// The deployment roles each of `users` holds.
pub async fn roles_of_users(
    conn: &mut AsyncPgConnection,
    users: &[UserId],
) -> app::Result<HashMap<UserId, Vec<DeploymentRoleId>>> {
    let rows: Vec<(UserId, DeploymentRoleId)> = user_deployment_role::table
        .inner_join(deployment_role::table)
        .select((user_deployment_role::user, user_deployment_role::role))
        .filter(user_deployment_role::user.eq_any(users.to_vec()))
        .order(deployment_role::position)
        .load(conn)
        .await?;
    let mut held: HashMap<UserId, Vec<DeploymentRoleId>> = HashMap::new();
    for (user, role) in rows {
        held.entry(user).or_default().push(role);
    }
    Ok(held)
}

fn validate_name(name: &str) -> app::Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_ROLE_NAME_CHARS {
        return Err(app::Error::Validation(t!(
            "roleNameLength",
            max = MAX_ROLE_NAME_CHARS
        )));
    }
    Ok(name.to_string())
}

/// Tells each holder of `role` what they may now do.
async fn announce_holders(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    role: DeploymentRoleId,
) -> app::Result<()> {
    let holders: Vec<UserId> = user_deployment_role::table
        .select(user_deployment_role::user)
        .filter(user_deployment_role::role.eq(role))
        .load(conn)
        .await?;
    for holder in holders {
        announce(state, conn, holder).await?;
    }
    Ok(())
}

/// Tells `user` what they may now do across the deployment.
async fn announce(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user: UserId,
) -> app::Result<()> {
    let access = deployment_access(conn, user).await?;
    publish_event(
        state,
        conn,
        EventScope::User(user),
        &ServerEvent::DeploymentAccessChanged {
            permissions: to_names(access.permissions),
        },
    )
    .await
}

/// Gives the roles dense positions from 1 in the order listed.
async fn renumber(conn: &mut AsyncPgConnection, order: &[DeploymentRoleId]) -> app::Result<()> {
    for (index, id) in order.iter().enumerate() {
        let position = i32::try_from(index + 1).unwrap_or(i32::MAX);
        diesel::update(deployment_role::table.filter(deployment_role::id.eq(*id)))
            .set(deployment_role::position.eq(position))
            .execute(conn)
            .await?;
    }
    Ok(())
}

/// Makes a role at the bottom, with permissions the caller holds.
pub async fn create_role(
    state: &GlobalServerContext,
    caller: UserId,
    name: &str,
    permissions: DeploymentPermissions,
) -> app::Result<DeploymentRoleRow> {
    let name = validate_name(name)?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = deployment_access(conn.as_mut(), caller).await?;
            access.require(DeploymentPermission::ManageDeploymentRoles)?;
            let permissions = permissions.valid();
            access.require_holds(permissions)?;
            let mut order: Vec<DeploymentRoleId> = load_roles(conn.as_mut())
                .await?
                .into_iter()
                .map(|r| r.id)
                .collect();
            let row = DeploymentRoleRow {
                id: DeploymentRoleId::new(),
                name,
                position: 1,
                permissions: permissions.0,
            };
            diesel::insert_into(deployment_role::table)
                .values(&row)
                .execute(conn.as_mut())
                .await?;
            order.insert(0, row.id);
            renumber(conn.as_mut(), &order).await?;
            Ok(row)
        }
        .scope_boxed()
    })
    .await
}

/// Renames a role below the caller's highest, or changes its permissions; what is given or
/// taken must be the caller's.
pub async fn update_role(
    state: &GlobalServerContext,
    caller: UserId,
    role_id: DeploymentRoleId,
    name: Option<&str>,
    permissions: Option<DeploymentPermissions>,
) -> app::Result<DeploymentRoleRow> {
    let name = name.map(validate_name).transpose()?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = deployment_access(conn.as_mut(), caller).await?;
            access.require(DeploymentPermission::ManageDeploymentRoles)?;
            let mut role: DeploymentRoleRow = deployment_role::table
                .select(DeploymentRoleRow::as_select())
                .filter(deployment_role::id.eq(role_id))
                .first(conn.as_mut())
                .await?;
            access.require_above(role.position)?;
            if let Some(permissions) = permissions.map(DeploymentPermissions::valid) {
                access.require_holds(DeploymentPermissions(permissions.0 ^ role.permissions))?;
                role.permissions = permissions.0;
            }
            if let Some(name) = name {
                role.name = name;
            }
            diesel::update(deployment_role::table.filter(deployment_role::id.eq(role_id)))
                .set((
                    deployment_role::name.eq(&role.name),
                    deployment_role::permissions.eq(role.permissions),
                ))
                .execute(conn.as_mut())
                .await?;
            if permissions.is_some() {
                announce_holders(state, conn.as_mut(), role_id).await?;
            }
            Ok(role)
        }
        .scope_boxed()
    })
    .await
}

/// Deletes a role below the caller's highest.
pub async fn delete_role(
    state: &GlobalServerContext,
    caller: UserId,
    role_id: DeploymentRoleId,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = deployment_access(conn.as_mut(), caller).await?;
            access.require(DeploymentPermission::ManageDeploymentRoles)?;
            let position: i32 = deployment_role::table
                .select(deployment_role::position)
                .filter(deployment_role::id.eq(role_id))
                .first(conn.as_mut())
                .await?;
            access.require_above(position)?;
            let holders: Vec<UserId> = user_deployment_role::table
                .select(user_deployment_role::user)
                .filter(user_deployment_role::role.eq(role_id))
                .load(conn.as_mut())
                .await?;
            diesel::delete(deployment_role::table.filter(deployment_role::id.eq(role_id)))
                .execute(conn.as_mut())
                .await?;
            let order: Vec<DeploymentRoleId> = load_roles(conn.as_mut())
                .await?
                .into_iter()
                .map(|r| r.id)
                .collect();
            renumber(conn.as_mut(), &order).await?;
            for holder in holders {
                announce(state, conn.as_mut(), holder).await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Reorders the roles below the caller's highest; `order` lists exactly those, lowest first.
pub async fn reorder_roles(
    state: &GlobalServerContext,
    caller: UserId,
    order: &[DeploymentRoleId],
) -> app::Result<Vec<DeploymentRoleRow>> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = deployment_access(conn.as_mut(), caller).await?;
            access.require(DeploymentPermission::ManageDeploymentRoles)?;
            let roles = load_roles(conn.as_mut()).await?;
            let rank = access.rank();
            let (movable, fixed): (Vec<_>, Vec<_>) =
                roles.into_iter().partition(|r| r.position < rank);
            let mut wanted = order.to_vec();
            wanted.sort_by_key(|id| id.0);
            let mut have: Vec<DeploymentRoleId> = movable.iter().map(|r| r.id).collect();
            have.sort_by_key(|id| id.0);
            if wanted != have {
                return Err(app::Error::Validation(t!("roleOrderMismatch")));
            }
            let sequence: Vec<DeploymentRoleId> = order
                .iter()
                .copied()
                .chain(fixed.iter().map(|r| r.id))
                .collect();
            renumber(conn.as_mut(), &sequence).await?;
            load_roles(conn.as_mut()).await
        }
        .scope_boxed()
    })
    .await
}

/// Gives `user` a role below the caller's highest, or takes it away. Returns whether anything
/// changed.
pub async fn set_user_role(
    state: &GlobalServerContext,
    caller: UserId,
    target: UserId,
    role_id: DeploymentRoleId,
    held: bool,
) -> app::Result<bool> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = deployment_access(conn.as_mut(), caller).await?;
            access.require(DeploymentPermission::ManageDeploymentRoles)?;
            let position: i32 = deployment_role::table
                .select(deployment_role::position)
                .filter(deployment_role::id.eq(role_id))
                .first(conn.as_mut())
                .await?;
            access.require_above(position)?;
            if target != caller {
                let theirs = deployment_access(conn.as_mut(), target).await?;
                access.require_above(theirs.rank())?;
            }
            let exists: bool = diesel::select(diesel::dsl::exists(
                user::table.filter(user::id.eq(target).and(user::deleted_at.is_null())),
            ))
            .get_result(conn.as_mut())
            .await?;
            if !exists {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            let changed = if held {
                diesel::insert_into(user_deployment_role::table)
                    .values((
                        user_deployment_role::user.eq(target),
                        user_deployment_role::role.eq(role_id),
                    ))
                    .on_conflict_do_nothing()
                    .execute(conn.as_mut())
                    .await?
            } else {
                diesel::delete(
                    user_deployment_role::table.filter(
                        user_deployment_role::user
                            .eq(target)
                            .and(user_deployment_role::role.eq(role_id)),
                    ),
                )
                .execute(conn.as_mut())
                .await?
            };
            if changed > 0 {
                announce(state, conn.as_mut(), target).await?;
            }
            Ok(changed > 0)
        }
        .scope_boxed()
    })
    .await
}

/// The terminal's grant: gives `user` the top role, making an Administrator role (every
/// permission but moderation) first when the deployment has none. Publishes nothing: the
/// terminal has no event stream, and the user's clients learn at their next read.
pub async fn grant_top_role(conn: &mut AsyncPgConnection, target: UserId) -> app::Result<String> {
    conn.transaction(|conn| {
        async move {
            let roles = load_roles(conn).await?;
            let top = match roles.last() {
                Some(top) => top.clone(),
                None => {
                    let row = DeploymentRoleRow {
                        id: DeploymentRoleId::new(),
                        name: t!("deploymentAdministratorRole").to_string(),
                        position: 1,
                        permissions: DeploymentPermissions::ADMINISTRATOR.0,
                    };
                    diesel::insert_into(deployment_role::table)
                        .values(&row)
                        .execute(conn)
                        .await?;
                    row
                }
            };
            diesel::insert_into(user_deployment_role::table)
                .values((
                    user_deployment_role::user.eq(target),
                    user_deployment_role::role.eq(top.id),
                ))
                .on_conflict_do_nothing()
                .execute(conn)
                .await?;
            Ok(top.name)
        }
        .scope_boxed()
    })
    .await
}

/// The terminal's change to the top role: allows `permission` to it, or denies it. Returns the
/// role's name. Refused when the deployment has no roles yet (`admin grant` makes the first).
pub async fn set_top_role_permission(
    conn: &mut AsyncPgConnection,
    permission: DeploymentPermission,
    allow: bool,
) -> app::Result<String> {
    let roles = load_roles(conn).await?;
    let Some(top) = roles.last() else {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    };
    let permissions = if allow {
        top.permissions | permission.bits().0
    } else {
        top.permissions & !permission.bits().0
    };
    diesel::update(deployment_role::table.filter(deployment_role::id.eq(top.id)))
        .set(deployment_role::permissions.eq(permissions))
        .execute(conn)
        .await?;
    Ok(top.name.clone())
}

/// The terminal's revoke: takes every deployment role from `user`.
pub async fn revoke_all(conn: &mut AsyncPgConnection, target: UserId) -> app::Result<usize> {
    Ok(
        diesel::delete(user_deployment_role::table.filter(user_deployment_role::user.eq(target)))
            .execute(conn)
            .await?,
    )
}

/// Everyone holding a deployment role, with the names of their roles, for the terminal.
pub async fn holders(conn: &mut AsyncPgConnection) -> app::Result<Vec<(String, String)>> {
    Ok(user_deployment_role::table
        .inner_join(user::table)
        .inner_join(deployment_role::table)
        .select((user::name, deployment_role::name))
        .filter(user::deleted_at.is_null())
        .order((user::name, deployment_role::position.desc()))
        .load(conn)
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_deployment_permission_has_one_name_and_back() {
        assert_eq!(
            from_names(&DeploymentPermission::ALL),
            DeploymentPermissions::ALL
        );
        assert_eq!(
            to_names(DeploymentPermissions::ALL),
            DeploymentPermission::ALL.to_vec()
        );
        // The number the migration gives existing administrators.
        assert_eq!(DeploymentPermissions::ADMINISTRATOR.0, 15);
    }

    #[test]
    fn rank_is_the_highest_role() {
        let access = DeploymentAccess {
            user: UserId::new(),
            positions: vec![1, 3],
            permissions: DeploymentPermissions::ADMINISTRATOR,
        };
        assert!(access.require_above(2).is_ok());
        assert!(access.require_above(3).is_err());
        assert!(
            access
                .require_holds(DeploymentPermissions::MODERATE_COMMUNITIES)
                .is_err()
        );
    }
}
