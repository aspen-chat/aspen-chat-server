//! Passkeys (WebAuthn).
//!
//! A passkey signs its user in on its own, counting as both factors, and also serves as the
//! second factor after a password. Every exchange with an authenticator is a ceremony: `start`
//! makes the options the browser passes to `navigator.credentials`, keeps the verifier's state
//! in Valkey under an unguessable id for `CEREMONY_LIFETIME_SECONDS`, and `complete` checks what
//! the authenticator returned. Keeping the state in Valkey rather than in process lets any API
//! server finish a ceremony another one started.
//!
//! A browser offers a passkey only to pages under the relying party's domain, and the desktop
//! and mobile apps are not such pages. They hand the ceremony to the system browser instead:
//! `start` is given a `Handoff` naming where to return and the SHA-256 of a secret only the app
//! knows (PKCE, RFC 7636). The page this server serves at `/auth/passkey` completes the
//! ceremony, which only checks the credential; the browser returns to the app with a one-time
//! return code added to the return address, and the app `claim`s the outcome with its secret
//! and that code, and only then does the ceremony take effect. Both are needed because the
//! page's link can be sent to someone else: whoever starts a ceremony knows the secret, but the
//! return code reaches only the return address, which is on the device whose browser ran the
//! ceremony (a loopback port or the `aspen:` scheme), so a ceremony started by one person and
//! run by another is never claimed. Adding a passkey and re-verifying act on the sign-in that
//! started them, and are completed (in the page that runs them) or claimed (after a handoff) by
//! that sign-in alone.

use crate::CHACHA_RNG;
use crate::app::context::GlobalServerContext;
use crate::app::ephemeral_token;
use crate::app::login::{self, Session};
use crate::app::two_factor::{self, Caller, PasskeySummary};
use crate::app::{self, PasskeyId, UserId};
use crate::aspen_config::AspenConfig;
use crate::database::schema::{passkey, user};
use crate::t;
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use webauthn_rs::prelude::{
    AuthenticationResult, DiscoverableAuthentication, DiscoverableKey, Passkey,
    PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    Url, Webauthn, WebauthnBuilder,
};

const CEREMONY_PREFIX: &str = "auth:ceremony";
/// Long enough to find a security key or a phone.
const CEREMONY_LIFETIME_SECONDS: i64 = 5 * 60;
/// How long a finished handoff waits for the app to claim it.
const CLAIM_LIFETIME_SECONDS: i64 = 2 * 60;
/// The longest passkey name kept.
pub const MAX_NAME_CHARS: usize = 64;

/// The relying party, called `name` in passkey prompts, or `None` where passkeys cannot work
/// (`AuthConfig::rp_id`). Its one origin is `public_url`, where every page that runs a ceremony
/// is: the web client's and the handoff page the apps open.
pub fn relying_party(config: &AspenConfig, name: &str) -> app::Result<Option<Webauthn>> {
    let Some(rp_id) = &config.auth.rp_id else {
        return Ok(None);
    };
    let origin = Url::parse(&config.public_url).map_err(|e| {
        app::Error::Config(config::ConfigError::Message(format!(
            "public_url {} is not a URL: {e}",
            config.public_url
        )))
    })?;
    Ok(Some(
        WebauthnBuilder::new(rp_id, &origin)?
            .rp_name(name)
            .build()?,
    ))
}

/// The relying party for a ceremony, called what the deployment is called now. Building one
/// only parses `public_url`, which the server checked as it started.
fn relying_party_of(state: &GlobalServerContext) -> app::Result<Webauthn> {
    relying_party(&state.config, state.settings().name())?.ok_or(app::Error::PasskeysUnavailable)
}

/// What a ceremony is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Purpose {
    /// Sign in with a passkey, on its own or as the second factor of a password sign-in.
    SignIn,
    /// Add a passkey to the caller's account.
    Register,
    /// Prove again who the caller is, before a change to security settings.
    Reauthenticate,
}

