//! The authorization code flows that sign in through a provider, link one to an account, and
//! verify a sign-in again (`docs/architecture/sign-in-and-security/identity-providers.md`).
//!
//! A flow is kept in Valkey under its id, which is also its OAuth `state`. The client starts it
//! with a PKCE challenge of its own; the server keeps a second PKCE verifier and a nonce for the
//! provider, which the client never sees. The callback takes the flow, exchanges the provider's
//! code, verifies the ID token, and keeps the outcome with the digest of a one-time return code,
//! which it adds to the return address. The client claims the outcome with its verifier and that
//! code, and only then does the flow take effect, so a flow started by one person and run by
//! another is never claimed, as with passkeys' hand-off (`app::passkey`).

use super::{CALLBACK_PATH, Provider, discovery, identity};
use crate::context::GlobalServerContext;
use crate::login::{self, LoginOutcome, ProviderSignIn, Session, SignInMethod};
use crate::passkey::{digest, validate_code_challenge, validate_return_to};
use crate::two_factor::Caller;
use crate::user::{NewAccount, NewEmail};
use crate::{CHACHA_RNG, IdentityProviderId, UserId, t};
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use diesel_async::AsyncConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use openidconnect::core::{
    CoreAuthPrompt, CoreAuthenticationFlow, CoreClient, CoreProviderMetadata,
};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse as _,
};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

const FLOW_PREFIX: &str = "auth:oidc:flow";
/// Long enough to sign in at the provider, a second factor there included.
const FLOW_LIFETIME_SECONDS: i64 = 10 * 60;
/// How long an outcome waits for its claim, and an account offered by a claim for its username.
const CLAIM_LIFETIME_SECONDS: i64 = 5 * 60;
/// The web client's page a flow it started returns to, under `public_url`.
pub const WEB_RETURN_PATH: &str = "/sign-in/provider";
/// How much earlier than a `reauthenticate` flow's start the provider's `auth_time` may be, for
/// clocks that differ.
const AUTH_TIME_SKEW_SECONDS: i64 = 30;

/// What a flow is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Purpose {
    /// Sign into the account the provider's identity is linked to, or make one.
    SignIn,
    /// Link the provider's identity to the caller's account.
    Link,
    /// Prove again who the caller is, by signing in at the provider afresh.
    Reauthenticate,
}

pub struct StartRequest {
    pub provider: IdentityProviderId,
    pub purpose: Purpose,
    /// `BASE64URL(SHA256(code_verifier))` of a verifier only the client knows.
    pub code_challenge: String,
    /// For the desktop and mobile apps: a loopback address or the `aspen:` scheme, as for a
    /// passkey's hand-off. Without it the flow returns to the web client's page.
    pub return_to: Option<String>,
}

pub struct Started {
    pub id: String,
    /// Where the client sends the browser: the provider's authorization endpoint.
    pub authorize_url: String,
    pub expires_at: DateTime<Utc>,
}

/// The sign-in a `link` or `reauthenticate` flow acts on, the only one that may claim it.
#[derive(Clone, Serialize, Deserialize)]
struct Starter {
    user: UserId,
    /// `login::sign_in_id`; the sign-in's tokens themselves are never kept here.
    sign_in: String,
}

#[derive(Serialize, Deserialize)]
struct Flow {
    provider: IdentityProviderId,
    purpose: Purpose,
    code_challenge: String,
    return_to: Option<String>,
    starter: Option<Starter>,
    nonce: String,
    /// The PKCE verifier the provider's code is exchanged with.
    provider_verifier: String,
    started_at: DateTime<Utc>,
    outcome: Option<Outcome>,
    /// The digest of the return code (`passkey::digest`), set with the outcome.
    return_code: Option<String>,
    /// A `signIn` flow whose claim offered an account, which `create_account` may now make.
    account_offered: bool,
}

#[derive(Serialize, Deserialize)]
enum Outcome {
    Identified(Identified),
    /// The provider refused, or its answer did not verify; the reason is localized for whoever
    /// ran the flow, in the language their browser asked for at the callback.
    Failed(String),
}

