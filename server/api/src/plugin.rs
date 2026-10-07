//! Plugins over HTTP (`app::plugin`): the catalogue every client draws plugins' contributions
//! from, communities turning plugins on and configuring them, people's annotations, the
//! dashboard's view of installed plugins, and plugins' own routes.

use crate::TAG_PLUGINS;
use crate::admin::AdminUser;
use crate::auth::SessionUser;
use crate::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::extract::{Json, NoContent, Path};
use crate::message_enum::{CommunityPlugin, UserAnnotation};
use aspen_app::context::GlobalServerContext;
use aspen_app::deployment::DeploymentPermission;
use aspen_app::permissions::Permission;
use aspen_app::plugin::manifest::Manifest;
use aspen_app::plugin::notice::NoticeRead;
use aspen_app::plugin::settings::{self, SettingField};
use aspen_app::plugin::{self, Mode, PluginPermission, install};
use aspen_app::{self as app, CommunityId, MessageId, PluginNoticeId, UserId};
use axum::body::Bytes;
use axum::extract::{RawQuery, State};
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use utoipa::ToSchema;

/// A plugin as every client knows it: what it is, and its text in the reader's language, from
/// which clients draw its annotations, `alteredBy`, and settings forms.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    /// A domain its author controls, reversed (`org.example.nocursing`).
    pub id: String,
    pub version: String,
    /// Its name, in the reader's language.
    pub name: String,
    /// What it does, in the reader's language.
    pub description: String,
    pub author: Option<String>,
    pub homepage: Option<String>,
    /// How far it reaches: `everywhere`, or `optIn` for the communities that turn it on.
    pub mode: Mode,
    /// Whether it runs in DMs, which every DM says while any plugin does.
    pub dms: bool,
    /// Its own account, when it has one.
    pub principal: Option<UserId>,
    /// The community permissions its account asks for where it is turned on.
    pub principal_permissions: Vec<Permission>,
    /// What a community that turns it on configures, labelled by keys of `messages`.
    pub community_settings: Vec<SettingField>,
    /// Its text in the reader's language, by key, with `%{name}` for what is filled in.
    pub messages: BTreeMap<String, String>,
    /// The kinds of channel it adds.
    pub channel_types: Vec<PluginChannelType>,
}

/// A kind of channel a plugin adds, as a client offers it and shows a channel of it.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PluginChannelType {
    /// What a channel of the kind names as its `pluginType`: the plugin's id and the kind's name
    /// (`org.example.forums:board`).
    pub plugin_type: String,
    /// The kind's name, in the reader's language.
    pub name: String,
    pub glyph: aspen_app::plugin::manifest::Glyph,
    /// Where the page that shows a channel of the kind is served, beneath the API's origin.
    pub view: String,
}

impl PluginInfo {
    fn new(
        manifest: &Manifest,
        mode: Mode,
        dms: bool,
        principal: Option<UserId>,
        locale: &str,
    ) -> Self {
        let text = |key: &str| {
            plugin::render(
                &manifest.messages,
                &manifest.default_language,
                locale,
                &plugin::PluginText {
                    key: key.to_string(),
                    args: Default::default(),
                },
            )
        };
        PluginInfo {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            name: text(&manifest.name),
            description: text(&manifest.description),
            author: manifest.author.clone(),
            homepage: manifest.homepage.clone(),
            mode,
            dms,
            principal,
            principal_permissions: manifest
                .principal
                .as_ref()
                .map(|p| p.permissions.clone())
                .unwrap_or_default(),
            community_settings: manifest.community_settings.clone(),
            messages: plugin::catalogue(&manifest.messages, &manifest.default_language, locale),
            channel_types: manifest
                .channel_types
                .iter()
                .map(|(kind, declared)| PluginChannelType {
                    plugin_type: format!("{}:{kind}", manifest.id),
                    name: text(&declared.name),
                    glyph: declared.glyph,
                    view: format!(
                        "{}/plugins/{}/assets/{}",
                        crate::API_PREFIX,
                        manifest.id,
                        declared.view
                    ),
                })
                .collect(),
        }
    }
}

/// The plugins this deployment runs, in the reader's language. Any signed-in caller may read
/// it; clients use it to draw what plugins contribute.
#[utoipa::path(
    get,
    path = "/plugins",
    tag = TAG_PLUGINS,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<PluginInfo>),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_plugins(
    State(state): State<GlobalServerContext>,
    _session: SessionUser,
) -> ApiResult<Json<Vec<PluginInfo>>> {
    let locale = app::locale::current();
    Ok(Json(
        state
            .plugins
            .loaded()
            .iter()
            .map(|p| {
                PluginInfo::new(
                    &p.manifest,
                    p.mode,
                    p.holds(PluginPermission::Dms),
                    p.principal,
                    locale,
                )
            })
            .collect(),
    ))
}

