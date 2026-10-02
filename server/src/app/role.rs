//! Managing a community's roles, who holds them, and its per-channel and per-category
//! overrides; removing members; and handing a community to a new owner. Who may do each is
//! `app::permissions`: managing roles and overrides takes Manage roles, Manage channels, or
//! Manage categories, and reaches only roles ranked below the caller's; giving roles takes
//! Assign roles; removing someone takes Remove members and a higher rank than theirs.
//!
//! Roles keep dense positions: everyone's at 0 and the rest at 1 and up, renumbered whenever
//! one is made, deleted, or moved, each renumbered role announced by its update.

use crate::api::message_enum::{self, server_event::*};
use crate::app::context::GlobalServerContext;
use crate::app::moderation_log::{ModerationAction, log_moderation};
use crate::app::permissions::{
    CommunityAccess, Permissions, missing, require_actual_member, require_member, to_names,
};
use crate::app::{
    self, CategoryId, ChannelId, CommunityId, EventScope, RoleId, UserId, publish_event,
};
use crate::database::schema::{
    category, category_override, channel, channel_override, community, community_member_role,
    community_role,
};
use crate::t;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::collections::HashMap;

/// The longest a role's name may be, in characters.
pub const MAX_ROLE_NAME_CHARS: usize = 64;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = community_role)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RoleRow {
    pub id: RoleId,
    pub community: CommunityId,
    pub name: String,
    pub position: i32,
    pub permissions: Permissions,
    pub everyone: bool,
    /// The bot this role was made for when it was added, whose alone it is.
    pub bot: Option<UserId>,
}

impl From<&RoleRow> for message_enum::Role {
    fn from(row: &RoleRow) -> Self {
        message_enum::Role {
            id: row.id,
            community: row.community,
            name: row.name.clone(),
            position: row.position,
            permissions: to_names(row.permissions),
            everyone: row.everyone,
            bot: row.bot,
        }
    }
}

/// Makes a new community's roles, in the transaction that makes it: everyone's, with the
/// member template, then Moderator and Admin. Returns the Admin role.
pub async fn create_default_roles(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
) -> app::Result<RoleId> {
    let roles = [
        (t!("roleEveryone"), 0, Permissions::MEMBER_TEMPLATE, true),
        (
            t!("roleModerator"),
            1,
            Permissions::MODERATOR_TEMPLATE,
            false,
        ),
        (t!("roleAdmin"), 2, Permissions::ADMIN_TEMPLATE, false),
    ];
    let rows: Vec<RoleRow> = roles
        .into_iter()
        .map(|(name, position, permissions, everyone)| RoleRow {
            id: RoleId::new(),
            community,
            name: name.to_string(),
            position,
            permissions,
            everyone,
            bot: None,
        })
        .collect();
    diesel::insert_into(community_role::table)
        .values(&rows)
        .execute(conn)
        .await?;
    Ok(rows[2].id)
}

async fn load_roles(
    conn: &mut AsyncPgConnection,
    community_id: CommunityId,
) -> app::Result<Vec<RoleRow>> {
    Ok(community_role::table
        .select(RoleRow::as_select())
        .filter(community_role::community.eq(community_id))
        .order((community_role::position, community_role::id))
        .load(conn)
        .await?)
}

async fn load_role(conn: &mut AsyncPgConnection, id: RoleId) -> app::Result<RoleRow> {
    Ok(community_role::table
        .select(RoleRow::as_select())
        .filter(community_role::id.eq(id))
        .first(conn)
        .await?)
}

/// The roles of every one of `communities`, lowest first.
pub async fn read_communities_roles(
    state: &GlobalServerContext,
    communities: &[CommunityId],
) -> app::Result<Vec<message_enum::Role>> {
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<RoleRow> = community_role::table
        .select(RoleRow::as_select())
        .filter(community_role::community.eq_any(communities.to_vec()))
        .order((
            community_role::community,
            community_role::position,
            community_role::id,
        ))
        .load(conn.as_mut())
        .await?;
    Ok(rows.iter().map(message_enum::Role::from).collect())
}

