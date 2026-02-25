use crate::app::{AttachmentId, UserId};
use crate::{app, aspen_config::aspen_config, nats_connection_manager::NatsConnectionManager};
use axum::routing::{get, post};
use diesel_async::{
    AsyncPgConnection,
    pooled_connection::{AsyncDieselConnectionManager, deadpool::Pool},
};
use std::fs;
pub(crate) mod category;
pub(crate) mod channel;
pub(crate) mod community;
mod event_stream;
pub(crate) mod icon;
pub(crate) mod login;
pub(crate) mod message;
pub(crate) mod message_enum;
pub(crate) mod react;
pub(crate) mod user;

use crate::api::login::SessionUser;
use axum::Extension;
use axum::routing::{delete, patch};
use diesel::{BoolExpressionMethods, ExpressionMethods as _, QueryDsl};
use futures_util::TryFutureExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower::{Layer, ServiceBuilder};
use utoipa::openapi;
use utoipa::openapi::{License, LicenseBuilder};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) async fn make_router(write_schema: bool) -> Result<axum::Router, app::Error> {
    let mut router = OpenApiRouter::new()
        .routes(routes!(login::login,))
        .routes(routes!(login::logout,))
        .routes(routes!(login::token_refresh,))
        .routes(routes!(login::change_password,))
        .routes(routes!(login::other_server_login,))
        .routes(routes!(
            // User
            user::create_user,
            user::read_user,
            user::update_user,
            user::delete_user,
        ))
        .routes(routes!(
            // Message
            message::create_message,
            message::read_message,
            message::update_message,
            message::delete_message,
        ))
        .routes(routes!(
            // Channel
            channel::create_channel,
            channel::read_channel,
            channel::update_channel,
            channel::delete_channel,
        ))
        .routes(routes!(
            // Category
            category::create_category,
            category::read_category,
            category::update_category,
            category::delete_category,
        ))
        .routes(routes!(
            // Community
            community::create_community,
            community::read_community,
            community::update_community,
            community::delete_community,
        ))
        .routes(routes!(
            // Icon
            icon::create_icon,
            icon::delete_icon,
        ))
        .routes(routes!(
            // React
            react::create_react,
            react::delete_react,
        ))
        // Events
        .route("/event_stream", get(event_stream::event_stream))
        .with_state(GlobalServerContext::new().await?);
    if write_schema {
        let mut openapi = router.to_openapi();
        openapi.info.title = "Aspen API".into();
        openapi.info.description = Some("API for an Aspen chat service".into());
        openapi.info.contact = None;
        openapi.info.license = Some(License::new("GPL-3.0-or-later"));
        openapi.info.version = env!("CARGO_PKG_VERSION").into();
        fs::write("openapi.yaml", openapi.to_yaml()?)?;
        std::process::exit(0);
    }
    Ok(router.into())
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct Attachment {
    mime_type: String,
    file_name: String,
    content: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct AttachmentMeta {
    attachment_id: AttachmentId,
    mime_type: String,
    file_name: String,
    preview: Vec<u8>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub enum ChannelType {
    Text,
    Voice,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema)]
pub struct ChannelPermissions {
    // TODO
}

#[derive(Clone)]
pub struct GlobalServerContext {
    pub connection_pool: Pool<AsyncPgConnection>,
    pub nats_connection_manager: Arc<RwLock<NatsConnectionManager>>,
}

impl GlobalServerContext {
    pub async fn new() -> Result<Self, app::Error> {
        let config = aspen_config().await;
        Ok(Self {
            connection_pool: {
                let conn_manager =
                    AsyncDieselConnectionManager::<AsyncPgConnection>::new(config.database_url);
                Pool::builder(conn_manager)
                    .build()
                    .expect("Failed to init database connection pool")
            },
            nats_connection_manager: Arc::new(RwLock::new(
                NatsConnectionManager::new(config.nats_url, config.nats_auth_token).await?,
            )),
        })
    }
}

/// If a client misses this many messages at once it will be forcefully disconnected.
const MAILBOX_SIZE: usize = 512;