/// Every plugin this deployment runs, with the community's use of it. Takes Manage plugins.
#[utoipa::path(
    get,
    path = "/communities/{community}/plugins",
    tag = TAG_PLUGINS,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<CommunityPlugin>),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: Manage plugins is missing", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_community_plugins(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
) -> ApiResult<Json<Vec<CommunityPlugin>>> {
    Ok(Json(
        plugin::community::list(&state, user.id, community).await?,
    ))
}

/// Turning a plugin on in a community.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CommunityPluginEnableRequest {
    /// Its settings there, laid over any it had (`null` restores a setting's default).
    #[serde(default)]
    #[schema(value_type = HashMap<String, serde_json::Value>)]
    pub settings: Map<String, Value>,
    /// What its account is given, on a role of its own; only permissions it asks for. Granting
    /// any takes Add bots, Manage roles, and Assign roles, and each must be held by the caller.
    #[serde(default)]
    pub grant: Vec<Permission>,
}

/// Turns a plugin on in the community, bringing in its account with what is granted; for a
/// plugin that runs everywhere, sets its settings there. Takes Manage plugins, and to bring in
/// its account, Add bots.
#[utoipa::path(
    put,
    path = "/communities/{community}/plugins/{plugin}",
    tag = TAG_PLUGINS,
    params(("community" = CommunityId, Path), ("plugin" = String, Path)),
    request_body = CommunityPluginEnableRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Turned on", body = CommunityPlugin),
        (status = OK, description = "Already on; its settings were changed", body = CommunityPlugin),
        (status = BAD_REQUEST, description = "`validation`: a setting is not what its field takes, or a permission granted is not one it asks for", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: Manage plugins, Add bots, or what granting the permissions takes, is missing", body = Problem),
        (status = NOT_FOUND, description = "No such community, or no such plugin running", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn enable_community_plugin(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((community, plugin_id)): Path<(CommunityId, String)>,
    Json(request): Json<CommunityPluginEnableRequest>,
) -> ApiResult<(StatusCode, Json<CommunityPlugin>)> {
    let (record, turned_on) = plugin::community::enable(
        &state,
        user.id,
        community,
        &plugin_id,
        request.settings,
        request.grant,
    )
    .await?;
    let status = if turned_on {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(record)))
}

/// Changes a plugin's settings in the community, as a JSON Merge Patch over them. Takes Manage
/// plugins.
#[utoipa::path(
    patch,
    path = "/communities/{community}/plugins/{plugin}",
    tag = TAG_PLUGINS,
    params(("community" = CommunityId, Path), ("plugin" = String, Path)),
    request_body(content = HashMap<String, serde_json::Value>, description = "Settings by name; `null` restores a setting's default"),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = CommunityPlugin),
        (status = BAD_REQUEST, description = "`validation`: a setting is not what its field takes", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: Manage plugins is missing", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn configure_community_plugin(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((community, plugin_id)): Path<(CommunityId, String)>,
    Json(patch): Json<Map<String, Value>>,
) -> ApiResult<Json<CommunityPlugin>> {
    Ok(Json(
        plugin::community::configure(&state, user.id, community, &plugin_id, patch).await?,
    ))
}