/// What a provider's verified ID token said.
#[derive(Clone, Serialize, Deserialize)]
struct Identified {
    /// The issuer the ID token was verified against. A claim made after the provider came to name
    /// another is refused, since a `sub` means something only at its own issuer.
    #[serde(default)]
    issuer: String,
    subject: String,
    /// The provider's session id (`sid`), which its back-channel logout names.
    session: Option<String>,
    two_factor: bool,
    auth_time: Option<DateTime<Utc>>,
    /// The address the provider reported, only when it reported it verified.
    verified_email: Option<String>,
    preferred_username: Option<String>,
    name: Option<String>,
}

impl Identified {
    fn method(&self) -> SignInMethod {
        if self.two_factor {
            SignInMethod::IdentityProviderTwoFactor
        } else {
            SignInMethod::IdentityProvider
        }
    }
}

fn new_secret() -> String {
    BASE64_URL_SAFE_NO_PAD.encode(CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 32]>()))
}

fn expired() -> crate::Error {
    crate::Error::IdentityProviderFlowExpired
}

/// The client this deployment is at `provider`, as its discovery `document` describes it,
/// returning to this deployment's callback.
fn client_of(
    state: &GlobalServerContext,
    provider: &Provider,
    document: &CoreProviderMetadata,
) -> crate::Result<
    CoreClient<
        openidconnect::EndpointSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointNotSet,
        openidconnect::EndpointMaybeSet,
        openidconnect::EndpointMaybeSet,
    >,
> {
    let redirect = RedirectUrl::new(format!(
        "{}{CALLBACK_PATH}",
        state.config.public_url.trim_end_matches('/')
    ))
    .map_err(|e| crate::Error::Config(config::ConfigError::Message(e.to_string())))?;
    Ok(CoreClient::from_provider_metadata(
        document.clone(),
        ClientId::new(provider.client_id.clone()),
        Some(ClientSecret::new(provider.client_secret.clone())),
    )
    .set_redirect_uri(redirect))
}

