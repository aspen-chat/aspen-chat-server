//! What people may do across the whole deployment, rather than in one community: open the
//! Administration Dashboard, manage registration invites, voice servers, and the deployment's
//! own roles, bots, and federation with other deployments, and moderate any community.
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
    self, AttachmentId, ChannelId, CommunityId, DeploymentRoleId, EventScope, MessageId, PollId,
    UserId, publish_event,
};
use crate::database::schema::{deployment_role, moderation_log, user, user_deployment_role};
use crate::t;
use diesel::prelude::*;
use diesel::{AsExpression, FromSqlRow};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
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
    }
}

app::bigint_sql_traits!(DeploymentPermissions);

impl DeploymentPermissions {
    /// What the terminal's `admin grant` gives: everything but moderation, which is given
    /// deliberately.
    pub const ADMINISTRATOR: Self = Self::all().difference(Self::MODERATE_COMMUNITIES);

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

/// One action for the moderation log, named there as `spec/moderation_actions.json` lists it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::IntoStaticStr, strum::VariantArray, strum::EnumString,
)]
#[strum(serialize_all = "camelCase")]
pub enum ModerationAction {
    ReadDm,
    DeleteMessage,
    RemoveAttachment,
    RemoveReaction,
    RemoveMember,
    BanMember,
    LiftBan,
    /// A ban's deletion of the person's recent messages; the subject is the person.
    DeleteRecentMessages,
    RenameChannel,
    DeleteChannel,
    RenameCommunity,
    DeleteCommunity,
    RemoveWriteIn,
    BanForeignUser,
    LiftForeignUserBan,
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
    let action: &'static str = action.into();
    tracing::info!(
        actor = %actor.0,
        action,
        community = ?community.map(|c| c.0),
        channel = ?channel.map(|c| c.0),
        subject = ?subject,
        "deployment moderation"
    );
    diesel::insert_into(moderation_log::table)
        .values((
            moderation_log::id.eq(uuid::Uuid::now_v7()),
            moderation_log::actor.eq(Some(actor)),
            moderation_log::action.eq(action),
            moderation_log::community.eq(community),
            moderation_log::channel.eq(channel),
            moderation_log::subject.eq(subject),
        ))
        .execute(conn)
        .await?;
    Ok(())
}

/// One entry of the moderation log, with what its ids name.
#[derive(Debug, Clone)]
pub struct ModerationEntry {
    pub id: uuid::Uuid,
    pub actor: Option<UserId>,
    pub action: String,
    pub community: Option<CommunityId>,
    pub channel: Option<ChannelId>,
    pub subject: Option<String>,
    pub at: chrono::DateTime<chrono::Utc>,
    pub details: ModerationDetails,
}