/// A community's roles, lowest first, for a member of it.
pub async fn read_roles(
    state: &GlobalServerContext,
    caller: UserId,
    community_id: CommunityId,
) -> app::Result<Vec<message_enum::Role>> {
    let mut conn = state.connection_pool.get().await?;
    require_member(conn.as_mut(), caller, community_id).await?;
    Ok(load_roles(conn.as_mut(), community_id)
        .await?
        .iter()
        .map(message_enum::Role::from)
        .collect())
}

/// Every override of the channels and categories of `communities`.
pub async fn read_communities_overrides(
    state: &GlobalServerContext,
    communities: &[CommunityId],
) -> app::Result<(
    Vec<message_enum::ChannelOverride>,
    Vec<message_enum::CategoryOverride>,
)> {
    let mut conn = state.connection_pool.get().await?;
    let channels: Vec<(ChannelId, RoleId, Permissions, Permissions)> = channel_override::table
        .inner_join(channel::table)
        .select((
            channel_override::channel,
            channel_override::role,
            channel_override::allow,
            channel_override::deny,
        ))
        .filter(channel::community.eq_any(communities.to_vec()))
        .filter(channel::deleted_at.is_null())
        .load(conn.as_mut())
        .await?;
    let categories: Vec<(CategoryId, RoleId, Permissions, Permissions)> = category_override::table
        .inner_join(category::table)
        .select((
            category_override::category,
            category_override::role,
            category_override::allow,
            category_override::deny,
        ))
        .filter(category::community.eq_any(communities.to_vec()))
        .filter(category::deleted_at.is_null())
        .load(conn.as_mut())
        .await?;
    Ok((
        channels
            .into_iter()
            .map(
                |(channel, role, allow, deny)| message_enum::ChannelOverride {
                    channel,
                    role,
                    allow: to_names(allow),
                    deny: to_names(deny),
                },
            )
            .collect(),
        categories
            .into_iter()
            .map(
                |(category, role, allow, deny)| message_enum::CategoryOverride {
                    category,
                    role,
                    allow: to_names(allow),
                    deny: to_names(deny),
                },
            )
            .collect(),
    ))
}

/// The roles each of `members` holds besides everyone's, keyed by community and user.
pub async fn roles_of_members(
    conn: &mut AsyncPgConnection,
    members: &[(CommunityId, UserId)],
) -> app::Result<HashMap<(CommunityId, UserId), Vec<RoleId>>> {
    if members.is_empty() {
        return Ok(HashMap::new());
    }
    let communities: Vec<CommunityId> = members.iter().map(|(c, _)| *c).collect();
    let users: Vec<UserId> = members.iter().map(|(_, u)| *u).collect();
    let rows: Vec<(CommunityId, UserId, RoleId)> = community_member_role::table
        .inner_join(community_role::table)
        .select((
            community_member_role::community,
            community_member_role::user,
            community_member_role::role,
        ))
        .filter(community_member_role::community.eq_any(communities))
        .filter(community_member_role::user.eq_any(users))
        .order((community_role::position, community_role::id))
        .load(conn)
        .await?;
    let mut held: HashMap<(CommunityId, UserId), Vec<RoleId>> = HashMap::new();
    for (community, user, role) in rows {
        held.entry((community, user)).or_default().push(role);
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

/// Gives the roles of a community dense positions in the order `order` lists them (everyone's
/// first, at 0), announcing each whose position changed.
async fn renumber(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community_id: CommunityId,
    order: &[RoleRow],
) -> app::Result<()> {
    for (position, role) in order.iter().enumerate() {
        let position = i32::try_from(position).unwrap_or(i32::MAX);
        if role.position == position {
            continue;
        }
        diesel::update(community_role::table.filter(community_role::id.eq(role.id)))
            .set(community_role::position.eq(position))
            .execute(conn)
            .await?;
        publish_event(
            state,
            conn,
            EventScope::Community(community_id),
            &ServerEvent::Role(RoleEvent::Update {
                id: role.id,
                name: None,
                position: Some(position),
                permissions: None,
            }),
        )
        .await?;
    }
    Ok(())
}

/// Makes a role, placed just above everyone's, with permissions the caller holds.
pub async fn create_role(
    state: &GlobalServerContext,
    caller: UserId,
    community_id: CommunityId,
    name: &str,
    permissions: Permissions,
) -> app::Result<message_enum::Role> {
    let name = validate_name(name)?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = require_member(conn.as_mut(), caller, community_id).await?;
            access.require(Permissions::MANAGE_ROLES)?;
            let permissions = permissions.valid();
            access.require_holds(permissions)?;
            insert_role(state, conn.as_mut(), community_id, name, permissions, None).await
        }
        .scope_boxed()
    })
    .await
}

