//! Managing a community's roles, who holds them, and its per-channel and per-category
//! overrides; removing members; and handing a community to a new owner. Who may do each is
//! `app::permissions`: managing roles and overrides takes Manage roles, Manage channels, or
//! Manage categories, and reaches only roles ranked below the caller's; giving roles takes
//! Assign roles; removing someone takes Remove members and a higher rank than theirs.
//!
//! Roles keep dense positions: everyone's at 0 and the rest at 1 and up, renumbered whenever
//! one is made, deleted, or moved, each renumbered role announced by its update.

use crate::api::message_enum::request::{RoleCreateRequest, RoleUpdateRequest};
use crate::api::message_enum::{self, server_event::*};
use crate::app::context::GlobalServerContext;
use crate::app::moderation_log::{ModerationAction, log_moderation};
use crate::app::permissions::{
    CommunityAccess, Permission, Permissions, channel_access, from_names, require_actual_member,
    require_member, to_names,
};
use crate::app::{
    self, CategoryId, ChannelId, CommunityId, EventScope, RoleId, UserId, publish_event,
};
use crate::database::schema::{
    category, category_override, channel, channel_override, community, community_member_role,
    community_role, community_user,
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
    /// The hue holders' names are drawn in, 0 to 359.
    pub hue: Option<i16>,
    /// Whether holders are shown apart in the member list and come first in its sample.
    pub hoist: bool,
}

