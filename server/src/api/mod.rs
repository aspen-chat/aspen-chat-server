use crate::api::error::{ApiError, ProblemCode};
use crate::app;
use crate::app::context::{GlobalServerContext, Role};
use crate::aspen_config::AspenConfig;
use axum::http::Method;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, LOCATION, RETRY_AFTER};
use axum::routing::any;
use tower_http::cors::{AllowOrigin, CorsLayer};

pub(crate) mod admin;
pub(crate) mod attachment;
pub(crate) mod auth;
pub(crate) mod ban;
pub(crate) mod block;
pub(crate) mod bot;
pub mod bot_command;
pub(crate) mod category;
pub(crate) mod category_collapse;
pub(crate) mod channel;
pub(crate) mod channel_mute;
pub(crate) mod community;
pub(crate) mod custom_emoji;
pub(crate) mod deployment;
pub(crate) mod deployment_settings;
pub(crate) mod device_link;
pub(crate) mod dm;
pub(crate) mod email;
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
pub mod notification_setting;
pub(crate) mod passkey_page;
pub(crate) mod plugin;
pub mod poll;
pub mod push;
pub(crate) mod rate_limit;
pub(crate) mod react;
pub(crate) mod read_state;
pub(crate) mod report;
pub(crate) mod role;
mod schema;
pub(crate) mod security;
pub(crate) mod user;
pub mod voice;
pub(crate) mod web_client;

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
pub const TAG_DMS: &str = "dms";
pub const TAG_SECURITY: &str = "security";
pub const TAG_ADMIN: &str = "administration";
pub const TAG_DEPLOYMENT: &str = "deployment";
pub const TAG_ROLES: &str = "roles";
pub const TAG_CUSTOM_EMOJI: &str = "custom emoji";
pub const TAG_BANS: &str = "bans";
pub const TAG_REPORTS: &str = "reports";
pub const TAG_PLUGINS: &str = "plugins";

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
        message::MessageHolding,
        dm::DmInclude,
        block::BlockInclude,
        poll::PollInclude,
        poll::PollOption,
        poll::PollVote,
        invite::InviteInclude,
        invite::RegistrationInviteInclude
    )),
    tags(
        (name = TAG_AUTH, description = "Signing in (password, second factor, passkey, or from another device by a QR code), re-verifying, signing out, and session refresh"),
        (name = TAG_USERS, description = "Accounts. `@me` addresses the calling user."),
        (name = TAG_SECURITY, description = "A user's second factors: authenticator app, passkeys, and recovery codes"),
        (name = TAG_COMMUNITIES, description = "Communities and their membership"),
        (name = TAG_ROLES, description = "Roles and permissions in a community: roles, who holds them, channel and category overrides, removing members, and ownership"),
        (name = TAG_CUSTOM_EMOJI, description = "A community's own emoji: listing, adding, renaming, and removing them"),
        (name = TAG_BANS, description = "Bans from a community: listing, banning, and lifting"),
        (name = TAG_REPORTS, description = "Reports of messages and profiles to the deployment's moderators, their review, and the categories they are made in"),
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
        (name = TAG_DEPLOYMENT, description = "How the deployment presents itself: its display name and icon"),
        (name = TAG_PLUGINS, description = "The deployment's plugins: their catalogue, communities turning them on and configuring them, what they say about people, and the dashboard's view of them. Each plugin's own routes, `/plugins/{plugin}/routes/{*path}`, are the plugin's and not described here."),
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

/// The CORS layer, which lets a page of any origin call the API. The web client is served at
/// the API's own origin, but the desktop app's pages (`file:`), the mobile apps' (an app-local
/// origin), and other deployments' web clients, whose users this deployment may admit, are each
/// elsewhere. Every request is authenticated by a bearer token rather than a cookie, so no
/// origin gains anything a page could not already do with the token it holds.
fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::any())
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE])
        .expose_headers([LOCATION, RETRY_AFTER])
        .max_age(Duration::from_secs(60 * 60))
}

