use axum::{
    http::StatusCode,
    routing::{get, post},
    Json,
};

mod command_response;
mod event_stream;
mod login;
mod message_enum;

use command_response::CommandResponse;
use login::{Login, LoginResponse};
use message_enum::command::{
    CategoryCommand, ChannelCommand, CommunityCommand, MessageCommand, ReactCommand, UserCommand,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::broadcast;
use crate::api::message_enum::server_event::ServerEvent;

pub(crate) fn make_router() -> axum::Router {
    axum::Router::new()
        .route("/login", post(login))
        .route("/user", post(user))
        .route("/message", post(message))
        .route("/react", post(react))
        .route("/channel", post(channel))
        .route("/category", post(category))
        .route("/community", post(community))
        .route("/event_stream", get(event_stream::event_stream))
}

async fn login(Json(login): Json<Login>) -> (StatusCode, Json<LoginResponse>) {
    let resp = login::try_login(&login).await;
    if matches!(resp, LoginResponse::InvalidCredentials) {
        (StatusCode::UNAUTHORIZED, resp.into())
    } else {
        (StatusCode::OK, resp.into())
    }
}

async fn user(Json(command): Json<UserCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        UserCommand::Create { name, icon } => todo!(),
        UserCommand::Read { id } => todo!(),
        UserCommand::Update { id, name, icon } => todo!(),
        UserCommand::Delete { id } => todo!(),
    }
}

async fn message(Json(command): Json<MessageCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        MessageCommand::Create {
            channel_id,
            content,
            attachments,
        } => todo!(),
        MessageCommand::Read { id } => todo!(),
        MessageCommand::Update {
            id,
            content,
            attachments,
        } => todo!(),
        MessageCommand::Delete { id } => todo!(),
    }
}

async fn react(Json(command): Json<ReactCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        ReactCommand::Create { message_id, emoji } => todo!(),
        ReactCommand::Delete {
            message_id,
            emoji,
            user_id,
        } => todo!(),
    }
}

async fn channel(Json(command): Json<ChannelCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        ChannelCommand::Create {
            parent_category,
            name,
            permissions,
            ty,
        } => todo!(),
        ChannelCommand::Read { id } => todo!(),
        ChannelCommand::Update {
            id,
            parent_category,
            name,
            permissions,
        } => todo!(),
        ChannelCommand::Delete { id } => todo!(),
    }
}

async fn category(Json(command): Json<CategoryCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        CategoryCommand::Create { community, name } => todo!(),
        CategoryCommand::Read { id } => todo!(),
        CategoryCommand::Update { id, name } => todo!(),
        CategoryCommand::Delete { id } => todo!(),
    }
}

async fn community(Json(command): Json<CommunityCommand>) -> (StatusCode, Json<CommandResponse>) {
    match command {
        CommunityCommand::Create { name, icon } => todo!(),
        CommunityCommand::Read { id } => todo!(),
        CommunityCommand::Update { id, name, icon } => todo!(),
        CommunityCommand::Delete { id } => todo!(),
    }
}

macro_rules! id_type {
    ($type_name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Copy, Deserialize, Serialize, Hash)]
        #[serde(transparent)]
        pub struct $type_name(uuid::Uuid);

        impl $type_name {
            pub fn new() -> Self {
                Self(uuid::Uuid::now_v7())
            }
        }

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

id_type!(IconId);


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