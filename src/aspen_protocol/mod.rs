use std::sync::Arc;

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use server_event::ServerEvent;
use tokio::sync::broadcast;

pub mod client_event;
pub mod server_event;

macro_rules! id_type {
    ($type_name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Copy, Deserialize, Serialize, Hash)]
        #[serde(transparent)]
        pub struct $type_name(uuid::Uuid);

        impl From<uuid::Uuid> for $type_name {
            fn from(value: uuid::Uuid) -> Self {
                Self(value)
            }
        }
    };
}

id_type!(CommunityId);

id_type!(UserId);

id_type!(ChannelId);

id_type!(MessageId);

id_type!(CategoryId);

id_type!(AttachmentId);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Attachment {
    mime_type: String,
    file_name: String,
    content: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AttachmentMeta {
    attachment_id: AttachmentId,
    mime_type: String,
    file_name: String,
    preview: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum ChannelType {
    Text,
    Voice,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChannelPermissions {
    // TODO
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

/// If a client misses this many messages at once it will be forcefully disconnected.
const MAILBOX_SIZE: usize = 512;

#[derive(Clone)]
pub struct CommunityMailboxManager {
    map: Arc<DashMap<CommunityId, broadcast::Sender<Arc<ServerEvent>>>>,
}

impl CommunityMailboxManager {
    pub fn new() -> Self {
        Self {
            map: Arc::new(DashMap::default()),
        }
    }

    pub fn subscribe_mailbox(&self, id: &CommunityId) -> broadcast::Receiver<Arc<ServerEvent>> {
        // First try to obtain in a read-only manner.
        let first_attempt = self.map.get(id);
        if let Some(s) = first_attempt {
            return s.subscribe();
        }
        // Someone else could have beat us to it, check again to see if it's initialized.
        let write_lock = self.map.entry(*id);
        match write_lock {
            dashmap::Entry::Occupied(occupied_entry) => occupied_entry.get().subscribe(),
            dashmap::Entry::Vacant(vacant_entry) => {
                let (sender, receiver) = broadcast::channel(MAILBOX_SIZE);
                vacant_entry.insert(sender);
                receiver
            }
        }
    }
}
