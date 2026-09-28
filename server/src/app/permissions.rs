//! Who may do what in a community: roles, their permissions, and the per-channel overrides that
//! adjust them.
//!
//! Every member holds the community's everyone role, and whichever others they are given. A
//! member's permissions across the community are the union of their roles'. In a channel, the
//! channel permissions among them are then adjusted twice, first by the overrides of the
//! channel's category and then by the channel's own: at each step the everyone role's override
//! applies first, and then those of the member's other roles together, denials before
//! allowances, so an allowance for one of their roles wins over a denial for everyone. The
//! owner holds every permission, and no override touches them.
//!
//! Roles are ranked by `position`, everyone's at 0. A member may manage, assign, or remove only
//! what ranks below their own highest role, and may give a role or override only permissions
//! they hold themselves, so no one can lift themselves or anyone else above their own reach.
//!
//! In a DM or group DM every recipient holds every channel permission; a thread takes its
//! parent channel's. Someone who may not view a channel is answered as though it did not exist.
//!
//! A deployment moderator (Moderate any community, `app::deployment`) reaches every community
//! and DM without belonging to it: they view every channel whatever the overrides, hold
//! `MODERATION` everywhere, and rank above every role but below the owner. `moderating` says
//! when an action was allowed by that alone, which the caller then logs.

use crate::api::ChannelType;
use crate::api::GlobalServerContext;
use crate::app::events::{ChannelHome, channel_home};
use crate::app::{self, CategoryId, ChannelId, CommunityId, RoleId, UserId};
use crate::database::schema::{
    category_override, channel, channel_override, community, community_member_role, community_role,
    community_user, dm_recipient,
};
use diesel::prelude::*;
use diesel::{AsExpression, FromSqlRow};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use rust_i18n::t;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

bitflags::bitflags! {
    /// A set of permissions, as the bits the database stores. The values are fixed: migrations
    /// write them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, FromSqlRow, AsExpression)]
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub struct Permissions: i64 {
        // Across the community.
        const MANAGE_COMMUNITY = 1 << 0;
        const MANAGE_CHANNELS = 1 << 1;
        const MANAGE_CATEGORIES = 1 << 2;
        const CREATE_INVITES = 1 << 3;
        const MANAGE_INVITES = 1 << 4;
        const MANAGE_ROLES = 1 << 5;
        const ASSIGN_ROLES = 1 << 6;
        const REMOVE_MEMBERS = 1 << 7;
        const MANAGE_MESSAGES = 1 << 8;
        const PIN_MESSAGES = 1 << 9;
        const MANAGE_CALLS = 1 << 10;

        // In a channel, and adjustable per channel and category.
        const VIEW_CHANNEL = 1 << 16;
        const SEND_MESSAGES = 1 << 17;
        const ATTACH_FILES = 1 << 18;
        const ADD_REACTIONS = 1 << 19;
        const START_THREADS = 1 << 20;
        const SEND_IN_THREADS = 1 << 21;
        const CREATE_POLLS = 1 << 22;
        const JOIN_VOICE = 1 << 23;
        const SPEAK = 1 << 24;
        const SHARE_SCREEN = 1 << 25;
    }
}

app::bigint_sql_traits!(Permissions);

impl Permissions {
    /// Every permission that holds across the community.
    pub const COMMUNITY: Self = Self::from_bits_retain((1 << 11) - 1);
    /// Every permission an override may adjust.
    pub const CHANNEL: Self = Self::from_bits_retain(((1 << 26) - 1) & !((1 << 16) - 1));

    /// The everyone role of a new community: taking part, and inviting others.
    pub const MEMBER_TEMPLATE: Self = Self::CHANNEL.union(Self::CREATE_INVITES);
    /// A new community's Moderator role.
    pub const MODERATOR_TEMPLATE: Self = Self::MEMBER_TEMPLATE
        .union(Self::MANAGE_INVITES)
        .union(Self::REMOVE_MEMBERS)
        .union(Self::MANAGE_MESSAGES)
        .union(Self::PIN_MESSAGES)
        .union(Self::MANAGE_CALLS);
    /// A new community's Admin role: everything but what only the owner may do.
    pub const ADMIN_TEMPLATE: Self = Self::all();