/// Where the system browser returns once the ceremony is done, and the PKCE challenge the app
/// will answer when it claims the outcome.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Handoff {
    /// `BASE64URL(SHA256(code_verifier))`, as in RFC 7636's `S256` method.
    pub code_challenge: String,
    pub return_to: String,
}

pub struct StartRequest {
    pub purpose: Purpose,
    /// For `SignIn` as a second factor: the ticket `login::try_login` issued. The passkey must
    /// then be the ticket's user's.
    pub ticket: Option<String>,
    /// For `Register`: what to call the passkey.
    pub name: Option<String>,
    pub handoff: Option<Handoff>,
}

pub struct Started {
    pub id: String,
    /// `{"publicKey": …}` for `navigator.credentials.create` (register) or `.get` (otherwise),
    /// with binary fields in base64url.
    pub options: serde_json::Value,
    pub expires_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
enum Pending {
    SignIn {
        state: DiscoverableAuthentication,
        ticket: Option<String>,
    },
    Register {
        user: UserId,
        /// `login::sign_in_id` of the sign-in that started it, the only one that may finish it
        /// and the one the ceremony acts on. The sign-in's tokens themselves are never kept here.
        starter: String,
        name: String,
        state: PasskeyRegistration,
    },
    Reauthenticate {
        user: UserId,
        /// As for `Register`.
        starter: String,
        state: PasskeyAuthentication,
    },
}

impl Pending {
    fn starter(&self) -> Option<&str> {
        match self {
            Pending::SignIn { .. } => None,
            Pending::Register { starter, .. } | Pending::Reauthenticate { starter, .. } => {
                Some(starter)
            }
        }
    }
}

/// A ceremony whose credential verified, and the effect it is to have. A handed-off ceremony
/// keeps it until it is claimed, so nothing happens for a ceremony that is never claimed.
#[derive(Serialize, Deserialize)]
enum Verified {
    SignedIn {
        user: UserId,
        ticket: Option<String>,
    },
    Registered {
        user: UserId,
        starter: String,
        name: String,
        key: Box<Passkey>,
    },
    Reauthenticated {
        user: UserId,
        starter: String,
    },
}

impl Verified {
    fn starter(&self) -> Option<&str> {
        match self {
            Verified::SignedIn { .. } => None,
            Verified::Registered { starter, .. } | Verified::Reauthenticated { starter, .. } => {
                Some(starter)
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Ceremony {
    purpose: Purpose,
    options: serde_json::Value,
    handoff: Option<Handoff>,
    pending: Option<Pending>,
    outcome: Option<Verified>,
    /// For a completed handoff: the SHA-256 of the return code added to the return address,
    /// which the claim must present.
    return_code: Option<String>,
}

/// A finished ceremony, as its starter receives it.
pub enum CeremonyResult {
    SignedIn(Session),
    PasskeyAdded {
        passkey: PasskeySummary,
        /// Set when this passkey turned two-factor sign-in on.
        recovery_codes: Option<Vec<String>>,
    },
    Reauthenticated {
        verified_until: DateTime<Utc>,
    },
}

pub enum Completion {
    Done(CeremonyResult),
    /// A handed-off ceremony: the page sends the browser here, and the app claims the result.
    HandedOff {
        return_to: String,
    },
}

/// A ceremony as the handoff page needs it.
pub struct Description {
    pub purpose: Purpose,
    pub options: serde_json::Value,
    pub return_to: Option<String>,
}

fn new_ceremony_id() -> String {
    BASE64_URL_SAFE_NO_PAD.encode(CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 32]>()))
}

fn rejected(e: impl std::fmt::Display) -> app::Error {
    app::Error::PasskeyRejected(e.to_string())
}

/// Checks where a handoff may send the browser back to: a loopback address with a port (the
/// desktop app's one-shot listener, RFC 8252 §7.3) or the `aspen:` scheme (the mobile apps).
/// Both reach only the device whose browser ran the ceremony, which the return code relies on.
/// No web page is accepted: the web client runs ceremonies in its own page, and a page that
/// forwarded its query anywhere would hand the code on.
pub fn validate_return_to(return_to: &str) -> app::Result<Url> {
    let invalid = || app::Error::Validation(t!("invalidReturnTo"));
    let url = Url::parse(return_to).map_err(|_| invalid())?;
    if url.fragment().is_some() || !url.username().is_empty() || url.password().is_some() {
        return Err(invalid());
    }
    let loopback_host = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(domain)) => domain == "localhost",
        None => false,
    };
    let loopback = url.scheme() == "http" && url.port().is_some() && loopback_host;
    let app_scheme = url.scheme() == "aspen";
    if loopback || app_scheme {
        Ok(url)
    } else {
        Err(invalid())
    }
}

fn validate_code_challenge(challenge: &str) -> app::Result<()> {
    let decoded = BASE64_URL_SAFE_NO_PAD
        .decode(challenge)
        .map_err(|_| app::Error::Validation(t!("invalidCodeChallenge")))?;
    if decoded.len() != 32 {
        return Err(app::Error::Validation(t!("invalidCodeChallenge")));
    }
    Ok(())
}

fn passkey_name(requested: Option<String>) -> app::Result<String> {
    let name = requested.map(|n| n.trim().to_string()).unwrap_or_default();
    if name.is_empty() {
        return Ok(t!("passkeyDefaultName").into_owned());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(app::Error::Validation(t!(
            "passkeyNameTooLong",
            max = MAX_NAME_CHARS
        )));
    }
    Ok(name)
}

/// Starts a ceremony. `caller` is the session the request carried, if any; adding a passkey and
/// re-verifying need one, and adding one needs a recent verification too.
pub async fn start(
    state: &GlobalServerContext,
    caller: Option<&Caller>,
    request: StartRequest,
) -> app::Result<Started> {
    let webauthn = relying_party_of(state)?;
    if let Some(handoff) = &request.handoff {
        validate_return_to(&handoff.return_to)?;
        validate_code_challenge(&handoff.code_challenge)?;
    }
    let mut conn = state.connection_pool.get().await?;
    let (options, pending) = match request.purpose {
        Purpose::SignIn => {
            if let Some(ticket) = &request.ticket
                && login::ticket_user(state, ticket).await?.is_none()
            {
                return Err(app::Error::InvalidTicket);
            }
            let (mut challenge, auth_state) = webauthn.start_discoverable_authentication()?;
            // The library asks for conditional mediation (autofill); Aspen shows the passkey
            // prompt when the user presses the button instead.
            challenge.mediation = None;
            (
                serde_json::to_value(&challenge)?,
                Pending::SignIn {
                    state: auth_state,
                    ticket: request.ticket,
                },
            )
        }
        Purpose::Register => {
            let caller = caller.ok_or(app::Error::Unauthenticated)?;
            caller.ensure_recently_verified(&state.config.auth)?;
            let name = passkey_name(request.name)?;
            let (user_name, display_name): (String, Option<String>) = user::table
                .select((user::name, user::display_name))
                .filter(user::id.eq(caller.user))
                .first(&mut conn)
                .await?;
            let existing = credentials(&mut conn, caller.user)
                .await?
                .into_iter()
                .map(|(_, key)| key.cred_id().clone())
                .collect();
            let (challenge, registration) = webauthn.start_passkey_registration(
                caller.user.0,
                &user_name,
                display_name.as_deref().unwrap_or(&user_name),
                Some(existing),
            )?;
            let mut options = serde_json::to_value(&challenge)?;
            require_discoverable(&mut options);
            (
                options,
                Pending::Register {
                    user: caller.user,
                    starter: login::sign_in_id(&caller.refresh_token),
                    name,
                    state: registration,
                },
            )
        }
        Purpose::Reauthenticate => {
            let caller = caller.ok_or(app::Error::Unauthenticated)?;
            let keys: Vec<Passkey> = credentials(&mut conn, caller.user)
                .await?
                .into_iter()
                .map(|(_, key)| key)
                .collect();
            if keys.is_empty() {
                return Err(app::Error::Validation(t!("noPasskeys")));
            }
            let (challenge, auth_state) = webauthn.start_passkey_authentication(&keys)?;
            (
                serde_json::to_value(&challenge)?,
                Pending::Reauthenticate {
                    user: caller.user,
                    starter: login::sign_in_id(&caller.refresh_token),
                    state: auth_state,
                },
            )
        }
    };
    let id = new_ceremony_id();
    let ceremony = Ceremony {
        purpose: request.purpose,
        options: options.clone(),
        handoff: request.handoff,
        pending: Some(pending),
        outcome: None,
        return_code: None,
    };
    ephemeral_token::put_token(
        state,
        CEREMONY_PREFIX,
        &id,
        &ceremony,
        CEREMONY_LIFETIME_SECONDS,
    )
    .await?;
    Ok(Started {
        id,
        options,
        expires_at: Utc::now() + chrono::Duration::seconds(CEREMONY_LIFETIME_SECONDS),
    })
}

/// Makes the credential discoverable, so it can sign its user in without a username. The
/// library leaves that to the authenticator; Aspen's sign-in depends on it.
fn require_discoverable(options: &mut serde_json::Value) {
    if let Some(selection) = options
        .get_mut("publicKey")
        .and_then(|public_key| public_key.get_mut("authenticatorSelection"))
        .and_then(serde_json::Value::as_object_mut)
    {
        selection.insert("residentKey".into(), "required".into());
        selection.insert("requireResidentKey".into(), true.into());
    }
}

/// The options of a handed-off ceremony still waiting for its authenticator, for the handoff
/// page. A ceremony that was not handed off is run by the page that started it, never by the
/// handoff page, so it reads as unknown here.
pub async fn describe(state: &GlobalServerContext, id: &str) -> app::Result<Description> {
    let ceremony: Ceremony = ephemeral_token::get_token(state, CEREMONY_PREFIX, id, false)
        .await?
        .filter(|ceremony: &Ceremony| ceremony.pending.is_some())
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))?;
    let handoff = ceremony
        .handoff
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))?;
    Ok(Description {
        purpose: ceremony.purpose,
        options: ceremony.options,
        return_to: Some(handoff.return_to),
    })
}

