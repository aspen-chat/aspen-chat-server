use crate::app::attachment::AttachmentInput;
use crate::app::channel::Channel;
use crate::app::user::User;
use crate::app::{AttachmentId, ChannelId, UserId};
use crate::app::{MaybeLoaded, MessageId};
use chrono::Utc;

pub struct Message {
    id: MessageId,
    channel: MaybeLoaded<Channel>,
    content: String,
    attachments: Vec<AttachmentId>,
    author: MaybeLoaded<User>,
    timestamp: chrono::DateTime<Utc>,
}

pub fn create_message(
    author: UserId,
    channel_id: ChannelId,
    content: String,
    attachment: Vec<AttachmentInput>,
) {
    let id = MessageId::new();
}
