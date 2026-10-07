//! What people may do across the whole deployment, rather than in one community: open the
//! Administration Dashboard, manage registration invites, voice servers, and the deployment's
//! own roles, federation with other deployments, report categories, settings, and installed
//! plugins' settings, moderate any community, review reports, remove reported content, ban users
//! from the deployment, message anyone, and send the newsletter.
//!
//! Deployment roles are ranked by `position`, like a community's. A holder of Manage deployment
//! roles may create, edit, reorder, delete, give, and take away only roles below their own
//! highest, and give a role only permissions they hold. Nothing ranks above the top role but the
//! terminal (`aspen-chat-server admin`), which is how a deployment gets its first
//! administrator and how the top role itself changes hands.
//!
//! Each permission does something whole on its own, and each step of a task takes the
//! permission for what that step does. Where one permission is part of another, holding the
//! greater gives the lesser (`DeploymentPermission::includes`), which a role editor shows.
//!
//! Moderate any community is the deployment's power over what its users post: it reads every
//! community, channel, and DM, and may delete messages, attachments, and reactions, remove
//! members (never an owner), rename and delete channels and communities. It includes Remove
//! content, which acts only on what a report case or a ban names. Each use that the community's
//! own permissions would not have allowed, and every reading of a DM by someone not in it, is
//! written to the moderation log (`moderation_log`). A change to what someone may do is
//! published to them as `deploymentAccessChanged`, which their event stream follows.

use crate::app::context::GlobalServerContext;
use crate::app::deployment_role::roles_of_users;
use crate::app::{self, DeploymentRoleId, UserId};
use crate::t;
use aspen_schema::{deployment_role, user_deployment_role};
pub use aspen_wire::deployment::{
    DeploymentPermission, DeploymentPermissions, from_names, to_names,
};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use std::collections::HashMap;

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

    /// Whether they hold any of `permissions`.
    pub fn has_any(&self, permissions: DeploymentPermissions) -> bool {
        self.permissions.intersects(permissions)
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
            .valid()
            .with_included(),
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
        // What `admin grant` gives: View dashboard, Manage registration invites, voice servers,
        // deployment roles, and federation, Manage report categories, Manage deployment
        // settings, Manage plugins, and Send newsletters, as the migrations give existing
        // administrators.
        assert_eq!(
            DeploymentPermissions::ADMINISTRATOR.bits(),
            1 | 2 | 4 | 8 | 64 | 256 | 2048 | 4096 | 16384
        );
    }

    /// `with_included` takes one step, which holds while no included permission includes more.
    #[test]
    fn inclusion_is_one_step() {
        for permission in DeploymentPermission::ALL {
            for included in permission.includes() {
                assert!(
                    included.includes().is_empty(),
                    "{permission} includes {included}"
                );
            }
        }
        assert!(
            DeploymentPermissions::MODERATE_COMMUNITIES
                .with_included()
                .contains(DeploymentPermissions::REMOVE_CONTENT)
        );
        assert_eq!(
            DeploymentPermissions::REMOVE_CONTENT.with_included(),
            DeploymentPermissions::REMOVE_CONTENT
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