/// Every API route, relative to `API_PREFIX`.
fn api_routes() -> OpenApiRouter<GlobalServerContext> {
    OpenApiRouter::new()
        .routes(routes!(auth::login))
        .routes(routes!(auth::login_second_factor))
        .routes(routes!(auth::auth_methods))
        .routes(routes!(
            deployment_settings::get_deployment_profile,
            deployment_settings::update_deployment_profile
        ))
        .routes(routes!(
            deployment_settings::get_settings,
            deployment_settings::update_settings
        ))
        .routes(routes!(auth::reauthenticate))
        .routes(routes!(auth::start_passkey_ceremony))
        .routes(routes!(auth::get_passkey_ceremony))
        .routes(routes!(auth::complete_passkey_ceremony))
        .routes(routes!(auth::claim_passkey_ceremony))
        .routes(routes!(device_link::start_device_link))
        .routes(routes!(
            device_link::get_device_link,
            device_link::cancel_device_link
        ))
        .routes(routes!(device_link::scan_device_link))
        .routes(routes!(device_link::approve_device_link))
        .routes(routes!(device_link::claim_device_link))
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
        .routes(routes!(ban::read_community_bans))
        .routes(routes!(ban::ban_community_member, ban::lift_community_ban))
        .routes(routes!(
            custom_emoji::read_emoji,
            custom_emoji::create_emoji
        ))
        .routes(routes!(
            custom_emoji::update_emoji,
            custom_emoji::delete_emoji
        ))
        .routes(routes!(role::update_role, role::delete_role))
        .routes(routes!(role::reorder_roles))
        .routes(routes!(role::add_member_role, role::remove_member_role))
        .routes(routes!(
            community::get_community_member,
            role::remove_member,
            bot::add_bot
        ))
        .routes(routes!(community::clear_nickname))
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
        .routes(routes!(invite::get_registration_invite))
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
        .routes(routes!(channel::get_channel_presence))
        .routes(routes!(
            message::create_message,
            message::list_channel_messages
        ))
        .routes(routes!(
            message::get_message,
            message::update_message,
            message::delete_message
        ))
        .routes(routes!(message::search_messages))
        .routes(routes!(message::list_held_messages))
        .routes(routes!(push::create_push_subscription))
        .routes(routes!(push::delete_push_subscription))
        .routes(routes!(message::open_thread))
        .routes(routes!(message::echo_reply))
        .routes(routes!(message::pin_message, message::unpin_message))
        .routes(routes!(message::remove_attachment))
        .routes(routes!(email::get_email, email::update_email))
        .routes(routes!(
            email::set_email_address,
            email::remove_email_address
        ))
        .routes(routes!(email::resend_email_verification))
        .routes(routes!(email::verify_email))
        .routes(routes!(email::start_password_reset))
        .routes(routes!(email::send_password_reset_code))
        .routes(routes!(email::complete_password_reset))
        .routes(routes!(
            email::list_newsletter_posts,
            email::create_newsletter_post
        ))
        .routes(routes!(
            email::get_newsletter_post,
            email::update_newsletter_post,
            email::delete_newsletter_post
        ))
        .routes(routes!(email::test_newsletter_post))
        .routes(routes!(email::send_newsletter_post))
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
        .routes(routes!(admin::ban_user, admin::lift_ban))
        .routes(routes!(report::list_report_categories))
        .routes(routes!(report::report_message))
        .routes(routes!(report::report_profile))
        .routes(routes!(report::report_nickname))
        .routes(routes!(report::list_report_cases))
        .routes(routes!(report::get_report_counts))
        .routes(routes!(report::get_report_case))
        .routes(routes!(report::get_report_context))
        .routes(routes!(report::resolve_report_case))
        .routes(routes!(
            report::dismiss_report_case,
            report::restore_report_case
        ))
        .routes(routes!(
            report::list_all_report_categories,
            report::create_report_category
        ))
        .routes(routes!(report::update_report_category))
        .routes(routes!(report::order_report_categories))
        .routes(routes!(federation::issue_assertion))
        .routes(routes!(federation::federated_sign_in))
        .routes(routes!(federation::list_foreign_deployments))
        .routes(routes!(federation::forget_foreign_deployment))
        .routes(routes!(federation::home_avatar))
        .routes(routes!(federation::receive_notice))
        .routes(routes!(federation::answer_standing))
        .routes(routes!(
            federation::get_federation,
            federation::update_federation
        ))
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
        .routes(routes!(deployment::read_file_transfer_log))
        .routes(routes!(deployment::list_user_dms))
        .routes(routes!(poll::create_poll))
        .routes(routes!(poll::get_poll))
        .routes(routes!(poll::close_poll))
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
        .routes(routes!(
            notification_setting::set_community_notifications,
            notification_setting::reset_community_notifications
        ))
        .routes(routes!(
            notification_setting::set_channel_notifications,
            notification_setting::reset_channel_notifications
        ))
        .routes(routes!(block::list_blocks))
        .routes(routes!(bot::list_bots, bot::create_bot))
        .routes(routes!(bot::rotate_bot_token))
        .routes(routes!(bot::transfer_bot))
        .routes(routes!(bot::update_bot, bot::delete_bot))
        .routes(routes!(
            bot_command::publish_bot_commands,
            bot_command::read_bot_commands
        ))
        .routes(routes!(
            bot_command::channel_commands,
            bot_command::invoke_command
        ))
        .routes(routes!(block::block_user, block::unblock_user))
        .routes(routes!(
            category_collapse::collapse_category,
            category_collapse::expand_category
        ))
        .routes(routes!(voice::join_voice))
        .routes(routes!(voice::get_channel_voice))
        .routes(routes!(voice::decline_call))
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
            attachment::update_attachment,
            attachment::delete_attachment
        ))
        .routes(routes!(icon::init_icon_upload))
        .routes(routes!(icon::confirm_icon_upload))
        .routes(routes!(icon::get_icon, icon::delete_icon))
        .routes(routes!(plugin::list_plugins))
        .routes(routes!(plugin::list_community_plugins))
        .routes(routes!(
            plugin::enable_community_plugin,
            plugin::configure_community_plugin,
            plugin::disable_community_plugin
        ))
        .routes(routes!(plugin::list_user_annotations))
        .routes(routes!(plugin::list_admin_plugins))
        .routes(routes!(plugin::update_admin_plugin))
        .routes(routes!(plugin::order_plugins))
        .routes(routes!(plugin::press_card_button))
        .routes(routes!(plugin::read_plugin_notice))
        // A plugin's routes, its views' files, and its capability URLs are its own, so they are
        // not in the OpenAPI document.
        .route(rate_limit::PLUGIN_ASSET, axum::routing::get(plugin::asset))
        .route(
            rate_limit::PLUGIN_CAPABILITY,
            axum::routing::get(plugin::capability),
        )
        .route(
            rate_limit::PLUGIN_ROUTE,
            axum::routing::get(plugin::route)
                .post(plugin::route)
                .put(plugin::route)
                .patch(plugin::route)
                .delete(plugin::route),
        )
        // The event stream is a WebSocket and has no OpenAPI representation; its frames are
        // described by `event_schema.json`.
        .route("/events", any(event_stream::event_stream))
}

