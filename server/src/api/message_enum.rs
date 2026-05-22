use crate::api::link_preview::LinkPreview;
use crate::api::{ChannelType, user::UserOnlineStatus};
use crate::app::{AttachmentId, CategoryId, ChannelId, CommunityId, IconId, MessageId, UserId};
use chrono::Utc;
use message_gen::message_enum_source;

// WARNING: message_enum_source is a special macro. The below enum will not appear in the final program, but this is responsible
// for generating all Command types, and Server events. This comment is not a doc comment. This is intentional.
#[message_enum_source]
enum MessageEnumSource {
    User {
        #[message_gen(id)]
        id: UserId,
        name: String,
        #[message_gen(secret)]
        password: String,
        icon: Option<IconId>,
        #[message_gen(server_authoritative)]
        online_status: UserOnlineStatus,
    },
    #[message_gen(custom_event)]
    UserStatus {
        id: UserId,
        status: UserOnlineStatus,
    },
    Message {
        #[message_gen(id)]
        id: MessageId,
        #[message_gen(permanent)]
        channel_id: ChannelId,
        content: String,
        #[message_gen(server_authoritative)]
        author: UserId,
        #[message_gen(server_authoritative)]
        timestamp: chrono::DateTime<Utc>,
        attachments: Vec<AttachmentId>,
        // Populated asynchronously by the server after the initial
        // create/update. Clients receive the final set via the
        // `MessageLinkPreviewsReady` custom event; REST reads serve the
        // most recent value directly.
        #[message_gen(server_authoritative)]
        link_previews: Vec<LinkPreview>,
    },
    // Delivered by the server once the async link-preview fetcher finishes
    // materialising previews for a message. The macro rewrites this variant
    // untouched thanks to `custom_event`, so it doesn't bring in the CRUD
    // machinery the rest of the enum generates. The variant-level
    // ``#[serde(rename_all = "camelCase")]`` is required because the enum-level
    // ``rename_all`` on ``ServerEvent`` renames variants but does not recurse
    // into struct-variant fields.
    #[message_gen(custom_event)]
    #[serde(rename_all = "camelCase")]
    MessageLinkPreviewsReady {
        message_id: MessageId,
        channel_id: ChannelId,
        previews: Vec<LinkPreview>,
    },
    Pin {
        #[message_gen(id = "client_authoritative")]
        message_id: MessageId,
        #[message_gen(server_authoritative)]
        timestamp: chrono::DateTime<Utc>,
        sort_index: i32,
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
        community: Option<CommunityId>,
        name: String,
        #[message_gen(permanent)]
        ty: ChannelType,
        sort_index: i32,
    },
    Category {
        #[message_gen(id)]
        id: CategoryId,
        #[message_gen(permanent)]
        community: CommunityId,
        name: String,
        sort_index: i32,
    },
    Community {
        #[message_gen(id)]
        id: CommunityId,
        name: String,
        icon: Option<IconId>,
    },
    UserCommunity {
        #[message_gen(id = "client_authoritative")]
        community: CommunityId,
        #[message_gen(id)]
        user: UserId,
        #[message_gen(secret)]
        invite_code: String,
    },
    #[message_gen(no_commands)]
    Invite {
        #[message_gen(id = "client_authoritative")]
        code: String,
        #[message_gen(permanent)]
        community: CommunityId,
        #[message_gen(server_authoritative)]
        created_by: UserId,
        #[message_gen(server_authoritative)]
        created_at: chrono::DateTime<Utc>,
        expires_at: Option<chrono::DateTime<Utc>>,
    },
}

#[cfg(test)]
mod tests {
    use crate::api;
    use crate::app::{MessageId, UserId};
    use serde_json::json;

    use super::server_event::ServerEvent;

    #[test]
    fn server_event_transparent() {
        let e = ServerEvent::React(super::server_event::ReactEvent::Create(
            api::message_enum::React {
                message_id: MessageId::new(),
                emoji: "😁".to_string(),
                user_id: UserId::new(),
            },
        ));
        let mut json_value = serde_json::to_value(e).unwrap();
        let object_mut = json_value.as_object_mut().unwrap();
        let create_obj = object_mut
            .get_mut("create")
            .unwrap()
            .as_object_mut()
            .unwrap();
        // Needs to be a value of some sort, not particular on which one
        assert!(create_obj.remove("messageId").is_some());
        assert!(create_obj.remove("userId").is_some());
        assert_eq!(
            json_value,
            json! ({
                "serverEvent": "react",
                "create": {
                    "emoji": "😁"
                }
            })
        )
    }
}