/// Refuses anyone but the sign-in that started a ceremony which acts on it.
fn ensure_starter(starter: Option<&str>, caller: Option<&Caller>) -> app::Result<()> {
    match starter {
        None => Ok(()),
        Some(starter)
            if caller.is_some_and(|caller| login::sign_in_id(&caller.refresh_token) == starter) =>
        {
            Ok(())
        }
        Some(_) => Err(app::Error::Forbidden(t!("passkeyCeremonyNotYours"))),
    }
}

fn digest(secret: &str) -> String {
    BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(secret.as_bytes()))
}

/// Checks what the authenticator returned. A ceremony is completed at most once: it is taken
/// from Valkey before anything is checked. One that was not handed off takes effect at once,
/// for `caller`, who must be the sign-in that started it when it acts on one; a handed-off one
/// waits for its claim.
pub async fn complete(
    state: &GlobalServerContext,
    id: &str,
    caller: Option<&Caller>,
    credential: serde_json::Value,
) -> app::Result<Completion> {
    let webauthn = relying_party_of(state)?;
    let mut ceremony: Ceremony = ephemeral_token::get_token(state, CEREMONY_PREFIX, id, true)
        .await?
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))?;
    let pending = ceremony
        .pending
        .take()
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))?;
    if ceremony.handoff.is_none() {
        ensure_starter(pending.starter(), caller)?;
    }
    let mut conn = state.connection_pool.get().await?;
    let verified = match pending {
        Pending::SignIn {
            state: auth_state,
            ticket,
        } => {
            let credential: PublicKeyCredential =
                serde_json::from_value(credential).map_err(rejected)?;
            let (user_uuid, _) = webauthn
                .identify_discoverable_authentication(&credential)
                .map_err(rejected)?;
            let user_id = UserId(user_uuid);
            if let Some(ticket) = &ticket
                && login::ticket_user(state, ticket).await? != Some(user_id)
            {
                return Err(app::Error::PasskeyRejected(
                    "the passkey belongs to another account".to_string(),
                ));
            }
            let stored = credentials(&mut conn, user_id).await?;
            let keys: Vec<DiscoverableKey> = stored.iter().map(|(_, key)| key.into()).collect();
            let result = webauthn
                .finish_discoverable_authentication(&credential, auth_state, &keys)
                .map_err(rejected)?;
            record_use(&mut conn, stored, &result).await?;
            Verified::SignedIn {
                user: user_id,
                ticket,
            }
        }
        Pending::Register {
            user,
            starter,
            name,
            state: registration,
        } => {
            let credential: RegisterPublicKeyCredential =
                serde_json::from_value(credential).map_err(rejected)?;
            let key = webauthn
                .finish_passkey_registration(&credential, &registration)
                .map_err(rejected)?;
            Verified::Registered {
                user,
                starter,
                name,
                key: Box::new(key),
            }
        }
        Pending::Reauthenticate {
            user,
            starter,
            state: auth_state,
        } => {
            let credential: PublicKeyCredential =
                serde_json::from_value(credential).map_err(rejected)?;
            let result = webauthn
                .finish_passkey_authentication(&credential, &auth_state)
                .map_err(rejected)?;
            let stored = credentials(&mut conn, user).await?;
            record_use(&mut conn, stored, &result).await?;
            Verified::Reauthenticated { user, starter }
        }
    };
    drop(conn);
    match ceremony.handoff.clone() {
        Some(handoff) => {
            let mut return_to = Url::parse(&handoff.return_to)
                .map_err(|_| app::Error::Validation(t!("invalidReturnTo")))?;
            let return_code = new_ceremony_id();
            return_to
                .query_pairs_mut()
                .append_pair("ceremony", id)
                .append_pair("outcome", "done")
                .append_pair("code", &return_code);
            ceremony.outcome = Some(verified);
            ceremony.return_code = Some(digest(&return_code));
            ephemeral_token::put_token(
                state,
                CEREMONY_PREFIX,
                id,
                &ceremony,
                CLAIM_LIFETIME_SECONDS,
            )
            .await?;
            Ok(Completion::HandedOff {
                return_to: return_to.to_string(),
            })
        }
        None => Ok(Completion::Done(take_effect(state, verified).await?)),
    }
}

