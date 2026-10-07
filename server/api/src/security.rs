//! A user's security settings: authenticator app, passkeys, recovery codes, and their other
//! sign-ins. Only the user themself may read or change them; anyone else is answered `403`.
//! Every change needs a recently verified session (`POST /auth/reauthenticate`), except renaming
//! a passkey.

use crate::auth::{EnrollingSessionUser, Passkey, SessionUser};
use crate::error::{ApiResult, Problem};
use crate::extract::{Created, Json, NoContent, Path};
use crate::user::{UserRef, not_your_account};
use crate::{API_PREFIX, TAG_SECURITY};
use aspen_app as app;
use aspen_app::PasskeyId;
use aspen_app::context::GlobalServerContext;
use axum::extract::State;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The caller must be the addressed user.
fn own(session: &SessionUser, user: UserRef) -> ApiResult<()> {
    if user.resolve(session) == session.user.id {
        Ok(())
    } else {
        Err(not_your_account(app::Error::Unauthorized))
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecuritySettings {
    /// A password alone no longer signs in: the account has a second factor.
    pub two_factor_enabled: bool,
    /// The server requires every account to have a second factor.
    pub two_factor_required: bool,
    /// An authenticator app is set up.
    pub totp: bool,
    pub passkeys: Vec<Passkey>,
    /// Unused recovery codes left.
    pub recovery_codes_remaining: i64,
    /// Until when this session may change security settings without verifying again.
    pub verified_until: DateTime<Utc>,
}

#[utoipa::path(
    get,
    path = "/users/{user}/security",
    tag = TAG_SECURITY,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = SecuritySettings),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_security(
    State(state): State<GlobalServerContext>,
    EnrollingSessionUser(session): EnrollingSessionUser,
    Path(user): Path<UserRef>,
) -> ApiResult<Json<SecuritySettings>> {
    own(&session, user)?;
    let overview = app::two_factor::overview(&state, &session.caller).await?;
    Ok(Json(SecuritySettings {
        two_factor_enabled: overview.totp || !overview.passkeys.is_empty(),
        two_factor_required: overview.required,
        totp: overview.totp,
        passkeys: overview.passkeys.into_iter().map(Passkey::from).collect(),
        recovery_codes_remaining: overview.recovery_codes_remaining,
        verified_until: overview.verified_until,
    }))
}

/// A new authenticator app secret. It becomes a factor once confirmed.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TotpEnrollment {
    /// The secret in base32, for typing into an app by hand.
    pub secret: String,
    /// The `otpauth://` URI, for showing as a QR code.
    pub uri: String,
}

/// Starts setting up an authenticator app, replacing any setup that was never confirmed.
#[utoipa::path(
    post,
    path = "/users/{user}/totp",
    tag = TAG_SECURITY,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = TotpEnrollment, headers(("Location" = String, description = "URL of the authenticator app"))),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` or `reauthenticationRequired`", body = Problem),
        (status = CONFLICT, description = "An authenticator app is already set up", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn begin_totp(
    State(state): State<GlobalServerContext>,
    EnrollingSessionUser(session): EnrollingSessionUser,
    Path(user): Path<UserRef>,
) -> ApiResult<Created<TotpEnrollment>> {
    own(&session, user)?;
    let enrollment = app::two_factor::begin_totp(&state, &session.caller).await?;
    Ok(Created::new(
        format!("{API_PREFIX}/users/{}/totp", session.user.id.0),
        TotpEnrollment {
            secret: enrollment.secret,
            uri: enrollment.uri,
        },
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TotpConfirmation {
    /// A current code from the app.
    pub code: String,
}

/// The effect of adding a second factor.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FactorAdded {
    /// Present when this factor turned two-factor sign-in on; every other session was signed
    /// out at the same time. Shown once.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery_codes: Option<Vec<String>>,
}

/// Confirms the pending authenticator app with a code from it, making it a second factor.
#[utoipa::path(
    post,
    path = "/users/{user}/totp/confirmation",
    tag = TAG_SECURITY,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = FactorAdded),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`, `reauthenticationRequired`, or `verificationFailed`", body = Problem),
        (status = NOT_FOUND, description = "No authenticator app is waiting to be confirmed", body = Problem),
        (status = TOO_MANY_REQUESTS, description = "`tooManyAttempts`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn confirm_totp(
    State(state): State<GlobalServerContext>,
    EnrollingSessionUser(session): EnrollingSessionUser,
    Path(user): Path<UserRef>,
    Json(request): Json<TotpConfirmation>,
) -> ApiResult<Json<FactorAdded>> {
    own(&session, user)?;
    let recovery_codes =
        app::two_factor::confirm_totp(&state, &session.caller, &request.code).await?;
    Ok(Json(FactorAdded { recovery_codes }))
}

