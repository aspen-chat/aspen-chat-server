use chrono::Utc;
use serde::Serialize;

use super::{
    AttachmentMeta, CategoryId, ChannelId, ChannelPermissions, ChannelType, CommunityId, MessageId,
    UserId,
};

/// Describes events that the server notifies the client of.
#[derive(Serialize)]
#[serde(tag = "serverEvent")]
pub enum ServerEvent {
    NewMessage(NewMessage),
    NewReact(NewReact),
    NewChannel(NewChannel),
    NewCategory(NewCategory),
    UserOnlineStatusChange(UserOnlineStatusChange),
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
pub struct NewReact {
    pub author_id: UserId,
    pub message_id: MessageId,
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
pub struct NewCategory {
    pub community: CommunityId,
    pub name: String,
    pub permissions: ChannelPermissions,
    pub ty: ChannelType,
}

#[derive(Serialize)]
pub struct UserOnlineStatusChange {
    pub user_id: UserId,
    pub now_online: bool,
}
