//! Signing in one device from another through a QR code: a device link. One side is signed in
//! (the giver) and the other is not (the receiver); whichever starts the link shows its code, and
//! the other scans it. A computer that is not signed in starts a `request` and a signed-in phone
//! scans it; a signed-in computer starts an `offer` and a phone that is not signed in scans it.
//! Either way the giver confirms the receiver by its name, with a tap, before the receiver may
//! claim a sign-in of the giver's account.
//!
//! The link's id is its QR code's secret. It lasts `LIFETIME_SECONDS` unscanned, and one scan
//! uses it (`SCAN_PREFIX` is taken with `NX`, so of two devices scanning at once exactly one
//! wins); the other learns the code was used, which is how a person whose code was
//! photographed finds out. Only the receiver can claim the sign-in, with the verifier whose
//! SHA-256 it gave (`code_challenge`), so a giver approving a stranger's request, or a stranger
//! holding the code, gets no session out of it. The new sign-in proves what the giver's did
//! (`method`, `verified_at`), so it is no stronger than the giver's and no more recently
//! verified; and the giver's sign-in must still stand when it is claimed, so signing out
//! everywhere or changing the password between the tap and the claim stops it.

use crate::CHACHA_RNG;
use crate::UserId;
use crate::context::GlobalServerContext;
use crate::ephemeral_token::token_key;
use crate::login::{self, Session, SignInMethod};
use crate::t;
use crate::two_factor::Caller;
use aspen_schema::{refresh_token, user};
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use fred::interfaces::KeysInterface;
use fred::types::{Expiration, SetOptions};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

const LINK_PREFIX: &str = "auth:device-link";
const SCAN_PREFIX: &str = "auth:device-link-scan";
/// How long a code shows before it must be made again: long enough to pick up a phone and
/// open its scanner, short enough that a photograph of the screen is soon worthless.
pub const LIFETIME_SECONDS: i64 = 60;
/// How long the giver has to confirm once the code is scanned.
const CONFIRM_SECONDS: i64 = 60;
/// How long the receiver has to claim the sign-in once it is approved; it asks every couple of
/// seconds.
const CLAIM_SECONDS: i64 = 30;
/// The longest name a device may give itself, in characters.
pub const MAX_DEVICE_NAME_CHARS: usize = 64;

/// Which side started the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Started by the device that is not signed in; a signed-in phone scans it.
    Request,
    /// Started by a signed-in device; a phone that is not signed in scans it.
    Offer,
}

/// The device that will be signed in.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Receiver {
    name: String,
    /// The SHA-256 of the receiver's verifier, unpadded base64url, as PKCE's `S256`.
    code_challenge: String,
}

/// The signed-in side, whose account the receiver signs in to.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Giver {
    user: UserId,
    /// `login::sign_in_id` of the giver's sign-in, which must still stand at the claim.
    sign_in: String,
    method: SignInMethod,
    verified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Link {
    kind: Kind,
    receiver: Option<Receiver>,
    giver: Option<Giver>,
    scanned: bool,
    approved: bool,
}

/// A link just started.
pub struct Started {
    pub id: String,
    pub kind: Kind,
    pub expires_at: DateTime<Utc>,
}

/// What the scanning device learns.
pub struct Scanned {
    pub kind: Kind,
    /// For a request, the name the receiver gave itself, for the giver to confirm.
    pub device_name: Option<String>,
    /// For an offer, the account the receiver will be signed in to.
    pub user: Option<UserId>,
}

/// Where a link stands, as its giver or receiver asks.
pub enum Progress {
    /// Not scanned yet.
    Waiting,
    /// Scanned and waiting for the giver to confirm `device_name`.
    Scanned { device_name: String },
    /// Confirmed, and the receiver's sign-in is ready to claim.
    Approved,
}

fn new_id() -> String {
    BASE64_URL_SAFE_NO_PAD.encode(CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 32]>()))
}

fn device_name(given: Option<String>) -> crate::Result<String> {
    let name = given.map(|n| n.trim().to_string()).unwrap_or_default();
    if name.is_empty()
        || name.chars().count() > MAX_DEVICE_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(crate::Error::Validation(t!(
            "deviceNameInvalid",
            max = MAX_DEVICE_NAME_CHARS
        )));
    }
    Ok(name)
}

fn code_challenge(given: Option<String>) -> crate::Result<String> {
    let invalid = || crate::Error::Validation(t!("invalidCodeChallenge"));
    let challenge = given.ok_or_else(invalid)?;
    let decoded = BASE64_URL_SAFE_NO_PAD
        .decode(&challenge)
        .map_err(|_| invalid())?;
    if decoded.len() != 32 {
        return Err(invalid());
    }
    Ok(challenge)
}

/// Refuses a caller who cannot give a sign-in away: a bot, whose token is its sign-in, and a
/// user of another deployment, whose sign-ins their home makes.
fn giver(caller: &Caller) -> crate::Result<Giver> {
    if caller.bot {
        return Err(crate::Error::Forbidden(t!("deviceLinkBot")));
    }
    if caller.foreign {
        return Err(crate::Error::Forbidden(t!("deviceLinkForeign")));
    }
    Ok(Giver {
        user: caller.user,
        sign_in: caller.sign_in(),
        method: caller.method,
        verified_at: caller.verified_at,
    })
}