/// Takes the outcome of a handed-off ceremony and gives it effect. `code_verifier` must be the
/// secret whose SHA-256 the app gave when it started the ceremony, `return_code` the code the
/// browser brought back to the return address, and `caller` the sign-in that started it when
/// the ceremony acts on one.
pub async fn claim(
    state: &GlobalServerContext,
    id: &str,
    caller: Option<&Caller>,
    code_verifier: &str,
    return_code: Option<&str>,
) -> app::Result<CeremonyResult> {
    let not_found = || app::Error::Diesel(diesel::result::Error::NotFound);
    let return_code = return_code.ok_or(app::Error::Validation(t!("passkeyClaimNeedsCode")))?;
    let ceremony: Ceremony = ephemeral_token::get_token(state, CEREMONY_PREFIX, id, false)
        .await?
        .ok_or_else(not_found)?;
    let (Some(handoff), Some(verified), Some(expected_code)) =
        (&ceremony.handoff, &ceremony.outcome, &ceremony.return_code)
    else {
        return Err(not_found());
    };
    let verifier_matches = digest(code_verifier)
        .as_bytes()
        .ct_eq(handoff.code_challenge.as_bytes());
    let code_matches = digest(return_code)
        .as_bytes()
        .ct_eq(expected_code.as_bytes());
    if !bool::from(verifier_matches & code_matches) {
        return Err(app::Error::VerificationFailed);
    }
    ensure_starter(verified.starter(), caller)?;
    let taken: Ceremony = ephemeral_token::get_token(state, CEREMONY_PREFIX, id, true)
        .await?
        .ok_or_else(not_found)?;
    take_effect(state, taken.outcome.ok_or_else(not_found)?).await
}

