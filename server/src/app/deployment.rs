//! What people may do across the whole deployment, rather than in one community: open the
//! Administration Dashboard, manage registration invites, voice servers, and the deployment's
//! own roles, bots, federation with other deployments, report categories, settings, and
//! installed plugins' settings, moderate any community, review reports, ban users from the
//! deployment, and message anyone.
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

use crate::app::context::GlobalServerContext;
use crate::app::deployment_role::roles_of_users;
use crate::app::{self, DeploymentRoleId, UserId};
use crate::database::schema::{deployment_role, user_deployment_role};
use crate::t;
use diesel::prelude::*;
use diesel::{AsExpression, FromSqlRow};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

bitflags::bitflags! {
    /// A set of deployment permissions, as the bits the database stores. The values are fixed:
    /// migrations write them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, FromSqlRow, AsExpression)]
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub struct DeploymentPermissions: i64 {
        const VIEW_DASHBOARD = 1 << 0;
        const MANAGE_REGISTRATION_INVITES = 1 << 1;
        const MANAGE_VOICE_SERVERS = 1 << 2;
        const MANAGE_DEPLOYMENT_ROLES = 1 << 3;
        const MODERATE_COMMUNITIES = 1 << 4;
        const MANAGE_BOTS = 1 << 5;
        const MANAGE_FEDERATION = 1 << 6;
        const REVIEW_REPORTS = 1 << 7;
        const MANAGE_REPORT_CATEGORIES = 1 << 8;
        const BAN_USERS = 1 << 9;
        const MESSAGE_ANY_USER = 1 << 10;
        const MANAGE_DEPLOYMENT_SETTINGS = 1 << 11;
        const MANAGE_PLUGINS = 1 << 12;
    }
}

app::bigint_sql_traits!(DeploymentPermissions);

impl DeploymentPermissions {
    /// The powers over what people post and who may stay: given deliberately, never by `admin
    /// grant` alone.
    pub const MODERATION: Self = Self::MODERATE_COMMUNITIES
        .union(Self::REVIEW_REPORTS)
        .union(Self::BAN_USERS)
        .union(Self::MESSAGE_ANY_USER);

    /// What the terminal's `admin grant` gives: everything but moderation.
    pub const ADMINISTRATOR: Self = Self::all().difference(Self::MODERATION);

    /// Every bit that names a permission, and no other.
    pub fn valid(self) -> Self {
        Self::from_bits_truncate(self.bits())
    }
}

/// Something a deployment role may allow.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    strum::VariantArray,
    strum::IntoStaticStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum DeploymentPermission {
    ViewDashboard,
    ManageRegistrationInvites,
    ManageVoiceServers,
    ManageDeploymentRoles,
    ModerateCommunities,
    ManageBots,
    ManageFederation,
    ReviewReports,
    ManageReportCategories,
    BanUsers,
    MessageAnyUser,
    ManageDeploymentSettings,
    ManagePlugins,
}

impl DeploymentPermission {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn bits(self) -> DeploymentPermissions {
        match self {
            Self::ViewDashboard => DeploymentPermissions::VIEW_DASHBOARD,
            Self::ManageRegistrationInvites => DeploymentPermissions::MANAGE_REGISTRATION_INVITES,
            Self::ManageVoiceServers => DeploymentPermissions::MANAGE_VOICE_SERVERS,
            Self::ManageDeploymentRoles => DeploymentPermissions::MANAGE_DEPLOYMENT_ROLES,
            Self::ModerateCommunities => DeploymentPermissions::MODERATE_COMMUNITIES,
            Self::ManageBots => DeploymentPermissions::MANAGE_BOTS,
            Self::ManageFederation => DeploymentPermissions::MANAGE_FEDERATION,
            Self::ReviewReports => DeploymentPermissions::REVIEW_REPORTS,
            Self::ManageReportCategories => DeploymentPermissions::MANAGE_REPORT_CATEGORIES,
            Self::BanUsers => DeploymentPermissions::BAN_USERS,
            Self::MessageAnyUser => DeploymentPermissions::MESSAGE_ANY_USER,
            Self::ManageDeploymentSettings => DeploymentPermissions::MANAGE_DEPLOYMENT_SETTINGS,
            Self::ManagePlugins => DeploymentPermissions::MANAGE_PLUGINS,
        }
    }

    fn describe(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::ViewDashboard => t!("deploymentViewDashboard"),
            Self::ManageRegistrationInvites => t!("deploymentManageRegistrationInvites"),
            Self::ManageVoiceServers => t!("deploymentManageVoiceServers"),
            Self::ManageDeploymentRoles => t!("deploymentManageDeploymentRoles"),
            Self::ModerateCommunities => t!("deploymentModerateCommunities"),
            Self::ManageBots => t!("deploymentManageBots"),
            Self::ManageFederation => t!("deploymentManageFederation"),
            Self::ReviewReports => t!("deploymentReviewReports"),
            Self::ManageReportCategories => t!("deploymentManageReportCategories"),
            Self::BanUsers => t!("deploymentBanUsers"),
            Self::MessageAnyUser => t!("deploymentMessageAnyUser"),
            Self::ManageDeploymentSettings => t!("deploymentManageDeploymentSettings"),
            Self::ManagePlugins => t!("deploymentManagePlugins"),
        }
    }
}

app::wire_name_traits!(DeploymentPermission);

pub fn to_names(permissions: DeploymentPermissions) -> Vec<DeploymentPermission> {
    DeploymentPermission::ALL
        .iter()
        .copied()
        .filter(|p| permissions.contains(p.bits()))
        .collect()
}

pub fn from_names(names: &[DeploymentPermission]) -> DeploymentPermissions {
    names.iter().map(|p| p.bits()).collect()
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
    mut conn: &AsyncPgConnection,
    user: UserId,
) -> app::Result<DeploymentAccess> {
    let rows: Vec<(i32, DeploymentPermissions)> = user_deployment_role::table
        .inner_join(deployment_role::table)
        .select((deployment_role::position, deployment_role::permissions))
        .filter(user_deployment_role::user.eq(user))
        .load(&mut conn)
        .await?;
    Ok(DeploymentAccess {
        user,
        positions: rows.iter().map(|(position, _)| *position).collect(),
        permissions: rows
            .iter()
            .map(|(_, p)| *p)
            .collect::<DeploymentPermissions>()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The terminal names each permission as the API does.
    #[test]
    fn permission_names_match_the_wire() {
        for permission in <DeploymentPermission as strum::VariantArray>::VARIANTS {
            assert_eq!(
                serde_json::to_value(permission).unwrap(),
                <&'static str>::from(permission)
            );
        }
    }

    #[test]
    fn every_deployment_permission_has_one_name_and_back() {
        assert_eq!(
            from_names(DeploymentPermission::ALL),
            DeploymentPermissions::all()
        );
        assert_eq!(
            to_names(DeploymentPermissions::all()),
            DeploymentPermission::ALL.to_vec()
        );
        // What `admin grant` gives: the first seven but moderation, Manage report categories,
        // Manage deployment settings, and Manage plugins, as the migrations give existing
        // administrators.
        assert_eq!(
            DeploymentPermissions::ADMINISTRATOR.bits(),
            111 | 256 | 2048 | 4096
        );
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
