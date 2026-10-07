//! The deployment's own roles (`deployment_role`, `user_deployment_role`), ranked by
//! `position` like a community's: making, editing, reordering, deleting, giving, and taking them,
//! from the dashboard within the caller's rank, and the terminal's changes to the top role.
//!
//! A role may have a hue, and each user's `name_hue` is the hue of the highest role they hold
//! that has one, which clients draw their name in everywhere. Every change that can move it (a
//! hue edited, roles reordered or deleted, a role given or taken) recomputes it for those it
//! touches in the same transaction and announces each change as an update of the user
//! (`refresh_name_hues`).

use crate::api::message_enum::server_event::{ServerEvent, UserEvent};
use crate::app::context::GlobalServerContext;
use crate::app::deployment::{
    DeploymentPermission, DeploymentPermissions, deployment_access, to_names,
};
use crate::app::events::Publishing;
use crate::app::{self, DeploymentRoleId, EventScope, UserId, publish_event};
use crate::t;
use aspen_schema::{deployment_role, user, user_deployment_role};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::collections::HashMap;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = deployment_role)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct DeploymentRoleRow {
    pub id: DeploymentRoleId,
    pub name: String,
    pub position: i32,
    pub permissions: DeploymentPermissions,
    /// The hue its holders' names are drawn in, 0 to 359.
    pub hue: Option<i16>,
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

/// Everyone holding `role`.
async fn holders_of(
    conn: &mut AsyncPgConnection,
    role: DeploymentRoleId,
) -> app::Result<Vec<UserId>> {
    Ok(user_deployment_role::table
        .select(user_deployment_role::user)
        .filter(user_deployment_role::role.eq(role))
        .load(conn)
        .await?)
}

/// Everyone holding any role.
async fn all_holders(conn: &mut AsyncPgConnection) -> app::Result<Vec<UserId>> {
    Ok(user_deployment_role::table
        .select(user_deployment_role::user)
        .distinct()
        .load(conn)
        .await?)
}

/// A user whose `name_hue` [`refresh_name_hues`] changed.
#[derive(QueryableByName)]
struct NameHue {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    id: UserId,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::SmallInt>)]
    name_hue: Option<i16>,
}

/// Sets each of `users`' `name_hue` to the hue of the highest role they hold that has one, and
/// announces it to everyone who sees them for each whose hue changed.
async fn refresh_name_hues(
    publisher: &impl Publishing,
    conn: &mut AsyncPgConnection,
    users: &[UserId],
) -> app::Result<()> {
    if users.is_empty() {
        return Ok(());
    }
    let changed: Vec<NameHue> = diesel::sql_query(
        r#"
        WITH wanted AS (
            SELECT u.id,
                   (SELECT r.hue
                    FROM user_deployment_role ur
                    JOIN deployment_role r ON r.id = ur.role
                    WHERE ur."user" = u.id AND r.hue IS NOT NULL
                    ORDER BY r.position DESC
                    LIMIT 1) AS hue
            FROM "user" u
            WHERE u.id = ANY($1) AND u.deleted_at IS NULL
        )
        UPDATE "user" u SET name_hue = wanted.hue
        FROM wanted
        WHERE u.id = wanted.id AND u.name_hue IS DISTINCT FROM wanted.hue
        RETURNING u.id, u.name_hue
        "#,
    )
    .bind::<diesel::sql_types::Array<diesel::sql_types::Uuid>, _>(
        users.iter().map(|u| u.0).collect::<Vec<_>>(),
    )
    .load(conn)
    .await?;
    for user in changed {
        publish_event(
            publisher,
            conn,
            EventScope::UserEverywhere(user.id),
            &ServerEvent::User(UserEvent::Update {
                id: user.id,
                name: None,
                icon: None,
                display_name: None,
                pronouns: None,
                bio: None,
                status: None,
                bot_owner: None,
                bot_public: None,
                name_hue: Some(user.name_hue),
                public_email: None,
            }),
        )
        .await?;
    }
    Ok(())
}

/// Tells each holder of `role` what they may now do.
async fn announce_holders(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    role: DeploymentRoleId,
) -> app::Result<()> {
    for holder in holders_of(conn, role).await? {
        announce(state, conn, holder).await?;
    }
    Ok(())
}