/// Carries out what a verified ceremony was for.
async fn take_effect(
    state: &GlobalServerContext,
    verified: Verified,
) -> app::Result<CeremonyResult> {
    let mut conn = state.connection_pool.get().await?;
    Ok(match verified {
        Verified::SignedIn { user, ticket } => CeremonyResult::SignedIn(match ticket {
            Some(ticket) => {
                drop(conn);
                login::finish_ticket(state, &ticket, Some(user))
                    .await?
                    .ok_or(app::Error::InvalidTicket)?
            }
            None => {
                login::issue_session(state, &mut conn, user, login::SignInMethod::Passkey, false)
                    .await?
            }
        }),
        Verified::Registered {
            user: user_id,
            starter,
            name,
            key,
        } => {
            let (passkey, recovery_codes) = conn
                .transaction(|conn| {
                    async move {
                        let first = !two_factor::methods(conn, user_id).await?.any_factor();
                        let summary = PasskeySummary {
                            id: PasskeyId::new(),
                            name,
                            created_at: Utc::now(),
                            last_used_at: None,
                        };
                        diesel::insert_into(passkey::table)
                            .values((
                                passkey::id.eq(summary.id),
                                passkey::user.eq(user_id),
                                passkey::credential_id.eq(key.cred_id().as_ref()),
                                passkey::credential.eq(serde_json::to_value(&key)?),
                                passkey::name.eq(&summary.name),
                                passkey::created_at.eq(summary.created_at),
                            ))
                            .execute(conn)
                            .await?;
                        let codes =
                            two_factor::factor_added(state, conn, user_id, &starter, first).await?;
                        app::Result::Ok((summary, codes))
                    }
                    .scope_boxed()
                })
                .await?;
            CeremonyResult::PasskeyAdded {
                passkey,
                recovery_codes,
            }
        }
        Verified::Reauthenticated { user, starter } => CeremonyResult::Reauthenticated {
            verified_until: two_factor::mark_verified(
                &mut conn,
                user,
                &starter,
                &state.config.auth,
            )
            .await?,
        },
    })
}