    /// Every bit that names a permission, and no other.
    pub fn valid(self) -> Self {
        Self::from_bits_truncate(self.bits())
    }
}

/// Something a member may be allowed to do. The first group holds across the community; the
/// second is what a channel or category override may allow or deny.
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
)]
#[serde(rename_all = "camelCase")]
pub enum Permission {
    ManageCommunity,
    ManageChannels,
    ManageCategories,
    CreateInvites,
    ManageInvites,
    ManageRoles,
    AssignRoles,
    RemoveMembers,
    ManageMessages,
    PinMessages,
    ManageCalls,
    ViewChannel,
    SendMessages,
    AttachFiles,
    AddReactions,
    StartThreads,
    SendInThreads,
    CreatePolls,
    JoinVoice,
    Speak,
    ShareScreen,
}

impl Permission {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn bits(self) -> Permissions {
        match self {
            Permission::ManageCommunity => Permissions::MANAGE_COMMUNITY,
            Permission::ManageChannels => Permissions::MANAGE_CHANNELS,
            Permission::ManageCategories => Permissions::MANAGE_CATEGORIES,
            Permission::CreateInvites => Permissions::CREATE_INVITES,
            Permission::ManageInvites => Permissions::MANAGE_INVITES,
            Permission::ManageRoles => Permissions::MANAGE_ROLES,
            Permission::AssignRoles => Permissions::ASSIGN_ROLES,
            Permission::RemoveMembers => Permissions::REMOVE_MEMBERS,
            Permission::ManageMessages => Permissions::MANAGE_MESSAGES,
            Permission::PinMessages => Permissions::PIN_MESSAGES,
            Permission::ManageCalls => Permissions::MANAGE_CALLS,
            Permission::ViewChannel => Permissions::VIEW_CHANNEL,
            Permission::SendMessages => Permissions::SEND_MESSAGES,
            Permission::AttachFiles => Permissions::ATTACH_FILES,
            Permission::AddReactions => Permissions::ADD_REACTIONS,
            Permission::StartThreads => Permissions::START_THREADS,
            Permission::SendInThreads => Permissions::SEND_IN_THREADS,
            Permission::CreatePolls => Permissions::CREATE_POLLS,
            Permission::JoinVoice => Permissions::JOIN_VOICE,
            Permission::Speak => Permissions::SPEAK,
            Permission::ShareScreen => Permissions::SHARE_SCREEN,
        }
    }
}

app::wire_name_traits!(Permission);

/// A set of permissions as the names the API uses, in their fixed order.
pub fn to_names(permissions: Permissions) -> Vec<Permission> {
    Permission::ALL
        .iter()
        .copied()
        .filter(|p| permissions.contains(p.bits()))
        .collect()
}

/// The names the API uses as a set of permissions.
pub fn from_names(names: &[Permission]) -> Permissions {
    names.iter().map(|p| p.bits()).collect()
}

/// A role as the resolver needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleGrant {
    pub id: RoleId,
    pub position: i32,
    pub permissions: Permissions,
    pub everyone: bool,
}

/// One role's override in a channel or category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Queryable)]
pub struct Override {
    pub role: RoleId,
    pub allow: Permissions,
    pub deny: Permissions,
}

/// What a member, or a deployment moderator, may do across a community.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityAccess {
    pub user: UserId,
    pub community: CommunityId,
    pub owner: bool,
    /// Whether they belong to the community; a deployment moderator need not.
    pub member: bool,
    /// Whether they hold Moderate any community (`app::deployment`).
    pub moderator: bool,
    /// Every role they hold, everyone's included.
    pub roles: Vec<RoleGrant>,
    /// What they may do, moderation included.
    pub permissions: Permissions,
    /// What their roles alone allow, which says whether an action is moderation.
    pub member_permissions: Permissions,
}

/// The rank that decides whom a member may act on: the owner's is above every role's.
pub const OWNER_RANK: i32 = i32::MAX;
/// A deployment moderator's rank: above every role, below only the owner.
pub const MODERATOR_RANK: i32 = i32::MAX - 1;

