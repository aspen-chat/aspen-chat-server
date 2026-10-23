//! OpenID Connect Back-Channel Logout: a provider posts a logout token when someone signs out
//! there or is removed, and the sign-ins it made of them end. `openidconnect` verifies ID tokens
//! but not logout tokens, whose claims differ, so the token's signature is checked here against
//! the provider's keys and its claims as the specification lists them.

use super::{Provider, discovery, identity};
use crate::context::GlobalServerContext;
use crate::login::{self, ProviderSignIns};
use aspen_schema::identity_provider;
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use openidconnect::JsonWebKey as _;
use openidconnect::core::{CoreJsonWebKey, CoreJwsSigningAlgorithm};
use serde::Deserialize;

/// The event a logout token carries.
const LOGOUT_EVENT: &str = "http://schemas.openid.net/event/backchannel-logout";
/// How old a logout token may be when it arrives.
const MAX_AGE_SECONDS: i64 = 5 * 60;
/// How far in the future a token's `iat` may be, for clocks that differ.
const SKEW_SECONDS: i64 = 60;
/// How long a token's `jti` is remembered, so it is used once: past the age a token may have.
const USED_FOR_SECONDS: i64 = MAX_AGE_SECONDS + SKEW_SECONDS;
/// The longest logout token read.
pub const MAX_TOKEN_BYTES: usize = 16 * 1024;

