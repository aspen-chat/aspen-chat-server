//! What the caller chooses to show of their presence (`app::presence_override`): invisible,
//! away, or do not disturb, for a while or until they change it.

use crate::TAG_USERS;
use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Json, NoContent};
use aspen_app as app;
use aspen_app::context::GlobalServerContext;
use aspen_app::presence_override::{ChosenPresence, PresenceOverride};
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What the caller shows of their presence in place of what their connections say.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PresenceOverrideState {
    /// The override in force; `null` when there is none.
    pub presence_override: Option<PresenceOverride>,
    /// When it ends by itself; `null` for one that lasts until it is changed, or for none.
    pub until: Option<DateTime<Utc>>,
}

impl From<Option<ChosenPresence>> for PresenceOverrideState {
    fn from(chosen: Option<ChosenPresence>) -> Self {
        PresenceOverrideState {
            presence_override: chosen.map(|c| c.presence_override),
            until: chosen.and_then(|c| c.until),
        }
    }
}

/// What to show of the caller's presence, and for how long.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PresenceOverrideRequest {
    pub presence_override: PresenceOverride,
    /// Seconds from now, at most `app::presence_override::MAX_OVERRIDE_SECONDS` (thirty days);
    /// absent or `null` to keep it until the caller changes it.
    #[serde(default)]
    pub duration_seconds: Option<u32>,
}

/// The caller's presence override in force, if any.
#[utoipa::path(
    get,
    path = "/users/@me/presence-override",
    tag = TAG_USERS,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, description = "The override in force, or `null` fields for none", body = PresenceOverrideState),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_presence_override(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
) -> ApiResult<Json<PresenceOverrideState>> {
    let chosen = app::presence_override::read(&state, user.id).await?;
    Ok(Json(chosen.into()))
}

/// Shows the caller as invisible (offline to everyone else), away, or in do not disturb while
/// they are connected, replacing any override in force. Do not disturb also keeps every phone
/// push and DM call ring from them, connected or not. The change reaches the caller's devices
/// as `presenceOverrideChanged`.
#[utoipa::path(
    put,
    path = "/users/@me/presence-override",
    tag = TAG_USERS,
    request_body = PresenceOverrideRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Set", body = PresenceOverrideState),
        (status = OK, description = "Replaced an override in force", body = PresenceOverrideState),
        (status = BAD_REQUEST, description = "`validation`: a duration of zero or over thirty days", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn put_presence_override(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(request): Json<PresenceOverrideRequest>,
) -> ApiResult<(StatusCode, Json<PresenceOverrideState>)> {
    let (chosen, existed) = app::presence_override::set(
        &state,
        user.id,
        request.presence_override,
        request.duration_seconds,
    )
    .await?;
    let status = if existed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(Some(chosen).into())))
}

/// Ends the caller's presence override, so their connections alone say it again. Nothing to
/// end is not an error.
#[utoipa::path(
    delete,
    path = "/users/@me/presence-override",
    tag = TAG_USERS,
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "No override is in force"),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_presence_override(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
) -> ApiResult<NoContent> {
    app::presence_override::clear(&state, user.id).await?;
    Ok(NoContent)
}