async fn read(state: &GlobalServerContext, id: &str) -> crate::Result<Link> {
    let raw: Option<String> = state.valkey.get(token_key(LINK_PREFIX, id)).await?;
    raw.map(|raw| serde_json::from_str(&raw).map_err(crate::Error::from))
        .transpose()?
        .ok_or(crate::Error::DeviceLinkExpired)
}

async fn write(
    state: &GlobalServerContext,
    id: &str,
    link: &Link,
    ttl_seconds: i64,
) -> crate::Result<()> {
    let _: () = state
        .valkey
        .set(
            token_key(LINK_PREFIX, id),
            serde_json::to_string(link)?,
            Some(Expiration::EX(ttl_seconds)),
            None,
            false,
        )
        .await?;
    Ok(())
}

/// Starts a link. With `caller` set it is an offer of the caller's account; without, a request
/// from a device naming itself `name`, which will claim with the verifier behind
/// `challenge`.
pub async fn start(
    state: &GlobalServerContext,
    caller: Option<&Caller>,
    name: Option<String>,
    challenge: Option<String>,
) -> crate::Result<Started> {
    let link = match caller {
        Some(caller) => Link {
            kind: Kind::Offer,
            receiver: None,
            giver: Some(giver(caller)?),
            scanned: false,
            approved: false,
        },
        None => Link {
            kind: Kind::Request,
            receiver: Some(Receiver {
                name: device_name(name)?,
                code_challenge: code_challenge(challenge)?,
            }),
            giver: None,
            scanned: false,
            approved: false,
        },
    };
    let id = new_id();
    write(state, &id, &link, LIFETIME_SECONDS).await?;
    Ok(Started {
        id,
        kind: link.kind,
        expires_at: Utc::now() + chrono::Duration::seconds(LIFETIME_SECONDS),
    })
}

/// Scans a link: for a request, by the signed-in `caller` who will give it their account; for
/// an offer, by a device that is not signed in, naming itself `name`, which will claim with the
/// verifier behind `challenge`. A link scans once.
pub async fn scan(
    state: &GlobalServerContext,
    id: &str,
    caller: Option<&Caller>,
    name: Option<String>,
    challenge: Option<String>,
) -> crate::Result<Scanned> {
    let mut link = read(state, id).await?;
    // Checked before the scan is taken, so a request scanned by someone signed out (or an
    // offer by a malformed client) does not use the code up.
    let (giver, receiver) = match link.kind {
        Kind::Request => {
            let caller = caller.ok_or(crate::Error::Unauthenticated)?;
            (Some(giver(caller)?), None)
        }
        Kind::Offer if caller.is_some() => {
            return Err(crate::Error::Validation(t!("deviceLinkAlreadySignedIn")));
        }
        Kind::Offer => (
            None,
            Some(Receiver {
                name: device_name(name)?,
                code_challenge: code_challenge(challenge)?,
            }),
        ),
    };
    let taken: Option<String> = state
        .valkey
        .set(
            token_key(SCAN_PREFIX, id),
            "1",
            Some(Expiration::EX(LIFETIME_SECONDS + CONFIRM_SECONDS)),
            Some(SetOptions::NX),
            false,
        )
        .await?;
    if taken.is_none() {
        return Err(crate::Error::DeviceLinkUsed);
    }
    link.scanned = true;
    if let Some(giver) = giver {
        link.giver = Some(giver);
    }
    if let Some(receiver) = receiver {
        link.receiver = Some(receiver);
    }
    write(state, id, &link, CONFIRM_SECONDS).await?;
    Ok(Scanned {
        kind: link.kind,
        device_name: match link.kind {
            Kind::Request => link.receiver.map(|r| r.name),
            Kind::Offer => None,
        },
        user: match link.kind {
            Kind::Request => None,
            Kind::Offer => link.giver.map(|g| g.user),
        },
    })
}

/// Refuses anyone but the link's giver, while they hold the same sign-in that gave it.
fn ensure_giver(link: &Link, caller: &Caller) -> crate::Result<()> {
    match &link.giver {
        Some(giver) if giver.user == caller.user && giver.sign_in == caller.sign_in() => Ok(()),
        // Someone else's link reads as gone, as an unknown one does.
        _ => Err(crate::Error::DeviceLinkExpired),
    }
}

/// Where the link stands, for its giver.
pub async fn progress(
    state: &GlobalServerContext,
    id: &str,
    caller: &Caller,
) -> crate::Result<Progress> {
    let link = read(state, id).await?;
    ensure_giver(&link, caller)?;
    Ok(progress_of(&link))
}

fn progress_of(link: &Link) -> Progress {
    match (&link.receiver, link.scanned, link.approved) {
        (_, _, true) => Progress::Approved,
        (Some(receiver), true, false) => Progress::Scanned {
            device_name: receiver.name.clone(),
        },
        _ => Progress::Waiting,
    }
}

