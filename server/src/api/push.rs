//! A phone asking to be woken when it is not running Aspen (`app::push`, `spec/push.md`).

use crate::api::auth::SessionUser;
use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::{Created, Json, NoContent, Path};
use crate::api::{API_PREFIX, GlobalServerContext, TAG_USERS};
use crate::app::{self, PushSubscriptionId};
use crate::t;
use axum::extract::State;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Where push stands on this deployment, in `GET /auth/methods`.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PushSupport {
    /// The deployment's push key (RFC 8292: the uncompressed P-256 point, base64url), which an
    /// app gives its relay or distributor when it subscribes, so that only this deployment can
    /// push to the subscription.
    pub application_server_key: String,
}

/// A phone to wake: what its relay or UnifiedPush distributor gave it, and its keys (RFC 8291).
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscriptionRequest {
    /// The endpoint to push to, an HTTPS URL.
    pub endpoint: String,
    /// The phone's P-256 public key, uncompressed, base64url.
    pub p256dh: String,
    /// The phone's 16-byte authentication secret, base64url.
    pub auth: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PushSubscription {
    pub id: PushSubscriptionId,
    pub endpoint: String,
    pub created_at: DateTime<Utc>,
}

impl From<app::push::PushSubscription> for PushSubscription {
    fn from(row: app::push::PushSubscription) -> Self {
        PushSubscription {
            id: row.id,
            endpoint: row.endpoint,
            created_at: row.created_at,
        }
    }
}

/// Registers the calling sign-in's phone to be woken, replacing any it registered before. It
/// is woken until the sign-in ends or it is deleted.
#[utoipa::path(
    post,
    path = "/users/@me/push-subscriptions",
    tag = TAG_USERS,
    request_body = PushSubscriptionRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = PushSubscription, headers(("Location" = String, description = "URL of the subscription"))),
        (status = BAD_REQUEST, description = "`validation`: the endpoint is not an HTTPS URL, the keys are malformed, push is off here, or the caller is a bot", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_push_subscription(
    State(state): State<GlobalServerContext>,
    SessionUser { caller, .. }: SessionUser,
    Json(request): Json<PushSubscriptionRequest>,
) -> ApiResult<Created<PushSubscription>> {
    let decode = |value: &str| URL_SAFE_NO_PAD.decode(value.trim_end_matches('='));
    let (Ok(p256dh), Ok(auth)) = (decode(&request.p256dh), decode(&request.auth)) else {
        return Err(ApiError::new(ProblemCode::Validation).with_detail(t!("pushKeys")));
    };
    let subscription = app::push::subscribe(
        &state,
        &caller,
        app::push::NewSubscription {
            endpoint: request.endpoint,
            p256dh,
            auth,
        },
    )
    .await?;
    Ok(Created::new(
        format!(
            "{API_PREFIX}/users/@me/push-subscriptions/{}",
            subscription.id.0
        ),
        subscription.into(),
    ))
}

/// Stops waking one of the caller's phones.
#[utoipa::path(
    delete,
    path = "/users/@me/push-subscriptions/{subscription}",
    tag = TAG_USERS,
    params(("subscription" = PushSubscriptionId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_push_subscription(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(subscription): Path<PushSubscriptionId>,
) -> ApiResult<NoContent> {
    app::push::unsubscribe(&state, user.id, subscription).await?;
    Ok(NoContent)
}