/// Starts a flow. `caller` is the session the request carried, if any; linking and verifying
/// again need one, of a person, and linking needs a recent verification too.
pub async fn start(
    state: &GlobalServerContext,
    caller: Option<&Caller>,
    request: StartRequest,
) -> crate::Result<Started> {
    validate_code_challenge(&request.code_challenge)?;
    if let Some(return_to) = &request.return_to {
        validate_return_to(return_to)?;
    }
    let starter = match request.purpose {
        Purpose::SignIn => None,
        Purpose::Link | Purpose::Reauthenticate => {
            let caller = caller.ok_or(crate::Error::Unauthenticated)?;
            if request.purpose == Purpose::Link {
                caller.ensure_recently_verified(&state.config.auth)?;
            } else {
                caller.ensure_person()?;
            }
            Some(Starter {
                user: caller.user,
                sign_in: caller.sign_in(),
            })
        }
    };
    let provider = {
        let mut conn = state.connection_pool.get().await?;
        let provider = super::usable(&mut conn, request.provider).await?;
        if let Some(starter) = &starter {
            let linked = identity::of(&mut conn, starter.user, &provider).await?;
            match (request.purpose, linked) {
                (Purpose::Link, Some(_)) => {
                    return Err(crate::Error::Conflict(t!(
                        "identityProviderAlreadyLinked",
                        provider = provider.name.as_str()
                    )));
                }
                (Purpose::Reauthenticate, None) => {
                    return Err(crate::Error::Validation(t!(
                        "identityProviderNotLinked",
                        provider = provider.name.as_str()
                    )));
                }
                _ => {}
            }
        }
        provider
    };
    // No database connection is held while the provider is called.
    let document = discovery::document(&state.config, &provider.issuer, false).await?;
    let client = client_of(state, &provider, &document)?;
    let id = new_secret();
    let nonce = new_secret();
    let (provider_challenge, provider_verifier) = PkceCodeChallenge::new_random_sha256();
    let mut authorize = client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            {
                let id = id.clone();
                move || CsrfToken::new(id)
            },
            {
                let nonce = nonce.clone();
                move || Nonce::new(nonce)
            },
        )
        .add_scope(Scope::new("email".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .set_pkce_challenge(provider_challenge);
    if request.purpose == Purpose::Reauthenticate {
        authorize = authorize
            .add_prompt(CoreAuthPrompt::Login)
            .set_max_age(std::time::Duration::ZERO);
    }
    let (authorize_url, _, _) = authorize.url();
    let flow = Flow {
        provider: provider.id,
        purpose: request.purpose,
        code_challenge: request.code_challenge,
        return_to: request.return_to,
        starter,
        nonce,
        provider_verifier: provider_verifier.secret().clone(),
        started_at: Utc::now(),
        outcome: None,
        return_code: None,
        account_offered: false,
    };
    crate::ephemeral_token::put_token(state, FLOW_PREFIX, &id, &flow, FLOW_LIFETIME_SECONDS)
        .await?;
    Ok(Started {
        id,
        authorize_url: authorize_url.to_string(),
        expires_at: Utc::now() + chrono::Duration::seconds(FLOW_LIFETIME_SECONDS),
    })
}

/// What the provider sent the browser back with.
pub struct Callback {
    /// The flow's id.
    pub state: String,
    pub code: Option<String>,
    /// An OAuth error code, when the provider did not sign them in.
    pub error: Option<String>,
}

/// Handles the provider's answer: takes the flow, exchanges the code, keeps the outcome for the
/// claim, and returns where to send the browser. A flow that is unknown, expired, or already
/// answered is `identityProviderFlowExpired`, which the callback's page shows, since there is no
/// return address to send it to.
pub async fn callback(state: &GlobalServerContext, answer: Callback) -> crate::Result<String> {
    let mut flow: Flow = crate::ephemeral_token::get_token(state, FLOW_PREFIX, &answer.state, true)
        .await?
        .filter(|flow: &Flow| flow.outcome.is_none())
        .ok_or_else(expired)?;
    let id = answer.state.clone();
    let outcome = match identify(state, &flow, answer).await {
        Ok(identified) => Outcome::Identified(identified),
        Err(error) => {
            tracing::info!(provider = %flow.provider.0, %error, "a sign-in through an identity provider failed");
            Outcome::Failed(failure_reason(error))
        }
    };
    let return_code = new_secret();
    flow.outcome = Some(outcome);
    flow.return_code = Some(digest(&return_code));
    let return_to = flow.return_to.clone();
    crate::ephemeral_token::put_token(state, FLOW_PREFIX, &id, &flow, CLAIM_LIFETIME_SECONDS)
        .await?;
    Ok(match return_to {
        Some(return_to) => {
            let mut url = url::Url::parse(&return_to)
                .map_err(|_| crate::Error::Validation(t!("invalidReturnTo")))?;
            url.query_pairs_mut()
                .append_pair("flow", &id)
                .append_pair("code", &return_code);
            url.to_string()
        }
        // In the fragment, so the code reaches no server's log.
        None => {
            let fragment = url::form_urlencoded::Serializer::new(String::new())
                .append_pair("flow", &id)
                .append_pair("code", &return_code)
                .finish();
            format!(
                "{}{WEB_RETURN_PATH}#{fragment}",
                state.config.public_url.trim_end_matches('/')
            )
        }
    })
}

/// The localized reason a failed flow's claim answers with.
fn failure_reason(error: crate::Error) -> String {
    match error {
        crate::Error::IdentityProviderFailed(reason)
        | crate::Error::IdentityProviderUnreachable(reason) => reason.into_owned(),
        crate::Error::IdentityProviderUnavailable => {
            t!("identityProviderUnavailableDetail").into_owned()
        }
        _ => t!("identityProviderFailedGenerally").into_owned(),
    }
}

/// Exchanges the provider's code and verifies its ID token for `flow`.
async fn identify(
    state: &GlobalServerContext,
    flow: &Flow,
    answer: Callback,
) -> crate::Result<Identified> {
    let provider: Provider = {
        let mut conn = state.connection_pool.get().await?;
        super::usable(&mut conn, flow.provider).await?
    };
    let name = provider.name.as_str();
    if let Some(error) = answer.error {
        return Err(crate::Error::IdentityProviderFailed(
            if error == "access_denied" {
                t!("identityProviderDenied", provider = name)
            } else {
                t!("identityProviderError", provider = name, error = error)
            },
        ));
    }
    let code = answer.code.ok_or_else(|| {
        crate::Error::IdentityProviderFailed(t!("identityProviderNoCode", provider = name))
    })?;
    let document = discovery::document(&state.config, &provider.issuer, false).await?;
    let http = discovery::http(&state.config)?;
    let token = client_of(state, &provider, &document)?
        .exchange_code(AuthorizationCode::new(code))
        .map_err(|_| {
            crate::Error::IdentityProviderFailed(t!(
                "identityProviderNoTokenEndpoint",
                provider = name
            ))
        })?
        .set_pkce_verifier(PkceCodeVerifier::new(flow.provider_verifier.clone()))
        .request_async(&http)
        .await
        .map_err(|error| exchange_failure(&provider, error))?;
    let id_token = token.id_token().ok_or_else(|| {
        crate::Error::IdentityProviderFailed(t!("identityProviderNoIdToken", provider = name))
    })?;
    let nonce = Nonce::new(flow.nonce.clone());
    let client = client_of(state, &provider, &document)?;
    let verifier = client.id_token_verifier();
    match id_token.claims(&verifier, &nonce) {
        Ok(claims) => Ok(identified(&provider, claims, &id_token.to_string())),
        // A key the cached document lacks may be one the provider has rotated to, so its
        // document is read afresh, at most once a minute per issuer. The code is spent by now, so
        // only the verification is tried again.
        Err(openidconnect::ClaimsVerificationError::SignatureVerification(_)) => {
            let document =
                discovery::document_for_unknown_key(&state.config, &provider.issuer).await?;
            let client = client_of(state, &provider, &document)?;
            let verifier = client.id_token_verifier();
            let claims = id_token
                .claims(&verifier, &nonce)
                .map_err(|error| token_invalid(name, error))?;
            Ok(identified(&provider, claims, &id_token.to_string()))
        }
        Err(error) => Err(token_invalid(name, error)),
    }
}

fn token_invalid(provider: &str, error: openidconnect::ClaimsVerificationError) -> crate::Error {
    tracing::info!(provider, %error, "an identity provider's ID token did not verify");
    crate::Error::IdentityProviderFailed(t!(
        "identityProviderTokenInvalid",
        provider = provider,
        detail = error.to_string()
    ))
}

/// Why exchanging the provider's code failed.
fn exchange_failure(
    provider: &Provider,
    error: openidconnect::RequestTokenError<
        discovery::HttpError,
        openidconnect::StandardErrorResponse<openidconnect::core::CoreErrorResponseType>,
    >,
) -> crate::Error {
    use openidconnect::RequestTokenError;
    match error {
        RequestTokenError::Request(error) => discovery::request_failure(&provider.issuer, &error),
        RequestTokenError::ServerResponse(response) => {
            tracing::info!(provider = provider.name, error = %response, "an identity provider refused a code");
            crate::Error::IdentityProviderFailed(t!(
                "identityProviderCodeRefused",
                provider = provider.name.as_str(),
                error = response.error().to_string()
            ))
        }
        other => {
            tracing::info!(provider = provider.name, error = %other, "an identity provider's token answer was unreadable");
            crate::Error::IdentityProviderFailed(t!(
                "identityProviderTokenUnreadable",
                provider = provider.name.as_str()
            ))
        }
    }
}

/// The `amr` values (RFC 8176) that show a second factor on their own.
const SECOND_FACTOR_METHODS: [&str; 4] = ["mfa", "otp", "hwk", "sms"];
/// The `amr` values that count as a factor of their own, so that two of them, or one and a
/// password, are two factors.
const FACTOR_METHODS: [&str; 13] = [
    "pwd", "pin", "otp", "hwk", "swk", "sms", "tel", "fpt", "face", "iris", "retina", "vbm", "sc",
];

/// Whether an `amr` claim names two factors: a method that is one on its own, or two methods of
/// different kinds.
fn two_factors(amr: &[String]) -> bool {
    if amr
        .iter()
        .any(|method| SECOND_FACTOR_METHODS.contains(&method.as_str()))
    {
        return true;
    }
    let factors: std::collections::HashSet<&str> = amr
        .iter()
        .map(String::as_str)
        .filter(|method| FACTOR_METHODS.contains(method))
        .collect();
    factors.len() >= 2
}

/// The `sid` claim, which `openidconnect`'s standard claims leave out, read from the token whose
/// signature has just been verified.
fn session_id(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = BASE64_URL_SAFE_NO_PAD.decode(payload).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("sid")?
        .as_str()
        .filter(|sid| !sid.is_empty() && sid.len() <= 255)
        .map(str::to_string)
}

fn identified(
    provider: &Provider,
    claims: &openidconnect::core::CoreIdTokenClaims,
    id_token: &str,
) -> Identified {
    let amr: Vec<String> = claims
        .auth_method_refs()
        .map(|methods| methods.iter().map(|m| m.to_string()).collect())
        .unwrap_or_default();
    Identified {
        issuer: provider.issuer.clone(),
        subject: claims.subject().to_string(),
        session: session_id(id_token),
        two_factor: provider.enforces_two_factor || two_factors(&amr),
        auth_time: claims.auth_time(),
        verified_email: claims
            .email()
            .filter(|_| claims.email_verified() == Some(true))
            .map(|email| email.to_string()),
        preferred_username: claims.preferred_username().map(|name| name.to_string()),
        name: claims
            .name()
            .and_then(|name| name.get(None))
            .map(|name| name.to_string()),
    }
}

/// A new account the claim of a `signIn` flow offers, when the provider's identity is linked to
/// none.
pub struct AccountOffer {
    pub provider_name: String,
    /// The provider's `preferred_username`, when an account could take it.
    pub suggested_name: Option<String>,
    pub display_name: Option<String>,
    pub invite_required: bool,
    /// The verified address the account will have, from the provider.
    pub email: Option<String>,
    /// Whether the person must give an address, the provider having given none it trusts.
    pub email_required: bool,
}

/// What a claimed flow did.
pub enum Claim {
    /// A `signIn` flow: the session, or a second factor still owed.
    SignedIn(LoginOutcome),
    AccountRequired(AccountOffer),
    Linked(identity::IdentitySummary),
    Reauthenticated {
        verified_until: DateTime<Utc>,
    },
}

/// Reads a flow whose outcome is in, checking the client's verifier and, unless `return_code`
/// is `None` for a flow already claimed, the return code.
async fn claimable(
    state: &GlobalServerContext,
    id: &str,
    code_verifier: &str,
    return_code: Option<&str>,
) -> crate::Result<Flow> {
    let flow: Flow = crate::ephemeral_token::get_token(state, FLOW_PREFIX, id, false)
        .await?
        .ok_or_else(expired)?;
    let Some(expected_code) = &flow.return_code else {
        return Err(expired());
    };
    let verifier_matches = digest(code_verifier)
        .as_bytes()
        .ct_eq(flow.code_challenge.as_bytes());
    let code_matches = match return_code {
        Some(code) => digest(code).as_bytes().ct_eq(expected_code.as_bytes()),
        None => subtle::Choice::from(u8::from(flow.account_offered)),
    };
    if !bool::from(verifier_matches & code_matches) {
        return Err(crate::Error::VerificationFailed);
    }
    Ok(flow)
}

async fn forget(state: &GlobalServerContext, id: &str) -> crate::Result<()> {
    let _: Option<Flow> = crate::ephemeral_token::get_token(state, FLOW_PREFIX, id, true).await?;
    Ok(())
}

/// Refuses anyone but the live sign-in that started a flow which acts on one.
async fn ensure_starter(
    state: &GlobalServerContext,
    starter: Option<&Starter>,
    caller: Option<&Caller>,
) -> crate::Result<UserId> {
    let (Some(starter), Some(caller)) = (starter, caller) else {
        return Err(crate::Error::Forbidden(t!("identityProviderFlowNotYours")));
    };
    if caller.user != starter.user || caller.sign_in() != starter.sign_in {
        return Err(crate::Error::Forbidden(t!("identityProviderFlowNotYours")));
    }
    let mut conn = state.connection_pool.get().await?;
    if !login::sign_in_live(&mut conn, starter.user, &starter.sign_in).await? {
        return Err(crate::Error::Forbidden(t!("identityProviderFlowNotYours")));
    }
    Ok(starter.user)
}

/// Claims a flow's outcome and gives it effect. `code_verifier` is the secret whose SHA-256 the
/// client gave as it started the flow, `return_code` the code the browser brought back, and
/// `caller` the sign-in that started a `link` or `reauthenticate` flow.
pub async fn claim(
    state: &GlobalServerContext,
    id: &str,
    caller: Option<&Caller>,
    code_verifier: &str,
    return_code: &str,
) -> crate::Result<Claim> {
    let mut flow = claimable(state, id, code_verifier, Some(return_code)).await?;
    let identified = match flow.outcome.take() {
        Some(Outcome::Identified(identified)) => identified,
        Some(Outcome::Failed(reason)) => {
            forget(state, id).await?;
            return Err(crate::Error::IdentityProviderFailed(reason.into()));
        }
        None => return Err(expired()),
    };
    let mut conn = state.connection_pool.get().await?;
    let provider = super::usable(&mut conn, flow.provider).await?;
    if identified.issuer != provider.issuer {
        forget(state, id).await?;
        return Err(expired());
    }
    match flow.purpose {
        Purpose::SignIn => {
            let linked = identity::account_of(&mut conn, &provider, &identified.subject).await?;
            match linked {
                Some(user) => {
                    forget(state, id).await?;
                    identity::used(&mut conn, &provider, &identified.subject).await?;
                    let outcome = login::first_factor_proved(
                        state,
                        &mut conn,
                        user,
                        identified.method(),
                        Some(ProviderSignIn {
                            provider: provider.id,
                            session: identified.session.clone(),
                        }),
                    )
                    .await?;
                    Ok(Claim::SignedIn(outcome))
                }
                None => {
                    drop(conn);
                    let offer = offer(state, &provider, &identified);
                    flow.outcome = Some(Outcome::Identified(identified));
                    flow.account_offered = true;
                    crate::ephemeral_token::put_token(
                        state,
                        FLOW_PREFIX,
                        id,
                        &flow,
                        CLAIM_LIFETIME_SECONDS,
                    )
                    .await?;
                    Ok(Claim::AccountRequired(offer))
                }
            }
        }
        Purpose::Link => {
            drop(conn);
            let user = ensure_starter(state, flow.starter.as_ref(), caller).await?;
            forget(state, id).await?;
            let mut conn = state.connection_pool.get().await?;
            let linked = identity::link(&mut conn, user, &provider, &identified.subject).await?;
            Ok(Claim::Linked(linked))
        }
        Purpose::Reauthenticate => {
            drop(conn);
            let user = ensure_starter(state, flow.starter.as_ref(), caller).await?;
            let caller = caller.ok_or(crate::Error::Unauthenticated)?;
            forget(state, id).await?;
            let fresh = identified.auth_time.is_some_and(|at| {
                at >= flow.started_at - chrono::Duration::seconds(AUTH_TIME_SKEW_SECONDS)
            });
            if !fresh {
                return Err(crate::Error::IdentityProviderFailed(t!(
                    "identityProviderNotFresh",
                    provider = provider.name.as_str()
                )));
            }
            let mut conn = state.connection_pool.get().await?;
            if identity::account_of(&mut conn, &provider, &identified.subject).await? != Some(user)
            {
                return Err(crate::Error::IdentityProviderFailed(t!(
                    "identityProviderOtherAccount",
                    provider = provider.name.as_str()
                )));
            }
            if caller.has_second_factor && !identified.two_factor {
                return Err(crate::Error::Validation(t!(
                    "reauthenticateWithSecondFactor"
                )));
            }
            let verified_until = crate::two_factor::mark_verified(
                &mut conn,
                user,
                &caller.sign_in(),
                &state.config.auth,
            )
            .await?;
            Ok(Claim::Reauthenticated { verified_until })
        }
    }
}

/// What the account a `signIn` flow offers would be.
fn offer(
    state: &GlobalServerContext,
    provider: &Provider,
    identified: &Identified,
) -> AccountOffer {
    let email = trusted_email(state, provider, identified);
    AccountOffer {
        provider_name: provider.name.clone(),
        suggested_name: identified
            .preferred_username
            .clone()
            .filter(|name| crate::user::validate_new_username(name).is_ok()),
        display_name: display_name(identified),
        invite_required: invite_required(state, provider),
        email_required: email.is_none()
            && state.settings().email_required
            && crate::email::available(state),
        email,
    }
}

fn invite_required(state: &GlobalServerContext, provider: &Provider) -> bool {
    state.settings().registration_invite_required && !provider.skip_invite
}

/// The provider's verified address, when the provider is trusted for it and the deployment
/// takes email.
fn trusted_email(
    state: &GlobalServerContext,
    provider: &Provider,
    identified: &Identified,
) -> Option<String> {
    if !provider.trust_email || !crate::email::available(state) {
        return None;
    }
    identified
        .verified_email
        .as_deref()
        .and_then(|address| crate::email::parse_address(address).ok())
}

/// The provider's `name`, when it is one a display name may be.
fn display_name(identified: &Identified) -> Option<String> {
    identified.name.clone().filter(|name| {
        crate::user::validate_profile(&aspen_wire::message_enum::request::UserUpdateRequest {
            display_name: Some(Some(name.clone())),
            ..Default::default()
        })
        .is_ok()
    })
}

/// What the person chose for the account a claim offered.
pub struct AccountRequest {
    pub name: String,
    pub invite_code: Option<String>,
    /// An address, when the provider gave none it trusts.
    pub email: Option<String>,
    pub newsletter: bool,
}

/// Makes the account a `signIn` flow's claim offered, linked to the provider's identity, and
/// signs it in. A name someone else holds leaves the flow as it was, so another may be tried.
pub async fn create_account(
    state: &GlobalServerContext,
    id: &str,
    code_verifier: &str,
    request: AccountRequest,
) -> crate::Result<Session> {
    let flow = claimable(state, id, code_verifier, None).await?;
    let Some(Outcome::Identified(identified)) = &flow.outcome else {
        return Err(expired());
    };
    if flow.purpose != Purpose::SignIn || !flow.account_offered {
        return Err(expired());
    }
    crate::user::validate_new_username(&request.name)?;
    let mut conn = state.connection_pool.get().await?;
    let provider = super::usable(&mut conn, flow.provider).await?;
    if identified.issuer != provider.issuer {
        return Err(expired());
    }
    let invite_required = invite_required(state, &provider);
    let invite_code = request
        .invite_code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty())
        .map(str::to_string);
    if invite_required && invite_code.is_none() {
        return Err(crate::Error::RegistrationInviteRequired);
    }
    let email = match trusted_email(state, &provider, identified) {
        Some(address) => Some(NewEmail {
            address,
            verified: true,
            newsletter: request.newsletter,
        }),
        None => crate::email::check_registration(state, request.email.as_deref())?.map(|address| {
            NewEmail {
                address,
                verified: false,
                newsletter: request.newsletter,
            }
        }),
    };
    let account = NewAccount {
        name: request.name,
        password_hash: String::new(),
        display_name: display_name(identified),
        pronouns: None,
        bio: None,
        status: None,
        email,
        invite_code,
        invite_required,
    };
    let subject = identified.subject.clone();
    let provider_id = provider.id;
    let linked_at = provider.clone();
    let (user, joined) = conn
        .transaction(|conn| {
            async move {
                let (user, joined) = crate::user::create_account(state, conn, account).await?;
                identity::insert(conn, user, &linked_at, &subject).await?;
                Ok::<_, crate::Error>((user, joined))
            }
            .scope_boxed()
        })
        .await?;
    forget(state, id).await?;
    crate::user::after_account_created(state, joined).await;
    let session = login::first_factor_proved(
        state,
        &mut conn,
        user,
        identified.method(),
        Some(ProviderSignIn {
            provider: provider_id,
            session: identified.session.clone(),
        }),
    )
    .await?;
    match session {
        LoginOutcome::SignedIn(session) => Ok(session),
        // A new account holds no second factor to ask for.
        LoginOutcome::SecondFactorRequired { .. } | LoginOutcome::InvalidCredentials => {
            Err(crate::Error::IdentityProviderFlowExpired)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn amr(methods: &[&str]) -> Vec<String> {
        methods.iter().map(|m| m.to_string()).collect()
    }

    #[test]
    fn two_factors_from_amr() {
        assert!(two_factors(&amr(&["mfa"])));
        assert!(two_factors(&amr(&["pwd", "otp"])));
        assert!(two_factors(&amr(&["hwk"])));
        assert!(two_factors(&amr(&["pwd", "fpt"])));
        assert!(two_factors(&amr(&["swk", "pin"])));
        assert!(!two_factors(&amr(&["pwd"])));
        assert!(!two_factors(&amr(&["swk"])));
        assert!(!two_factors(&amr(&["fpt", "fpt"])));
        assert!(!two_factors(&amr(&["kba", "pwd"])));
        assert!(!two_factors(&[]));
    }

    #[test]
    fn session_id_is_read_from_the_payload() {
        let payload = BASE64_URL_SAFE_NO_PAD.encode(br#"{"sub":"a","sid":"s-1"}"#);
        assert_eq!(
            session_id(&format!("h.{payload}.s")).as_deref(),
            Some("s-1")
        );
        let payload = BASE64_URL_SAFE_NO_PAD.encode(br#"{"sub":"a"}"#);
        assert_eq!(session_id(&format!("h.{payload}.s")), None);
        assert_eq!(session_id("not a token"), None);
    }
}