/// The giver confirms the receiver, letting it claim a sign-in. Approving twice is the same as
/// once.
pub async fn approve(state: &GlobalServerContext, id: &str, caller: &Caller) -> crate::Result<()> {
    let mut link = read(state, id).await?;
    ensure_giver(&link, caller)?;
    if !link.scanned {
        return Err(crate::Error::Conflict(t!("deviceLinkNotScanned")));
    }
    if !link.approved {
        link.approved = true;
        write(state, id, &link, CLAIM_SECONDS).await?;
    }
    Ok(())
}

/// Ends a link before it is claimed: the giver declining, or either device giving up. Holding
/// the code is enough, since it ends nothing but the link.
pub async fn cancel(state: &GlobalServerContext, id: &str) -> crate::Result<()> {
    let _: i64 = state.valkey.del(token_key(LINK_PREFIX, id)).await?;
    Ok(())
}

/// What the receiver's claim finds.
pub enum Claim {
    Pending(Progress),
    SignedIn(Session),
}

/// The receiver asks for its sign-in with the verifier whose SHA-256 it gave: the progress
/// until the giver approves, then the sign-in, once.
pub async fn claim(
    state: &GlobalServerContext,
    id: &str,
    code_verifier: &str,
) -> crate::Result<Claim> {
    let link = read(state, id).await?;
    let Some(receiver) = &link.receiver else {
        // An offer nobody has scanned has no receiver to answer to yet.
        return Ok(Claim::Pending(Progress::Waiting));
    };
    let answered = BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()));
    if !bool::from(
        answered
            .as_bytes()
            .ct_eq(receiver.code_challenge.as_bytes()),
    ) {
        return Err(crate::Error::VerificationFailed);
    }
    if !link.approved {
        return Ok(Claim::Pending(progress_of(&link)));
    }
    let taken: Option<String> = state.valkey.getdel(token_key(LINK_PREFIX, id)).await?;
    let link: Link = taken
        .map(|raw| serde_json::from_str(&raw))
        .transpose()?
        .ok_or(crate::Error::DeviceLinkExpired)?;
    let giver = link.giver.ok_or(crate::Error::DeviceLinkExpired)?;
    let mut conn = state.connection_pool.get().await?;
    if !sign_in_stands(conn.as_mut(), &giver).await? {
        return Err(crate::Error::DeviceLinkExpired);
    }
    Ok(Claim::SignedIn(
        login::issue_linked_session(
            state,
            &mut conn,
            giver.user,
            giver.method,
            giver.verified_at,
        )
        .await?,
    ))
}

/// Whether the giver's sign-in is still live and its account not deleted.
async fn sign_in_stands(
    conn: &mut diesel_async::AsyncPgConnection,
    giver: &Giver,
) -> crate::Result<bool> {
    let live: i64 = refresh_token::table
        .inner_join(user::table)
        .filter(refresh_token::user.eq(giver.user))
        .filter(user::deleted_at.is_null())
        .filter(refresh_token::expires.gt(Utc::now().naive_utc()))
        .filter(
            diesel::dsl::sql::<diesel::sql_types::Bool>(login::SIGN_IN_ID_IS_SQL)
                .bind::<diesel::sql_types::Text, _>(&giver.sign_in),
        )
        .count()
        .get_result(conn)
        .await?;
    Ok(live > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_names_are_bounded_and_printable() {
        assert_eq!(
            device_name(Some("  Firefox on Linux ".into())).unwrap(),
            "Firefox on Linux"
        );
        assert!(device_name(None).is_err());
        assert!(device_name(Some("   ".into())).is_err());
        assert!(device_name(Some("a\u{7}b".into())).is_err());
        assert!(device_name(Some("é".repeat(MAX_DEVICE_NAME_CHARS))).is_ok());
        assert!(device_name(Some("é".repeat(MAX_DEVICE_NAME_CHARS + 1))).is_err());
    }

    #[test]
    fn code_challenges_are_sha256_digests() {
        let digest = BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(b"verifier"));
        assert_eq!(code_challenge(Some(digest.clone())).unwrap(), digest);
        assert!(code_challenge(None).is_err());
        assert!(code_challenge(Some("short".into())).is_err());
        assert!(code_challenge(Some(BASE64_URL_SAFE_NO_PAD.encode([0u8; 16]))).is_err());
    }

    #[test]
    fn progress_follows_the_scan_and_the_approval() {
        let receiver = Receiver {
            name: "Pixel 8".into(),
            code_challenge: String::new(),
        };
        let mut link = Link {
            kind: Kind::Offer,
            receiver: None,
            giver: None,
            scanned: false,
            approved: false,
        };
        assert!(matches!(progress_of(&link), Progress::Waiting));
        link.receiver = Some(receiver);
        link.scanned = true;
        assert!(
            matches!(progress_of(&link), Progress::Scanned { device_name } if device_name == "Pixel 8")
        );
        link.approved = true;
        assert!(matches!(progress_of(&link), Progress::Approved));
    }
}