/// A user's stored credentials. A deleted user has none.
async fn credentials(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
) -> app::Result<Vec<(PasskeyId, Passkey)>> {
    let rows: Vec<(PasskeyId, serde_json::Value)> = passkey::table
        .inner_join(user::table)
        .select((passkey::id, passkey::credential))
        .filter(passkey::user.eq(user_id))
        .filter(user::deleted_at.is_null())
        .load(conn)
        .await?;
    rows.into_iter()
        .map(|(id, credential)| Ok((id, serde_json::from_value(credential)?)))
        .collect()
}

/// Records that the credential `result` names was used, keeping its signature counter current
/// so a cloned authenticator can be told apart from the original.
async fn record_use(
    conn: &mut AsyncPgConnection,
    stored: Vec<(PasskeyId, Passkey)>,
    result: &AuthenticationResult,
) -> app::Result<()> {
    for (id, mut key) in stored {
        let Some(changed) = key.update_credential(result) else {
            continue;
        };
        let target = passkey::table.filter(passkey::id.eq(id));
        if changed {
            diesel::update(target)
                .set((
                    passkey::last_used_at.eq(diesel::dsl::now),
                    passkey::credential.eq(serde_json::to_value(&key)?),
                ))
                .execute(conn)
                .await?;
        } else {
            diesel::update(target)
                .set(passkey::last_used_at.eq(diesel::dsl::now))
                .execute(conn)
                .await?;
        }
    }
    Ok(())
}