/// What Moderate any community gives in every community: seeing everything, and the powers
/// that take things away (deleting messages, attachments, reactions, and write-ins, and removing
/// members). Renaming and deleting channels and communities are checked by name where they are
/// done.
pub const MODERATION: Permissions = Permissions::VIEW_CHANNEL
    .union(Permissions::MANAGE_MESSAGES)
    .union(Permissions::REMOVE_MEMBERS);

impl CommunityAccess {
    /// Resolves a member's permissions from the roles they hold.
    pub fn resolve(
        user: UserId,
        community: CommunityId,
        owner: bool,
        roles: Vec<RoleGrant>,
    ) -> Self {
        let permissions = if owner {
            Permissions::all()
        } else {
            roles
                .iter()
                .map(|role| role.permissions)
                .collect::<Permissions>()
                .valid()
        };
        CommunityAccess {
            user,
            community,
            owner,
            member: true,
            moderator: false,
            roles,
            permissions,
            member_permissions: permissions,
        }
    }

    /// The same access with Moderate any community added.
    pub fn with_moderation(mut self) -> Self {
        self.moderator = true;
        self.permissions |= MODERATION;
        self
    }

    /// Whether doing what `permission` allows is moderation: allowed only by Moderate any
    /// community, and so to be logged.
    pub fn moderating(&self, permission: Permissions) -> bool {
        self.moderator && !self.member_permissions.contains(permission)
    }

    /// Their highest role's position, or `OWNER_RANK` for the owner.
    pub fn rank(&self) -> i32 {
        if self.moderator && !self.owner {
            MODERATOR_RANK
        } else {
            self.role_rank()
        }
    }

    /// Their rank from what they are in the community alone: their highest role's position, or
    /// `OWNER_RANK` for the owner. Moderating the deployment does not make anyone harder to act
    /// on in a community.
    pub fn role_rank(&self) -> i32 {
        if self.owner {
            OWNER_RANK
        } else {
            self.roles.iter().map(|r| r.position).max().unwrap_or(0)
        }
    }

    pub fn has(&self, permission: Permissions) -> bool {
        self.permissions.contains(permission)
    }

    /// Refuses, naming what is missing, unless they hold `permission`.
    pub fn require(&self, permission: Permissions) -> app::Result<()> {
        if self.has(permission) {
            Ok(())
        } else {
            Err(missing(permission))
        }
    }

    /// Refuses unless `position` ranks below their own.
    pub fn require_above(&self, position: i32) -> app::Result<()> {
        if position < self.rank() {
            Ok(())
        } else {
            Err(app::Error::Forbidden(t!("permissionRank")))
        }
    }

    /// Refuses unless they hold every one of `permissions`, for giving them to a role or an
    /// override.
    pub fn require_holds(&self, permissions: Permissions) -> app::Result<()> {
        if self.permissions.contains(permissions) {
            Ok(())
        } else {
            Err(app::Error::Forbidden(t!("permissionNotHeld")))
        }
    }

    /// Their permissions in a channel of the community, after the category's and then the
    /// channel's overrides.
    pub fn in_channel(&self, category: &[Override], channel: &[Override]) -> Permissions {
        if self.owner {
            return Permissions::all();
        }
        let mut permissions = self.permissions;
        for layer in [category, channel] {
            permissions = self.apply(permissions, layer);
        }
        // No override hides a channel from a moderator.
        if self.moderator {
            permissions |= Permissions::VIEW_CHANNEL;
        }
        permissions
    }

    fn apply(&self, permissions: Permissions, overrides: &[Override]) -> Permissions {
        let everyone = self.roles.iter().find(|r| r.everyone).map(|r| r.id);
        let mut permissions = permissions;
        if let Some(o) = overrides.iter().find(|o| Some(o.role) == everyone) {
            permissions =
                (permissions & !(o.deny & Permissions::CHANNEL)) | (o.allow & Permissions::CHANNEL);
        }
        let (deny, allow) = overrides
            .iter()
            .filter(|o| Some(o.role) != everyone && self.roles.iter().any(|r| r.id == o.role))
            .fold(
                (Permissions::empty(), Permissions::empty()),
                |(deny, allow), o| (deny | o.deny, allow | o.allow),
            );
        (permissions & !(deny & Permissions::CHANNEL)) | (allow & Permissions::CHANNEL)
    }
}

