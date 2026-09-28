use crate::app;
use crate::app::ASPEN_NATS_STREAM_NAME;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, LOCATION, RETRY_AFTER};
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

pub(crate) mod admin;
pub(crate) mod attachment;
pub(crate) mod auth;
pub(crate) mod block;
pub(crate) mod bot;
pub(crate) mod category;
pub(crate) mod category_collapse;
pub(crate) mod channel;
pub(crate) mod channel_mute;
pub(crate) mod community;
pub(crate) mod deployment;
pub(crate) mod dm;
pub(crate) mod error;
mod event_stream;
pub(crate) mod extract;
pub(crate) mod federation;
pub(crate) mod icon;
pub(crate) mod include;
pub(crate) mod invite;
pub(crate) mod link_preview;
pub(crate) mod message;
pub(crate) mod message_enum;
pub(crate) mod metrics;
pub(crate) mod passkey_page;
pub mod poll;
pub(crate) mod rate_limit;
pub(crate) mod react;
pub(crate) mod read_state;
pub(crate) mod role;
pub(crate) mod security;
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
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
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
pub const TAG_DMS: &str = "dms";
pub const TAG_SECURITY: &str = "security";
pub const TAG_ADMIN: &str = "administration";
pub const TAG_ROLES: &str = "roles";

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
        dm::DmInclude,
        block::BlockInclude,
        poll::PollInclude,
        poll::PollOption,
        poll::PollVote,
        invite::InviteInclude
    )),
    tags(
        (name = TAG_AUTH, description = "Signing in (password, second factor, passkey), re-verifying, signing out, and session refresh"),
        (name = TAG_USERS, description = "Accounts. `@me` addresses the calling user."),
        (name = TAG_SECURITY, description = "A user's second factors: authenticator app, passkeys, and recovery codes"),
        (name = TAG_COMMUNITIES, description = "Communities and their membership"),
        (name = TAG_ROLES, description = "Roles and permissions in a community: roles, who holds them, channel and category overrides, removing members, and ownership"),
        (name = TAG_CATEGORIES, description = "Groupings of channels inside a community"),
        (name = TAG_CHANNELS, description = "Text and voice channels, threads, and DMs"),
        (name = TAG_MESSAGES, description = "Messages within a channel, and the threads they start"),
        (name = TAG_DMS, description = "Direct messages: DMs between two people and group DMs, outside any community"),
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
            .expose_headers([LOCATION, RETRY_AFTER])
            .max_age(Duration::from_secs(60 * 60)),
    )
}