/// Makes a role just above everyone's, inside the caller's transaction, and announces it; the
/// caller has checked who may. `bot` names the bot it is made for, whose alone it then is.
pub(crate) async fn insert_role(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community_id: CommunityId,
    name: String,
    permissions: Permissions,
    bot: Option<UserId>,
) -> app::Result<message_enum::Role> {
    let mut roles = load_roles(conn, community_id).await?;
    let row = RoleRow {
        id: RoleId::new(),
        community: community_id,
        name,
        position: 1,
        permissions,
        everyone: false,
        bot,
    };
    diesel::insert_into(community_role::table)
        .values(&row)
        .execute(conn)
        .await?;
    let record = message_enum::Role::from(&row);
    publish_event(
        state,
        conn,
        EventScope::Community(community_id),
        &ServerEvent::Role(RoleEvent::Create(record.clone())),
    )
    .await?;
    // Everyone's first, then the new role, then the rest as they were.
    let rest = roles.split_off(1.min(roles.len()));
    let mut order = roles;
    order.push(row);
    order.extend(rest);
    renumber(state, conn, community_id, &order).await?;
    Ok(record)
}

/// Deletes the role a bot was given when it was added to `community_id`, as the bot leaves,
/// inside the caller's transaction.
pub(crate) async fn delete_bot_role(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community_id: CommunityId,
    bot: UserId,
) -> app::Result<()> {
    let deleted: Vec<RoleId> = diesel::delete(
        community_role::table.filter(
            community_role::community
                .eq(community_id)
                .and(community_role::bot.eq(bot)),
        ),
    )
    .returning(community_role::id)
    .get_results(conn)
    .await?;
    for id in &deleted {
        publish_event(
            state,
            conn,
            EventScope::Community(community_id),
            &ServerEvent::Role(RoleEvent::Delete { id: *id }),
        )
        .await?;
    }
    if !deleted.is_empty() {
        let order = load_roles(conn, community_id).await?;
        renumber(state, conn, community_id, &order).await?;
    }
    Ok(())
}

/// Renames a role or changes its permissions. The role must rank below the caller, and any
/// permission given or taken must be one the caller holds.
pub async fn update_role(
    state: &GlobalServerContext,
    caller: UserId,
    role_id: RoleId,
    name: Option<&str>,
    permissions: Option<Permissions>,
) -> app::Result<message_enum::Role> {
    let name = name.map(validate_name).transpose()?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let mut role = load_role(conn.as_mut(), role_id).await?;
            if role.everyone && name.is_some() {
                return Err(app::Error::Validation(t!("everyoneRoleFixed")));
            }
            let access = require_member(conn.as_mut(), caller, role.community).await?;
            access.require(Permissions::MANAGE_ROLES)?;
            access.require_above(role.position)?;
            let permissions = permissions.map(Permissions::valid);
            if let Some(permissions) = permissions {
                // What changes must be the caller's to give or take.
                access.require_holds(permissions.symmetric_difference(role.permissions))?;
                role.permissions = permissions;
            }
            if let Some(name) = &name {
                role.name = name.clone();
            }
            diesel::update(community_role::table.filter(community_role::id.eq(role_id)))
                .set((
                    community_role::name.eq(&role.name),
                    community_role::permissions.eq(role.permissions),
                ))
                .execute(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Community(role.community),
                &ServerEvent::Role(RoleEvent::Update {
                    id: role_id,
                    name,
                    position: None,
                    permissions: permissions.map(to_names),
                }),
            )
            .await?;
            Ok(message_enum::Role::from(&role))
        }
        .scope_boxed()
    })
    .await
}

