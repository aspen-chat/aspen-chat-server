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
//! when an action was allowed by that alone, which the caller then logs. A DM they are not in
//! is not found to `channel_access`: only `channel_access_reading`, which logs the reading, and
//! `channel_access_moderating`, for taking things out of it, reach one.

use crate::channel::ChannelType;
use crate::events::{ChannelHome, channel_home};
use crate::t;
use crate::{CategoryId, ChannelId, CommunityId, RoleId, UserId};
use aspen_schema::{
    category_override, channel, channel_override, community, community_member_role, community_role,
    community_user, dm_recipient, user as user_table,
};
pub use aspen_wire::permissions::{Permission, Permissions, from_names, to_names};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

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
/// that take things away (deleting messages, attachments, reactions, and write-ins, removing
/// members, and clearing nicknames). Renaming and deleting channels and communities are checked by name where they are
/// done.
pub const MODERATION: Permissions = Permissions::VIEW_CHANNEL
    .union(Permissions::MANAGE_MESSAGES)
    .union(Permissions::REMOVE_MEMBERS)
    .union(Permissions::MANAGE_NICKNAMES);

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
    pub fn require(&self, permission: Permissions) -> crate::Result<()> {
        if self.has(permission) {
            Ok(())
        } else {
            Err(missing(permission))
        }
    }

    /// Refuses unless `position` ranks below their own.
    pub fn require_above(&self, position: i32) -> crate::Result<()> {
        if position < self.rank() {
            Ok(())
        } else {
            Err(crate::Error::Forbidden(t!("permissionRank")))
        }
    }

    /// Refuses unless `position` ranks below their own in the community alone (`role_rank`):
    /// for managing, giving, and taking roles and setting overrides, which hand on what roles
    /// allow, so moderating the deployment, which takes things away, does not reach them.
    pub fn require_role_above(&self, position: i32) -> crate::Result<()> {
        if position < self.role_rank() {
            Ok(())
        } else {
            Err(crate::Error::Forbidden(t!("permissionRank")))
        }
    }

    /// Refuses unless their roles allow every one of `permissions`, for giving them to a role
    /// or an override. What moderating the deployment adds is theirs to use, not to hand on.
    pub fn require_holds(&self, permissions: Permissions) -> crate::Result<()> {
        if self.member_permissions.contains(permissions) {
            Ok(())
        } else {
            Err(crate::Error::Forbidden(t!("permissionNotHeld")))
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
pub fn missing(permission: Permissions) -> crate::Error {
    crate::Error::Forbidden(t!("permissionMissing", permission = describe(permission)))
}

/// A permission's name, as the error for lacking it says it: one permission's own name, or the
/// owner's for anything more.
pub fn describe(permission: Permissions) -> std::borrow::Cow<'static, str> {
    let key = match Permission::ALL.iter().find(|p| p.bits() == permission) {
        Some(named) => match named {
            Permission::ManageCommunity => "permissionManageCommunity",
            Permission::ManageChannels => "permissionManageChannels",
            Permission::ManageCategories => "permissionManageCategories",
            Permission::CreateInvites => "permissionCreateInvites",
            Permission::ManageInvites => "permissionManageInvites",
            Permission::ManageRoles => "permissionManageRoles",
            Permission::AssignRoles => "permissionAssignRoles",
            Permission::RemoveMembers => "permissionRemoveMembers",
            Permission::ManageMessages => "permissionManageMessages",
            Permission::PinMessages => "permissionPinMessages",
            Permission::ManageCalls => "permissionManageCalls",
            Permission::AddBots => "permissionAddBots",
            Permission::ManageCustomEmoji => "permissionManageCustomEmoji",
            Permission::BanMembers => "permissionBanMembers",
            Permission::ChangeNickname => "permissionChangeNickname",
            Permission::ManageNicknames => "permissionManageNicknames",
            Permission::ManagePlugins => "permissionManagePlugins",
            Permission::ViewChannel => "permissionViewChannel",
            Permission::SendMessages => "permissionSendMessages",
            Permission::AttachFiles => "permissionAttachFiles",
            Permission::AddReactions => "permissionAddReactions",
            Permission::StartThreads => "permissionStartThreads",
            Permission::SendInThreads => "permissionSendInThreads",
            Permission::CreatePolls => "permissionCreatePolls",
            Permission::JoinVoice => "permissionJoinVoice",
            Permission::Speak => "permissionSpeak",
            Permission::ShareScreen => "permissionShareScreen",
            Permission::MentionMembers => "permissionMentionMembers",
            Permission::MentionRoles => "permissionMentionRoles",
            Permission::MentionEveryone => "permissionMentionEveryone",
            Permission::TransferFiles => "permissionTransferFiles",
            Permission::UseCamera => "permissionUseCamera",
        },
        None => "permissionOwner",
    };
    t!(key)
}

/// That a `ChannelAccess` came from `channel_access`: it has no public constructor, so one cannot
/// be made anywhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Checked;

/// What the caller may do in one channel, which they may view: only `channel_access` makes one
/// (and `into_unmade_thread`, from the parent's, for a thread its first reply makes), and it
/// refuses a channel they may not view, so holding one proves the check was made. A
/// function that reads or writes a channel's contents for someone takes theirs, or checks it
/// itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelAccess {
    pub channel: ChannelId,
    /// The community's access, for a community channel; `None` in a DM.
    pub community: Option<CommunityAccess>,
    pub permissions: Permissions,
    /// Whether the channel is a thread, whose posting takes `SEND_IN_THREADS`.
    pub thread: bool,
    /// The channel's type, read with the check, so callers need not read the channel again.
    pub ty: crate::channel::ChannelType,
    /// Whether the caller reads a DM they are not in, by Moderate any community, which only
    /// `channel_access_reading` and `channel_access_moderating` allow.
    pub dm_moderator: bool,
    /// Whether this is a one-to-one DM with a block between its two people, either way
    /// (`app::block`): they may read it and take their own messages out of it, and nothing
    /// else. A holder of Message any user is never blocked.
    pub blocked: bool,
    /// Whether this is a one-to-one DM from the system account (`app::system_account`), whose
    /// notices the person reads and cannot answer.
    pub from_system: bool,
    checked: Checked,
}

