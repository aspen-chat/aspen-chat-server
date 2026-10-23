//! Signing in through OpenID Connect providers (`app::identity_provider`): the flows clients
//! start and claim, unlinking a provider from an account, the providers' administration, and the
//! two endpoints providers themselves call, outside `/api/v1` and the OpenAPI document: the
//! callback the browser returns to, and back-channel logout.

use crate::admin::AdminUser;
use crate::auth::{EnrollingSessionUser, LoginResponse, LoginResult, SessionUser, login_result};
use crate::error::{ApiError, ApiResult, Problem};
use crate::extract::{Created, Json, NoContent, Path, Query};
use crate::rate_limit::Failures;
use crate::user::{UserRef, not_your_account};
use crate::{API_PREFIX, TAG_ADMIN, TAG_AUTH, TAG_SECURITY, t};
use aspen_app as app;
use aspen_app::context::GlobalServerContext;
use aspen_app::identity_provider::flow::{self, Claim, Purpose};
use aspen_app::identity_provider::identity::IdentitySummary;
use aspen_app::identity_provider::{self as providers, ProviderSettings};
use aspen_app::{IdentityProviderId, UserIdentityId};
use axum::Extension;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, LOCATION, REFERRER_POLICY,
    X_CONTENT_TYPE_OPTIONS,
};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A provider a sign-in screen offers, as `GET /auth/methods` lists it.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OfferedIdentityProvider {
    pub id: IdentityProviderId,
    /// What its button says: "Continue with {name}".
    pub name: String,
}

impl From<providers::Offered> for OfferedIdentityProvider {
    fn from(offered: providers::Offered) -> Self {
        Self {
            id: offered.id,
            name: offered.name,
        }
    }
}

/// What a flow is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum IdentityProviderPurpose {
    /// Sign in, or make an account; needs no session.
    SignIn,
    /// Link the provider to the caller's account; needs a recently verified session.
    Link,
    /// Prove again who the caller is, by signing in at the provider afresh.
    Reauthenticate,
}

