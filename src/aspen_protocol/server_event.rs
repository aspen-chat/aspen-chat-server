use chrono::{DateTime, Utc};
use serde::Serialize;

use super::{
    AttachmentMeta, CategoryId, ChannelId, ChannelPermissions, ChannelType, CommunityId, MessageId,
    UserId,
};

/// Describes events that the server notifies the client of. These are reliable messages and will
/// always be delivered in order.
#[derive(Serialize)]
#[serde(tag = "serverEvent")]
pub enum ServerEvent {
    UserJoinedCommunity(UserJoinedCommunity),
    UserLeftCommunity(UserLeftCommunity),
    UserDeleted(UserDeleted),
    NewMessage(NewMessage),
    MessageDeleted(MessageDeleted),
    NewReact(NewReact),
    ReactDeleted(ReactDeleted),
    NewChannel(NewChannel),
    ChannelDeleted(ChannelDeleted),
    NewCategory(NewCategory),
    CategoryDeleted(CategoryDeleted),
    UserOnlineStatusChange(UserOnlineStatusChange),
}

#[derive(Serialize)]
pub struct UserJoinedCommunity {
    pub community: CommunityId,
    pub username: String,
    pub id: UserId,
    pub icon: Vec<u8>,
    pub icon_mime_type: String,
}

#[derive(Serialize)]
pub struct UserLeftCommunity {
    pub community: CommunityId,
    pub user: UserId,
}

/// In order to be GDPR compliant the client should delete all information
/// relating to the user from its stores at this point. Either in memory or on disk.
#[derive(Serialize)]
pub struct UserDeleted {
    pub user: UserId,
}

#[derive(Serialize)]
pub struct NewMessage {
    pub author_id: UserId,
    pub channel_id: ChannelId,
    pub content: String,
    #[serde(with = "super::timestamp_serde")]
    pub timestamp: chrono::DateTime<Utc>,
    pub attachments: Vec<AttachmentMeta>,
}

#[derive(Serialize)]
pub struct MessageDeleted {
    pub message: MessageId,
}

#[derive(Serialize)]
pub struct NewReact {
    pub community: CommunityId,
    pub channel: ChannelId,
    pub author_id: UserId,
    pub message_id: MessageId,
    pub emoji: String,
}

#[derive(Serialize)]
pub struct ReactDeleted {
    pub community: CommunityId,
    pub channel: ChannelId,
    pub author_id: UserId,
    pub message: MessageId,
    pub emoji: String,
}

#[derive(Serialize)]
pub struct NewChannel {
    pub community: CommunityId,
    pub parent_category: Option<CategoryId>,
    pub name: String,
    pub permissions: ChannelPermissions,
    pub ty: ChannelType,
}

#[derive(Serialize)]
pub struct ChannelDeleted {
    pub community: CommunityId,
    pub channel: ChannelId,
}

#[derive(Serialize)]
pub struct NewCategory {
    pub community: CommunityId,
    pub name: String,
    pub permissions: ChannelPermissions,
    pub ty: ChannelType,
}

#[derive(Serialize)]
pub struct CategoryDeleted {
    pub community: CommunityId,
    pub category: CategoryId,
}

#[derive(Serialize)]
pub struct UserOnlineStatusChange {
    pub user_id: UserId,
    pub now_online: bool,
    #[serde(with = "super::timestamp_serde")]
    pub time: DateTime<Utc>,
}