/// The columns a `PasskeySummary` is read from.
type SummaryRow = (PasskeyId, String, DateTime<Utc>, Option<DateTime<Utc>>);

fn summary((id, name, created_at, last_used_at): SummaryRow) -> PasskeySummary {
    PasskeySummary {
        id,
        name,
        created_at,
        last_used_at,
    }
}

pub async fn list(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
) -> app::Result<Vec<PasskeySummary>> {
    let rows: Vec<SummaryRow> = passkey::table
        .select((
            passkey::id,
            passkey::name,
            passkey::created_at,
            passkey::last_used_at,
        ))
        .filter(passkey::user.eq(user_id))
        .order(passkey::created_at.asc())
        .load(conn)
        .await?;
    Ok(rows.into_iter().map(summary).collect())
}

/// Renames one of the caller's passkeys.
pub async fn rename(
    state: &GlobalServerContext,
    caller: &Caller,
    id: PasskeyId,
    name: Option<String>,
) -> app::Result<PasskeySummary> {
    let name = passkey_name(name)?;
    let mut conn = state.connection_pool.get().await?;
    let row = diesel::update(
        passkey::table
            .filter(passkey::id.eq(id))
            .filter(passkey::user.eq(caller.user)),
    )
    .set(passkey::name.eq(name))
    .returning((
        passkey::id,
        passkey::name,
        passkey::created_at,
        passkey::last_used_at,
    ))
    .get_result::<SummaryRow>(&mut conn)
    .await?;
    Ok(summary(row))
}

/// Removes one of the caller's passkeys.
pub async fn remove(
    state: &GlobalServerContext,
    caller: &Caller,
    id: PasskeyId,
) -> app::Result<()> {
    caller.ensure_recently_verified(&state.config.auth)?;
    let user_id = caller.user;
    let require = state.settings().require_two_factor;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let removed = diesel::delete(
                passkey::table
                    .filter(passkey::id.eq(id))
                    .filter(passkey::user.eq(user_id)),
            )
            .execute(conn)
            .await?;
            if removed == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            two_factor::factor_removed(conn, user_id, require).await
        }
        .scope_boxed()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn return_to_accepts_loopback_listeners_and_the_app_scheme_alone() {
        for ok in [
            "http://127.0.0.1:53817/passkey",
            "http://[::1]:53817/passkey",
            "http://localhost:53817/",
            "aspen://auth/passkey",
        ] {
            assert!(validate_return_to(ok).is_ok(), "{ok}");
        }
        for bad in [
            "http://127.0.0.1/passkey",
            "https://127.0.0.1:53817/passkey",
            "http://evil.example:53817/",
            "https://evil.example/",
            "javascript:alert(1)",
            "http://user@127.0.0.1:53817/",
            "aspen://auth/passkey#x",
            "not a url",
            "https://chat.example.org/login",
            "http://chat.example.org/login",
        ] {
            assert!(validate_return_to(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn code_challenges_are_sha256_digests() {
        let challenge = BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(b"verifier"));
        assert!(validate_code_challenge(&challenge).is_ok());
        assert!(validate_code_challenge("short").is_err());
        assert!(validate_code_challenge("not base64url!").is_err());
    }

    #[test]
    fn registration_options_require_a_discoverable_credential() {
        let mut options = serde_json::json!({
            "publicKey": {"authenticatorSelection": {"residentKey": "discouraged", "requireResidentKey": false}}
        });
        require_discoverable(&mut options);
        assert_eq!(
            options["publicKey"]["authenticatorSelection"]["residentKey"],
            "required"
        );
        assert_eq!(
            options["publicKey"]["authenticatorSelection"]["requireResidentKey"],
            true
        );
    }
}