/// Tells `user` what they may now do across the deployment.
async fn announce(
    state: &impl Publishing,
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
    hue: Option<i16>,
) -> app::Result<DeploymentRoleRow> {
    let name = validate_name(name)?;
    app::role::validate_hue(hue)?;
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
                permissions,
                hue,
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

/// Renames a role below the caller's highest, or changes its permissions or hue (`Some(None)`
/// clears it); what is given or taken must be the caller's.
pub async fn update_role(
    state: &GlobalServerContext,
    caller: UserId,
    role_id: DeploymentRoleId,
    name: Option<&str>,
    permissions: Option<DeploymentPermissions>,
    hue: Option<Option<i16>>,
) -> app::Result<DeploymentRoleRow> {
    let name = name.map(validate_name).transpose()?;
    if let Some(hue) = hue {
        app::role::validate_hue(hue)?;
    }
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
                access.require_holds(permissions.symmetric_difference(role.permissions))?;
                role.permissions = permissions;
            }
            if let Some(name) = name {
                role.name = name;
            }
            if let Some(hue) = hue {
                role.hue = hue;
            }
            diesel::update(deployment_role::table.filter(deployment_role::id.eq(role_id)))
                .set((
                    deployment_role::name.eq(&role.name),
                    deployment_role::permissions.eq(role.permissions),
                    deployment_role::hue.eq(role.hue),
                ))
                .execute(conn.as_mut())
                .await?;
            if permissions.is_some() {
                announce_holders(state, conn.as_mut(), role_id).await?;
            }
            if hue.is_some() {
                let holders = holders_of(conn.as_mut(), role_id).await?;
                refresh_name_hues(state, conn.as_mut(), &holders).await?;
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
            let holders = holders_of(conn.as_mut(), role_id).await?;
            diesel::delete(deployment_role::table.filter(deployment_role::id.eq(role_id)))
                .execute(conn.as_mut())
                .await?;
            let order: Vec<DeploymentRoleId> = load_roles(conn.as_mut())
                .await?
                .into_iter()
                .map(|r| r.id)
                .collect();
            renumber(conn.as_mut(), &order).await?;
            for &holder in &holders {
                announce(state, conn.as_mut(), holder).await?;
            }
            refresh_name_hues(state, conn.as_mut(), &holders).await?;
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
            wanted.sort();
            let mut have: Vec<DeploymentRoleId> = movable.iter().map(|r| r.id).collect();
            have.sort();
            if wanted != have {
                return Err(app::Error::Validation(t!("roleOrderMismatch")));
            }
            let sequence: Vec<DeploymentRoleId> = order
                .iter()
                .copied()
                .chain(fixed.iter().map(|r| r.id))
                .collect();
            renumber(conn.as_mut(), &sequence).await?;
            let holders = all_holders(conn.as_mut()).await?;
            refresh_name_hues(state, conn.as_mut(), &holders).await?;
            load_roles(conn.as_mut()).await
        }
        .scope_boxed()
    })
    .await
}

/// Gives `user` a role below the caller's highest, or takes it away; giving it takes holding every
/// permission it allows. Returns whether anything changed.
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
            let (position, permissions): (i32, DeploymentPermissions) = deployment_role::table
                .select((deployment_role::position, deployment_role::permissions))
                .filter(deployment_role::id.eq(role_id))
                .first(conn.as_mut())
                .await?;
            access.require_above(position)?;
            // Giving a role gives what it allows, which must be the caller's to give, as for
            // making or editing one; taking it away takes rank alone.
            if held {
                access.require_holds(permissions)?;
            }
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
                refresh_name_hues(state, conn.as_mut(), &[target]).await?;
            }
            Ok(changed > 0)
        }
        .scope_boxed()
    })
    .await
}

/// The terminal's grant: gives `user` the top role, making an Administrator role (every
/// permission but moderation) first when the deployment has none, and tells them.
pub async fn grant_top_role(
    publisher: &impl Publishing,
    conn: &mut AsyncPgConnection,
    target: UserId,
) -> app::Result<String> {
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
                        permissions: DeploymentPermissions::ADMINISTRATOR,
                        hue: None,
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
            announce(publisher, conn, target).await?;
            refresh_name_hues(publisher, conn, &[target]).await?;
            Ok(top.name)
        }
        .scope_boxed()
    })
    .await
}

/// The terminal's change to the top role: allows `permission` to it, or denies it, and tells
/// its holders. Returns the role's name. Refused when the deployment has no roles yet (`admin
/// grant` makes the first), and when denying a permission that one the role allows includes.
pub async fn set_top_role_permission(
    publisher: &impl Publishing,
    conn: &mut AsyncPgConnection,
    permission: DeploymentPermission,
    allow: bool,
) -> app::Result<String> {
    conn.transaction(|conn| {
        async move {
            let roles = load_roles(conn).await?;
            let Some(top) = roles.last() else {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            };
            let permissions = if allow {
                top.permissions | permission.bits()
            } else {
                // Denying what an allowed permission includes would change nothing it does.
                if let Some(including) = DeploymentPermission::ALL.iter().find(|p| {
                    top.permissions.contains(p.bits()) && p.includes().contains(&permission)
                }) {
                    return Err(app::Error::Validation(t!(
                        "deploymentPermissionIncluded",
                        permission = permission.describe(),
                        including = including.describe()
                    )));
                }
                top.permissions.difference(permission.bits())
            };
            diesel::update(deployment_role::table.filter(deployment_role::id.eq(top.id)))
                .set(deployment_role::permissions.eq(permissions))
                .execute(conn)
                .await?;
            announce_holders(publisher, conn, top.id).await?;
            Ok(top.name.clone())
        }
        .scope_boxed()
    })
    .await
}

/// The terminal's revoke: takes every deployment role from `user`, and tells them.
pub async fn revoke_all(
    publisher: &impl Publishing,
    conn: &mut AsyncPgConnection,
    target: UserId,
) -> app::Result<usize> {
    conn.transaction(|conn| {
        async move {
            let taken = diesel::delete(
                user_deployment_role::table.filter(user_deployment_role::user.eq(target)),
            )
            .execute(conn)
            .await?;
            if taken > 0 {
                announce(publisher, conn, target).await?;
                refresh_name_hues(publisher, conn, &[target]).await?;
            }
            Ok(taken)
        }
        .scope_boxed()
    })
    .await
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