/// The error for a permission the caller lacks, naming it.
pub fn missing(permission: Permissions) -> app::Error {
    app::Error::Forbidden(t!("permissionMissing", permission = describe(permission)))
}

/// A permission's name, as the error for lacking it says it.
pub fn describe(permission: Permissions) -> std::borrow::Cow<'static, str> {
    let key = match permission {
        Permissions::MANAGE_COMMUNITY => "permissionManageCommunity",
        Permissions::MANAGE_CHANNELS => "permissionManageChannels",
        Permissions::MANAGE_CATEGORIES => "permissionManageCategories",
        Permissions::CREATE_INVITES => "permissionCreateInvites",
        Permissions::MANAGE_INVITES => "permissionManageInvites",
        Permissions::MANAGE_ROLES => "permissionManageRoles",
        Permissions::ASSIGN_ROLES => "permissionAssignRoles",
        Permissions::REMOVE_MEMBERS => "permissionRemoveMembers",
        Permissions::MANAGE_MESSAGES => "permissionManageMessages",
        Permissions::PIN_MESSAGES => "permissionPinMessages",
        Permissions::MANAGE_CALLS => "permissionManageCalls",
        Permissions::VIEW_CHANNEL => "permissionViewChannel",
        Permissions::SEND_MESSAGES => "permissionSendMessages",
        Permissions::ATTACH_FILES => "permissionAttachFiles",
        Permissions::ADD_REACTIONS => "permissionAddReactions",
        Permissions::START_THREADS => "permissionStartThreads",
        Permissions::SEND_IN_THREADS => "permissionSendInThreads",
        Permissions::CREATE_POLLS => "permissionCreatePolls",
        Permissions::JOIN_VOICE => "permissionJoinVoice",
        Permissions::SPEAK => "permissionSpeak",
        Permissions::SHARE_SCREEN => "permissionShareScreen",
        _ => "permissionOwner",
    };
    t!(key)
}

/// What the caller may do in one channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelAccess {
    pub channel: ChannelId,
    /// The community's access, for a community channel; `None` in a DM.
    pub community: Option<CommunityAccess>,
    pub permissions: Permissions,
    /// Whether the channel is a thread, whose posting takes `SEND_IN_THREADS`.
    pub thread: bool,
    /// Whether the caller reads a DM they are not in, by Moderate any community.
    pub dm_moderator: bool,
    /// Whether this is a one-to-one DM with a block between its two people, either way
    /// (`app::block`): they may read it and take their own messages out of it, and nothing
    /// else.
    pub blocked: bool,
}

impl ChannelAccess {
    pub fn has(&self, permission: Permissions) -> bool {
        self.permissions.contains(permission)
    }

    pub fn require(&self, permission: Permissions) -> app::Result<()> {
        if self.has(permission) {
            Ok(())
        } else if self.blocked {
            Err(app::Error::Blocked)
        } else {
            Err(missing(permission))
        }
    }

    /// Refuses anything that would reach the other person of a blocked DM: editing a message,
    /// pinning, voting. What takes a permission is refused by `require` instead.
    pub fn ensure_unblocked(&self) -> app::Result<()> {
        if self.blocked {
            Err(app::Error::Blocked)
        } else {
            Ok(())
        }
    }

    /// What posting a message here takes: sending in threads for a thread, sending messages
    /// elsewhere.
    pub fn send_permission(&self) -> Permissions {
        if self.thread {
            Permissions::SEND_IN_THREADS
        } else {
            Permissions::SEND_MESSAGES
        }
    }

    /// Whether the caller may act on what someone else posted here with a community
    /// permission, such as deleting their message.
    pub fn community_has(&self, permission: Permissions) -> bool {
        self.community.as_ref().is_some_and(|c| c.has(permission))
            || (self.dm_moderator && MODERATION.contains(permission))
    }