/// Every API route, relative to `API_PREFIX`.
fn api_routes() -> OpenApiRouter<GlobalServerContext> {
    OpenApiRouter::new()
        .routes(routes!(auth::login))
        .routes(routes!(auth::login_second_factor))
        .routes(routes!(auth::auth_methods))
        .routes(routes!(auth::reauthenticate))
        .routes(routes!(auth::start_passkey_ceremony))
        .routes(routes!(auth::get_passkey_ceremony))
        .routes(routes!(auth::complete_passkey_ceremony))
        .routes(routes!(auth::claim_passkey_ceremony))
        .routes(routes!(auth::logout))
        .routes(routes!(auth::token_refresh))
        .routes(routes!(user::create_user))
        .routes(routes!(
            user::get_user,
            user::update_user,
            user::delete_user
        ))
        .routes(routes!(user::get_preferences, user::update_preferences))
        .routes(routes!(user::get_statuses))
        .routes(routes!(user::list_user_communities))
        .routes(routes!(user::change_password))
        .routes(routes!(security::get_security))
        .routes(routes!(security::begin_totp, security::remove_totp))
        .routes(routes!(security::confirm_totp))
        .routes(routes!(security::rename_passkey, security::remove_passkey))
        .routes(routes!(security::regenerate_recovery_codes))
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
        .routes(routes!(role::list_roles, role::create_role))
        .routes(routes!(role::update_role, role::delete_role))
        .routes(routes!(role::reorder_roles))
        .routes(routes!(role::add_member_role, role::remove_member_role))
        .routes(routes!(role::remove_member, bot::add_bot))
        .routes(routes!(role::transfer_ownership))
        .routes(routes!(
            role::set_channel_override,
            role::clear_channel_override
        ))
        .routes(routes!(
            role::set_category_override,
            role::clear_category_override
        ))
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
        .routes(routes!(message::open_thread))
        .routes(routes!(message::pin_message, message::unpin_message))
        .routes(routes!(message::remove_attachment))
        .routes(routes!(dm::open_dm, dm::list_dms))
        .routes(routes!(dm::add_recipient))
        .routes(routes!(dm::leave_dm))
        .routes(routes!(react::add_reaction, react::remove_reaction))
        .routes(routes!(react::list_reactors))
        .routes(routes!(react::remove_users_reaction))
        .routes(routes!(admin::get_admin_access))
        .routes(routes!(admin::get_overview))
        .routes(routes!(admin::list_users))
        .routes(routes!(admin::list_communities))
        .routes(routes!(
            admin::list_registration_invites,
            admin::create_registration_invite
        ))
        .routes(routes!(admin::revoke_registration_invite))
        .routes(routes!(admin::get_fleet))
        .routes(routes!(admin::get_growth))
        .routes(routes!(federation::get_federation))
        .routes(routes!(
            federation::list_deployments,
            federation::add_deployment
        ))
        .routes(routes!(
            federation::get_deployment,
            federation::update_deployment,
            federation::remove_deployment
        ))
        .routes(routes!(federation::contact_deployment))
        .routes(routes!(federation::accept_key))
        .routes(routes!(
            federation::add_to_list,
            federation::remove_from_list
        ))
        .routes(routes!(
            deployment::list_deployment_roles,
            deployment::create_deployment_role
        ))
        .routes(routes!(
            deployment::update_deployment_role,
            deployment::delete_deployment_role
        ))
        .routes(routes!(deployment::reorder_deployment_roles))
        .routes(routes!(
            deployment::add_user_deployment_role,
            deployment::remove_user_deployment_role
        ))
        .routes(routes!(deployment::read_moderation_log))
        .routes(routes!(deployment::list_user_dms))
        .routes(routes!(poll::create_poll))
        .routes(routes!(poll::get_poll))
        .routes(routes!(poll::add_vote, poll::remove_vote))
        .routes(routes!(poll::add_write_in))
        .routes(routes!(poll::remove_write_in))
        .routes(routes!(
            read_state::get_read_state,
            read_state::put_read_state
        ))
        .routes(routes!(
            channel_mute::mute_channel,
            channel_mute::unmute_channel
        ))
        .routes(routes!(block::list_blocks))
        .routes(routes!(bot::list_bots, bot::create_bot))
        .routes(routes!(bot::rotate_bot_token))
        .routes(routes!(bot::transfer_bot))
        .routes(routes!(bot::update_bot, bot::delete_bot))
        .routes(routes!(block::block_user, block::unblock_user))
        .routes(routes!(
            category_collapse::collapse_category,
            category_collapse::expand_category
        ))
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
        .route("/events", any(event_stream::event_stream))
}

/// Refreshes the gauges that are sampled rather than kept current.
fn spawn_samplers(context: GlobalServerContext) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(aspen_metrics::SAMPLE_INTERVAL);
        loop {
            interval.tick().await;
            let status = context.connection_pool.status();
            for (state, value) in [
                ("size", status.size),
                ("available", status.available),
                ("waiting", status.waiting),
                ("max", status.max_size),
            ] {
                ::metrics::gauge!(aspen_metrics::api::DB_POOL, "state" => state).set(value as f64);
            }
            let suspended = context.rate_limiter.suspension().current().is_some();
            ::metrics::gauge!(aspen_metrics::api::RATE_LIMITS_SUSPENDED).set(if suspended {
                1.0
            } else {
                0.0
            });
        }
    });
}

