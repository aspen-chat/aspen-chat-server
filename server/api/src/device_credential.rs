//! Earning a device credential (`app::device_credential`): the puzzle's challenge, and the
//! credential its solution earns. A request without a session presents the credential as
//! `Aspen-Device` (`DEVICE_HEADER`), and is answered with a fresh one as
//! `Aspen-Device-Credential` (`RENEWAL_HEADER`) when its own is past half its life.

use crate::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::extract::Json;
use crate::{TAG_AUTH, t};
use aspen_app::context::GlobalServerContext;
use aspen_app::device_credential::{self, CHALLENGE_LIFETIME, Refusal};
use axum::extract::State;
use axum::http::HeaderName;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The request header a device credential is presented in.
pub const DEVICE_HEADER: HeaderName = HeaderName::from_static("aspen-device");
/// The response header a renewed device credential is given in.
pub const RENEWAL_HEADER: HeaderName = HeaderName::from_static("aspen-device-credential");

/// A puzzle whose solution earns a device credential: a nonce for which
/// `SHA-256(SHA-256(challenge) || nonce)`, with `challenge` base64url-decoded and the nonce as
/// eight big-endian bytes, begins with `difficulty` zero bits.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceChallenge {
    /// Base64url without padding; decode it to hash it, and send it back as it is.
    pub challenge: String,
    pub difficulty: u8,
    /// Its solution is refused after this.
    pub expires_at: DateTime<Utc>,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCredentialRequest {
    pub challenge: String,
    /// Below 2^53, so that it survives being a JSON number in every language.
    pub nonce: u64,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCredential {
    /// Sent as the `Aspen-Device` header on every request made without a session.
    pub credential: String,
    /// It is refused after this; a request presenting it past half its life is answered with
    /// a fresh one in `Aspen-Device-Credential`.
    pub expires_at: DateTime<Utc>,
}

/// Starts earning a device credential, which a client that has not signed in presents on every
/// request (`Aspen-Device`). Solve the challenge and send the solution to
/// `POST /auth/device-credentials`.
#[utoipa::path(
    post,
    path = "/auth/device-challenges",
    tag = TAG_AUTH,
    responses(
        (status = OK, body = DeviceChallenge),
    )
)]
pub async fn challenge(State(state): State<GlobalServerContext>) -> Json<DeviceChallenge> {
    let made = device_credential::challenge(
        &state.device_key,
        state.config.rate_limits.device_puzzle_bits,
        Utc::now(),
    );
    Json(DeviceChallenge {
        challenge: made.token,
        difficulty: made.difficulty,
        expires_at: made.expires_at,
    })
}

/// Earns a device credential with a solved challenge. Each challenge earns one.
#[utoipa::path(
    post,
    path = "/auth/device-credentials",
    tag = TAG_AUTH,
    request_body = DeviceCredentialRequest,
    responses(
        (status = OK, body = DeviceCredential),
        (status = BAD_REQUEST, description = "`deviceChallengeInvalid`", body = Problem),
        (status = SERVICE_UNAVAILABLE, description = "`serverBusy`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn redeem(
    State(state): State<GlobalServerContext>,
    Json(request): Json<DeviceCredentialRequest>,
) -> ApiResult<Json<DeviceCredential>> {
    let earned = device_credential::redeem(&state, &request.challenge, request.nonce)
        .await
        .map_err(|e| match e {
            aspen_app::Error::Valkey(_) => ApiError::new(ProblemCode::ServerBusy)
                .with_retry_after(std::time::Duration::from_secs(10)),
            other => other.into(),
        })?;
    match earned {
        Ok(credential) => Ok(Json(DeviceCredential {
            credential: credential.token,
            expires_at: credential.expires_at,
        })),
        Err(refusal) => Err(
            ApiError::new(ProblemCode::DeviceChallengeInvalid).with_detail(match refusal {
                Refusal::Malformed => t!("deviceChallengeMalformed"),
                Refusal::Expired => t!(
                    "deviceChallengeExpired",
                    minutes = CHALLENGE_LIFETIME.num_minutes()
                ),
                Refusal::Unsolved { bits } => t!("deviceChallengeUnsolved", bits = bits),
                Refusal::Used => t!("deviceChallengeUsed"),
            }),
        ),
    }
}