/// Turns a plugin off in the community, taking its account out; its settings are kept. A
/// plugin that runs everywhere cannot be turned off. Takes Manage plugins.
#[utoipa::path(
    delete,
    path = "/communities/{community}/plugins/{plugin}",
    tag = TAG_PLUGINS,
    params(("community" = CommunityId, Path), ("plugin" = String, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Off"),
        (status = BAD_REQUEST, description = "`validation`: it runs everywhere", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: Manage plugins is missing", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn disable_community_plugin(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((community, plugin_id)): Path<(CommunityId, String)>,
) -> ApiResult<NoContent> {
    plugin::community::disable(&state, user.id, community, &plugin_id).await?;
    Ok(NoContent)
}

/// What plugins say about a person, for whoever shares a community with them and for themself;
/// anyone else is answered an empty list, as the events of them reach no one else either.
#[utoipa::path(
    get,
    path = "/users/{user}/annotations",
    tag = TAG_PLUGINS,
    params(("user" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<UserAnnotation>),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_user_annotations(
    State(state): State<GlobalServerContext>,
    SessionUser { user: viewer, .. }: SessionUser,
    Path(user): Path<UserId>,
) -> ApiResult<Json<Vec<UserAnnotation>>> {
    Ok(Json(
        plugin::annotation::of_user(&state, viewer.id, user).await?,
    ))
}

/// An installed plugin as the dashboard shows it.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AdminPlugin {
    pub plugin: PluginInfo,
    /// Whether it runs.
    pub enabled: bool,
    /// Its place in the order intercepting plugins run in.
    pub position: i32,
    /// What the operator granted.
    pub granted: Vec<PluginPermission>,
    /// What the operator configures, labelled by keys of `plugin.messages`.
    pub settings_fields: Vec<SettingField>,
    /// The deployment's settings, without their secrets.
    #[schema(value_type = HashMap<String, serde_json::Value>)]
    pub settings: Value,
    /// The secret settings that are set, whose values are never read back.
    pub secrets_set: Vec<String>,
    /// The hosts it may call.
    pub hosts: Vec<String>,
    /// What it keeps of what it sees, and for how long, in the reader's language.
    pub retention: String,
    /// The bytes of storage it keeps, and the most it may.
    pub storage_bytes: i64,
    pub storage_quota: Option<u64>,
}

impl AdminPlugin {
    fn new(installed: install::Installed, locale: &str) -> Self {
        let (shown, secrets_set) =
            settings::readable(&installed.manifest.settings, &installed.settings);
        let retention = plugin::render(
            &installed.manifest.messages,
            &installed.manifest.default_language,
            locale,
            &plugin::PluginText {
                key: installed.manifest.retention.clone(),
                args: Default::default(),
            },
        );
        AdminPlugin {
            plugin: PluginInfo::new(
                &installed.manifest,
                installed.mode,
                installed.granted.contains(&PluginPermission::Dms),
                installed.principal,
                locale,
            ),
            enabled: installed.enabled,
            position: installed.position,
            granted: installed.granted.iter().copied().collect(),
            settings_fields: installed.manifest.settings.clone(),
            settings: Value::Object(shown),
            secrets_set,
            hosts: installed.manifest.hosts.clone(),
            retention,
            storage_bytes: installed.storage_bytes,
            storage_quota: installed.manifest.storage_quota,
        }
    }
}

/// Every installed plugin, on or off, in order. Takes View dashboard or Manage plugins.
#[utoipa::path(
    get,
    path = "/admin/plugins",
    tag = TAG_PLUGINS,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<AdminPlugin>),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without View dashboard or Manage plugins", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_admin_plugins(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<Vec<AdminPlugin>>> {
    if !access.has(DeploymentPermission::ManagePlugins) {
        access.require(DeploymentPermission::ViewDashboard)?;
    }
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let locale = app::locale::current();
    Ok(Json(
        install::list(conn.as_mut(), false)
            .await?
            .into_iter()
            .map(|p| AdminPlugin::new(p, locale))
            .collect(),
    ))
}

/// A change to an installed plugin. An absent field is unchanged.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AdminPluginUpdateRequest {
    /// Whether it runs. Turning it on needs every required setting.
    pub enabled: Option<bool>,
    pub mode: Option<Mode>,
    /// Its settings, laid over them (`null` restores a setting's default).
    #[schema(value_type = Option<HashMap<String, serde_json::Value>>)]
    pub settings: Option<Map<String, Value>>,
}

/// Turns an installed plugin on or off, changes its mode, or configures it. Takes Manage
/// plugins. Installing, upgrading, and removing plugins, and granting their permissions, are
/// the terminal's (`aspen-chat-server plugins`).
#[utoipa::path(
    patch,
    path = "/admin/plugins/{plugin}",
    tag = TAG_PLUGINS,
    params(("plugin" = String, Path)),
    request_body = AdminPluginUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = AdminPlugin),
        (status = BAD_REQUEST, description = "`validation`: a setting is not what its field takes, or one it needs to run is missing", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage plugins", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_admin_plugin(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(plugin_id): Path<String>,
    Json(request): Json<AdminPluginUpdateRequest>,
) -> ApiResult<Json<AdminPlugin>> {
    access.require(DeploymentPermission::ManagePlugins)?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let updated = install::update(
        conn.as_mut(),
        &plugin_id,
        request.enabled,
        request.mode,
        request.settings.as_ref(),
    )
    .await?;
    drop(conn);
    plugin::registry::announce(&state.nats_context.client(), None).await?;
    Ok(Json(AdminPlugin::new(updated, app::locale::current())))
}

/// The order intercepting plugins run in.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginOrderRequest {
    /// Plugins' ids, first to run first; those left out follow in the order they were in.
    pub plugins: Vec<String>,
}

