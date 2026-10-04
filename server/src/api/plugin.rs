//! Plugins over HTTP (`app::plugin`): the catalogue every client draws plugins' contributions
//! from, communities turning plugins on and configuring them, people's annotations, the
//! dashboard's view of installed plugins, and plugins' own routes.

use crate::api::TAG_PLUGINS;
use crate::api::admin::AdminUser;
use crate::api::auth::SessionUser;
use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::{Json, NoContent, Path};
use crate::api::message_enum::{CommunityPlugin, UserAnnotation};
use crate::app::context::GlobalServerContext;
use crate::app::deployment::DeploymentPermission;
use crate::app::permissions::Permission;
use crate::app::plugin::manifest::Manifest;
use crate::app::plugin::settings::{self, SettingField};
use crate::app::plugin::{self, Mode, PluginPermission, install};
use crate::app::{self, CommunityId, UserId};
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

/// What plugins say about a person.
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
    _session: SessionUser,
    Path(user): Path<UserId>,
) -> ApiResult<Json<Vec<UserAnnotation>>> {
    Ok(Json(plugin::annotation::of_user(&state, user).await?))
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
        Ok(answer) => (
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
            .into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}
