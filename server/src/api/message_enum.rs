
use message_gen::message_enum_source;

use crate::aspen_protocol::{Attachment, CategoryId, ChannelId, ChannelPermissions, ChannelType, CommunityId, MessageId, UserId};

#[message_enum_source]
enum MessageEnumSource {
    User {
        #[message_gen(id)]
        id: UserId,
        name: String,
    },
    Message {
        #[message_gen(id)]
        id: MessageId,
        #[message_gen(permanent)]
        channel_id: ChannelId,
        content: String,
        attachments: Vec<Attachment>,
    },
    React {
        #[message_gen(id = "client_authoritative")]
        message_id: MessageId,
        #[message_gen(id = "client_authoritative")]
        emoji: String,
        #[message_gen(id)]
        user_id: UserId,
    },
    Channel {
        #[message_gen(id)]
        id: ChannelId,
        parent_category: Option<CategoryId>,
        name: String,
        permissions: ChannelPermissions,
        #[message_gen(permanent)]
        ty: ChannelType,
    },
    Category {
        #[message_gen(id)]
        id: CategoryId,
        #[message_gen(permanent)]
        community: CommunityId,
        name: String,
    },
    Community {
        #[message_gen(id)]
        id: CommunityId,
        name: String,
    },
    UserCommunity {
        #[message_gen(id = "client_authoritative")]
        community: CommunityId,
        #[message_gen(id)]
        user: UserId,
    }
}