/// The OpenAPI document of every API route.
pub(crate) fn openapi() -> utoipa::openapi::OpenApi {
    let mut openapi = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest(API_PREFIX, api_routes())
        .to_openapi();
    rate_limit::document_rate_limits(&mut openapi);
    openapi
}

pub(crate) async fn make_router(write_schema: bool) -> Result<axum::Router, app::Error> {
    if write_schema {
        fs::write("openapi.yaml", openapi().to_yaml()?)?;
        let event_schema = schema_for!(event_stream::EventStreamProtocol);
        fs::write(
            "event_schema.json",
            serde_json::to_string_pretty(&event_schema)?,
        )?;
        std::process::exit(0);
    }
    let context = GlobalServerContext::new(&rate_limit::routes()).await?;
    // A route layer runs only for matched routes, after routing, so it knows the route's
    // template.
    // Layers run outermost last-added first: metrics see every request, refused ones too.
    let v1 = api_routes()
        .route_layer(axum::middleware::from_fn_with_state(
            context.clone(),
            rate_limit::limit_requests,
        ))
        .route_layer(axum::middleware::from_fn(metrics::observe));
    // A page, not an API: the desktop and mobile apps open it in the system browser to run a
    // passkey ceremony (`api::passkey_page`). It is limited like the API.
    let page = OpenApiRouter::<GlobalServerContext>::new()
        .route(
            rate_limit::PASSKEY_PAGE.1,
            axum::routing::get(passkey_page::page),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            context.clone(),
            rate_limit::limit_requests,
        ))
        .route_layer(axum::middleware::from_fn(metrics::observe));
    // Other deployments read this deployment's document here (`app::federation`).
    let well_known = OpenApiRouter::<GlobalServerContext>::new()
        .route(
            rate_limit::WELL_KNOWN.1,
            axum::routing::get(federation::well_known),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            context.clone(),
            rate_limit::limit_requests,
        ))
        .route_layer(axum::middleware::from_fn(metrics::observe));
    let router = OpenApiRouter::<GlobalServerContext>::new()
        .nest(API_PREFIX, v1)
        .merge(page)
        .merge(well_known);
    if context.config.metrics.enabled {
        aspen_metrics::install(context.config.metrics.listen_addr)
            .map_err(|message| app::Error::Config(config::ConfigError::Message(message)))?;
        spawn_samplers(context.clone());
    }
    app::poll::spawn_closer(context.clone());
    app::voice::seed_servers(&context).await?;
    app::voice::spawn_report_listener(context.clone()).await?;
    app::voice::spawn_reaper(context.clone());
    app::fleet::spawn_heartbeat(context.clone());
    if context.config.federation.domain.is_some() {
        app::federation::ensure_key(context.connection_pool.get().await?.as_mut()).await?;
    }
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
#[diesel(sql_type = crate::database::schema::sql_types::ChannelType)]
pub enum ChannelType {
    Text,
    Voice,
    /// Replies to one message of a text channel, DM, or group DM; see `Channel.parentChannel`.
    Thread,
    /// A conversation between two people, outside any community.
    Dm,
    /// A conversation among up to `app::dm::MAX_RECIPIENTS` people, outside any community.
    GroupDm,
}

impl ToSql<crate::database::schema::sql_types::ChannelType, Pg> for ChannelType {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        out.write_all(match self {
            ChannelType::Text => b"text",
            ChannelType::Voice => b"voice",
            ChannelType::Thread => b"thread",
            ChannelType::Dm => b"dm",
            ChannelType::GroupDm => b"group_dm",
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
            b"thread" => Ok(ChannelType::Thread),
            b"dm" => Ok(ChannelType::Dm),
            b"group_dm" => Ok(ChannelType::GroupDm),
            _ => Err(format!(
                "Unrecognized enum variant: {:?}",
                String::from_utf8_lossy(bytes.as_bytes())
            )
            .into()),
        }
    }
}