    /// Whether acting with `permission` here is moderation, to be logged.
    pub fn moderating(&self, permission: Permissions) -> bool {
        self.dm_moderator
            || self
                .community
                .as_ref()
                .is_some_and(|c| c.moderating(permission))
    }
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = community_role)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct RoleRow {
    id: RoleId,
    position: i32,
    permissions: Permissions,
    everyone: bool,
}

impl From<RoleRow> for RoleGrant {
    fn from(row: RoleRow) -> Self {
        RoleGrant {
            id: row.id,
            position: row.position,
            permissions: row.permissions,
            everyone: row.everyone,
        }
    }
}

/// What `user` may do across `community`; `None` when they are neither a member nor a deployment
/// moderator.
pub async fn community_access(
    conn: &mut AsyncPgConnection,
    user: UserId,
    community_id: CommunityId,
) -> app::Result<Option<CommunityAccess>> {
    let member: bool = diesel::select(diesel::dsl::exists(
        community_user::table.filter(
            community_user::user
                .eq(user)
                .and(community_user::community.eq(community_id)),
        ),
    ))
    .get_result(conn)
    .await?;
    let moderator = app::deployment::is_moderator(conn, user).await?;
    if !member && !moderator {
        return Ok(None);
    }
    let owner: Option<Option<UserId>> = community::table
        .select(community::owner)
        .filter(
            community::id
                .eq(community_id)
                .and(community::deleted_at.is_null()),
        )
        .first(conn)
        .await
        .optional()?;
    let Some(owner) = owner else {
        return Ok(None);
    };
    let roles: Vec<RoleRow> = if member {
        let held = community_member_role::table
            .filter(
                community_member_role::user
                    .eq(user)
                    .and(community_member_role::community.eq(community_id)),
            )
            .select(community_member_role::role);
        community_role::table
            .select(RoleRow::as_select())
            .filter(community_role::community.eq(community_id))
            .filter(
                community_role::everyone
                    .eq(true)
                    .or(community_role::id.eq_any(held)),
            )
            .load(conn)
            .await?
    } else {
        Vec::new()
    };
    let mut access = CommunityAccess::resolve(
        user,
        community_id,
        owner == Some(user),
        roles.into_iter().map(RoleGrant::from).collect(),
    );
    access.member = member;
    Ok(Some(if moderator {
        access.with_moderation()
    } else {
        access
    }))
}

/// What a member of `community` may do there, refusing as not found anyone who is not one,
/// deployment moderators included: for the people an action is done to.
pub async fn require_actual_member(
    conn: &mut AsyncPgConnection,
    user: UserId,
    community_id: CommunityId,
) -> app::Result<CommunityAccess> {
    match community_access(conn, user, community_id).await? {
        Some(access) if access.member => Ok(access),
        _ => Err(app::Error::Diesel(diesel::result::Error::NotFound)),
    }
}

/// What `user` may do across `community`, refusing as not found when they are neither a member
/// nor a deployment moderator.
pub async fn require_member(
    conn: &mut AsyncPgConnection,
    user: UserId,
    community_id: CommunityId,
) -> app::Result<CommunityAccess> {
    community_access(conn, user, community_id)
        .await?
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))
}

async fn overrides_of_channel(
    conn: &mut AsyncPgConnection,
    channel_id: ChannelId,
) -> app::Result<Vec<Override>> {
    Ok(channel_override::table
        .select((
            channel_override::role,
            channel_override::allow,
            channel_override::deny,
        ))
        .filter(channel_override::channel.eq(channel_id))
        .load(conn)
        .await?)
}

async fn overrides_of_category(
    conn: &mut AsyncPgConnection,
    category_id: CategoryId,
) -> app::Result<Vec<Override>> {
    Ok(category_override::table
        .select((
            category_override::role,
            category_override::allow,
            category_override::deny,
        ))
        .filter(category_override::category.eq(category_id))
        .load(conn)
        .await?)
}