/// Takes `permission` from a community's everyone role for the deployment itself, and
/// announces it; whether the role held it.
pub(crate) async fn take_from_everyone(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community_id: CommunityId,
    permission: Permissions,
) -> app::Result<bool> {
    let role: RoleRow = community_role::table
        .select(RoleRow::as_select())
        .filter(
            community_role::community
                .eq(community_id)
                .and(community_role::everyone),
        )
        .first(conn)
        .await?;
    if !role.permissions.contains(permission) {
        return Ok(false);
    }
    let permissions = role.permissions.difference(permission);
    diesel::update(community_role::table.filter(community_role::id.eq(role.id)))
        .set(community_role::permissions.eq(permissions))
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        EventScope::Community(community_id),
        &ServerEvent::Role(RoleEvent::Update {
            id: role.id,
            name: None,
            position: None,
            permissions: Some(to_names(permissions)),
        }),
    )
    .await?;
    Ok(true)
}

/// Deletes a role below the caller; its holders and overrides lose it.
pub async fn delete_role(
    state: &GlobalServerContext,
    caller: UserId,
    role_id: RoleId,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let role = load_role(conn.as_mut(), role_id).await?;
            if role.everyone {
                return Err(app::Error::Validation(t!("everyoneRoleFixed")));
            }
            if role.bot.is_some() {
                return Err(app::Error::Validation(t!("botRoleFixed")));
            }
            let access = require_member(conn.as_mut(), caller, role.community).await?;
            access.require(Permissions::MANAGE_ROLES)?;
            access.require_above(role.position)?;
            // Its holders' memberships change, and each is announced.
            let holders: Vec<UserId> = community_member_role::table
                .select(community_member_role::user)
                .filter(community_member_role::role.eq(role_id))
                .load(conn.as_mut())
                .await?;
            diesel::delete(community_role::table.filter(community_role::id.eq(role_id)))
                .execute(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Community(role.community),
                &ServerEvent::Role(RoleEvent::Delete { id: role_id }),
            )
            .await?;
            for user in holders {
                announce_member_roles(state, conn.as_mut(), role.community, user).await?;
            }
            let order = load_roles(conn.as_mut(), role.community).await?;
            renumber(state, conn.as_mut(), role.community, &order).await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Reorders the roles ranked below the caller: `order` lists exactly those, lowest first,
/// everyone's excepted; the roles at and above the caller's rank keep their places above them.
pub async fn reorder_roles(
    state: &GlobalServerContext,
    caller: UserId,
    community_id: CommunityId,
    order: &[RoleId],
) -> app::Result<Vec<message_enum::Role>> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = require_member(conn.as_mut(), caller, community_id).await?;
            access.require(Permissions::MANAGE_ROLES)?;
            let roles = load_roles(conn.as_mut(), community_id).await?;
            let rank = access.rank();
            let (everyone, others): (Vec<RoleRow>, Vec<RoleRow>) =
                roles.into_iter().partition(|r| r.everyone);
            let (movable, fixed): (Vec<RoleRow>, Vec<RoleRow>) =
                others.into_iter().partition(|r| r.position < rank);
            let mut wanted: Vec<RoleId> = order.to_vec();
            wanted.sort();
            let mut have: Vec<RoleId> = movable.iter().map(|r| r.id).collect();
            have.sort();
            if wanted != have {
                return Err(app::Error::Validation(t!("roleOrderMismatch")));
            }
            let mut by_id: HashMap<RoleId, RoleRow> =
                movable.into_iter().map(|r| (r.id, r)).collect();
            let mut sequence = everyone;
            sequence.extend(order.iter().filter_map(|id| by_id.remove(id)));
            sequence.extend(fixed);
            renumber(state, conn.as_mut(), community_id, &sequence).await?;
            Ok(load_roles(conn.as_mut(), community_id)
                .await?
                .iter()
                .map(message_enum::Role::from)
                .collect())
        }
        .scope_boxed()
    })
    .await
}

