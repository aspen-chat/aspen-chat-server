use crate::app;
use crate::app::ASPEN_NATS_STREAM_NAME;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, LOCATION};
use axum::http::{HeaderValue, Method};
use axum::routing::any;
use diesel_async::{
    AsyncPgConnection,
    pooled_connection::{AsyncDieselConnectionManager, deadpool::Pool},
};
use fred::prelude::ClientLike;
use std::fs;
use std::io::Write;
use tower_http::cors::{AllowOrigin, CorsLayer};

pub(crate) mod attachment;
pub(crate) mod auth;
pub(crate) mod category;
pub(crate) mod channel;
pub(crate) mod community;
pub(crate) mod error;
mod event_stream;
pub(crate) mod extract;
pub(crate) mod icon;
pub(crate) mod include;
pub(crate) mod invite;
pub(crate) mod link_preview;
pub(crate) mod message;
pub(crate) mod message_enum;
pub mod poll;
pub(crate) mod react;
pub(crate) mod user;
pub mod voice;

use crate::aspen_config::{AspenConfig, CorsConfig, load_config};
use async_nats::ConnectOptions;
use async_nats::jetstream::stream::{ConsumerLimits, DiscardPolicy, StorageType};
use diesel::FromSqlRow;
use diesel::deserialize::FromSql;
use diesel::expression::AsExpression;
use diesel::pg::Pg;
use diesel::serialize::{IsNull, Output, ToSql};
use schemars::schema_for;
use serde::{Deserialize, Deserializer, Serialize};
use std::sync::Arc;
use std::time::Duration;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};
use utoipa::{Modify, OpenApi, openapi};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

/// Every REST route and the event stream live under this prefix. Bumping the version means a
/// breaking change to the wire contract; additive changes stay within `v1`.
pub const API_PREFIX: &str = "/api/v1";

pub const TAG_AUTH: &str = "auth";
pub const TAG_USERS: &str = "users";
pub const TAG_COMMUNITIES: &str = "communities";
pub const TAG_CATEGORIES: &str = "categories";
pub const TAG_CHANNELS: &str = "channels";
pub const TAG_MESSAGES: &str = "messages";
pub const TAG_REACTIONS: &str = "reactions";
pub const TAG_POLLS: &str = "polls";
pub const TAG_VOICE: &str = "voice";
pub const TAG_INVITES: &str = "invites";
pub const TAG_ATTACHMENTS: &str = "attachments";
pub const TAG_ICONS: &str = "icons";

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Aspen",
        description = "REST API of the Aspen chat server. Every response body is JSON; every \
            error is an RFC 9457 Problem Details document (`application/problem+json`) whose \
            `code` field is the stable discriminator clients branch on. Updates use JSON Merge \
            Patch semantics: omitted fields are unchanged, `null` clears a nullable field. \
            Reads that accept `include` return `{data, included}`, with the requested related \
            records under `included` keyed by record type. \
            Real-time changes are delivered over the WebSocket at `/api/v1/events`; the event \
            payloads are described by the companion `event_schema.json`."
    ),
    modifiers(&SecurityAddon),
    // Enums that appear only inside query parameters are not collected from handlers the way
    // body types are, so they are registered here to keep them named components.
    components(schemas(
        community::CommunityInclude,
        message::MessageInclude,
        poll::PollInclude,
        poll::PollOption,
        poll::PollVote,
        invite::InviteInclude
    )),
    tags(
        (name = TAG_AUTH, description = "Login, logout, and session refresh"),
        (name = TAG_USERS, description = "Accounts. `@me` addresses the calling user."),
        (name = TAG_COMMUNITIES, description = "Communities and their membership"),
        (name = TAG_CATEGORIES, description = "Groupings of channels inside a community"),
        (name = TAG_CHANNELS, description = "Text and voice channels"),
        (name = TAG_MESSAGES, description = "Messages within a channel"),
        (name = TAG_REACTIONS, description = "Emoji reactions on messages"),
        (name = TAG_POLLS, description = "Timed polls posted to a channel, and votes on them"),
        (name = TAG_VOICE, description = "Voice calls: joining a channel's call, the voice server registry, and failure reports"),
        (name = TAG_INVITES, description = "Invite codes for joining communities"),
        (name = TAG_ATTACHMENTS, description = "Files attached to messages (two-phase direct-to-storage upload)"),
        (name = TAG_ICONS, description = "User and community icons (two-phase direct-to-storage upload)"),
    )
)]
struct ApiDoc;

struct SecurityAddon;
impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut openapi::OpenApi) {
        openapi
            .components
            .get_or_insert(Default::default())
            .security_schemes
            .insert(
                "bearerAuth".to_string(),
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("opaque")
                        .description(Some(
                            "Session token from `POST /auth/login` or `POST /auth/token-refresh`.",
                        ))
                        .build(),
                ),
            );
    }
}

/// Deserializes `Option<Option<T>>` for JSON Merge Patch fields. Plain serde folds a JSON `null`
/// into the outer `None`, which would make "clear this field" indistinguishable from "leave it
/// alone". Routing the field through this function (together with `#[serde(default)]` for the
/// absent case) maps a present `null` to `Some(None)` and a present value to `Some(Some(v))`.
pub fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