impl ChannelAccess {
    /// The access to `thread`, a thread of this channel not stored yet, which is made in the
    /// transaction that posts its first reply (`app::thread::open_in`): a thread's permissions
    /// are its parent's, as `channel_access` reads them for one that is stored.
    pub(crate) fn into_unmade_thread(self, thread: ChannelId) -> ChannelAccess {
        ChannelAccess {
            channel: thread,
            thread: true,
            ty: crate::channel::ChannelType::Thread,
            ..self
        }
    }

    pub fn has(&self, permission: Permissions) -> bool {
        self.permissions.contains(permission)
    }

    pub fn require(&self, permission: Permissions) -> crate::Result<()> {
        if self.has(permission) {
            Ok(())
        } else if self.blocked {
            Err(crate::Error::Blocked)
        } else if self.from_system {
            Err(crate::Error::Forbidden(t!("systemNoticesReadOnly")))
        } else {
            Err(missing(permission))
        }
    }

    /// Refuses anything that would reach the other person of a blocked DM: editing a message,
    /// pinning, voting. What takes a permission is refused by `require` instead.
    pub fn ensure_unblocked(&self) -> crate::Result<()> {
        if self.blocked {
            Err(crate::Error::Blocked)
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
    mut conn: &AsyncPgConnection,
    user: UserId,
    community_id: CommunityId,
) -> crate::Result<Option<CommunityAccess>> {
    // The four reads are independent, so they go to the database together, pipelined on the
    // one connection: one round trip rather than four.
    let member = diesel::select(diesel::dsl::exists(
        community_user::table.filter(
            community_user::user
                .eq(user)
                .and(community_user::community.eq(community_id)),
        ),
    ))
    .get_result::<bool>(&mut conn);
    let owner = community::table
        .select(community::owner)
        .filter(
            community::id
                .eq(community_id)
                .and(community::deleted_at.is_null()),
        )
        .load::<Option<UserId>>(&mut conn);
    // The everyone role and those held; someone not a member holds none, and the rows are then
    // set aside.
    let held = community_member_role::table
        .filter(
            community_member_role::user
                .eq(user)
                .and(community_member_role::community.eq(community_id)),
        )
        .select(community_member_role::role);
    let roles = community_role::table
        .select(RoleRow::as_select())
        .filter(community_role::community.eq(community_id))
        .filter(community_role::deleted_at.is_null())
        .filter(
            community_role::everyone
                .eq(true)
                .or(community_role::id.eq_any(held)),
        )
        .load::<RoleRow>(&mut conn);
    let (member, deployment, owner, roles) = futures_util::try_join!(
        async { Ok::<_, crate::Error>(member.await?) },
        crate::deployment::deployment_access(conn, user),
        async { Ok(owner.await?) },
        async { Ok(roles.await?) },
    )?;
    let moderator = deployment.has(crate::deployment::DeploymentPermission::ModerateCommunities);
    if !member && !moderator {
        return Ok(None);
    }
    let Some(owner) = owner.into_iter().next() else {
        return Ok(None);
    };
    let roles = if member { roles } else { Vec::new() };
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
) -> crate::Result<CommunityAccess> {
    match community_access(conn, user, community_id).await? {
        Some(access) if access.member => Ok(access),
        _ => Err(crate::Error::Diesel(diesel::result::Error::NotFound)),
    }
}

/// What `user` may do across `community`, refusing as not found when they are neither a member
/// nor a deployment moderator.
pub async fn require_member(
    conn: &mut AsyncPgConnection,
    user: UserId,
    community_id: CommunityId,
) -> crate::Result<CommunityAccess> {
    community_access(conn, user, community_id)
        .await?
        .ok_or(crate::Error::Diesel(diesel::result::Error::NotFound))
}

async fn overrides_of_channel(
    mut conn: &AsyncPgConnection,
    channel_id: ChannelId,
) -> crate::Result<Vec<Override>> {
    Ok(channel_override::table
        .select((
            channel_override::role,
            channel_override::allow,
            channel_override::deny,
        ))
        .filter(channel_override::channel.eq(channel_id))
        .load(&mut conn)
        .await?)
}

/// A live category's overrides; a deleted one's apply to nothing.
async fn overrides_of_category(
    mut conn: &AsyncPgConnection,
    category_id: CategoryId,
) -> crate::Result<Vec<Override>> {
    Ok(category_override::table
        .inner_join(aspen_schema::category::table)
        .select((
            category_override::role,
            category_override::allow,
            category_override::deny,
        ))
        .filter(
            category_override::category
                .eq(category_id)
                .and(aspen_schema::category::deleted_at.is_null()),
        )
        .load(&mut conn)
        .await?)
}

/// What `access`'s holder may do across `category`, a category of their community: their
/// permissions after the category's overrides alone, as in a channel of it with none of its own.
pub async fn in_category(
    conn: &AsyncPgConnection,
    access: &CommunityAccess,
    category: CategoryId,
) -> crate::Result<Permissions> {
    let overrides = overrides_of_category(conn, category).await?;
    Ok(access.in_channel(&overrides, &[]))
}

/// What stands between `user` and the other person of `dm`, when it is a one-to-one DM: a
/// block either way, and whether the other is the system account.
async fn dm_peer(
    conn: &mut AsyncPgConnection,
    dm: ChannelId,
    user: UserId,
) -> crate::Result<(bool, bool)> {
    let other: Option<(UserId, bool)> = dm_recipient::table
        .inner_join(channel::table.on(channel::id.eq(dm_recipient::channel)))
        .inner_join(user_table::table.on(user_table::id.eq(dm_recipient::user)))
        .select((dm_recipient::user, user_table::system))
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
        Some((other, system)) => {
            // Message any user reaches anyone, past a block either way.
            let blocked = crate::block::any_between(conn, &[user, other]).await?
                && !crate::deployment::deployment_access(conn, user)
                    .await?
                    .has(crate::deployment::DeploymentPermission::MessageAnyUser);
            Ok((blocked, system))
        }
        None => Ok((false, false)),
    }
}

/// How a deployment moderator who is not in a DM may reach it.
enum DmModerator {
    /// Not at all: the DM is not found to them.
    Refused,
    /// To read what is in it, `subject` naming what, which is logged as they are let in.
    Reading(Option<String>),
    /// To take something out of it, or read its own record, which their DM list already shows
    /// them; each caller logs what it does.
    Moderating,
}

/// What `user` may do in `channel_id`. A channel they may not view, in a community they are
/// not in, or a DM they are not a recipient of is answered as not found, deployment moderators
/// included: a path that lets them into DMs says so with `channel_access_reading` or
/// `channel_access_moderating`.
pub async fn channel_access(
    state: &impl crate::events::Publishing,
    conn: &mut AsyncPgConnection,
    user: UserId,
    channel_id: ChannelId,
) -> crate::Result<ChannelAccess> {
    access_to_channel(state, conn, user, channel_id, DmModerator::Refused).await
}

/// `channel_access` for reading what a channel holds, which also lets a deployment moderator
/// read a DM they are not in, writing each such reading to the moderation log (`ReadDm`, with
/// `subject` naming what was read) before anything is read. A read of several messages calls it
/// once per channel, so a DM is logged once however many of its messages were read.
pub async fn channel_access_reading(
    state: &impl crate::events::Publishing,
    conn: &mut AsyncPgConnection,
    user: UserId,
    channel_id: ChannelId,
    subject: Option<String>,
) -> crate::Result<ChannelAccess> {
    access_to_channel(state, conn, user, channel_id, DmModerator::Reading(subject)).await
}

/// `channel_access` for taking something out of a channel, or reading its own record, which also
/// lets a deployment moderator into a DM they are not in, holding `VIEW_CHANNEL` and what
/// `community_has` gives them there. The caller logs what they do, as moderation.
pub async fn channel_access_moderating(
    state: &impl crate::events::Publishing,
    conn: &mut AsyncPgConnection,
    user: UserId,
    channel_id: ChannelId,
) -> crate::Result<ChannelAccess> {
    access_to_channel(state, conn, user, channel_id, DmModerator::Moderating).await
}

async fn access_to_channel(
    state: &impl crate::events::Publishing,
    conn: &mut AsyncPgConnection,
    user: UserId,
    channel_id: ChannelId,
    dm_moderator: DmModerator,
) -> crate::Result<ChannelAccess> {
    let not_found = || crate::Error::Diesel(diesel::result::Error::NotFound);
    // The channel and, for a thread, its parent, in one read: a thread's permissions are its
    // parent channel's, and it goes with its parent, so a thread of a deleted channel is not
    // found.
    #[derive(diesel::QueryableByName)]
    struct Found {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
        parent: Option<ChannelId>,
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
        category: Option<CategoryId>,
        #[diesel(sql_type = aspen_schema::sql_types::ChannelType)]
        ty: crate::channel::ChannelType,
    }
    let found: Found = diesel::sql_query(
        "SELECT c.parent_channel AS parent, \
                CASE WHEN c.parent_channel IS NULL THEN c.parent_category \
                     ELSE p.parent_category END AS category, \
                c.ty \
         FROM channel c LEFT JOIN channel p ON p.id = c.parent_channel \
         WHERE c.id = $1 AND c.deleted_at IS NULL \
           AND (c.parent_channel IS NULL OR (p.id IS NOT NULL AND p.deleted_at IS NULL))",
    )
    .bind::<diesel::sql_types::Uuid, _>(channel_id.0)
    .get_result(conn)
    .await
    .optional()?
    .ok_or_else(not_found)?;
    let (parent, category, thread) = (found.parent, found.category, found.parent.is_some());
    let ty = found.ty;
    let governing = parent.unwrap_or(channel_id);
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
                // A deployment moderator reads any DM, logged, and may take things out of it.
                if !matches!(dm_moderator, DmModerator::Refused)
                    && crate::deployment::is_moderator(conn, user).await?
                {
                    if let DmModerator::Reading(subject) = dm_moderator {
                        crate::moderation_log::log_moderation(
                            conn,
                            user,
                            crate::moderation_log::ModerationAction::ReadDm,
                            None,
                            Some(channel_id),
                            subject,
                        )
                        .await?;
                    }
                    return Ok(ChannelAccess {
                        channel: channel_id,
                        community: None,
                        permissions: Permissions::VIEW_CHANNEL,
                        thread,
                        dm_moderator: true,
                        blocked: false,
                        from_system: false,
                        ty,
                        checked: Checked,
                    });
                }
                return Err(not_found());
            }
            let (blocked, from_system) = dm_peer(conn, dm, user).await?;
            Ok(ChannelAccess {
                channel: channel_id,
                community: None,
                permissions: if blocked || from_system {
                    Permissions::VIEW_CHANNEL
                } else {
                    Permissions::CHANNEL
                },
                thread,
                dm_moderator: false,
                blocked,
                from_system,
                ty,
                checked: Checked,
            })
        }
        ChannelHome::Community {
            community: community_id,
            ..
        } => {
            // Pipelined together, as in `community_access`.
            let conn: &AsyncPgConnection = conn;
            let (access, category_overrides, channel_overrides) = futures_util::try_join!(
                community_access(conn, user, community_id),
                async {
                    match category {
                        Some(category) => overrides_of_category(conn, category).await,
                        None => Ok(Vec::new()),
                    }
                },
                overrides_of_channel(conn, governing),
            )?;
            let access = access.ok_or_else(not_found)?;
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
                from_system: false,
                ty,
                checked: Checked,
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
    fn moderating_the_deployment_hands_nothing_on() {
        let access = member(vec![
            role(EVERYONE, 0, Permissions::MEMBER_TEMPLATE, true),
            role(MODERATOR, 3, Permissions::ASSIGN_ROLES, false),
        ])
        .with_moderation();
        // A moderator acts on anyone below the owner, yet gives and manages roles only below
        // their own, and hands on only what their roles allow.
        assert!(access.require_above(5).is_ok());
        assert!(access.require_role_above(2).is_ok());
        assert!(access.require_role_above(3).is_err());
        assert!(access.has(Permissions::MANAGE_MESSAGES));
        assert!(access.require_holds(Permissions::MANAGE_MESSAGES).is_err());
        assert!(access.require_holds(Permissions::ASSIGN_ROLES).is_ok());
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
        assert_eq!(Permissions::MEMBER_TEMPLATE.bits(), 114_344_914_337_800);
        assert_eq!(Permissions::MODERATOR_TEMPLATE.bits(), 140_733_193_453_464);
        assert_eq!(Permissions::ADMIN_TEMPLATE.bits(), 140_733_193_519_103);
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