/// Announces a member's roles as they now stand, as an update of their membership.
async fn announce_member_roles(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community_id: CommunityId,
    user: UserId,
) -> app::Result<Vec<RoleId>> {
    let roles = roles_of_members(conn, &[(community_id, user)])
        .await?
        .remove(&(community_id, user))
        .unwrap_or_default();
    publish_event(
        state,
        conn,
        EventScope::Membership {
            community: community_id,
            user,
        },
        &ServerEvent::UserCommunity(UserCommunityEvent::Update {
            community: community_id,
            user,
            sort_index: None,
            roles: Some(roles.clone()),
        }),
    )
    .await?;
    Ok(roles)
}

/// Whether the caller may act on `member`: only when they are a member who ranks below the
/// caller, or the caller themself; never the owner. Returns their rank in the community.
async fn member_below(
    conn: &mut AsyncPgConnection,
    access: &CommunityAccess,
    member: UserId,
) -> app::Result<i32> {
    let target = require_actual_member(conn, member, access.community).await?;
    if member == access.user {
        return Ok(target.role_rank());
    }
    if target.owner {
        return Err(app::Error::Forbidden(t!("permissionRank")));
    }
    access.require_above(target.role_rank())?;
    Ok(target.role_rank())
}

/// Gives `member` a role, or takes it away. Returns whether anything changed.
pub async fn set_member_role(
    state: &GlobalServerContext,
    caller: UserId,
    community_id: CommunityId,
    member: UserId,
    role_id: RoleId,
    held: bool,
) -> app::Result<bool> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = require_member(conn.as_mut(), caller, community_id).await?;
            access.require(Permissions::ASSIGN_ROLES)?;
            let role = load_role(conn.as_mut(), role_id).await?;
            if role.community != community_id || role.everyone {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            if role.bot.is_some() {
                return Err(app::Error::Validation(t!("botRoleFixed")));
            }
            access.require_above(role.position)?;
            member_below(conn.as_mut(), &access, member).await?;
            let changed = if held {
                diesel::insert_into(community_member_role::table)
                    .values((
                        community_member_role::user.eq(member),
                        community_member_role::community.eq(community_id),
                        community_member_role::role.eq(role_id),
                    ))
                    .on_conflict_do_nothing()
                    .execute(conn.as_mut())
                    .await?
            } else {
                diesel::delete(
                    community_member_role::table.filter(
                        community_member_role::user
                            .eq(member)
                            .and(community_member_role::role.eq(role_id)),
                    ),
                )
                .execute(conn.as_mut())
                .await?
            };
            if changed > 0 {
                announce_member_roles(state, conn.as_mut(), community_id, member).await?;
            }
            Ok(changed > 0)
        }
        .scope_boxed()
    })
    .await
}

/// Removes `member` from the community, which ends their membership as leaving does.
pub async fn remove_member(
    state: &GlobalServerContext,
    caller: UserId,
    community_id: CommunityId,
    member: UserId,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    {
        let access = require_member(conn.as_mut(), caller, community_id).await?;
        access.require(Permissions::REMOVE_MEMBERS)?;
        if member == caller {
            return Err(app::Error::Validation(t!("removeSelf")));
        }
        let their_rank = member_below(conn.as_mut(), &access, member).await?;
        // Moderation when the community's own permissions would not have allowed it.
        if access.moderator
            && !(access
                .member_permissions
                .contains(Permissions::REMOVE_MEMBERS)
                && their_rank < access.role_rank())
        {
            log_moderation(
                conn.as_mut(),
                caller,
                ModerationAction::RemoveMember,
                Some(community_id),
                None,
                Some(member.0.to_string()),
            )
            .await?;
        }
    }
    app::community::end_membership(state, conn.as_mut(), member, community_id).await
}

