use crate::app::{AttachmentId, UserId};
use crate::{aspen_config::aspen_config, nats_connection_manager::NatsConnectionManager};
use axum::routing::{get, post};
use diesel_async::{
    pooled_connection::{deadpool::Pool, AsyncDieselConnectionManager}, AsyncPgConnection,
};
mod event_stream;
mod message_enum;
pub(crate) mod login;
pub(crate) mod user;
pub(crate) mod category;
pub(crate) mod community;
pub(crate) mod channel;
pub(crate) mod icon;
pub(crate) mod message;
pub(crate) mod react;

use diesel::{BoolExpressionMethods, ExpressionMethods as _, QueryDsl};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use axum::routing::{delete, patch};
use tokio::sync::RwLock;
pub(crate) async fn make_router() -> axum::Router {
    axum::Router::new()
        // Auth
        .route("/login", post(login::login))
        .route("/logout", post(login::logout))
        .route("/token_refresh", post(login::token_refresh))
        .route("/change_password", post(login::change_password))
        .route("/other_server_login", post(login::other_server_login))
        // Create
        .route("/user", post(user::create_user))
        .route("/message", post(message::create_message))
        .route("/react", post(react::create_react))
        .route("/channel", post(channel::create_channel))
        .route("/category", post(category::create_category))
        .route("/community", post(community::create_community))
        .route("/icon", post(icon::create_icon))
        // Read
        .route("/user", get(user::read_user))
        .route("/message", get(message::read_message))
        .route("/channel", get(channel::read_channel))
        .route("/category", get(category::read_category))
        .route("/community", get(community::read_community))
        .route("/icon", get(icon::read_icon))
        // Update
        .route("/user", patch(user::update_user))
        .route("/message", patch(message::update_message))
        .route("/channel", patch(channel::update_channel))
        .route("/category", patch(category::update_category))
        .route("/community", patch(community::update_community))
        // Delete
        .route("/user", delete(user::delete_user))
        .route("/message", delete(message::delete_message))
        .route("/react", delete(react::delete_react))
        .route("/channel", delete(channel::delete_channel))
        .route("/category", delete(category::delete_category))
        .route("/community", delete(community::delete_community))
        .route("/icon", delete(icon::delete_icon))
        // Events
        .route("/event_stream", get(event_stream::event_stream))
        .with_state(GlobalServerContext::new().await)
}

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

#[derive(Clone)]
pub struct GlobalServerContext {
    pub connection_pool: Pool<AsyncPgConnection>,
    pub nats_connection_manager: Arc<RwLock<NatsConnectionManager>>,
}

impl GlobalServerContext {
    pub async fn new() -> Self {
        let config = aspen_config().await;
        Self {
            connection_pool: {
                let conn_manager = AsyncDieselConnectionManager::<AsyncPgConnection>::new(
                    config.database_url
                );
                Pool::builder(conn_manager)
                    .build()
                    .expect("Failed to init database connection pool")
            },
            nats_connection_manager: Arc::new(RwLock::new(NatsConnectionManager::new(todo!()))),
        }
    }
}

/// If a client misses this many messages at once it will be forcefully disconnected.
const MAILBOX_SIZE: usize = 512;

pub struct SessionContext {
    pub signed_in_user: Option<UserId>,
}

impl SessionContext {
    pub fn new() -> Self {
        Self {
            signed_in_user: None,
        }
    }
}