/// What a message is. Both poll kinds and echoes carry no `content`; the client renders the poll
/// kinds from the poll record the message's `poll` field names, and an echo from its reply.
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
    /// A thread reply shown in the thread's parent channel, by reference: `echoOf` names the
    /// reply, and the echo has no content of its own. Its `author` is the reply's.
    ThreadEcho,
}

impl ToSql<crate::database::schema::sql_types::MessageKind, Pg> for MessageKind {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        out.write_all(match self {
            MessageKind::Standard => b"standard",
            MessageKind::Poll => b"poll",
            MessageKind::PollClosed => b"poll_closed",
            MessageKind::ThreadEcho => b"thread_echo",
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
            b"thread_echo" => Ok(MessageKind::ThreadEcho),
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
    pub rate_limiter: Arc<app::rate_limit::RateLimiter>,
    /// The WebAuthn relying party, when `[auth.passkeys]` is configured.
    pub webauthn: Option<Arc<webauthn_rs::Webauthn>>,
    /// Where each channel belongs (`app::events::channel_home`), filled as it is asked; a
    /// channel never moves.
    pub channel_homes: Arc<Mutex<HashMap<app::ChannelId, app::events::ChannelHome>>>,
    /// The server's one reading of the event stream, which every event stream connection
    /// registers with.
    pub event_feed: app::event_feed::EventFeed,
    /// What every call to another deployment is made with (`app::federation::fetch`).
    pub federation_client: reqwest::Client,
}

impl GlobalServerContext {
    /// Connects to everything the server needs. `routes` are the API's routes, which the rate
    /// limits are checked against.
    pub async fn new(routes: &[app::rate_limit::Route]) -> Result<Self, app::Error> {
        let config = load_config()?;
        app::login::configure_password_work(
            config.auth.password_hashing_threads,
            Duration::from_secs(config.auth.password_hashing_wait_seconds),
        );
        let rate_limiter = app::rate_limit::RateLimiter::compile(&config.rate_limits, routes)
            .map_err(|message| app::Error::Config(config::ConfigError::Message(message)))?;
        let client = async_nats::connect_with_options(
            &config.nats_url,
            ConnectOptions::new().token(config.nats_auth_token.clone()),
        )
        .await?;
        aspen_limits::suspension::watch(client.clone(), rate_limiter.suspension().clone(), "api");
        let context = async_nats::jetstream::new(client);
        context
            .create_or_update_stream(async_nats::jetstream::stream::Config {
                name: ASPEN_NATS_STREAM_NAME.to_string(),
                subjects: vec![format!("{}.>", app::events::SUBJECT_ROOT)],
                discard: DiscardPolicy::Old,
                max_messages: 1_000_000_000,
                max_bytes: 8 * 1024 * 1024 * 1024,
                max_age: app::event_feed::MAX_EVENT_AGE,
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

        let media_store = Arc::new(app::media_store::MediaStore::new(&config).await?);
        let webauthn = app::passkey::relying_party(&config.auth)?;
        let federation_client = app::federation::fetch::client(&config.federation)?;

        Ok(Self {
            channel_homes: Arc::new(Mutex::new(HashMap::new())),
            connection_pool: {
                let conn_manager =
                    AsyncDieselConnectionManager::<AsyncPgConnection>::new(&config.database_url);
                Pool::builder(conn_manager).build()?
            },
            event_feed: app::event_feed::EventFeed::start(
                context.clone(),
                config.event_queue_size,
                config.event_feed_shards,
            ),
            nats_context: Arc::new(context),
            valkey,
            media_store,
            rate_limiter: Arc::new(rate_limiter),
            webauthn,
            federation_client,
            config: config.into(),
        })
    }
}