#[derive(Deserialize)]
struct Header {
    alg: CoreJwsSigningAlgorithm,
    kid: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

impl Audience {
    fn contains(&self, client_id: &str) -> bool {
        match self {
            Audience::One(aud) => aud == client_id,
            Audience::Many(auds) => auds.iter().any(|aud| aud == client_id),
        }
    }
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: Audience,
    iat: i64,
    exp: Option<i64>,
    jti: String,
    events: serde_json::Map<String, serde_json::Value>,
    sub: Option<String>,
    sid: Option<String>,
    /// Present only in an ID token; a logout token that carries one is refused, so an ID token
    /// cannot be passed off as one.
    nonce: Option<serde_json::Value>,
}

/// Why a back-channel logout did not end anything.
#[derive(Debug, thiserror::Error)]
pub enum LogoutError {
    /// The token is refused, for the reason given, which goes only to the log: the provider is
    /// told no more than that the request was invalid.
    #[error("refused a logout token: {0}")]
    Refused(&'static str),
    #[error(transparent)]
    Server(#[from] crate::Error),
}

fn refused(reason: &'static str) -> LogoutError {
    LogoutError::Refused(reason)
}

impl From<diesel::result::Error> for LogoutError {
    fn from(error: diesel::result::Error) -> Self {
        Self::Server(error.into())
    }
}

fn decode<T: serde::de::DeserializeOwned>(part: &str) -> Result<T, LogoutError> {
    let bytes = BASE64_URL_SAFE_NO_PAD
        .decode(part)
        .map_err(|_| refused("the token is malformed"))?;
    serde_json::from_slice(&bytes).map_err(|_| refused("the token is malformed"))
}

/// Verifies `token` and ends the sign-ins it names; returns how many ended.
pub async fn back_channel_logout(
    state: &GlobalServerContext,
    token: &str,
) -> Result<usize, LogoutError> {
    if token.len() > MAX_TOKEN_BYTES {
        return Err(refused("the token is malformed"));
    }
    let mut parts = token.split('.');
    let (Some(header), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(refused("the token is malformed"));
    };
    let header: Header = decode(header)?;
    // Only the issuer is read before the signature is checked, to find whose keys check it.
    let claims: Claims = decode(payload)?;
    let signature = BASE64_URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| refused("the token is malformed"))?;
    let provider: Provider = {
        let mut conn = state
            .connection_pool
            .get()
            .await
            .map_err(crate::Error::from)?;
        identity_provider::table
            .select(Provider::as_select())
            .filter(identity_provider::issuer.eq(&claims.iss))
            .filter(identity_provider::removed_at.is_null())
            .first(&mut conn)
            .await
            .optional()?
            .ok_or_else(|| refused("its issuer is no identity provider of this server"))?
    };
    let signed = &token[..header_and_payload_len(token)];
    if !signature_matches(state, &provider, &header, signed, &signature, false).await?
        && !signature_matches(state, &provider, &header, signed, &signature, true).await?
    {
        return Err(refused(
            "its signature does not verify against the provider's keys",
        ));
    }
    let which = check_claims(&provider, &claims)?;
    remember_once(state, &provider, &claims.jti).await?;
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(crate::Error::from)?;
    let which = match which {
        Which::Session(sid) => {
            let user = match &claims.sub {
                Some(sub) => identity::account_of(&mut conn, provider.id, sub).await?,
                None => None,
            };
            ProviderSignIns::Session { sid, user }
        }
        Which::Subject(sub) => match identity::account_of(&mut conn, provider.id, &sub).await? {
            Some(user) => ProviderSignIns::User(user),
            None => return Ok(0),
        },
    };
    let provider_id = provider.id;
    Ok(conn
        .transaction(|conn| {
            async move { login::end_provider_sign_ins(state, conn, provider_id, which).await }
                .scope_boxed()
        })
        .await?)
}

fn header_and_payload_len(token: &str) -> usize {
    token.rfind('.').unwrap_or(0)
}

/// Whether one of the provider's keys, from its cached document or `fresh`ly read, signed
/// `signed` with `header`'s algorithm. Symmetric algorithms and `none` are never accepted.
async fn signature_matches(
    state: &GlobalServerContext,
    provider: &Provider,
    header: &Header,
    signed: &str,
    signature: &[u8],
    fresh: bool,
) -> crate::Result<bool> {
    use CoreJwsSigningAlgorithm as Alg;
    if matches!(
        header.alg,
        Alg::None | Alg::HmacSha256 | Alg::HmacSha384 | Alg::HmacSha512
    ) {
        return Ok(false);
    }
    let document = discovery::document(&state.config, &provider.issuer, fresh).await?;
    let keys: Vec<&CoreJsonWebKey> = document
        .jwks()
        .keys()
        .iter()
        .filter(|key| match &header.kid {
            Some(kid) => key.key_id().is_some_and(|id| id.as_str() == kid),
            None => true,
        })
        .collect();
    Ok(keys.iter().any(|key| {
        key.verify_signature(&header.alg, signed.as_bytes(), signature)
            .is_ok()
    }))
}

enum Which {
    Session(String),
    Subject(String),
}

/// Checks a logout token's claims as Back-Channel Logout 1.0 §2.6 lists them, once its signature
/// has verified.
fn check_claims(provider: &Provider, claims: &Claims) -> Result<Which, LogoutError> {
    let now = chrono::Utc::now().timestamp();
    if claims.iss != provider.issuer || !claims.aud.contains(&provider.client_id) {
        return Err(refused("it is not for this server's client"));
    }
    if claims.iat < now - MAX_AGE_SECONDS
        || claims.iat > now + SKEW_SECONDS
        || claims.exp.is_some_and(|exp| exp < now)
    {
        return Err(refused("it is too old, or from the future"));
    }
    if !claims.events.contains_key(LOGOUT_EVENT) || claims.nonce.is_some() {
        return Err(refused("it is not a logout token"));
    }
    if claims.jti.is_empty() || claims.jti.len() > 255 {
        return Err(refused("the token is malformed"));
    }
    match (&claims.sid, &claims.sub) {
        (Some(sid), _) if !sid.is_empty() => Ok(Which::Session(sid.clone())),
        (_, Some(sub)) if !sub.is_empty() => Ok(Which::Subject(sub.clone())),
        _ => Err(refused("it names no session and no subject")),
    }
}

/// Remembers a token's `jti` until it could no longer be accepted, refusing one already seen.
async fn remember_once(
    state: &GlobalServerContext,
    provider: &Provider,
    jti: &str,
) -> Result<(), LogoutError> {
    use fred::interfaces::KeysInterface as _;
    let key =
        crate::ephemeral_token::token_key("auth:oidc:logout", &format!("{}:{jti}", provider.id.0));
    let first: Option<String> = state
        .valkey
        .set(
            key,
            "1",
            Some(fred::types::Expiration::EX(USED_FOR_SECONDS)),
            Some(fred::types::SetOptions::NX),
            false,
        )
        .await
        .map_err(crate::Error::from)?;
    if first.is_none() {
        return Err(refused("it was already used"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> Provider {
        Provider {
            id: crate::IdentityProviderId::new(),
            name: "Example".into(),
            issuer: "https://id.example.org".into(),
            client_id: "aspen".into(),
            client_secret: "secret".into(),
            enabled: true,
            skip_invite: false,
            enforces_two_factor: false,
            trust_email: true,
            created_at: chrono::Utc::now(),
            removed_at: None,
        }
    }

    fn claims(json: serde_json::Value) -> Claims {
        serde_json::from_value(json).unwrap()
    }

    fn valid() -> serde_json::Value {
        serde_json::json!({
            "iss": "https://id.example.org",
            "aud": ["aspen", "other"],
            "iat": chrono::Utc::now().timestamp(),
            "jti": "j-1",
            "events": {LOGOUT_EVENT: {}},
            "sid": "s-1",
        })
    }

    #[test]
    fn a_valid_token_names_its_session_before_its_subject() {
        let mut json = valid();
        json["sub"] = "u-1".into();
        assert!(matches!(
            check_claims(&provider(), &claims(json)),
            Ok(Which::Session(sid)) if sid == "s-1"
        ));
        let mut json = valid();
        json.as_object_mut().unwrap().remove("sid");
        json["sub"] = "u-1".into();
        json["aud"] = "aspen".into();
        assert!(matches!(
            check_claims(&provider(), &claims(json)),
            Ok(Which::Subject(sub)) if sub == "u-1"
        ));
    }

    #[test]
    fn tokens_that_are_not_logouts_for_us_are_refused() {
        for change in [
            ("iss", serde_json::json!("https://other.example.org")),
            ("aud", serde_json::json!("someone-else")),
            ("iat", serde_json::json!(0)),
            ("events", serde_json::json!({})),
            ("nonce", serde_json::json!("n")),
            ("jti", serde_json::json!("")),
        ] {
            let mut json = valid();
            json[change.0] = change.1;
            assert!(
                check_claims(&provider(), &claims(json)).is_err(),
                "{}",
                change.0
            );
        }
        let mut json = valid();
        json.as_object_mut().unwrap().remove("sid");
        assert!(check_claims(&provider(), &claims(json)).is_err());
    }
}