impl From<IdentityProviderPurpose> for Purpose {
    fn from(purpose: IdentityProviderPurpose) -> Self {
        match purpose {
            IdentityProviderPurpose::SignIn => Purpose::SignIn,
            IdentityProviderPurpose::Link => Purpose::Link,
            IdentityProviderPurpose::Reauthenticate => Purpose::Reauthenticate,
        }
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IdentityProviderFlowRequest {
    pub provider: IdentityProviderId,
    pub purpose: IdentityProviderPurpose,
    /// `BASE64URL(SHA256(codeVerifier))` (RFC 7636 `S256`), where `codeVerifier` is a secret
    /// the client keeps and presents to `claim`.
    pub code_challenge: String,
    /// For the desktop and mobile apps: where the browser returns, with `flow` and `code` added
    /// to the query; a loopback `http` address with a port, or an `aspen:` URI. Left out, it
    /// returns to the web client's `/sign-in/provider` page with them in the fragment.
    #[serde(default)]
    pub return_to: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IdentityProviderFlow {
    /// Secret enough: it is also the OAuth `state`, so it passes through the provider, but only
    /// the code verifier and the return code claim the flow.
    pub id: String,
    /// Where to send the browser: the provider's sign-in.
    pub authorize_url: String,
    pub expires_at: DateTime<Utc>,
}

/// Starts signing in at a provider, linking one, or verifying again through one. `signIn` needs
/// no session; `link` needs a recently verified one, and `reauthenticate` one of an account
/// linked to the provider.
#[utoipa::path(
    post,
    path = "/auth/identity-provider-flows",
    tag = TAG_AUTH,
    security((), ("bearerAuth" = [])),
    responses(
        (status = CREATED, body = IdentityProviderFlow, headers(("Location" = String, description = "URL of the flow"))),
        (status = BAD_REQUEST, description = "`badRequest` or `validation`", body = Problem),
        (status = UNAUTHORIZED, description = "`unauthorized`: no session for `link` or `reauthenticate`", body = Problem),
        (status = FORBIDDEN, description = "`reauthenticationRequired`, or `forbidden` for a bot or a user of another deployment", body = Problem),
        (status = NOT_FOUND, description = "`identityProviderUnavailable`", body = Problem),
        (status = CONFLICT, description = "`conflict`: the account is linked to that provider already", body = Problem),
        (status = BAD_GATEWAY, description = "`identityProviderUnreachable`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn start_flow(
    State(state): State<GlobalServerContext>,
    session: Option<EnrollingSessionUser>,
    Json(request): Json<IdentityProviderFlowRequest>,
) -> ApiResult<Created<IdentityProviderFlow>> {
    let started = flow::start(
        &state,
        session.as_ref().map(|EnrollingSessionUser(s)| &s.caller),
        flow::StartRequest {
            provider: request.provider,
            purpose: request.purpose.into(),
            code_challenge: request.code_challenge,
            return_to: request.return_to,
        },
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/auth/identity-provider-flows/{}", started.id),
        IdentityProviderFlow {
            id: started.id,
            authorize_url: started.authorize_url,
            expires_at: started.expires_at,
        },
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IdentityProviderClaimRequest {
    /// The secret whose digest the flow's `codeChallenge` was.
    pub code_verifier: String,
    /// The `code` the browser brought back.
    pub code: String,
}

/// An identity at a provider, linked to the caller's account.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkedIdentity {
    pub id: UserIdentityId,
    pub provider: IdentityProviderId,
    pub provider_name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

impl From<IdentitySummary> for LinkedIdentity {
    fn from(identity: IdentitySummary) -> Self {
        Self {
            id: identity.id,
            provider: identity.provider,
            provider_name: identity.provider_name,
            created_at: identity.created_at,
            last_used_at: identity.last_used_at,
        }
    }
}

/// The account a `signIn` flow offers to make, when the provider's identity is linked to none.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AccountOffer {
    pub provider_name: String,
    /// The provider's username for them, when an account could take it.
    pub suggested_name: Option<String>,
    /// What the account's display name will be.
    pub display_name: Option<String>,
    /// A registration invite must be given (`inviteCode`).
    pub invite_required: bool,
    /// The verified address the account will have, from the provider.
    pub email: Option<String>,
    /// An address must be given (`email`), the provider having given none the deployment takes.
    pub email_required: bool,
}

/// What a claimed flow did.
#[derive(Serialize, ToSchema)]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum IdentityProviderClaim {
    /// A `signIn` flow: signed in, or a second factor of the account's own is still owed.
    SignedIn(LoginResult),
    /// A `signIn` flow whose identity is linked to no account: choose a username and make one
    /// with `POST …/account` within five minutes.
    AccountRequired(AccountOffer),
    #[serde(rename_all = "camelCase")]
    Linked { identity: LinkedIdentity },
    #[serde(rename_all = "camelCase")]
    Reauthenticated { verified_until: DateTime<Utc> },
}

/// Claims what the provider said, once the browser is back, and gives it effect. A `link` or
/// `reauthenticate` flow is claimed only by the session that started it.
#[utoipa::path(
    post,
    path = "/auth/identity-provider-flows/{flow}/claim",
    tag = TAG_AUTH,
    security((), ("bearerAuth" = [])),
    params(("flow" = String, Path, description = "The flow's id")),
    responses(
        (status = OK, body = IdentityProviderClaim),
        (status = BAD_REQUEST, description = "`badRequest` or `validation`", body = Problem),
        (status = UNAUTHORIZED, description = "`identityProviderFailed`: the provider did not sign them in, or its answer did not verify; `detail` says why", body = Problem),
        (status = FORBIDDEN, description = "`verificationFailed`: wrong code verifier or return code; `forbidden`: another sign-in started this flow; `deploymentBanned`", body = Problem),
        (status = NOT_FOUND, description = "`identityProviderFlowExpired` or `identityProviderUnavailable`", body = Problem),
        (status = CONFLICT, description = "`identityTaken`: another account holds that identity", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn claim_flow(
    State(state): State<GlobalServerContext>,
    session: Option<EnrollingSessionUser>,
    failures: Option<Extension<Failures>>,
    Path(id): Path<String>,
    Json(request): Json<IdentityProviderClaimRequest>,
) -> ApiResult<Json<IdentityProviderClaim>> {
    let claimed = flow::claim(
        &state,
        &id,
        session.as_ref().map(|EnrollingSessionUser(s)| &s.caller),
        &request.code_verifier,
        &request.code,
    )
    .await;
    let claimed = match claimed {
        Err(app::Error::VerificationFailed) => {
            crate::rate_limit::note_failure(&state, &failures).await;
            return Err(app::Error::VerificationFailed.into());
        }
        other => other?,
    };
    Ok(Json(match claimed {
        Claim::SignedIn(outcome) => IdentityProviderClaim::SignedIn(login_result(&state, outcome)?),
        Claim::AccountRequired(offer) => IdentityProviderClaim::AccountRequired(AccountOffer {
            provider_name: offer.provider_name,
            suggested_name: offer.suggested_name,
            display_name: offer.display_name,
            invite_required: offer.invite_required,
            email: offer.email,
            email_required: offer.email_required,
        }),
        Claim::Linked(identity) => IdentityProviderClaim::Linked {
            identity: identity.into(),
        },
        Claim::Reauthenticated { verified_until } => {
            IdentityProviderClaim::Reauthenticated { verified_until }
        }
    }))
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IdentityProviderAccountRequest {
    /// The same secret the claim presented.
    pub code_verifier: String,
    pub name: String,
    #[serde(default)]
    pub invite_code: Option<String>,
    /// An address, when the offer's `emailRequired` asks for one.
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub newsletter: Option<bool>,
}

/// Makes the account a claim offered, linked to the provider's identity, and signs it in. A
/// username someone holds is refused with `usernameTaken`, and another may be tried while the
/// offer lasts.
#[utoipa::path(
    post,
    path = "/auth/identity-provider-flows/{flow}/account",
    tag = TAG_AUTH,
    params(("flow" = String, Path, description = "The flow's id")),
    responses(
        (status = CREATED, body = LoginResponse, headers(("Location" = String, description = "URL of the new user"))),
        (status = BAD_REQUEST, description = "`badRequest` or `validation`", body = Problem),
        (status = FORBIDDEN, description = "`verificationFailed`, `registrationInviteRequired`, or `registrationInviteInvalid`", body = Problem),
        (status = NOT_FOUND, description = "`identityProviderFlowExpired` or `identityProviderUnavailable`", body = Problem),
        (status = CONFLICT, description = "`usernameTaken` or `identityTaken`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_account(
    State(state): State<GlobalServerContext>,
    failures: Option<Extension<Failures>>,
    Path(id): Path<String>,
    Json(request): Json<IdentityProviderAccountRequest>,
) -> ApiResult<Created<LoginResponse>> {
    let made = flow::create_account(
        &state,
        &id,
        &request.code_verifier,
        flow::AccountRequest {
            name: request.name,
            invite_code: request.invite_code,
            email: request.email,
            newsletter: request.newsletter.unwrap_or(false),
        },
    )
    .await;
    let session = match made {
        Ok(session) => session,
        Err(error @ (app::Error::VerificationFailed | app::Error::RegistrationInviteInvalid)) => {
            crate::rate_limit::note_failure(&state, &failures).await;
            return Err(error.into());
        }
        Err(error) => return Err(crate::user::username_taken(error)),
    };
    Ok(Created::new(
        format!("{API_PREFIX}/users/{}", session.user_id),
        session.into(),
    ))
}

/// Unlinks a provider from the account. The sign-ins made through it stay. Refused when the
/// account would be left no way in: no password, no passkey, and no other provider.
#[utoipa::path(
    delete,
    path = "/users/{user}/identities/{identity}",
    tag = TAG_SECURITY,
    params(("user" = inline(UserRef), Path), ("identity" = UserIdentityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` or `reauthenticationRequired`", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`lastSignInMethod`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn unlink_identity(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path((user, identity)): Path<(UserRef, UserIdentityId)>,
) -> ApiResult<NoContent> {
    if user.resolve(&session) != session.user.id {
        return Err(not_your_account(app::Error::Unauthorized));
    }
    providers::identity::unlink(&state, &session.caller, identity).await?;
    Ok(NoContent)
}

/// A provider as its administrators see it. The client secret is never shown.
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IdentityProviderSettings {
    pub id: IdentityProviderId,
    pub name: String,
    pub issuer: String,
    pub client_id: String,
    pub enabled: bool,
    /// A first sign-in makes an account without a registration invite where one is required.
    pub skip_invite: bool,
    /// Every sign-in there counts as two factors, whatever it reports.
    pub enforces_two_factor: bool,
    /// An address it reports as verified becomes a new account's verified address.
    pub trust_email: bool,
    pub created_at: DateTime<Utc>,
    /// Accounts linked to it, counted up to 1001; show more than 1000 as "1000+".
    pub linked_accounts: i64,
}

impl From<ProviderSettings> for IdentityProviderSettings {
    fn from(settings: ProviderSettings) -> Self {
        Self {
            id: settings.id,
            name: settings.name,
            issuer: settings.issuer,
            client_id: settings.client_id,
            enabled: settings.enabled,
            skip_invite: settings.skip_invite,
            enforces_two_factor: settings.enforces_two_factor,
            trust_email: settings.trust_email,
            created_at: settings.created_at,
            linked_accounts: settings.linked_accounts,
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct IdentityProviderList {
    pub providers: Vec<IdentityProviderSettings>,
    /// The redirect URI to register at each provider.
    pub redirect_uri: String,
    /// The back-channel logout URI to register at each provider that offers it.
    pub back_channel_logout_uri: String,
    /// The most providers the deployment may have.
    pub max_providers: i64,
}

/// The deployment's identity providers. Needs Manage identity providers.
#[utoipa::path(
    get,
    path = "/admin/identity-providers",
    tag = TAG_ADMIN,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = IdentityProviderList),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage identity providers", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_providers(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<IdentityProviderList>> {
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let listed = providers::list(&access, &mut conn).await?;
    let base = state.config.public_url.trim_end_matches('/');
    Ok(Json(IdentityProviderList {
        providers: listed.into_iter().map(Into::into).collect(),
        redirect_uri: format!("{base}{}", providers::CALLBACK_PATH),
        back_channel_logout_uri: format!("{base}{}", providers::BACK_CHANNEL_LOGOUT_PATH),
        max_providers: providers::MAX_PROVIDERS,
    }))
}

/// Only this deployment's own dashboard and terminal send it.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentityProviderCreateRequest {
    pub name: String,
    /// The issuer URL; `{issuer}/.well-known/openid-configuration` must name it.
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    #[serde(default)]
    pub skip_invite: bool,
    #[serde(default)]
    pub enforces_two_factor: bool,
    #[serde(default = "enabled_by_default")]
    pub trust_email: bool,
}

fn enabled_by_default() -> bool {
    true
}

/// Adds a provider, once its discovery document names the issuer given. Needs Manage identity
/// providers.
#[utoipa::path(
    post,
    path = "/admin/identity-providers",
    tag = TAG_ADMIN,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = IdentityProviderSettings, headers(("Location" = String, description = "URL of the provider"))),
        (status = BAD_REQUEST, description = "`badRequest` or `validation`", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage identity providers", body = Problem),
        (status = CONFLICT, description = "`conflict`: a provider of that issuer exists", body = Problem),
        (status = BAD_GATEWAY, description = "`identityProviderUnreachable`: its discovery document could not be read; `detail` says why", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_provider(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Json(request): Json<IdentityProviderCreateRequest>,
) -> ApiResult<Created<IdentityProviderSettings>> {
    let added = providers::add(
        &state,
        &access,
        providers::NewProvider {
            name: request.name,
            issuer: request.issuer,
            client_id: request.client_id,
            client_secret: request.client_secret,
            enabled: request.enabled,
            skip_invite: request.skip_invite,
            enforces_two_factor: request.enforces_two_factor,
            trust_email: request.trust_email,
        },
    )
    .await?;
    Ok(Created::new(
        format!("{API_PREFIX}/admin/identity-providers/{}", added.id),
        added.into(),
    ))
}

/// Only this deployment's own dashboard sends it. Absent fields are unchanged.
#[derive(Deserialize, ToSchema, Default)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IdentityProviderUpdateRequest {
    pub name: Option<String>,
    pub issuer: Option<String>,
    pub client_id: Option<String>,
    /// A new secret; the old is never shown.
    pub client_secret: Option<String>,
    pub enabled: Option<bool>,
    pub skip_invite: Option<bool>,
    pub enforces_two_factor: Option<bool>,
    pub trust_email: Option<bool>,
}

/// Changes a provider. A changed issuer is checked against its discovery document first. Needs
/// Manage identity providers.
#[utoipa::path(
    patch,
    path = "/admin/identity-providers/{provider}",
    tag = TAG_ADMIN,
    params(("provider" = IdentityProviderId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = IdentityProviderSettings),
        (status = BAD_REQUEST, description = "`badRequest` or `validation`", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage identity providers", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`conflict`: a provider of that issuer exists", body = Problem),
        (status = BAD_GATEWAY, description = "`identityProviderUnreachable`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_provider(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(provider): Path<IdentityProviderId>,
    Json(request): Json<IdentityProviderUpdateRequest>,
) -> ApiResult<Json<IdentityProviderSettings>> {
    let updated = providers::update(
        &state,
        &access,
        provider,
        providers::ProviderChanges {
            name: request.name,
            issuer: request.issuer,
            client_id: request.client_id,
            client_secret: request.client_secret,
            enabled: request.enabled,
            skip_invite: request.skip_invite,
            enforces_two_factor: request.enforces_two_factor,
            trust_email: request.trust_email,
        },
    )
    .await?;
    Ok(Json(updated.into()))
}

/// Removes a provider: it offers sign-in no more, and its links are deleted in the background.
/// The sign-ins it made last until they end. Accounts whose only way in it was can no longer
/// sign in. Needs Manage identity providers.
#[utoipa::path(
    delete,
    path = "/admin/identity-providers/{provider}",
    tag = TAG_ADMIN,
    params(("provider" = IdentityProviderId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`adminRequired`, or `forbidden` without Manage identity providers", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_provider(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(provider): Path<IdentityProviderId>,
) -> ApiResult<NoContent> {
    providers::remove(&state, &access, provider).await?;
    Ok(NoContent)
}

/// What a provider sends the browser back with.
#[derive(Deserialize)]
pub struct CallbackQuery {
    pub state: Option<String>,
    pub code: Option<String>,
    pub error: Option<String>,
}

/// Inline style only, nothing fetched, never framed.
const PAGE_POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// The page shown when the callback has nowhere to send the browser: the flow is unknown,
/// expired, or already answered.
fn error_page(status: StatusCode, message: &str) -> Response {
    let title = html_escape(&t!("identityProviderPageTitle"));
    let message = html_escape(message);
    let lang = html_escape(app::locale::current());
    let body = format!(
        "<!doctype html><html lang=\"{lang}\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{title}</title><style>body{{font-family:system-ui,sans-serif;max-width:32rem;\
         margin:4rem auto;padding:0 1rem;line-height:1.5}}</style></head>\
         <body><h1>{title}</h1><p>{message}</p></body></html>"
    );
    (
        status,
        [
            (CONTENT_TYPE, "text/html; charset=utf-8"),
            (CONTENT_SECURITY_POLICY, PAGE_POLICY),
            (X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (REFERRER_POLICY, "no-referrer"),
            (CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// `GET /auth/oidc/callback`: where providers send the browser back. It sends the browser on to
/// the flow's return address, where the client claims the outcome, or shows why it cannot.
pub async fn callback(
    State(state): State<GlobalServerContext>,
    Query(query): Query<CallbackQuery>,
) -> Response {
    let Some(flow_id) = query.state else {
        return error_page(
            StatusCode::BAD_REQUEST,
            &t!("identityProviderFlowExpiredDetail"),
        );
    };
    let answered = flow::callback(
        &state,
        flow::Callback {
            state: flow_id,
            code: query.code,
            error: query.error,
        },
    )
    .await;
    match answered {
        Ok(location) => (
            StatusCode::SEE_OTHER,
            [
                (LOCATION, location.as_str()),
                (CACHE_CONTROL, "no-store"),
                (REFERRER_POLICY, "no-referrer"),
            ],
        )
            .into_response(),
        Err(app::Error::IdentityProviderFlowExpired) => error_page(
            StatusCode::NOT_FOUND,
            &t!("identityProviderFlowExpiredDetail"),
        ),
        Err(error) => {
            tracing::error!(%error, "the identity provider callback failed");
            error_page(
                StatusCode::INTERNAL_SERVER_ERROR,
                &t!("identityProviderFailedGenerally"),
            )
        }
    }
}

#[derive(Deserialize)]
pub struct LogoutForm {
    pub logout_token: String,
}

/// `POST /auth/oidc/back-channel-logout`: a provider ends the sign-ins it made of someone
/// (OpenID Connect Back-Channel Logout 1.0). It answers `200` once they have ended, or `400`
/// with an OAuth error body when the token is refused, saying no more than the specification
/// asks; why goes to the log.
pub async fn back_channel_logout(
    State(state): State<GlobalServerContext>,
    form: axum::Form<LogoutForm>,
) -> Response {
    let no_store = [(CACHE_CONTROL, "no-store")];
    match providers::logout::back_channel_logout(&state, &form.logout_token).await {
        Ok(ended) => {
            tracing::info!(
                ended,
                "an identity provider's back-channel logout ended sign-ins"
            );
            (StatusCode::OK, no_store).into_response()
        }
        Err(providers::logout::LogoutError::Refused(reason)) => {
            tracing::info!(reason, "refused a back-channel logout");
            (
                StatusCode::BAD_REQUEST,
                no_store,
                axum::Json(serde_json::json!({"error": "invalid_request"})),
            )
                .into_response()
        }
        Err(providers::logout::LogoutError::Server(error)) => {
            (no_store, ApiError::from(error)).into_response()
        }
    }
}
