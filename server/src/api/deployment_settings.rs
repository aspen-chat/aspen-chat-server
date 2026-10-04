//! The deployment's settings (`app::deployment_settings`): how it presents itself at
//! `/deployment`, read by anyone, since the sign-in screen shows it before anyone has signed in,
//! and the policies its administrators set at `/admin/settings`. Changing either takes Manage
//! deployment settings. The federation gates are changed at `/admin/federation`
//! (`api::federation`).

use crate::api::admin::AdminUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Json, double_option};
use crate::api::icon::{Icon, icon_to_api};
use crate::api::{TAG_ADMIN, TAG_DEPLOYMENT};
use crate::app::IconId;
use crate::app::context::GlobalServerContext;
use crate::app::deployment::DeploymentPermission;
use crate::app::deployment_settings::{self, SettingsChange, WithIcon};
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// How the deployment presents itself.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentProfile {
    /// What the deployment calls itself; `null` when it has not said.
    pub display_name: Option<String>,
    /// Its picture; `null` when it has none.
    pub icon: Option<Icon>,
    /// Where its web client is served (`[web_client] url`), which links and QR codes for
    /// invites and signing in name; `null` when it has not said.
    pub web_client_url: Option<String>,
}

impl DeploymentProfile {
    fn new(state: &GlobalServerContext, read: WithIcon) -> Self {
        Self {
            display_name: read.settings.display_name,
            icon: read.icon.map(|icon| icon_to_api(state, icon)),
            web_client_url: state.config.web_client.url.clone(),
        }
    }
}

/// A change to how the deployment presents itself. An absent field is unchanged; `null` clears
/// it.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentProfileUpdateRequest {
    /// Trimmed, from 1 to 64 characters. It is also what the deployment's system account and
    /// the authenticator apps and passkey prompts of its users call it; without one, they say
    /// "Aspen".
    #[serde(default, deserialize_with = "double_option")]
    #[schema(nullable)]
    pub display_name: Option<Option<String>>,
    /// An icon whose upload is confirmed (`POST /icons`).
    #[serde(default, deserialize_with = "double_option")]
    #[schema(nullable)]
    pub icon: Option<Option<IconId>>,
}

impl From<DeploymentProfileUpdateRequest> for SettingsChange {
    fn from(request: DeploymentProfileUpdateRequest) -> Self {
        Self {
            display_name: request.display_name,
            icon: request.icon,
            ..Self::default()
        }
    }
}

/// How the deployment presents itself: its display name and icon. Unauthenticated.
#[utoipa::path(
    get,
    path = "/deployment",
    tag = TAG_DEPLOYMENT,
    responses(
        (status = OK, body = DeploymentProfile),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_deployment_profile(
    State(state): State<GlobalServerContext>,
) -> ApiResult<Json<DeploymentProfile>> {
    let read = deployment_settings::read_with_icon(&state).await?;
    Ok(Json(DeploymentProfile::new(&state, read)))
}

/// Changes the deployment's display name or icon. Takes Manage deployment settings.
#[utoipa::path(
    patch,
    path = "/deployment",
    tag = TAG_DEPLOYMENT,
    request_body = DeploymentProfileUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = DeploymentProfile),
        (status = BAD_REQUEST, description = "A display name that is blank or too long, or an icon whose upload is not confirmed", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage deployment settings", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_deployment_profile(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Json(request): Json<DeploymentProfileUpdateRequest>,
) -> ApiResult<Json<DeploymentProfile>> {
    let read = deployment_settings::update_as(&state, &access, request.into()).await?;
    Ok(Json(DeploymentProfile::new(&state, read)))
}

/// The policies the deployment's administrators set.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentSettings {
    /// Whether creating an account takes a registration invite.
    pub registration_invite_required: bool,
    /// Whether every person's account must have a second factor. A session of an account
    /// without one may only add one, or sign out, until it does.
    pub require_two_factor: bool,
    /// Whether people may make bots. Bots already made keep working either way.
    pub bots_enabled: bool,
    /// The most bots one person may own.
    pub bots_max_per_user: u32,
    /// How many members a community gains before Mention everyone is taken from its everyone
    /// role and its owner told why; 0 never.
    pub everyone_mention_limit: u32,
    /// The most custom emoji one community may hold.
    pub custom_emoji_limit: u32,
    /// Whether people may offer files to one another in calls. Off, no one may, whatever a
    /// channel's permissions say.
    pub file_transfers: bool,
}

impl From<&deployment_settings::DeploymentSettings> for DeploymentSettings {
    fn from(settings: &deployment_settings::DeploymentSettings) -> Self {
        Self {
            registration_invite_required: settings.registration_invite_required,
            require_two_factor: settings.require_two_factor,
            bots_enabled: settings.bots_enabled,
            bots_max_per_user: settings.bots_max_per_user,
            everyone_mention_limit: settings.everyone_mention_limit,
            custom_emoji_limit: settings.custom_emoji_limit,
            file_transfers: settings.file_transfers,
        }
    }
}

/// A change to the deployment's policies. An absent field is unchanged.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentSettingsUpdateRequest {
    pub registration_invite_required: Option<bool>,
    pub require_two_factor: Option<bool>,
    pub bots_enabled: Option<bool>,
    /// At most 2147483647, as are the other counts.
    pub bots_max_per_user: Option<u32>,
    pub everyone_mention_limit: Option<u32>,
    pub custom_emoji_limit: Option<u32>,
    pub file_transfers: Option<bool>,
}

impl From<DeploymentSettingsUpdateRequest> for SettingsChange {
    fn from(request: DeploymentSettingsUpdateRequest) -> Self {
        Self {
            registration_invite_required: request.registration_invite_required,
            require_two_factor: request.require_two_factor,
            bots_enabled: request.bots_enabled,
            bots_max_per_user: request.bots_max_per_user,
            everyone_mention_limit: request.everyone_mention_limit,
            custom_emoji_limit: request.custom_emoji_limit,
            file_transfers: request.file_transfers,
            ..Self::default()
        }
    }
}

/// The deployment's policies. Takes Manage deployment settings.
#[utoipa::path(
    get,
    path = "/admin/settings",
    tag = TAG_ADMIN,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = DeploymentSettings),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage deployment settings", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_settings(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<DeploymentSettings>> {
    access.require(DeploymentPermission::ManageDeploymentSettings)?;
    let read = deployment_settings::read_with_icon(&state).await?;
    Ok(Json((&read.settings).into()))
}

/// Changes the deployment's policies, for every server at once. Turning on two factors closes
/// the event streams of accounts without one; turning file transfers on or off brings every
/// call in line. Takes Manage deployment settings.
#[utoipa::path(
    patch,
    path = "/admin/settings",
    tag = TAG_ADMIN,
    request_body = DeploymentSettingsUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = DeploymentSettings),
        (status = BAD_REQUEST, description = "A count too large", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage deployment settings", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_settings(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Json(request): Json<DeploymentSettingsUpdateRequest>,
) -> ApiResult<Json<DeploymentSettings>> {
    let read = deployment_settings::update_as(&state, &access, request.into()).await?;
    Ok(Json((&read.settings).into()))
}
