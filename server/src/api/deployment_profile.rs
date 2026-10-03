//! How the deployment presents itself (`app::deployment_profile`) at `/deployment`: read by
//! anyone, since the sign-in screen shows it before anyone has signed in, and changed from the
//! Administration Dashboard with Manage federation.

use crate::api::TAG_DEPLOYMENT;
use crate::api::admin::AdminUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Json, double_option};
use crate::api::icon::{Icon, icon_to_api};
use crate::app::IconId;
use crate::app::context::GlobalServerContext;
use crate::app::deployment_profile::{self, DeploymentProfileChange};
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
}

impl DeploymentProfile {
    fn new(state: &GlobalServerContext, profile: deployment_profile::DeploymentProfile) -> Self {
        Self {
            display_name: profile.display_name,
            icon: profile.icon.map(|icon| icon_to_api(state, icon)),
        }
    }
}

/// A change to how the deployment presents itself. An absent field is unchanged; `null` clears
/// it.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentProfileUpdateRequest {
    /// Trimmed, from 1 to 64 characters.
    #[serde(default, deserialize_with = "double_option")]
    #[schema(nullable)]
    pub display_name: Option<Option<String>>,
    /// An icon whose upload is confirmed (`POST /icons`).
    #[serde(default, deserialize_with = "double_option")]
    #[schema(nullable)]
    pub icon: Option<Option<IconId>>,
}

impl From<DeploymentProfileUpdateRequest> for DeploymentProfileChange {
    fn from(request: DeploymentProfileUpdateRequest) -> Self {
        Self {
            display_name: request.display_name,
            icon: request.icon,
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
    let profile = deployment_profile::read_profile(&state).await?;
    Ok(Json(DeploymentProfile::new(&state, profile)))
}

/// Changes the deployment's display name or icon. Takes Manage federation.
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
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage federation", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_deployment_profile(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Json(request): Json<DeploymentProfileUpdateRequest>,
) -> ApiResult<Json<DeploymentProfile>> {
    let profile = deployment_profile::update_profile(&state, &access, request.into()).await?;
    Ok(Json(DeploymentProfile::new(&state, profile)))
}
