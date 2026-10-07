//! Community permissions: the set the database stores, and their names on the wire.

use diesel::{AsExpression, FromSqlRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

bitflags::bitflags! {
    /// A set of permissions, as the bits the database stores. The values are fixed: migrations
    /// write them.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, FromSqlRow, AsExpression)]
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pub struct Permissions: i64 {
        // Across the community, bits 0 to 31.
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
        const ADD_BOTS = 1 << 11;
        const MANAGE_CUSTOM_EMOJI = 1 << 12;
        const BAN_MEMBERS = 1 << 13;
        const CHANGE_NICKNAME = 1 << 14;
        const MANAGE_NICKNAMES = 1 << 15;
        const MANAGE_PLUGINS = 1 << 16;

        // In a channel, and adjustable per channel and category, bits 32 to 62.
        const VIEW_CHANNEL = 1 << 32;
        const SEND_MESSAGES = 1 << 33;
        const ATTACH_FILES = 1 << 34;
        const ADD_REACTIONS = 1 << 35;
        const START_THREADS = 1 << 36;
        const SEND_IN_THREADS = 1 << 37;
        const CREATE_POLLS = 1 << 38;
        const JOIN_VOICE = 1 << 39;
        const SPEAK = 1 << 40;
        const SHARE_SCREEN = 1 << 41;
        const MENTION_MEMBERS = 1 << 42;
        const MENTION_ROLES = 1 << 43;
        const MENTION_EVERYONE = 1 << 44;
        const TRANSFER_FILES = 1 << 45;
        const USE_CAMERA = 1 << 46;
    }
}

crate::bigint_sql_traits!(Permissions);

impl Permissions {
    /// Every permission that holds across the community: those named in bits 0 to 31.
    pub const COMMUNITY: Self = Self::all().intersection(Self::from_bits_retain((1 << 32) - 1));
    /// Every permission an override may adjust: those named in bits 32 to 62.
    pub const CHANNEL: Self =
        Self::all().intersection(Self::from_bits_retain(i64::MAX & !((1 << 32) - 1)));

    /// The everyone role of a new community: taking part, tagging one another, inviting
    /// others, and choosing a nickname. Tagging roles and everyone at once is left to
    /// moderators.
    pub const MEMBER_TEMPLATE: Self = Self::CHANNEL
        .difference(Self::MENTION_ROLES)
        .difference(Self::MENTION_EVERYONE)
        .union(Self::CREATE_INVITES)
        .union(Self::CHANGE_NICKNAME);
    /// A new community's Moderator role.
    pub const MODERATOR_TEMPLATE: Self = Self::MEMBER_TEMPLATE
        .union(Self::MENTION_ROLES)
        .union(Self::MENTION_EVERYONE)
        .union(Self::MANAGE_INVITES)
        .union(Self::REMOVE_MEMBERS)
        .union(Self::MANAGE_MESSAGES)
        .union(Self::PIN_MESSAGES)
        .union(Self::MANAGE_CALLS)
        .union(Self::ADD_BOTS)
        .union(Self::MANAGE_CUSTOM_EMOJI)
        .union(Self::BAN_MEMBERS)
        .union(Self::MANAGE_NICKNAMES);
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
    AddBots,
    ManageCustomEmoji,
    BanMembers,
    ChangeNickname,
    ManageNicknames,
    ManagePlugins,
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
    MentionMembers,
    MentionRoles,
    MentionEveryone,
    TransferFiles,
    UseCamera,
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
            Permission::AddBots => Permissions::ADD_BOTS,
            Permission::ManageCustomEmoji => Permissions::MANAGE_CUSTOM_EMOJI,
            Permission::BanMembers => Permissions::BAN_MEMBERS,
            Permission::ChangeNickname => Permissions::CHANGE_NICKNAME,
            Permission::ManageNicknames => Permissions::MANAGE_NICKNAMES,
            Permission::ManagePlugins => Permissions::MANAGE_PLUGINS,
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
            Permission::MentionMembers => Permissions::MENTION_MEMBERS,
            Permission::MentionRoles => Permissions::MENTION_ROLES,
            Permission::MentionEveryone => Permissions::MENTION_EVERYONE,
            Permission::TransferFiles => Permissions::TRANSFER_FILES,
            Permission::UseCamera => Permissions::USE_CAMERA,
        }
    }
}

crate::wire_name_traits!(Permission);

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