/// A role to make, before it has an id or a place.
pub(crate) struct NewRole {
    pub name: String,
    pub permissions: Permissions,
    pub hue: Option<i16>,
    pub hoist: bool,
    /// The bot it is made for, whose alone it then is.
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
            hue: row.hue,
            hoist: row.hoist,
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
            hue: None,
            hoist: false,
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

/// Every override of the categories of `visible`'s communities that its user may learn of, and
/// of the channels in them its user may view.
pub async fn read_communities_overrides(
    state: &GlobalServerContext,
    visible: &app::visibility::Visibility,
) -> app::Result<(
    Vec<message_enum::ChannelOverride>,
    Vec<message_enum::CategoryOverride>,
)> {
    let communities = visible.communities();
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
    let categories: Vec<(CategoryId, RoleId, Permissions, Permissions, CommunityId)> =
        category_override::table
            .inner_join(category::table)
            .select((
                category_override::category,
                category_override::role,
                category_override::allow,
                category_override::deny,
                category::community,
            ))
            .filter(category::community.eq_any(communities.to_vec()))
            .filter(category::deleted_at.is_null())
            .load(conn.as_mut())
            .await?;

    Ok((
        channels
            .into_iter()
            .filter(|(channel, ..)| visible.can_view(*channel))
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
            .filter(|(category, .., community)| visible.can_view_category(*community, *category))
            .map(
                |(category, role, allow, deny, _)| message_enum::CategoryOverride {
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

/// The highest hue, of 0 to 359 around the colour wheel.
pub const MAX_HUE: i16 = 359;

/// Checks that `hue`, when there is one, is on the colour wheel. Deployment roles' hues are
/// checked here too.
pub fn validate_hue(hue: Option<i16>) -> app::Result<()> {
    match hue {
        Some(hue) if !(0..=MAX_HUE).contains(&hue) => {
            Err(app::Error::Validation(t!("roleHueRange", max = MAX_HUE)))
        }
        _ => Ok(()),
    }
}

/// Refuses a hue or showing apart for everyone's role: a colour every member shares marks
/// nobody, and showing everyone apart shows nobody apart.
fn check_everyone_plain(everyone: bool, hue: Option<i16>, hoist: bool) -> app::Result<()> {
    if everyone && (hue.is_some() || hoist) {
        return Err(app::Error::Validation(t!("everyoneRolePlain")));
    }
    Ok(())
}

/// Gives the roles of a community dense positions in the order `order` lists them (everyone's
/// first, at 0), announcing each whose position changed.
async fn renumber(
    state: &impl crate::app::events::Publishing,
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
                hue: None,
                hoist: None,
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
    request: &RoleCreateRequest,
) -> app::Result<message_enum::Role> {
    let name = validate_name(&request.name)?;
    validate_hue(request.hue)?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = require_member(conn.as_mut(), caller, community_id).await?;
            access.require(Permissions::MANAGE_ROLES)?;
            let permissions = from_names(&request.permissions).valid();
            access.require_holds(permissions)?;
            let role = NewRole {
                name,
                permissions,
                hue: request.hue,
                hoist: request.hoist,
                bot: None,
            };
            insert_role(state, conn.as_mut(), community_id, role).await
        }
        .scope_boxed()
    })
    .await
}

/// Makes a role just above everyone's, inside the caller's transaction, and announces it; the
/// caller has checked who may.
pub(crate) async fn insert_role(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community_id: CommunityId,
    role: NewRole,
) -> app::Result<message_enum::Role> {
    let mut roles = load_roles(conn, community_id).await?;
    let row = RoleRow {
        id: RoleId::new(),
        community: community_id,
        name: role.name,
        position: 1,
        permissions: role.permissions,
        everyone: false,
        bot: role.bot,
        hue: role.hue,
        hoist: role.hoist,
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
    state: &impl crate::app::events::Publishing,
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

/// Renames a role, or changes its permissions, hue, or whether it is shown apart. The role must
/// rank below the caller, and any permission given or taken must be one the caller holds.
pub async fn update_role(
    state: &GlobalServerContext,
    caller: UserId,
    role_id: RoleId,
    request: &RoleUpdateRequest,
) -> app::Result<message_enum::Role> {
    let name = request.name.as_deref().map(validate_name).transpose()?;
    if let Some(hue) = request.hue {
        validate_hue(hue)?;
    }
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let mut role = load_role(conn.as_mut(), role_id).await?;
            if role.everyone && name.is_some() {
                return Err(app::Error::Validation(t!("everyoneRoleFixed")));
            }
            check_everyone_plain(
                role.everyone,
                request.hue.flatten(),
                request.hoist.unwrap_or(false),
            )?;
            let access = require_member(conn.as_mut(), caller, role.community).await?;
            access.require(Permissions::MANAGE_ROLES)?;
            access.require_above(role.position)?;
            let permissions = request
                .permissions
                .as_deref()
                .map(|names| from_names(names).valid());
            if let Some(permissions) = permissions {
                // What changes must be the caller's to give or take.
                access.require_holds(permissions.symmetric_difference(role.permissions))?;
                role.permissions = permissions;
            }
            if let Some(name) = &name {
                role.name = name.clone();
            }
            if let Some(hue) = request.hue {
                role.hue = hue;
            }
            if let Some(hoist) = request.hoist {
                role.hoist = hoist;
            }
            diesel::update(community_role::table.filter(community_role::id.eq(role_id)))
                .set((
                    community_role::name.eq(&role.name),
                    community_role::permissions.eq(role.permissions),
                    community_role::hue.eq(role.hue),
                    community_role::hoist.eq(role.hoist),
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
                    hue: request.hue,
                    hoist: request.hoist,
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
            hue: None,
            hoist: None,
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
            // Its holders lose it and its overrides go with it, by the foreign keys. The one
            // event says so: readers of it (the event feed, clients) take the role from its
            // holders and its overrides away themselves, however many there are.
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

/// Announces a member's roles as they now stand, as an update of their membership. The caller
/// holds the lock on the member's `community_user` row, so no other change to their roles is
/// read half made.
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
            nickname: None,
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

/// Gives `member` a role, or takes it away, which takes Assign roles, the role and the member
/// ranking below the caller, and, to give it, holding every permission it allows. Returns
/// whether anything changed.
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
            // Giving a role gives what it allows, which must be the caller's to give, as for
            // making or editing one; taking it away takes rank alone.
            if held {
                access.require_holds(role.permissions)?;
            }
            // The member's row is locked first, so that changes to their roles are made, read
            // whole, and announced one at a time: each announcement then carries the list as it
            // stands after every change committed before it, in the order they commit.
            community_user::table
                .select(community_user::user)
                .filter(
                    community_user::community
                        .eq(community_id)
                        .and(community_user::user.eq(member)),
                )
                .for_no_key_update()
                .first::<UserId>(conn.as_mut())
                .await?;
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
    // Checked in the transaction that removes them, so a rank that changes meanwhile decides.
    conn.transaction(|conn| {
        async move {
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
            app::community::end_membership(state, conn.as_mut(), member, community_id).await
        }
        .scope_boxed()
    })
    .await
}

/// Clears `member`'s nickname in the community. Anyone may clear their own; anyone else's takes
/// Manage nicknames, and a member ranked below the caller, never the owner. Returns whether
/// there was one to clear.
pub async fn clear_nickname(
    state: &GlobalServerContext,
    caller: UserId,
    community_id: CommunityId,
    member: UserId,
) -> app::Result<bool> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            if member != caller {
                let access = require_member(conn.as_mut(), caller, community_id).await?;
                access.require(Permissions::MANAGE_NICKNAMES)?;
                let their_rank = member_below(conn.as_mut(), &access, member).await?;
                // Moderation when the community's own permissions would not have allowed it.
                if access.moderator
                    && !(access
                        .member_permissions
                        .contains(Permissions::MANAGE_NICKNAMES)
                        && their_rank < access.role_rank())
                {
                    log_moderation(
                        conn.as_mut(),
                        caller,
                        ModerationAction::ClearNickname,
                        Some(community_id),
                        None,
                        Some(member.0.to_string()),
                    )
                    .await?;
                }
            } else {
                require_actual_member(conn.as_mut(), member, community_id).await?;
            }
            app::community::erase_nickname(state, conn.as_mut(), community_id, member).await
        }
        .scope_boxed()
    })
    .await
}

/// Makes `owner` the community's owner, or leaves it with none, and announces it, in the
/// caller's transaction, with no checks: the callers decide who may hand a community on and to
/// whom.
pub(crate) async fn set_owner(
    state: &impl crate::app::events::Publishing,
    conn: &mut AsyncPgConnection,
    community_id: CommunityId,
    owner: Option<UserId>,
) -> app::Result<()> {
    diesel::update(community::table.filter(community::id.eq(community_id)))
        .set(community::owner.eq(owner))
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        EventScope::Community(community_id),
        &ServerEvent::Community(CommunityEvent::Update {
            id: community_id,
            name: None,
            icon: None,
            owner: Some(owner),
        }),
    )
    .await
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
            set_owner(state, conn.as_mut(), community_id, Some(new_owner)).await
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

/// One role's override as a request names it before its channel exists (`overrides` on a new
/// channel): what the role is allowed and denied there besides its own permissions.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
    schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct RoleOverride {
    pub role: RoleId,
    pub allow: Vec<Permission>,
    pub deny: Vec<Permission>,
}

/// An override checked for its caller and ready to write.
#[derive(Debug, Clone, Copy)]
pub struct GrantedOverride {
    pub role: RoleId,
    pub allow: Permissions,
    pub deny: Permissions,
}

/// Refuses an override that names a community permission or both allows and denies one.
fn check_override_permissions(allow: Permissions, deny: Permissions) -> app::Result<()> {
    if !Permissions::CHANNEL.contains(allow | deny) {
        return Err(app::Error::Validation(t!("overrideCommunityPermission")));
    }
    if allow & deny != Permissions::empty() {
        return Err(app::Error::Validation(t!("overrideConflict")));
    }
    Ok(())
}

/// Refuses unless `access` may set `role`'s override to `allow` and `deny`: the role ranks
/// below the caller's highest, and the caller holds every permission it names.
fn check_grantable(
    access: &CommunityAccess,
    role: &RoleRow,
    allow: Permissions,
    deny: Permissions,
) -> app::Result<()> {
    access.require_above(role.position)?;
    access.require_holds(allow | deny)
}

/// Checks the overrides a new channel of `access`'s community is to start with, on the terms
/// setting each afterwards would be checked on, and with each role named at most once. One
/// that allows and denies nothing is no override and is left out.
pub(crate) async fn check_initial_overrides(
    conn: &mut AsyncPgConnection,
    access: &CommunityAccess,
    overrides: &[RoleOverride],
) -> app::Result<Vec<GrantedOverride>> {
    if overrides.is_empty() {
        return Ok(Vec::new());
    }
    let roles: HashMap<RoleId, RoleRow> = load_roles(conn, access.community)
        .await?
        .into_iter()
        .map(|role| (role.id, role))
        .collect();
    let mut granted: Vec<GrantedOverride> = Vec::with_capacity(overrides.len());
    for o in overrides {
        let (allow, deny) = (from_names(&o.allow), from_names(&o.deny));
        check_override_permissions(allow, deny)?;
        let role = roles
            .get(&o.role)
            .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))?;
        check_grantable(access, role, allow, deny)?;
        if granted.iter().any(|g| g.role == o.role) {
            return Err(app::Error::Validation(t!("overrideRoleRepeated")));
        }
        if !(allow | deny).is_empty() {
            granted.push(GrantedOverride {
                role: o.role,
                allow,
                deny,
            });
        }
    }
    Ok(granted)
}

/// Writes a new channel's overrides and announces each, in the transaction that makes it and
/// before the channel's own creation, so that creation reaches only those they let view it.
pub(crate) async fn insert_initial_overrides(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    channel_id: ChannelId,
    overrides: &[GrantedOverride],
) -> app::Result<()> {
    if overrides.is_empty() {
        return Ok(());
    }
    diesel::insert_into(channel_override::table)
        .values(
            overrides
                .iter()
                .map(|o| {
                    (
                        channel_override::channel.eq(channel_id),
                        channel_override::role.eq(o.role),
                        channel_override::allow.eq(o.allow),
                        channel_override::deny.eq(o.deny),
                    )
                })
                .collect::<Vec<_>>(),
        )
        .execute(conn)
        .await?;
    for o in overrides {
        publish_event(
            state,
            conn,
            EventScope::ChannelDefinition {
                channel: channel_id,
                departed: None,
            },
            &ServerEvent::ChannelOverride(ChannelOverrideEvent::Create(
                message_enum::ChannelOverride {
                    channel: channel_id,
                    role: o.role,
                    allow: to_names(o.allow),
                    deny: to_names(o.deny),
                },
            )),
        )
        .await?;
    }
    Ok(())
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
/// channels for a channel the caller may view and Manage categories for a category whose own
/// overrides let them view it, the role ranking below the caller, and permissions the caller
/// holds. Only channel permissions may be overridden, and
/// none may be both allowed and denied.
pub async fn set_override(
    state: &GlobalServerContext,
    caller: UserId,
    target: OverrideTarget,
    role_id: RoleId,
    permissions: Option<(Permissions, Permissions)>,
) -> app::Result<OverrideOutcome> {
    if let Some((allow, deny)) = permissions {
        check_override_permissions(allow, deny)?;
    }
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            // Only someone who may view the channel (`channel_access`), or what the category's
            // overrides leave them (`managed_category`), may change what hides it from them.
            let (community_id, access) = match target {
                OverrideTarget::Channel(id) => {
                    let access = channel_access(state, conn.as_mut(), caller, id).await?;
                    // Threads follow their parent; DMs have no roles.
                    let (Some(access), false) = (access.community, access.thread) else {
                        return Err(app::Error::Validation(t!("overrideTarget")));
                    };
                    // A community permission, which no override changes.
                    access.require(Permissions::MANAGE_CHANNELS)?;
                    (access.community, access)
                }
                OverrideTarget::Category(id) => {
                    let (category, access) =
                        app::category::managed_category(conn.as_mut(), caller, id).await?;
                    (*category.community.id(), access)
                }
            };
            let role = load_role(conn.as_mut(), role_id).await?;
            if role.community != community_id {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            let (allow, deny) = permissions.unwrap_or((Permissions::empty(), Permissions::empty()));
            check_grantable(&access, &role, allow, deny)?;
            let cleared = OverrideOutcome {
                allow,
                deny,
                changed_presence: false,
            };
            let scope = EventScope::Community(community_id);
            match target {
                OverrideTarget::Channel(channel_id) => {
                    // The channel's own event, so it reaches those who may view the channel
                    // before or after it, and no one else learns of the channel.
                    let scope = EventScope::ChannelDefinition {
                        channel: channel_id,
                        departed: None,
                    };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hues_lie_on_the_colour_wheel() {
        assert!(validate_hue(None).is_ok());
        assert!(validate_hue(Some(0)).is_ok());
        assert!(validate_hue(Some(MAX_HUE)).is_ok());
        assert!(validate_hue(Some(MAX_HUE + 1)).is_err());
        assert!(validate_hue(Some(-1)).is_err());
    }

    #[test]
    fn everyones_role_stays_plain() {
        assert!(check_everyone_plain(true, None, false).is_ok());
        assert!(check_everyone_plain(true, Some(10), false).is_err());
        assert!(check_everyone_plain(true, None, true).is_err());
        assert!(check_everyone_plain(false, Some(10), true).is_ok());
    }
}
