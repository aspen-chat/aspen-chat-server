use crate::api::{Attachment, ChannelPermissions, ChannelType};
use crate::api::{CategoryId, ChannelId, CommunityId, IconId, MessageId, UserId};
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
    },
    Message {
        #[message_gen(id)]
        id: MessageId,
        #[message_gen(permanent)]
        channel_id: ChannelId,
        content: String,
        attachments: Vec<Attachment>,
        #[message_gen(server_authoritative)]
        author: UserId,
        #[message_gen(server_authoritative)]
        #[serde(with = "timestamp_serde")]
        timestamp: chrono::DateTime<Utc>,
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
        icon: Option<IconId>,
    },
    UserCommunity {
        #[message_gen(id = "client_authoritative")]
        community: CommunityId,
        #[message_gen(id)]
        user: UserId,
    },
    Icon {
        #[message_gen(id)]
        id: IconId,
        #[message_gen(permanent)]
        data: Vec<u8>,
        #[message_gen(permanent)]
        mime_type: String,
    },
}

mod timestamp_serde {
    use chrono::{DateTime, Utc};
    use serde::Serializer;

    pub fn serialize<S>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        s.serialize_i64(t.timestamp_micros())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::api::{MessageId, UserId};

    use super::server_event::ServerEvent;

    #[test]
    fn server_event_transparent() {
        let e = ServerEvent::React(super::server_event::sub_variant::React::Create {
            message_id: MessageId::new(),
            emoji: "😁".to_string(),
            user_id: UserId::new(),
        });
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