/// Removes the authenticator app. Removing the last second factor turns two-factor sign-in off
/// and discards the recovery codes.
#[utoipa::path(
    delete,
    path = "/users/{user}/totp",
    tag = TAG_SECURITY,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` or `reauthenticationRequired`", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`lastSecondFactor`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_totp(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path(user): Path<UserRef>,
) -> ApiResult<NoContent> {
    own(&session, user)?;
    app::two_factor::remove_totp(&state, &session.caller).await?;
    Ok(NoContent)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyUpdateRequest {
    pub name: Option<String>,
}

/// Renames a passkey. An empty name resets it to the generic one.
#[utoipa::path(
    patch,
    path = "/users/{user}/passkeys/{passkey}",
    tag = TAG_SECURITY,
    params(("user" = inline(UserRef), Path), ("passkey" = PasskeyId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Passkey),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn rename_passkey(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path((user, passkey)): Path<(UserRef, PasskeyId)>,
    Json(request): Json<PasskeyUpdateRequest>,
) -> ApiResult<Json<Passkey>> {
    own(&session, user)?;
    let renamed = app::passkey::rename(&state, &session.caller, passkey, request.name).await?;
    Ok(Json(renamed.into()))
}

/// Removes a passkey. Removing the last second factor turns two-factor sign-in off and discards
/// the recovery codes.
#[utoipa::path(
    delete,
    path = "/users/{user}/passkeys/{passkey}",
    tag = TAG_SECURITY,
    params(("user" = inline(UserRef), Path), ("passkey" = PasskeyId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` or `reauthenticationRequired`", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`lastSecondFactor`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_passkey(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path((user, passkey)): Path<(UserRef, PasskeyId)>,
) -> ApiResult<NoContent> {
    own(&session, user)?;
    app::passkey::remove(&state, &session.caller, passkey).await?;
    Ok(NoContent)
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecoveryCodes {
    /// Shown once; the server keeps only their digests.
    pub codes: Vec<String>,
}

/// Replaces the recovery codes with a new set. The old ones stop working.
#[utoipa::path(
    post,
    path = "/users/{user}/recovery-codes",
    tag = TAG_SECURITY,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = RecoveryCodes, headers(("Location" = String, description = "URL of the recovery codes"))),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` or `reauthenticationRequired`", body = Problem),
        (status = CONFLICT, description = "Two-factor sign-in is off", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn regenerate_recovery_codes(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path(user): Path<UserRef>,
) -> ApiResult<Created<RecoveryCodes>> {
    own(&session, user)?;
    let codes = app::two_factor::regenerate_recovery_codes(&state, &session.caller).await?;
    Ok(Created::new(
        format!("{API_PREFIX}/users/{}/recovery-codes", session.user.id.0),
        RecoveryCodes { codes },
    ))
}

/// Signs out everywhere else: ends every sign-in of the user but the caller's own, closing their
/// event streams and no longer waking their phones, and ends every plugin capability URL of
/// theirs. Needs a recently verified session.
#[utoipa::path(
    delete,
    path = "/users/{user}/sign-ins",
    tag = TAG_SECURITY,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` or `reauthenticationRequired`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn end_other_sign_ins(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path(user): Path<UserRef>,
) -> ApiResult<NoContent> {
    own(&session, user)?;
    app::login::end_other_sign_ins(&state, &session.caller).await?;
    Ok(NoContent)
}