/// Hands the community to another member. Only its owner may, and they stay a member.
pub async fn transfer_ownership(
    state: &GlobalServerContext,
    caller: UserId,
    community_id: CommunityId,
    new_owner: UserId,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = require_member(conn.as_mut(), caller, community_id).await?;
            if !access.owner {
                return Err(app::Error::Forbidden(t!("permissionOwnerOnly")));
            }
            require_actual_member(conn.as_mut(), new_owner, community_id).await?;
            diesel::update(community::table.filter(community::id.eq(community_id)))
                .set(community::owner.eq(Some(new_owner)))
                .execute(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Community(community_id),
                &ServerEvent::Community(CommunityEvent::Update {
                    id: community_id,
                    name: None,
                    icon: None,
                    owner: Some(Some(new_owner)),
                }),
            )
            .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Where an override applies: one channel, or every channel of a category.
#[derive(Debug, Clone, Copy)]
pub enum OverrideTarget {
    Channel(ChannelId),
    Category(CategoryId),
}

/// What setting or clearing an override did.
#[derive(Debug, Clone, Copy)]
pub struct OverrideOutcome {
    pub allow: Permissions,
    pub deny: Permissions,
    /// For a set, whether the override is new; for a clear, whether there was one.
    pub changed_presence: bool,
}

/// Sets, or with `None` clears, `role`'s override of a channel or category. It takes Manage
/// channels for a channel and Manage categories for a category, the role ranking below the
/// caller, and permissions the caller holds. Only channel permissions may be overridden, and
/// none may be both allowed and denied.
pub async fn set_override(
    state: &GlobalServerContext,
    caller: UserId,
    target: OverrideTarget,
    role_id: RoleId,
    permissions: Option<(Permissions, Permissions)>,
) -> app::Result<OverrideOutcome> {
    if let Some((allow, deny)) = permissions {
        if !Permissions::CHANNEL.contains(allow | deny) {
            return Err(app::Error::Validation(t!("overrideCommunityPermission")));
        }
        if allow & deny != Permissions::empty() {
            return Err(app::Error::Validation(t!("overrideConflict")));
        }
    }
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let (community_id, needed) = match target {
                OverrideTarget::Channel(id) => {
                    let (community, parent): (Option<CommunityId>, Option<ChannelId>) =
                        channel::table
                            .select((channel::community, channel::parent_channel))
                            .filter(channel::id.eq(id).and(channel::deleted_at.is_null()))
                            .first(conn.as_mut())
                            .await?;
                    // Threads follow their parent; DMs have no roles.
                    match (community, parent) {
                        (Some(community), None) => (community, Permissions::MANAGE_CHANNELS),
                        _ => return Err(app::Error::Validation(t!("overrideTarget"))),
                    }
                }
                OverrideTarget::Category(id) => {
                    let community: CommunityId = category::table
                        .select(category::community)
                        .filter(category::id.eq(id).and(category::deleted_at.is_null()))
                        .first(conn.as_mut())
                        .await?;
                    (community, Permissions::MANAGE_CATEGORIES)
                }
            };
            let access = require_member(conn.as_mut(), caller, community_id).await?;
            if !access.has(needed) {
                return Err(missing(needed));
            }
            let role = load_role(conn.as_mut(), role_id).await?;
            if role.community != community_id {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            access.require_above(role.position)?;
            let (allow, deny) = permissions.unwrap_or((Permissions::empty(), Permissions::empty()));
            access.require_holds(allow | deny)?;
            let cleared = OverrideOutcome {
                allow,
                deny,
                changed_presence: false,
            };
            let scope = EventScope::Community(community_id);
            match target {
                OverrideTarget::Channel(channel_id) => {
                    let existed: bool = diesel::select(diesel::dsl::exists(
                        channel_override::table.filter(
                            channel_override::channel
                                .eq(channel_id)
                                .and(channel_override::role.eq(role_id)),
                        ),
                    ))
                    .get_result(conn.as_mut())
                    .await?;
                    if permissions.is_none() {
                        diesel::delete(
                            channel_override::table.filter(
                                channel_override::channel
                                    .eq(channel_id)
                                    .and(channel_override::role.eq(role_id)),
                            ),
                        )
                        .execute(conn.as_mut())
                        .await?;
                        if existed {
                            publish_event(
                                state,
                                conn.as_mut(),
                                scope,
                                &ServerEvent::ChannelOverride(ChannelOverrideEvent::Delete {
                                    channel: channel_id,
                                    role: role_id,
                                }),
                            )
                            .await?;
                        }
                        return Ok(OverrideOutcome {
                            changed_presence: existed,
                            ..cleared
                        });
                    }
                    diesel::insert_into(channel_override::table)
                        .values((
                            channel_override::channel.eq(channel_id),
                            channel_override::role.eq(role_id),
                            channel_override::allow.eq(allow),
                            channel_override::deny.eq(deny),
                        ))
                        .on_conflict((channel_override::channel, channel_override::role))
                        .do_update()
                        .set((
                            channel_override::allow.eq(allow),
                            channel_override::deny.eq(deny),
                        ))
                        .execute(conn.as_mut())
                        .await?;
                    let record = message_enum::ChannelOverride {
                        channel: channel_id,
                        role: role_id,
                        allow: to_names(allow),
                        deny: to_names(deny),
                    };
                    let event = if existed {
                        ChannelOverrideEvent::Update {
                            channel: channel_id,
                            role: role_id,
                            allow: Some(record.allow.clone()),
                            deny: Some(record.deny.clone()),
                        }
                    } else {
                        ChannelOverrideEvent::Create(record)
                    };
                    publish_event(
                        state,
                        conn.as_mut(),
                        scope,
                        &ServerEvent::ChannelOverride(event),
                    )
                    .await?;
                    Ok(OverrideOutcome {
                        changed_presence: !existed,
                        ..cleared
                    })
                }
                OverrideTarget::Category(category_id) => {
                    let existed: bool = diesel::select(diesel::dsl::exists(
                        category_override::table.filter(
                            category_override::category
                                .eq(category_id)
                                .and(category_override::role.eq(role_id)),
                        ),
                    ))
                    .get_result(conn.as_mut())
                    .await?;
                    if permissions.is_none() {
                        diesel::delete(
                            category_override::table.filter(
                                category_override::category
                                    .eq(category_id)
                                    .and(category_override::role.eq(role_id)),
                            ),
                        )
                        .execute(conn.as_mut())
                        .await?;
                        if existed {
                            publish_event(
                                state,
                                conn.as_mut(),
                                scope,
                                &ServerEvent::CategoryOverride(CategoryOverrideEvent::Delete {
                                    category: category_id,
                                    role: role_id,
                                }),
                            )
                            .await?;
                        }
                        return Ok(OverrideOutcome {
                            changed_presence: existed,
                            ..cleared
                        });
                    }
                    diesel::insert_into(category_override::table)
                        .values((
                            category_override::category.eq(category_id),
                            category_override::role.eq(role_id),
                            category_override::allow.eq(allow),
                            category_override::deny.eq(deny),
                        ))
                        .on_conflict((category_override::category, category_override::role))
                        .do_update()
                        .set((
                            category_override::allow.eq(allow),
                            category_override::deny.eq(deny),
                        ))
                        .execute(conn.as_mut())
                        .await?;
                    let record = message_enum::CategoryOverride {
                        category: category_id,
                        role: role_id,
                        allow: to_names(allow),
                        deny: to_names(deny),
                    };
                    let event = if existed {
                        CategoryOverrideEvent::Update {
                            category: category_id,
                            role: role_id,
                            allow: Some(record.allow.clone()),
                            deny: Some(record.deny.clone()),
                        }
                    } else {
                        CategoryOverrideEvent::Create(record)
                    };
                    publish_event(
                        state,
                        conn.as_mut(),
                        scope,
                        &ServerEvent::CategoryOverride(event),
                    )
                    .await?;
                    Ok(OverrideOutcome {
                        changed_presence: !existed,
                        ..cleared
                    })
                }
            }
        }
        .scope_boxed()
    })
    .await
}