/// Orders the installed plugins. Takes Manage plugins.
#[utoipa::path(
    put,
    path = "/admin/plugin-order",
    tag = TAG_PLUGINS,
    request_body = PluginOrderRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Ordered"),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage plugins", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn order_plugins(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Json(request): Json<PluginOrderRequest>,
) -> ApiResult<NoContent> {
    access.require(DeploymentPermission::ManagePlugins)?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    install::order(conn.as_mut(), &request.plugins).await?;
    drop(conn);
    plugin::registry::announce(&state.nats_context.client(), None).await?;
    Ok(NoContent)
}

/// A request to a plugin's route, `/plugins/{plugin}/routes/{*path}`, outside the OpenAPI
/// document since each plugin's routes are its own. Answered by the plugin as the caller.
pub async fn route(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    axum::extract::Path((plugin_id, path)): axum::extract::Path<(String, String)>,
    method: Method,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if body.len() > plugin::route::MAX_BODY {
        return ApiError::new(ProblemCode::Validation)
            .with_detail(crate::t!(
                "pluginRouteBodyTooLarge",
                max = plugin::route::MAX_BODY
            ))
            .into_response();
    }
    let request = plugin::route::Request {
        method: method.as_str().to_string(),
        path,
        query: query.unwrap_or_default(),
        content_type: headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
        body: body.to_vec(),
    };
    match plugin::route::answer(&state, &plugin_id, user.id, request).await {
        Ok(answer) => answered(answer),
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// A file of a plugin's views, served to the frame that shows it, which holds no session, with a
/// sandbox that gives the page an origin of its own (`app::plugin::asset::VIEW_POLICY`).
pub async fn asset(
    State(state): State<GlobalServerContext>,
    axum::extract::Path((plugin_id, path)): axum::extract::Path<(String, String)>,
) -> Response {
    let found = state
        .plugins
        .get(&plugin_id)
        .filter(|p| p.holds(PluginPermission::Views))
        .and_then(|p| p.assets.get(&path).cloned());
    match found {
        Some(asset) => (
            [
                (header::CONTENT_TYPE, asset.content_type),
                (
                    header::CONTENT_SECURITY_POLICY,
                    plugin::asset::VIEW_POLICY.to_string(),
                ),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
                (header::CACHE_CONTROL, "no-cache".to_string()),
            ],
            asset.bytes,
        )
            .into_response(),
        None => ApiError::new(ProblemCode::NotFound).into_response(),
    }
}

/// Someone following a private URL a plugin gave them: the plugin answers as them.
pub async fn capability(
    State(state): State<GlobalServerContext>,
    axum::extract::Path((plugin_id, secret)): axum::extract::Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    match plugin::capability::follow(&state, &plugin_id, &secret, query.unwrap_or_default()).await {
        Ok(answer) => answered(answer),
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// A plugin's answer, made safe to serve.
fn answered(answer: plugin::route::Answer) -> Response {
    (
        StatusCode::from_u16(answer.status).unwrap_or(StatusCode::OK),
        [
            (header::CONTENT_TYPE, answer.content_type),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; sandbox".to_string(),
            ),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff".to_string()),
        ],
        answer.body,
    )
        .into_response()
}

/// Presses a button of a message's card: the card's plugin answers as the caller, who must be
/// able to read the message. The answer is the plugin's, as it gave it.
#[utoipa::path(
    post,
    path = "/messages/{message}/card/buttons/{button}",
    tag = TAG_PLUGINS,
    params(("message" = MessageId, Path), ("button" = String, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "The plugin's answer, as it gave it"),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: the caller may not read the message", body = Problem),
        (status = NOT_FOUND, description = "No such message, card, or button, or its plugin does not run there", body = Problem),
        (status = SERVICE_UNAVAILABLE, description = "`pluginUnavailable`: the plugin could not answer", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn press_card_button(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((message, button)): Path<(MessageId, String)>,
) -> Response {
    match plugin::card::press(&state, user.id, message, &button).await {
        Ok(answer) => answered(answer),
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// One of the caller's notices from a plugin, as their phone reads it when woken for it: the
/// plugin's name and what it says, in the caller's language, while they may still view its
/// channel.
#[utoipa::path(
    get,
    path = "/users/@me/plugin-notices/{notice}",
    tag = TAG_PLUGINS,
    params(("notice" = PluginNoticeId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = NoticeRead),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: the caller may no longer view its channel", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_plugin_notice(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(notice): Path<PluginNoticeId>,
) -> ApiResult<Json<NoticeRead>> {
    Ok(Json(plugin::notice::read(&state, user.id, notice).await?))
}