/// The OpenAPI document of every API route.
pub(crate) fn openapi() -> utoipa::openapi::OpenApi {
    let mut openapi = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .nest(API_PREFIX, api_routes())
        .to_openapi();
    rate_limit::document_rate_limits(&mut openapi);
    openapi
}

/// Handles a request inside `app::events::settle_after`, which settles what it published once
/// it is done (`app::events::settle`): the calls its events changed access to are rechecked,
/// and when it failed, or its client went away before it finished, after publishing about
/// communities, those are announced as possibly not having happened, since its transaction may
/// have been rolled back after they were published.
async fn settle_after_request(
    axum::extract::State(state): axum::extract::State<GlobalServerContext>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    app::events::settle_after(&state, next.run(request), |response| {
        let status = response.status();
        status.is_client_error() || status.is_server_error()
    })
    .await
}

/// Starts the server's work: connects to the services, starts the metrics listener and the
/// background tasks, and, for a [`Role::Public`] server, which refuses to start without the web
/// client, returns the router that serves the deployment.
pub(crate) async fn start(
    write_schema: bool,
    role: Role,
) -> Result<Option<(axum::Router, Arc<AspenConfig>)>, app::Error> {
    if write_schema {
        schema::write_schemas_and_exit()?;
    }
    let context = GlobalServerContext::new(&rate_limit::routes(), role).await?;
    if role == Role::Public {
        web_client::check(&context.config)?;
    }
    if context.config.metrics.enabled {
        aspen_metrics::install(context.config.metrics.listen_addr)
            .map_err(|message| app::Error::Config(config::ConfigError::Message(message)))?;
        metrics::spawn_samplers(context.clone());
    }
    app::context::start_background_tasks(&context).await?;
    if role == Role::PrivateWorker {
        return Ok(None);
    }
    // A route layer runs only for matched routes, after routing, so it knows the route's
    // template.
    // Layers run outermost last-added first: metrics see every request, refused ones too.
    let v1 = api_routes()
        .route_layer(axum::middleware::from_fn_with_state(
            context.clone(),
            settle_after_request,
        ))
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
    // Every unsubscribe link in mail opens this page, which answers mail programs' one-click
    // unsubscribe too (`api::email`).
    let unsubscribe = OpenApiRouter::<GlobalServerContext>::new()
        .route(
            rate_limit::UNSUBSCRIBE_PAGE.1,
            axum::routing::get(email::unsubscribe_page).post(email::unsubscribe),
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
    // An invite's page, which checks its own limits (`api::web_client`). A path under `/api/`
    // that no route has, such as a newer version's, is not found rather than a page.
    let web = OpenApiRouter::<GlobalServerContext>::new()
        .route(
            rate_limit::WEB_CLIENT_INVITE.1,
            axum::routing::get(web_client::invite_page),
        )
        .route_layer(axum::middleware::from_fn(metrics::observe))
        .route(
            "/api/{*rest}",
            any(|| async { ApiError::new(ProblemCode::NotFound) }),
        );
    let router = OpenApiRouter::<GlobalServerContext>::new()
        .nest(API_PREFIX, v1)
        .merge(page)
        .merge(unsubscribe)
        .merge(well_known)
        .merge(web);
    // Every other path is the web client's: a file of it, or its page.
    let files = web_client::files(context.clone());
    let config = context.config.clone();
    Ok(Some((
        axum::Router::from(router.with_state(context))
            .fallback_service(files)
            .layer(axum::middleware::from_fn(app::locale::layer))
            .layer(cors_layer()),
        config,
    )))
}
