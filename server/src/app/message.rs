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
    _author: UserId,
    _channel_id: ChannelId,
    _content: String,
    _attachment: Vec<AttachmentInput>,
) {
    let _id = MessageId::new();
}
