use crate::app;
use crate::app::ASPEN_NATS_STREAM_NAME;
use axum::routing::any;
use diesel_async::{
    AsyncPgConnection,
    pooled_connection::{AsyncDieselConnectionManager, deadpool::Pool},
};
use fred::prelude::ClientLike;
use std::fs;
use std::io::Write;

pub(crate) mod attachment;
pub(crate) mod category;
pub(crate) mod channel;
pub(crate) mod community;
mod event_stream;
pub(crate) mod icon;
pub(crate) mod invite;
pub(crate) mod link_preview;
pub(crate) mod login;
pub(crate) mod message;
pub(crate) mod message_enum;
pub(crate) mod react;
pub(crate) mod user;

use crate::api::message_enum::server_event::ServerEvent;
use crate::aspen_config::{AspenConfig, load_config};
use async_nats::ConnectOptions;
use async_nats::jetstream::stream::{ConsumerLimits, DiscardPolicy, StorageType};
use diesel::FromSqlRow;
use diesel::deserialize::FromSql;
use diesel::expression::AsExpression;
use diesel::pg::Pg;
use diesel::serialize::{IsNull, Output, ToSql};
use schemars::schema_for;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi, openapi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

#[derive(OpenApi)]
#[openapi(modifiers(&SecurityAddon))]
struct ApiDoc;

struct SecurityAddon;
impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut openapi::OpenApi) {
        openapi
            .components
            .get_or_insert(Default::default())
            .security_schemes
            .insert(
                "loginKey".to_string(),
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("Authorization"))),
            );
    }
}

pub(crate) async fn make_router(write_schema: bool) -> Result<axum::Router, app::Error> {
    let mut router = OpenApiRouter::with_openapi(ApiDoc::openapi())
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
            // User Communities
            user::read_user_communities,
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
            // Channel Messages
            channel::read_channel_messages,
        ))
        .routes(routes!(
            // Channel Pins
            channel::read_channel_pins,
        ))
        .routes(routes!(
            // Category
            category::create_category,
            category::read_category,
            category::update_category,
            category::delete_category,
        ))
        .routes(routes!(
            // Category Channels
            category::read_category_channels,
        ))
        .routes(routes!(
            // Community
            community::create_community,
            community::read_community,
            community::update_community,
            community::delete_community,
        ))
        .routes(routes!(
            // UserCommunity
            community::join_community,
            community::leave_community,
        ))
        .routes(routes!(
            // Community Channels
            community::read_community_channels,
        ))
        .routes(routes!(
            // Community Categories
            community::read_community_categories,
        ))
        .routes(routes!(
            // Community Users
            community::read_community_users,
        ))
        .routes(routes!(
            // Attachment metadata
            attachment::read_attachment,
            attachment::delete_attachment,
        ))
        .routes(routes!(attachment::init_attachment_upload))
        .routes(routes!(attachment::confirm_attachment_upload))
        .routes(routes!(
            // Icon metadata
            icon::read_icon,
            icon::delete_icon,
        ))
        .routes(routes!(icon::init_icon_upload))
        .routes(routes!(icon::confirm_icon_upload))
        .routes(routes!(
            // React
            react::create_react,
            react::delete_react,
        ))
        .routes(routes!(
            // Invite
            invite::create_invite,
            invite::read_community_invites,
            invite::update_invite,
            invite::revoke_invite,
        ))
        // Events
        .route("/event_stream", any(event_stream::event_stream));
    if write_schema {
        let openapi = router.to_openapi();
        fs::write("openapi.yaml", openapi.to_yaml()?)?;
        let event_schema = schema_for!(ServerEvent);
        fs::write(
            "event_schema.json",
            serde_json::to_string_pretty(&event_schema)?,
        )?;
        std::process::exit(0);
    }
    let router = router.with_state(GlobalServerContext::new().await?);

    Ok(router.into())
}

#[derive(
    Debug,
    Clone,
    Copy,
    Deserialize,
    Serialize,
    utoipa::ToSchema,
    schemars::JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[diesel(sql_type = crate::database::schema::sql_types::ChannelType)]
pub enum ChannelType {
    Text,
    Voice,
}

impl ToSql<crate::database::schema::sql_types::ChannelType, Pg> for ChannelType {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        out.write_all(match self {
            ChannelType::Text => b"text",
            ChannelType::Voice => b"voice",
        })?;
        Ok(IsNull::No)
    }
}

impl FromSql<crate::database::schema::sql_types::ChannelType, Pg> for ChannelType {
    fn from_sql(
        bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
    ) -> diesel::deserialize::Result<Self> {
        match bytes.as_bytes() {
            b"voice" => Ok(ChannelType::Voice),
            b"text" => Ok(ChannelType::Text),
            _ => Err(format!(
                "Unrecognized enum variant: {:?}",
                String::from_utf8_lossy(bytes.as_bytes())
            )
            .into()),
        }
    }
}

#[derive(Clone)]
pub struct GlobalServerContext {
    pub connection_pool: Pool<AsyncPgConnection>,
    pub nats_context: Arc<async_nats::jetstream::Context>,
    pub valkey: fred::clients::Client,
    pub media_store: Arc<app::media_store::MediaStore>,
    pub config: Arc<AspenConfig>,
}

impl GlobalServerContext {
    pub async fn new() -> Result<Self, app::Error> {
        let config = load_config()?;
        let client = async_nats::connect_with_options(
            &config.nats_url,
            ConnectOptions::new().token(config.nats_auth_token.clone()),
        )
        .await?;
        let context = async_nats::jetstream::new(client);
        context
            .create_or_update_stream(async_nats::jetstream::stream::Config {
                name: ASPEN_NATS_STREAM_NAME.to_string(),
                discard: DiscardPolicy::Old,
                max_messages: 1_000_000_000,
                max_bytes: 8 * 1024 * 1024 * 1024,
                max_age: MAX_EVENT_AGE,
                storage: StorageType::Memory,
                consumer_limits: Some(ConsumerLimits {
                    max_ack_pending: 1000,
                    inactive_threshold: Duration::from_secs(60),
                }),
                ..Default::default()
            })
            .await?;
        let valkey_config = fred::prelude::Config::from_url(&config.valkey_url)?;
        let valkey = fred::prelude::Client::new(valkey_config.clone(), None, None, None);
        valkey.init().await?;

        let valkey_subscribe = fred::prelude::Client::new(valkey_config, None, None, None);
        valkey_subscribe.init().await?;

        // Enable expired key notifications and subscribe
        use fred::interfaces::ConfigInterface;
        use fred::prelude::PubsubInterface;
        valkey_subscribe
            .config_set("notify-keyspace-events", "Ex")
            .await?;
        valkey_subscribe
            .psubscribe("__keyevent@*__:expired")
            .await?;

        let nats_arc: Arc<async_nats::jetstream::Context> = context.into();
        app::user_status::spawn_expiry_listener(valkey_subscribe.clone(), nats_arc.clone());
        let media_store = Arc::new(app::media_store::MediaStore::new(&config).await?);

        Ok(Self {
            connection_pool: {
                let conn_manager =
                    AsyncDieselConnectionManager::<AsyncPgConnection>::new(&config.database_url);
                Pool::builder(conn_manager).build()?
            },
            nats_context: nats_arc,
            valkey,
            media_store,
            config: config.into(),
        })
    }
}

const MAX_EVENT_AGE: Duration = Duration::from_secs(60);