/// What an entry of the moderation log names, as it stands when the log is read, so its reader
/// sees names rather than ids. Whatever is no longer found is left out, and the entry's own ids
/// remain.
#[derive(Debug, Clone, Default, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModerationDetails {
    pub community: Option<LoggedCommunity>,
    pub channel: Option<LoggedChannel>,
    /// The person acted on: a member removed, a user of another deployment banned or let back,
    /// or whoever's reaction was removed.
    pub user: Option<UserId>,
    /// The message acted on or read: deleted, stripped of an attachment or a reaction, or the
    /// poll a write-in was taken from.
    pub message: Option<LoggedMessage>,
    /// The reaction removed.
    pub emoji: Option<String>,
    /// The removed attachment's file name.
    pub attachment: Option<String>,
    /// The removed write-in's text.
    pub write_in: Option<String>,
    /// The name a community or channel was given.
    pub renamed_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoggedCommunity {
    pub name: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoggedChannel {
    pub name: String,
    pub ty: crate::api::ChannelType,
    pub deleted: bool,
    /// The channel a thread belongs to.
    pub parent_channel: Option<ChannelId>,
    /// A DM's people, who name it, since a DM has no name of its own.
    pub recipients: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoggedMessage {
    pub id: MessageId,
    pub channel: ChannelId,
    pub author: UserId,
    pub deleted: bool,
}

/// What an entry's `subject` names, which depends on its action.
#[derive(Debug, Clone, PartialEq)]
enum Subject {
    User(UserId),
    Message(MessageId),
    Attachment(MessageId, AttachmentId),
    Reaction(MessageId, String, UserId),
    WriteIn(PollId, i32),
    Name(String),
}

/// Reads a `subject` as `log_moderation`'s callers write it for `action`.
fn subject_of(action: &str, subject: &str) -> Option<Subject> {
    let id = |text: &str| uuid::Uuid::parse_str(text).ok();
    match action.parse::<ModerationAction>().ok()? {
        ModerationAction::RemoveMember
        | ModerationAction::BanMember
        | ModerationAction::LiftBan
        | ModerationAction::DeleteRecentMessages
        | ModerationAction::BanForeignUser
        | ModerationAction::LiftForeignUserBan => Some(Subject::User(UserId(id(subject)?))),
        ModerationAction::DeleteMessage | ModerationAction::ReadDm => {
            Some(Subject::Message(MessageId(id(subject)?)))
        }
        ModerationAction::RemoveAttachment => {
            let (message, attachment) = subject.split_once('/')?;
            Some(Subject::Attachment(
                MessageId(id(message)?),
                AttachmentId(id(attachment)?),
            ))
        }
        ModerationAction::RemoveReaction => {
            let (message, rest) = subject.split_once('/')?;
            let (emoji, author) = rest.rsplit_once('/')?;
            Some(Subject::Reaction(
                MessageId(id(message)?),
                emoji.to_string(),
                UserId(id(author)?),
            ))
        }
        ModerationAction::RemoveWriteIn => {
            let (poll, option) = subject.split_once('/')?;
            Some(Subject::WriteIn(PollId(id(poll)?), option.parse().ok()?))
        }
        ModerationAction::RenameChannel | ModerationAction::RenameCommunity => {
            Some(Subject::Name(subject.to_string()))
        }
        ModerationAction::DeleteChannel | ModerationAction::DeleteCommunity => None,
    }
}

/// The newest entries of the moderation log, before `before` when given, each with what its
/// ids name, read one query per kind of record for the whole page.
pub async fn read_moderation_log(
    state: &GlobalServerContext,
    before: Option<uuid::Uuid>,
    limit: i64,
) -> app::Result<Vec<ModerationEntry>> {
    use crate::database::schema::{
        attachment, channel, community, dm_recipient, message, poll_option,
    };
    type Row = (
        uuid::Uuid,
        Option<UserId>,
        String,
        Option<CommunityId>,
        Option<ChannelId>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
    );
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
    let rows: Vec<Row> = query.load(conn.as_mut()).await?;
    let subjects: Vec<Option<Subject>> = rows
        .iter()
        .map(|(_, _, action, _, _, subject, _)| {
            subject.as_deref().and_then(|s| subject_of(action, s))
        })
        .collect();

    let community_ids: Vec<CommunityId> = rows.iter().filter_map(|r| r.3).collect();
    let communities: HashMap<CommunityId, LoggedCommunity> = community::table
        .select((
            community::id,
            community::name,
            community::deleted_at.is_not_null(),
        ))
        .filter(community::id.eq_any(&community_ids))
        .load::<(CommunityId, String, bool)>(conn.as_mut())
        .await?
        .into_iter()
        .map(|(id, name, deleted)| (id, LoggedCommunity { name, deleted }))
        .collect();

    let mut message_ids: Vec<MessageId> = Vec::new();
    let mut poll_ids: Vec<PollId> = Vec::new();
    let mut attachment_ids: Vec<AttachmentId> = Vec::new();
    for subject in subjects.iter().flatten() {
        match subject {
            Subject::Message(id) | Subject::Reaction(id, _, _) => message_ids.push(*id),
            Subject::Attachment(id, attachment) => {
                message_ids.push(*id);
                attachment_ids.push(*attachment);
            }
            Subject::WriteIn(poll, _) => poll_ids.push(*poll),
            Subject::User(_) | Subject::Name(_) => {}
        }
    }
    let messages: Vec<(MessageId, ChannelId, UserId, bool, Option<PollId>)> = message::table
        .select((
            message::id,
            message::channel,
            message::author,
            message::deleted_at.is_not_null(),
            message::poll,
        ))
        .filter(
            message::id
                .eq_any(&message_ids)
                .or(message::poll.eq_any(&poll_ids)),
        )
        .load(conn.as_mut())
        .await?;
    let logged = |(id, channel, author, deleted, _): &(
        MessageId,
        ChannelId,
        UserId,
        bool,
        Option<PollId>,
    )| {
        LoggedMessage {
            id: *id,
            channel: *channel,
            author: *author,
            deleted: *deleted,
        }
    };
    let by_id: HashMap<MessageId, LoggedMessage> =
        messages.iter().map(|m| (m.0, logged(m))).collect();
    let by_poll: HashMap<PollId, LoggedMessage> = messages
        .iter()
        .filter_map(|m| m.4.map(|poll| (poll, logged(m))))
        .collect();
    let attachments: HashMap<AttachmentId, String> = attachment::table
        .select((attachment::id, attachment::file_name))
        .filter(attachment::id.eq_any(&attachment_ids))
        .load(conn.as_mut())
        .await?
        .into_iter()
        .collect();
    let write_ins: HashMap<(PollId, i32), String> = poll_option::table
        .select((poll_option::poll, poll_option::index, poll_option::label))
        .filter(poll_option::poll.eq_any(&poll_ids))
        .load::<(PollId, i32, String)>(conn.as_mut())
        .await?
        .into_iter()
        .map(|(poll, index, label)| ((poll, index), label))
        .collect();

    let channel_ids: Vec<ChannelId> = rows.iter().filter_map(|r| r.4).collect();
    let channels: Vec<(
        ChannelId,
        String,
        crate::api::ChannelType,
        bool,
        Option<ChannelId>,
    )> = channel::table
        .select((
            channel::id,
            channel::name,
            channel::ty,
            channel::deleted_at.is_not_null(),
            channel::parent_channel,
        ))
        .filter(channel::id.eq_any(&channel_ids))
        .load(conn.as_mut())
        .await?;
    let mut recipients: HashMap<ChannelId, Vec<UserId>> = HashMap::new();
    for (channel, user) in dm_recipient::table
        .select((dm_recipient::channel, dm_recipient::user))
        .filter(dm_recipient::channel.eq_any(&channel_ids))
        .order((dm_recipient::channel, dm_recipient::joined_at))
        .load::<(ChannelId, UserId)>(conn.as_mut())
        .await?
    {
        recipients.entry(channel).or_default().push(user);
    }
    let channels: HashMap<ChannelId, LoggedChannel> = channels
        .into_iter()
        .map(|(id, name, ty, deleted, parent_channel)| {
            let channel = LoggedChannel {
                name,
                ty,
                deleted,
                parent_channel,
                recipients: recipients.remove(&id).unwrap_or_default(),
            };
            (id, channel)
        })
        .collect();

    Ok(rows
        .into_iter()
        .zip(subjects)
        .map(
            |((id, actor, action, community, channel, subject, at), named)| {
                let mut details = ModerationDetails {
                    community: community.and_then(|c| communities.get(&c).cloned()),
                    channel: channel.and_then(|c| channels.get(&c).cloned()),
                    ..ModerationDetails::default()
                };
                match named {
                    Some(Subject::User(user)) => details.user = Some(user),
                    Some(Subject::Message(message)) => {
                        details.message = by_id.get(&message).cloned()
                    }
                    Some(Subject::Attachment(message, attachment)) => {
                        details.message = by_id.get(&message).cloned();
                        details.attachment = attachments.get(&attachment).cloned();
                    }
                    Some(Subject::Reaction(message, emoji, author)) => {
                        details.message = by_id.get(&message).cloned();
                        details.emoji = Some(emoji);
                        details.user = Some(author);
                    }
                    Some(Subject::Name(name)) => details.renamed_to = Some(name),
                    Some(Subject::WriteIn(poll, option)) => {
                        details.message = by_poll.get(&poll).cloned();
                        details.write_in = write_ins.get(&(poll, option)).cloned();
                    }
                    None => {}
                }
                ModerationEntry {
                    id,
                    actor,
                    action,
                    community,
                    channel,
                    subject,
                    at,
                    details,
                }
            },
        )
        .collect())
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
    pub permissions: DeploymentPermissions,
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
                permissions,
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
                access.require_holds(permissions.symmetric_difference(role.permissions))?;
                role.permissions = permissions;
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
                        permissions: DeploymentPermissions::ADMINISTRATOR,
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
        top.permissions | permission.bits()
    } else {
        top.permissions.difference(permission.bits())
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

    /// Each action's subject reads back as what its caller wrote, an emoji with a slash in its
    /// neighbours included, and a subject that does not parse names nothing.
    #[test]
    fn subjects_read_back_by_action() {
        let a = "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
        let b = "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5c";
        let id = |text: &str| uuid::Uuid::parse_str(text).unwrap();
        assert_eq!(
            subject_of("removeMember", a),
            Some(Subject::User(UserId(id(a))))
        );
        assert_eq!(
            subject_of("liftForeignUserBan", a),
            Some(Subject::User(UserId(id(a))))
        );
        assert_eq!(
            subject_of("readDm", a),
            Some(Subject::Message(MessageId(id(a))))
        );
        assert_eq!(
            subject_of("removeAttachment", &format!("{a}/{b}")),
            Some(Subject::Attachment(MessageId(id(a)), AttachmentId(id(b))))
        );
        assert_eq!(
            subject_of("removeReaction", &format!("{a}/👍🏽/{b}")),
            Some(Subject::Reaction(
                MessageId(id(a)),
                "👍🏽".into(),
                UserId(id(b))
            ))
        );
        assert_eq!(
            subject_of("removeWriteIn", &format!("{a}/3")),
            Some(Subject::WriteIn(PollId(id(a)), 3))
        );
        assert_eq!(subject_of("deleteMessage", "not an id"), None);
        assert_eq!(
            subject_of("renameChannel", "a new / name"),
            Some(Subject::Name("a new / name".into()))
        );
        assert_eq!(subject_of("deleteChannel", a), None);
        assert_eq!(subject_of("somethingNew", a), None);
    }

    /// The log's action names are the ones `spec/moderation_actions.json` lists, which the
    /// client names in its dashboard.
    #[test]
    fn moderation_actions_match_the_spec() {
        #[derive(serde::Deserialize)]
        struct Spec {
            actions: Vec<String>,
        }
        let spec: Spec =
            serde_json::from_str(include_str!("../../../spec/moderation_actions.json")).unwrap();
        let names: Vec<String> = <ModerationAction as strum::VariantArray>::VARIANTS
            .iter()
            .map(|action| <&str>::from(*action).to_string())
            .collect();
        assert_eq!(names, spec.actions);
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
        // The number the migration gives existing administrators.
        assert_eq!(DeploymentPermissions::ADMINISTRATOR.bits(), 111);
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
