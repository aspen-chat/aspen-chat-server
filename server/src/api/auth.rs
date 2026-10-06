//! Session lifecycle: signing in (with a password, a second factor, or a passkey), signing out,
//! session refresh, re-verification, and the bearer-token extractors every authenticated handler
//! uses.
//!
//! Aspen issues two opaque tokens at sign-in. The long-lived refresh token is exchanged for
//! short-lived session tokens through `POST /auth/token-refresh`; the session token is sent on
//! every other request as `Authorization: Bearer <session token>`.

use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::{Created, Json, NoContent, Path};
use crate::api::{API_PREFIX, TAG_AUTH};
use crate::app;
use crate::app::UserId;
use crate::app::context::GlobalServerContext;
use crate::app::login::{LoginOutcome, SecondFactorOutcome, TokenRefreshOutcome};
use crate::app::passkey::{CeremonyResult, Completion, Purpose};
use crate::app::two_factor::{Caller, PasskeySummary, Proof, SecondFactor};
use crate::app::user::UserPg;
use axum::extract::{FromRequestParts, OptionalFromRequestParts, State};
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use hyper::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use tracing::error;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// The credentials of a completed sign-in.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub user_id: UserId,
    /// Long-lived token. Store it securely and exchange it for session tokens.
    pub refresh_token: String,
    /// Short-lived token sent as `Authorization: Bearer <sessionToken>`.
    pub session_token: String,
    pub session_token_expires: DateTime<Utc>,
    /// The server requires a second factor this account has not added. Until it adds one
    /// (`POST /users/@me/totp` or a passkey `register` ceremony), every other endpoint answers
    /// `twoFactorEnrollmentRequired`.
    pub two_factor_enrollment_required: bool,
}

impl From<app::login::Session> for LoginResponse {
    fn from(session: app::login::Session) -> Self {
        Self {
            user_id: session.user_id,
            refresh_token: session.refresh_token,
            session_token: session.session_token,
            session_token_expires: session.session_token_expires,
            two_factor_enrollment_required: session.enrollment_required,
        }
    }
}

/// A second factor an account can finish signing in with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum SecondFactorMethod {
    /// A code from an authenticator app.
    Totp,
    /// A passkey, through a `signIn` ceremony that carries the ticket.
    Passkey,
    /// One of the account's single-use recovery codes.
    RecoveryCode,
}

/// The password was right; the account has two-factor sign-in on.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecondFactorChallenge {
    /// Stands for this half-finished sign-in for five minutes. Present it to `POST
    /// /auth/login/second-factor`, or in a passkey `signIn` ceremony.
    pub ticket: String,
    /// What this account can finish with.
    pub methods: Vec<SecondFactorMethod>,
}

#[derive(Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LoginResult {
    SignedIn(LoginResponse),
    SecondFactorRequired(SecondFactorChallenge),
}