/// What `user` may do in `channel_id`. A channel they may not view, in a community they are
/// not in, or a DM they are not a recipient of is answered as not found.
/// Whether `dm` is a one-to-one DM whose other person and `user` have a block between them,
/// either way.
async fn dm_blocked(
    conn: &mut AsyncPgConnection,
    dm: ChannelId,
    user: UserId,
) -> app::Result<bool> {
    let other: Option<UserId> = dm_recipient::table
        .inner_join(channel::table.on(channel::id.eq(dm_recipient::channel)))
        .select(dm_recipient::user)
        .filter(
            dm_recipient::channel
                .eq(dm)
                .and(dm_recipient::user.ne(user))
                .and(channel::ty.eq(ChannelType::Dm)),
        )
        .first(conn)
        .await
        .optional()?;
    match other {
        Some(other) => app::block::any_between(conn, &[user, other]).await,
        None => Ok(false),
    }
}

pub async fn channel_access(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user: UserId,
    channel_id: ChannelId,
) -> app::Result<ChannelAccess> {
    let not_found = || app::Error::Diesel(diesel::result::Error::NotFound);
    let (parent, category, thread): (Option<ChannelId>, Option<CategoryId>, bool) = {
        let (parent, category): (Option<ChannelId>, Option<CategoryId>) = channel::table
            .select((channel::parent_channel, channel::parent_category))
            .filter(
                channel::id
                    .eq(channel_id)
                    .and(channel::deleted_at.is_null()),
            )
            .first(conn)
            .await
            .optional()?
            .ok_or_else(not_found)?;
        (parent, category, parent.is_some())
    };
    // A thread's permissions are its parent channel's.
    let governing = parent.unwrap_or(channel_id);
    let category = if parent.is_some() {
        channel::table
            .select(channel::parent_category)
            .filter(channel::id.eq(governing))
            .first(conn)
            .await
            .optional()?
            .flatten()
    } else {
        category
    };
    match channel_home(state, conn, channel_id).await? {
        ChannelHome::Direct(dm) => {
            let recipient: bool = diesel::select(diesel::dsl::exists(
                dm_recipient::table.filter(
                    dm_recipient::channel
                        .eq(dm)
                        .and(dm_recipient::user.eq(user)),
                ),
            ))
            .get_result(conn)
            .await?;
            if !recipient {
                // A deployment moderator reads any DM, and may take things out of it.
                if app::deployment::is_moderator(conn, user).await? {
                    return Ok(ChannelAccess {
                        channel: channel_id,
                        community: None,
                        permissions: Permissions::VIEW_CHANNEL,
                        thread,
                        dm_moderator: true,
                        blocked: false,
                    });
                }
                return Err(not_found());
            }
            let blocked = dm_blocked(conn, dm, user).await?;
            Ok(ChannelAccess {
                channel: channel_id,
                community: None,
                permissions: if blocked {
                    Permissions::VIEW_CHANNEL
                } else {
                    Permissions::CHANNEL
                },
                thread,
                dm_moderator: false,
                blocked,
            })
        }
        ChannelHome::Community {
            community: community_id,
            ..
        } => {
            let access = community_access(conn, user, community_id)
                .await?
                .ok_or_else(not_found)?;
            let category_overrides = match category {
                Some(category) => overrides_of_category(conn, category).await?,
                None => Vec::new(),
            };
            let channel_overrides = overrides_of_channel(conn, governing).await?;
            let permissions = access.in_channel(&category_overrides, &channel_overrides);
            if !permissions.contains(Permissions::VIEW_CHANNEL) {
                return Err(not_found());
            }
            Ok(ChannelAccess {
                channel: channel_id,
                community: Some(access),
                permissions,
                thread,
                dm_moderator: false,
                blocked: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn role(n: u128, position: i32, permissions: Permissions, everyone: bool) -> RoleGrant {
        RoleGrant {
            id: RoleId(Uuid::from_u128(n)),
            position,
            permissions,
            everyone,
        }
    }

    fn member(roles: Vec<RoleGrant>) -> CommunityAccess {
        CommunityAccess::resolve(
            UserId(Uuid::from_u128(100)),
            CommunityId(Uuid::from_u128(200)),
            false,
            roles,
        )
    }

    const EVERYONE: u128 = 1;
    const MODERATOR: u128 = 2;

    #[test]
    fn a_member_holds_the_union_of_their_roles() {
        let access = member(vec![
            role(EVERYONE, 0, Permissions::MEMBER_TEMPLATE, true),
            role(MODERATOR, 1, Permissions::MANAGE_MESSAGES, false),
        ]);
        assert!(access.has(Permissions::SEND_MESSAGES));
        assert!(access.has(Permissions::MANAGE_MESSAGES));
        assert!(!access.has(Permissions::MANAGE_ROLES));
        assert_eq!(access.rank(), 1);
    }

    #[test]
    fn the_owner_holds_everything_whatever_the_overrides() {
        let access = CommunityAccess::resolve(
            UserId(Uuid::from_u128(100)),
            CommunityId(Uuid::from_u128(200)),
            true,
            vec![role(EVERYONE, 0, Permissions::empty(), true)],
        );
        let deny_all = Override {
            role: RoleId(Uuid::from_u128(EVERYONE)),
            allow: Permissions::empty(),
            deny: Permissions::CHANNEL,
        };
        assert_eq!(access.in_channel(&[], &[deny_all]), Permissions::all());
        assert_eq!(access.rank(), OWNER_RANK);
    }

    #[test]
    fn a_role_allowance_wins_over_a_denial_for_everyone() {
        let access = member(vec![
            role(EVERYONE, 0, Permissions::MEMBER_TEMPLATE, true),
            role(MODERATOR, 1, Permissions::empty(), false),
        ]);
        let hidden = [
            Override {
                role: RoleId(Uuid::from_u128(EVERYONE)),
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
            },
            Override {
                role: RoleId(Uuid::from_u128(MODERATOR)),
                allow: Permissions::VIEW_CHANNEL,
                deny: Permissions::empty(),
            },
        ];
        assert!(
            access
                .in_channel(&[], &hidden)
                .contains(Permissions::VIEW_CHANNEL)
        );
        let everyone_only = member(vec![role(EVERYONE, 0, Permissions::MEMBER_TEMPLATE, true)]);
        assert!(
            !everyone_only
                .in_channel(&[], &hidden)
                .contains(Permissions::VIEW_CHANNEL)
        );
    }

    #[test]
    fn a_channel_override_follows_its_categorys() {
        let access = member(vec![role(EVERYONE, 0, Permissions::MEMBER_TEMPLATE, true)]);
        let everyone = RoleId(Uuid::from_u128(EVERYONE));
        let read_only_category = [Override {
            role: everyone,
            allow: Permissions::empty(),
            deny: Permissions::SEND_MESSAGES,
        }];
        let open_channel = [Override {
            role: everyone,
            allow: Permissions::SEND_MESSAGES,
            deny: Permissions::empty(),
        }];
        assert!(
            !access
                .in_channel(&read_only_category, &[])
                .contains(Permissions::SEND_MESSAGES)
        );
        assert!(
            access
                .in_channel(&read_only_category, &open_channel)
                .contains(Permissions::SEND_MESSAGES)
        );
    }

    #[test]
    fn overrides_never_touch_community_permissions() {
        let access = member(vec![role(EVERYONE, 0, Permissions::MEMBER_TEMPLATE, true)]);
        let grab = [Override {
            role: RoleId(Uuid::from_u128(EVERYONE)),
            allow: Permissions::MANAGE_ROLES,
            deny: Permissions::CREATE_INVITES,
        }];
        let permissions = access.in_channel(&[], &grab);
        assert!(!permissions.contains(Permissions::MANAGE_ROLES));
        assert!(permissions.contains(Permissions::CREATE_INVITES));
    }

    #[test]
    fn rank_decides_whom_a_member_may_act_on() {
        let access = member(vec![
            role(EVERYONE, 0, Permissions::MEMBER_TEMPLATE, true),
            role(MODERATOR, 3, Permissions::MANAGE_ROLES, false),
        ]);
        assert!(access.require_above(2).is_ok());
        assert!(access.require_above(3).is_err());
        assert!(access.require_holds(Permissions::MANAGE_ROLES).is_ok());
        assert!(access.require_holds(Permissions::ASSIGN_ROLES).is_err());
    }

    #[test]
    fn names_parse_back() {
        for &p in Permission::ALL {
            assert_eq!(p.to_string().parse::<Permission>().unwrap(), p);
        }
        assert_eq!(Permission::ManageInvites.to_string(), "manageInvites");
        assert!("manage_invites".parse::<Permission>().is_err());
    }

    #[test]
    fn every_permission_has_one_name_and_back() {
        assert_eq!(from_names(Permission::ALL), Permissions::all());
        assert_eq!(to_names(Permissions::all()), Permission::ALL.to_vec());
        let distinct: std::collections::HashSet<i64> =
            Permission::ALL.iter().map(|p| p.bits().bits()).collect();
        assert_eq!(distinct.len(), Permission::ALL.len());
    }

    #[test]
    fn the_templates_match_the_numbers_migrations_write() {
        assert_eq!(Permissions::MEMBER_TEMPLATE.bits(), 67_043_336);
        assert_eq!(Permissions::MODERATOR_TEMPLATE.bits(), 67_045_272);
        assert_eq!(Permissions::ADMIN_TEMPLATE.bits(), 67_045_375);
    }

    /// The cases in `spec/permission_vectors.json`, which the client's resolver also runs.
    #[test]
    fn the_shared_vectors_resolve_as_written() {
        #[derive(serde::Deserialize)]
        struct Vectors {
            cases: Vec<Case>,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Case {
            name: String,
            roles: Vec<VectorRole>,
            owner: bool,
            moderator: bool,
            holds: Option<Vec<Uuid>>,
            category_overrides: Vec<VectorOverride>,
            channel_overrides: Vec<VectorOverride>,
            community: Vec<Permission>,
            channel: Vec<Permission>,
            rank: i32,
        }
        #[derive(serde::Deserialize)]
        struct VectorRole {
            id: Uuid,
            position: i32,
            permissions: Vec<Permission>,
            everyone: bool,
        }
        #[derive(serde::Deserialize)]
        struct VectorOverride {
            role: Uuid,
            allow: Vec<Permission>,
            deny: Vec<Permission>,
        }
        let overrides = |list: &[VectorOverride]| -> Vec<Override> {
            list.iter()
                .map(|o| Override {
                    role: RoleId(o.role),
                    allow: from_names(&o.allow),
                    deny: from_names(&o.deny),
                })
                .collect()
        };
        let vectors: Vectors =
            serde_json::from_str(include_str!("../../../spec/permission_vectors.json"))
                .expect("the vectors parse");
        assert!(!vectors.cases.is_empty());
        for case in vectors.cases {
            // Someone who is not a member holds no role, not even everyone's.
            let roles = match &case.holds {
                None => Vec::new(),
                Some(holds) => case
                    .roles
                    .iter()
                    .filter(|r| r.everyone || holds.contains(&r.id))
                    .map(|r| role_grant(r.id, r.position, &r.permissions, r.everyone))
                    .collect(),
            };
            let access = CommunityAccess::resolve(
                UserId(Uuid::from_u128(100)),
                CommunityId(Uuid::from_u128(200)),
                case.owner,
                roles,
            );
            let access = if case.moderator {
                access.with_moderation()
            } else {
                access
            };
            assert_eq!(
                to_names(access.permissions),
                case.community,
                "{}",
                case.name
            );
            assert_eq!(
                to_names(access.in_channel(
                    &overrides(&case.category_overrides),
                    &overrides(&case.channel_overrides)
                )),
                case.channel,
                "{}",
                case.name
            );
            assert_eq!(access.rank(), case.rank, "{}", case.name);
        }
    }

    fn role_grant(
        id: Uuid,
        position: i32,
        permissions: &[Permission],
        everyone: bool,
    ) -> RoleGrant {
        RoleGrant {
            id: RoleId(id),
            position,
            permissions: from_names(permissions),
            everyone,
        }
    }
}