fn cors_layer(config: &CorsConfig) -> Option<CorsLayer> {
    if config.allowed_origins.is_empty() {
        return None;
    }
    let origin = if config.allowed_origins.iter().any(|o| o == "*") {
        AllowOrigin::any()
    } else {
        AllowOrigin::list(
            config
                .allowed_origins
                .iter()
                .filter_map(|o| HeaderValue::from_str(o).ok()),
        )
    };
    Some(
        CorsLayer::new()
            .allow_origin(origin)
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
            ])
            .allow_headers([AUTHORIZATION, CONTENT_TYPE])
            .expose_headers([LOCATION])
            .max_age(Duration::from_secs(60 * 60)),
    )
}

pub(crate) async fn make_router(write_schema: bool) -> Result<axum::Router, app::Error> {
    let v1 = OpenApiRouter::new()
        .routes(routes!(auth::login))
        .routes(routes!(auth::logout))
        .routes(routes!(auth::token_refresh))
        .routes(routes!(auth::other_server_token))
        .routes(routes!(user::create_user))
        .routes(routes!(
            user::get_user,
            user::update_user,
            user::delete_user
        ))
        .routes(routes!(user::list_user_communities))
        .routes(routes!(user::change_password))
        .routes(routes!(community::create_community))
        .routes(routes!(
            community::get_community,
            community::update_community,
            community::delete_community
        ))
        .routes(routes!(community::list_community_members))
        .routes(routes!(
            community::join_community,
            community::update_membership,
            community::leave_community
        ))
        .routes(routes!(community::list_community_channels))
        .routes(routes!(
            category::create_category,
            category::list_community_categories
        ))
        .routes(routes!(
            invite::create_invite,
            invite::list_community_invites
        ))
        .routes(routes!(
            invite::get_invite,
            invite::update_invite,
            invite::revoke_invite
        ))
        .routes(routes!(
            category::get_category,
            category::update_category,
            category::delete_category
        ))
        .routes(routes!(category::list_category_channels))
        .routes(routes!(channel::create_channel))
        .routes(routes!(
            channel::get_channel,
            channel::update_channel,
            channel::delete_channel
        ))
        .routes(routes!(channel::list_channel_pins))
        .routes(routes!(
            message::create_message,
            message::list_channel_messages
        ))
        .routes(routes!(
            message::get_message,
            message::update_message,
            message::delete_message
        ))
        .routes(routes!(react::add_reaction, react::remove_reaction))
        .routes(routes!(poll::create_poll))
        .routes(routes!(poll::get_poll))
        .routes(routes!(poll::add_vote, poll::remove_vote))
        .routes(routes!(voice::join_voice))
        .routes(routes!(voice::get_channel_voice))
        .routes(routes!(
            voice::moderate_voice_participant,
            voice::kick_voice_participant
        ))
        .routes(routes!(
            voice::list_voice_servers,
            voice::create_voice_server
        ))
        .routes(routes!(
            voice::update_voice_server,
            voice::delete_voice_server
        ))
        .routes(routes!(voice::report_voice_server_failure))
        .routes(routes!(attachment::init_attachment_upload))
        .routes(routes!(attachment::confirm_attachment_upload))
        .routes(routes!(
            attachment::get_attachment,
            attachment::delete_attachment
        ))
        .routes(routes!(icon::init_icon_upload))
        .routes(routes!(icon::confirm_icon_upload))
        .routes(routes!(icon::get_icon, icon::delete_icon))
        // The event stream is a WebSocket and has no OpenAPI representation; its frames are
        // described by `event_schema.json`.
        .route("/events", any(event_stream::event_stream));
    let mut router = OpenApiRouter::with_openapi(ApiDoc::openapi()).nest(API_PREFIX, v1);
    if write_schema {
        let openapi = router.to_openapi();
        fs::write("openapi.yaml", openapi.to_yaml()?)?;
        let event_schema = schema_for!(event_stream::EventStreamProtocol);
        fs::write(
            "event_schema.json",
            serde_json::to_string_pretty(&event_schema)?,
        )?;
        std::process::exit(0);
    }
    let context = GlobalServerContext::new().await?;
    app::poll::spawn_closer(context.clone());
    app::voice::seed_servers(&context).await?;
    app::voice::spawn_report_listener(context.clone()).await?;
    app::voice::spawn_reaper(context.clone());
    let cors = cors_layer(&context.config.cors);
    let router: axum::Router = router.with_state(context).into();
    Ok(match cors {
        Some(cors) => router.layer(cors),
        None => router,
    })
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

/// What a message is. Both poll kinds carry no `content`; the client renders them from the poll
/// record the message's `poll` field names.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    Serialize,
    utoipa::ToSchema,
    schemars::JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = crate::database::schema::sql_types::MessageKind)]
pub enum MessageKind {
    /// Text written by its author.
    Standard,
    /// The message a poll was opened with.
    Poll,
    /// The system message announcing a poll's outcome; its `author` is the poll's creator.
    PollClosed,
}

impl ToSql<crate::database::schema::sql_types::MessageKind, Pg> for MessageKind {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        out.write_all(match self {
            MessageKind::Standard => b"standard",
            MessageKind::Poll => b"poll",
            MessageKind::PollClosed => b"poll_closed",
        })?;
        Ok(IsNull::No)
    }
}

impl FromSql<crate::database::schema::sql_types::MessageKind, Pg> for MessageKind {
    fn from_sql(
        bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
    ) -> diesel::deserialize::Result<Self> {
        match bytes.as_bytes() {
            b"standard" => Ok(MessageKind::Standard),
            b"poll" => Ok(MessageKind::Poll),
            b"poll_closed" => Ok(MessageKind::PollClosed),
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
        app::user_status::spawn_expiry_listener(
            valkey_subscribe.clone(),
            valkey.clone(),
            nats_arc.clone(),
        );
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