/// Signs in with a username and password. An account with two-factor sign-in on gets a
/// `secondFactorRequired` challenge instead of credentials.
#[utoipa::path(
    post,
    path = "/auth/login",
    tag = TAG_AUTH,
    responses(
        (status = OK, body = LoginResult),
        (status = UNAUTHORIZED, description = "`invalidCredentials`", body = Problem),
        (status = SERVICE_UNAVAILABLE, description = "`serverBusy`: too many password checks queued; `Retry-After` says when to try again", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn login(
    State(state): State<GlobalServerContext>,
    Json(request): Json<LoginRequest>,
) -> ApiResult<Json<LoginResult>> {
    crate::api::rate_limit::limit_sign_in(&state, &request.username).await?;
    match app::login::try_login(&state, &request.username, &request.password).await? {
        LoginOutcome::SignedIn(session) => Ok(Json(LoginResult::SignedIn(session.into()))),
        LoginOutcome::SecondFactorRequired { ticket, methods } => {
            let mut offered = Vec::new();
            if methods.totp {
                offered.push(SecondFactorMethod::Totp);
            }
            if methods.passkey && state.config.auth.rp_id.is_some() {
                offered.push(SecondFactorMethod::Passkey);
            }
            if methods.recovery_code {
                offered.push(SecondFactorMethod::RecoveryCode);
            }
            Ok(Json(LoginResult::SecondFactorRequired(
                SecondFactorChallenge {
                    ticket,
                    methods: offered,
                },
            )))
        }
        LoginOutcome::InvalidCredentials => Err(ApiError::new(ProblemCode::InvalidCredentials)),
    }
}

/// A second factor typed in: an authenticator code or a recovery code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum TypedSecondFactor {
    Totp,
    RecoveryCode,
}

impl TypedSecondFactor {
    fn with_code(self, code: String) -> SecondFactor {
        match self {
            TypedSecondFactor::Totp => SecondFactor::Totp(code),
            TypedSecondFactor::RecoveryCode => SecondFactor::RecoveryCode(code),
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecondFactorRequest {
    /// From the `secondFactorRequired` challenge.
    pub ticket: String,
    pub method: TypedSecondFactor,
    pub code: String,
}

/// Finishes a sign-in with an authenticator code or a recovery code. A wrong code leaves the
/// ticket usable until it expires; ten wrong codes within fifteen minutes lock the account's
/// second factors for the rest of that time.
#[utoipa::path(
    post,
    path = "/auth/login/second-factor",
    tag = TAG_AUTH,
    responses(
        (status = OK, body = LoginResponse),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, description = "`invalidToken`: the ticket is unknown, expired, or used", body = Problem),
        (status = FORBIDDEN, description = "`verificationFailed`", body = Problem),
        (status = TOO_MANY_REQUESTS, description = "`tooManyAttempts`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn login_second_factor(
    State(state): State<GlobalServerContext>,
    Json(request): Json<SecondFactorRequest>,
) -> ApiResult<Json<LoginResponse>> {
    let factor = request.method.with_code(request.code);
    match app::login::complete_second_factor(&state, &request.ticket, &factor).await? {
        SecondFactorOutcome::SignedIn(session) => Ok(Json(session.into())),
        SecondFactorOutcome::InvalidTicket => Err(ApiError::new(ProblemCode::InvalidToken)),
        SecondFactorOutcome::Rejected => Err(ApiError::new(ProblemCode::VerificationFailed)),
    }
}

/// What a client needs to know before signing in.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AuthMethods {
    /// Present when the server offers passkeys.
    pub passkeys: Option<PasskeySupport>,
    /// Every account must have a second factor.
    pub two_factor_required: bool,
    /// Creating an account takes an invite from the server's administrators
    /// (`UserCreateRequest.inviteCode`).
    pub registration_invite_required: bool,
    /// This deployment's name among deployments, with `:port` when not 443; `null` when it takes
    /// no part in federation. Its users sign in at others from here (`POST /auth/assertions`).
    pub federation_domain: Option<String>,
    /// The Aspen protocol this deployment speaks, which a client of another deployment checks
    /// before using it.
    pub protocol: crate::app::federation::protocol::Protocol,
    /// The software it runs, for people to read.
    pub software: crate::app::federation::protocol::Software,
    /// Present when the deployment wakes phones (`POST /users/@me/push-subscriptions`).
    pub push: Option<crate::api::push::PushSupport>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeySupport {
    /// The domain passkeys are bound to. A web client whose host is this domain or under it
    /// runs ceremonies in its own page; anything else hands them to the page at
    /// `/auth/passkey` on this server, opened in the system browser.
    pub rp_id: String,
}

/// How this server lets people sign in and register. Unauthenticated.
#[utoipa::path(
    get,
    path = "/auth/methods",
    tag = TAG_AUTH,
    responses((status = OK, body = AuthMethods))
)]
pub async fn auth_methods(State(state): State<GlobalServerContext>) -> Json<AuthMethods> {
    let auth = &state.config.auth;
    let settings = state.settings();
    Json(AuthMethods {
        passkeys: auth.rp_id.clone().map(|rp_id| PasskeySupport { rp_id }),
        two_factor_required: settings.require_two_factor,
        registration_invite_required: settings.registration_invite_required,
        federation_domain: app::federation::own_domain(&state.config.federation).map(String::from),
        push: app::push::application_server_key(&state).map(|application_server_key| {
            crate::api::push::PushSupport {
                application_server_key,
            }
        }),
        protocol: app::federation::protocol::Protocol::with_plugins(&state.plugins),
        software: app::federation::protocol::Software::ours(),
    })
}

/// How the caller proves who they are again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ReauthenticationMethod {
    /// Only for an account without two-factor sign-in.
    Password,
    Totp,
    RecoveryCode,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReauthenticateRequest {
    pub method: ReauthenticationMethod,
    /// The password or code.
    pub secret: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Verification {
    /// Until when the session may change security settings without verifying again.
    pub verified_until: DateTime<Utc>,
}

/// Proves again who the caller is, so the session may change security settings for a while.
/// An account with two-factor sign-in on presents an authenticator code or a recovery code (or
/// uses a passkey `reauthenticate` ceremony); one without presents its password.
#[utoipa::path(
    post,
    path = "/auth/reauthenticate",
    tag = TAG_AUTH,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Verification),
        (status = BAD_REQUEST, description = "`validation`: the method does not suit the account", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`verificationFailed`", body = Problem),
        (status = TOO_MANY_REQUESTS, description = "`tooManyAttempts`", body = Problem),
        (status = SERVICE_UNAVAILABLE, description = "`serverBusy`: too many password checks queued; `Retry-After` says when to try again", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn reauthenticate(
    State(state): State<GlobalServerContext>,
    EnrollingSessionUser(session): EnrollingSessionUser,
    Json(request): Json<ReauthenticateRequest>,
) -> ApiResult<Json<Verification>> {
    let proof = match request.method {
        ReauthenticationMethod::Password => Proof::Password(request.secret),
        ReauthenticationMethod::Totp => Proof::SecondFactor(SecondFactor::Totp(request.secret)),
        ReauthenticationMethod::RecoveryCode => {
            Proof::SecondFactor(SecondFactor::RecoveryCode(request.secret))
        }
    };
    let verified_until = app::two_factor::reauthenticate(&state, &session.caller, proof).await?;
    Ok(Json(Verification { verified_until }))
}

/// A passkey on the caller's account.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Passkey {
    pub id: app::PasskeyId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl From<PasskeySummary> for Passkey {
    fn from(summary: PasskeySummary) -> Self {
        Self {
            id: summary.id,
            name: summary.name,
            created_at: summary.created_at,
            last_used_at: summary.last_used_at,
        }
    }
}

/// What a passkey ceremony is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PasskeyPurpose {
    /// Sign in with a passkey. Unauthenticated.
    SignIn,
    /// Add a passkey to the caller's account. Needs a recently verified session.
    Register,
    /// Prove again who the caller is.
    Reauthenticate,
}

impl From<PasskeyPurpose> for Purpose {
    fn from(purpose: PasskeyPurpose) -> Self {
        match purpose {
            PasskeyPurpose::SignIn => Purpose::SignIn,
            PasskeyPurpose::Register => Purpose::Register,
            PasskeyPurpose::Reauthenticate => Purpose::Reauthenticate,
        }
    }
}

impl From<Purpose> for PasskeyPurpose {
    fn from(purpose: Purpose) -> Self {
        match purpose {
            Purpose::SignIn => PasskeyPurpose::SignIn,
            Purpose::Register => PasskeyPurpose::Register,
            Purpose::Reauthenticate => PasskeyPurpose::Reauthenticate,
        }
    }
}

/// Hands the ceremony to the page at `/auth/passkey` in the system browser.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyHandoff {
    /// `BASE64URL(SHA256(codeVerifier))` (RFC 7636 `S256`), where `codeVerifier` is a secret
    /// the app keeps and later presents to `claim`.
    pub code_challenge: String,
    /// Where the page sends the browser when done, with `ceremony` and `outcome` (`done` or
    /// `cancelled`) added to the query, and, when done, the `code` the claim presents: a
    /// loopback `http` address with a port, or an `aspen:` URI.
    pub return_to: String,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyCeremonyRequest {
    pub purpose: PasskeyPurpose,
    /// `signIn` only: the ticket of a password sign-in waiting for its second factor.
    #[serde(default)]
    pub ticket: Option<String>,
    /// `register` only: what to call the passkey. Defaults to a generic name.
    #[serde(default)]
    pub name: Option<String>,
    /// Omit when the caller's own page runs the ceremony.
    #[serde(default)]
    pub handoff: Option<PasskeyHandoff>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyCeremony {
    /// Secret: whoever holds it can run the ceremony's authenticator. Its effect goes only to
    /// its starter: the caller of a ceremony that was not handed off, and the claimer holding
    /// the code verifier and the return code of one that was.
    pub id: String,
    pub purpose: PasskeyPurpose,
    /// `{"publicKey": …}` for `navigator.credentials.create` (`register`) or `.get`
    /// (otherwise), with every binary field in base64url.
    #[schema(value_type = HashMap<String, serde_json::Value>)]
    pub options: serde_json::Value,
    pub expires_at: DateTime<Utc>,
}

/// Starts a passkey ceremony. `signIn` needs no session; `register` and `reauthenticate` do,
/// and `register` needs a recently verified one.
#[utoipa::path(
    post,
    path = "/auth/passkey-ceremonies",
    tag = TAG_AUTH,
    security((), ("bearerAuth" = [])),
    responses(
        (status = CREATED, body = PasskeyCeremony, headers(("Location" = String, description = "URL of the ceremony"))),
        (status = BAD_REQUEST, description = "`badRequest` or `validation`", body = Problem),
        (status = UNAUTHORIZED, description = "`unauthorized` (no session for `register` or `reauthenticate`) or `invalidToken` (the ticket)", body = Problem),
        (status = FORBIDDEN, description = "`reauthenticationRequired`", body = Problem),
        (status = NOT_FOUND, description = "`passkeysUnavailable`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn start_passkey_ceremony(
    State(state): State<GlobalServerContext>,
    session: Option<EnrollingSessionUser>,
    Json(request): Json<PasskeyCeremonyRequest>,
) -> ApiResult<Created<PasskeyCeremony>> {
    let purpose = request.purpose;
    let started = app::passkey::start(
        &state,
        session.as_ref().map(|EnrollingSessionUser(s)| &s.caller),
        app::passkey::StartRequest {
            purpose: purpose.into(),
            ticket: request.ticket,
            name: request.name,
            handoff: request.handoff.map(|handoff| app::passkey::Handoff {
                code_challenge: handoff.code_challenge,
                return_to: handoff.return_to,
            }),
        },
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/auth/passkey-ceremonies/{}", started.id),
        PasskeyCeremony {
            id: started.id,
            purpose,
            options: started.options,
            expires_at: started.expires_at,
        },
    ))
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyCeremonyDescription {
    pub purpose: PasskeyPurpose,
    #[schema(value_type = HashMap<String, serde_json::Value>)]
    pub options: serde_json::Value,
    /// Where a handed-off ceremony returns to.
    pub return_to: Option<String>,
}

/// A handed-off ceremony still waiting for its authenticator, for the page at `/auth/passkey`.
/// The ceremony id is the only credential. A ceremony that was not handed off reads as unknown.
#[utoipa::path(
    get,
    path = "/auth/passkey-ceremonies/{ceremony}",
    tag = TAG_AUTH,
    params(("ceremony" = String, Path, description = "The ceremony id")),
    responses(
        (status = OK, body = PasskeyCeremonyDescription),
        (status = NOT_FOUND, description = "Unknown, expired, or completed", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_passkey_ceremony(
    State(state): State<GlobalServerContext>,
    Path(ceremony): Path<String>,
) -> ApiResult<Json<PasskeyCeremonyDescription>> {
    let description = app::passkey::describe(&state, &ceremony).await?;
    Ok(Json(PasskeyCeremonyDescription {
        purpose: description.purpose.into(),
        options: description.options,
        return_to: description.return_to,
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyCredentialRequest {
    /// The `PublicKeyCredential` the browser returned, as JSON with binary fields in base64url
    /// (the shape `PublicKeyCredential.toJSON()` produces).
    #[schema(value_type = HashMap<String, serde_json::Value>)]
    pub credential: serde_json::Value,
}

/// The result of a finished passkey ceremony.
#[derive(Serialize, ToSchema)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum PasskeyCeremonyOutcome {
    /// A handed-off ceremony: send the browser to `returnTo`; the app claims the result.
    #[serde(rename_all = "camelCase")]
    HandedOff {
        return_to: String,
    },
    SignedIn(LoginResponse),
    #[serde(rename_all = "camelCase")]
    PasskeyAdded {
        passkey: Passkey,
        /// Present when this passkey turned two-factor sign-in on. Shown once.
        #[serde(skip_serializing_if = "Option::is_none")]
        recovery_codes: Option<Vec<String>>,
    },
    #[serde(rename_all = "camelCase")]
    Reauthenticated {
        verified_until: DateTime<Utc>,
    },
}

impl From<CeremonyResult> for PasskeyCeremonyOutcome {
    fn from(result: CeremonyResult) -> Self {
        match result {
            CeremonyResult::SignedIn(session) => PasskeyCeremonyOutcome::SignedIn(session.into()),
            CeremonyResult::PasskeyAdded {
                passkey,
                recovery_codes,
            } => PasskeyCeremonyOutcome::PasskeyAdded {
                passkey: passkey.into(),
                recovery_codes,
            },
            CeremonyResult::Reauthenticated { verified_until } => {
                PasskeyCeremonyOutcome::Reauthenticated { verified_until }
            }
        }
    }
}

/// Completes a ceremony with the authenticator's response. A ceremony completes at most once,
/// whether or not the response verifies. A `register` or `reauthenticate` ceremony that was not
/// handed off is completed by the session that started it; a handed-off one takes effect when
/// it is claimed.
#[utoipa::path(
    post,
    path = "/auth/passkey-ceremonies/{ceremony}/credential",
    tag = TAG_AUTH,
    security((), ("bearerAuth" = [])),
    params(("ceremony" = String, Path, description = "The ceremony id")),
    responses(
        (status = OK, body = PasskeyCeremonyOutcome),
        (status = BAD_REQUEST, description = "`badRequest` or `passkeyRejected`", body = Problem),
        (status = UNAUTHORIZED, description = "`invalidToken`: the sign-in ticket expired meanwhile", body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: another sign-in started this ceremony", body = Problem),
        (status = NOT_FOUND, description = "Unknown, expired, or completed", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn complete_passkey_ceremony(
    State(state): State<GlobalServerContext>,
    session: Option<EnrollingSessionUser>,
    Path(ceremony): Path<String>,
    Json(request): Json<PasskeyCredentialRequest>,
) -> ApiResult<Json<PasskeyCeremonyOutcome>> {
    let caller = session.as_ref().map(|EnrollingSessionUser(s)| &s.caller);
    Ok(Json(
        match app::passkey::complete(&state, &ceremony, caller, request.credential).await? {
            Completion::Done(result) => result.into(),
            Completion::HandedOff { return_to } => PasskeyCeremonyOutcome::HandedOff { return_to },
        },
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PasskeyClaimRequest {
    /// The secret whose digest the ceremony's `handoff.codeChallenge` was.
    pub code_verifier: String,
    /// The `code` the handoff page added to the return address. Required; a claim without it
    /// is refused with `validation`.
    #[serde(default)]
    pub code: Option<String>,
}

/// Takes the result of a handed-off ceremony once the system browser has returned, and gives
/// it effect. A result can be claimed once, within two minutes of completion; a `register` or
/// `reauthenticate` ceremony only by the session that started it.
#[utoipa::path(
    post,
    path = "/auth/passkey-ceremonies/{ceremony}/claim",
    tag = TAG_AUTH,
    security((), ("bearerAuth" = [])),
    params(("ceremony" = String, Path, description = "The ceremony id")),
    responses(
        (status = OK, body = PasskeyCeremonyOutcome),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (no return code)", body = Problem),
        (status = UNAUTHORIZED, description = "`invalidToken`: the sign-in ticket expired meanwhile", body = Problem),
        (status = FORBIDDEN, description = "`verificationFailed`: wrong code verifier or return code; `forbidden`: another sign-in started this ceremony", body = Problem),
        (status = NOT_FOUND, description = "Unknown, expired, not completed, or claimed", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn claim_passkey_ceremony(
    State(state): State<GlobalServerContext>,
    session: Option<EnrollingSessionUser>,
    Path(ceremony): Path<String>,
    Json(request): Json<PasskeyClaimRequest>,
) -> ApiResult<Json<PasskeyCeremonyOutcome>> {
    let result = app::passkey::claim(
        &state,
        &ceremony,
        session.as_ref().map(|EnrollingSessionUser(s)| &s.caller),
        &request.code_verifier,
        request.code.as_deref(),
    )
    .await?;
    Ok(Json(result.into()))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogoutRequest {
    pub refresh_token: String,
}

/// Revokes a refresh token and every session token issued from it.
///
/// Revocation is idempotent in the sense of RFC 7009: a token that is unknown or already revoked
/// still yields `204`, because the end state the client asked for already holds.
#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = TAG_AUTH,
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn logout(
    State(state): State<GlobalServerContext>,
    _: EnrollingSessionUser,
    Json(request): Json<LogoutRequest>,
) -> ApiResult<NoContent> {
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    app::login::try_logout(&state, conn.as_mut(), &request.refresh_token).await?;
    Ok(NoContent)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenRefreshRequest {
    pub refresh_token: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenRefreshResponse {
    pub session_token: String,
    pub session_token_expires: DateTime<Utc>,
}

#[utoipa::path(
    post,
    path = "/auth/token-refresh",
    tag = TAG_AUTH,
    responses(
        (status = OK, body = TokenRefreshResponse),
        (status = UNAUTHORIZED, description = "`invalidToken`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn token_refresh(
    State(state): State<GlobalServerContext>,
    Json(request): Json<TokenRefreshRequest>,
) -> ApiResult<Json<TokenRefreshResponse>> {
    let conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    match app::login::try_token_refresh(conn, &request.refresh_token).await? {
        TokenRefreshOutcome::Ok {
            session_token,
            session_token_expires,
        } => Ok(Json(TokenRefreshResponse {
            session_token,
            session_token_expires,
        })),
        TokenRefreshOutcome::InvalidToken => Err(ApiError::new(ProblemCode::InvalidToken)),
    }
}

/// The authenticated caller. Extracting it requires a valid `Authorization: Bearer <session
/// token>` header; anything else is rejected with a `401` Problem. A session whose account
/// still owes the server a second factor is rejected with `twoFactorEnrollmentRequired`, and
/// one whose email address the server requires verified with `emailVerificationRequired`; the
/// few endpoints such a session may use take `EnrollingSessionUser` instead.
#[derive(Clone)]
pub struct SessionUser {
    pub user: UserPg,
    pub caller: Caller,
}

/// A caller whose account may still owe the server a second factor or a verified email address:
/// what signing out, re-verifying, adding a first factor, and verifying or changing the address
/// accept.
#[derive(Clone)]
pub struct EnrollingSessionUser(pub SessionUser);

impl FromRequestParts<GlobalServerContext> for EnrollingSessionUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &GlobalServerContext,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(bearer_token)
            .ok_or_else(|| ApiError::new(ProblemCode::Unauthorized))?;
        let (user, caller) = match app::user::user_for_token(state, token).await {
            Ok(Some(found)) => found,
            Ok(None) => return Err(ApiError::new(ProblemCode::Unauthorized)),
            Err(e) => {
                error!("error during authentication: {e}");
                return Err(ApiError::new(ProblemCode::Internal));
            }
        };
        crate::api::rate_limit::limit_session(state, parts, user.id).await?;
        app::user_status::mark_user_online(state, &user);
        Ok(EnrollingSessionUser(SessionUser { user, caller }))
    }
}

/// Optional authentication: no `Authorization` header is `None`; a header with a bad token is
/// still a `401`, so a client never silently loses its identity.
impl OptionalFromRequestParts<GlobalServerContext> for EnrollingSessionUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &GlobalServerContext,
    ) -> Result<Option<Self>, Self::Rejection> {
        if !parts.headers.contains_key(AUTHORIZATION) {
            return Ok(None);
        }
        <Self as FromRequestParts<GlobalServerContext>>::from_request_parts(parts, state)
            .await
            .map(Some)
    }
}

impl FromRequestParts<GlobalServerContext> for SessionUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &GlobalServerContext,
    ) -> Result<Self, Self::Rejection> {
        let EnrollingSessionUser(session) = <EnrollingSessionUser as FromRequestParts<
            GlobalServerContext,
        >>::from_request_parts(parts, state)
        .await?;
        let settings = state.settings();
        if session.caller.enrollment_required(&settings) {
            return Err(ApiError::new(ProblemCode::TwoFactorEnrollmentRequired));
        }
        if session.caller.verification_required(&settings) {
            return Err(ApiError::new(ProblemCode::EmailVerificationRequired));
        }
        Ok(session)
    }
}

/// Extracts the credential from an `Authorization` header value using the `Bearer` scheme
/// (RFC 6750). The scheme name is case-insensitive; the token is returned verbatim.
fn bearer_token(header: &str) -> Option<&str> {
    let (scheme, token) = header.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::bearer_token;

    #[test]
    fn bearer_scheme_is_case_insensitive() {
        assert_eq!(bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(bearer_token("bearer abc"), Some("abc"));
        assert_eq!(bearer_token("BEARER abc"), Some("abc"));
    }

    #[test]
    fn other_schemes_are_rejected() {
        assert_eq!(bearer_token("Token abc"), None);
        assert_eq!(bearer_token("Basic abc"), None);
        assert_eq!(bearer_token("abc"), None);
        assert_eq!(bearer_token("Bearer "), None);
    }
}
