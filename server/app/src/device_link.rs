//! Signing in one device from another: a device link. One side is signed in (the giver) and the
//! other is not (the receiver); whichever starts the link shows its QR code, and the other scans
//! it. A computer that is not signed in starts a `request` and a signed-in phone scans it; a
//! signed-in computer starts an `offer` and a phone that is not signed in scans it. Either way
//! the giver confirms the receiver by its name, with a tap, before the receiver may claim a
//! sign-in of the giver's account.
//!
//! The link's id is its QR code's secret. It lasts `LIFETIME_SECONDS` unscanned, and one scan
//! uses it (`SCAN_PREFIX` is taken with `NX`, so of two devices scanning at once exactly one
//! wins); the other learns the code was used, which is how a person whose code was
//! photographed finds out. Only the receiver can claim the sign-in, with the verifier whose
//! SHA-256 it gave (`code_challenge`), so a giver approving a stranger's request, or a stranger
//! holding the code, gets no session out of it. The new sign-in proves what the giver's did
//! (`method`, `verified_at`), so it is no stronger than the giver's and no more recently
//! verified; and the giver's sign-in must still stand when it is claimed, so signing out
//! everywhere or changing the password between the tap and the claim stops it. Giving a sign-in
//! (starting an offer, scanning a request) takes a recently verified sign-in.
//!
//! A link started `with_code` also has a user code, as OAuth's device authorization grant has
//! (RFC 8628): `CODE_LENGTH` letters of `CODE_ALPHABET`, which the other device may type instead
//! of scanning, and which lasts `CODE_LIFETIME_SECONDS`. Typing it ([`enter`]) takes the link as
//! a scan does. Unlike the id, a code can be guessed, so a code that matches nothing is a
//! failure the API counts (`api::rate_limit::Failures`), every way an entry can miss answers
//! alike (`DeviceLinkCodeUnknown`), and the receiver of a request taken by a typed code must
//! confirm the giver's account before its claim succeeds (`Claim::ConfirmAccount`), since a
//! giver who guessed a stranger's code would otherwise sign the stranger in to the guesser's
//! account.

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
use unicode_normalization::UnicodeNormalization;

const LINK_PREFIX: &str = "auth:device-link";
const SCAN_PREFIX: &str = "auth:device-link-scan";
/// A user code's link id, under the code's digest.
const CODE_PREFIX: &str = "auth:device-link-code";
/// How long a code shows before it must be made again: long enough to pick up a phone and
/// open its scanner, short enough that a photograph of the screen is soon worthless.
pub const LIFETIME_SECONDS: i64 = 60;
/// How long a link with a user code lasts unscanned: long enough to read eight letters off one
/// screen and type them on another, with a television's remote or after the giver confirms it's
/// them again (`reauthenticationRequired`, a password and a second factor). The QR code beside
/// it lasts as long, since a photograph of the screen shows the user code too.
pub const CODE_LIFETIME_SECONDS: i64 = 300;
/// How long the giver has to confirm once the code is scanned.
const CONFIRM_SECONDS: i64 = 60;
/// How long the receiver has to claim the sign-in once it is approved; it asks every couple of
/// seconds.
const CLAIM_SECONDS: i64 = 30;
/// How long the receiver of a request taken by a typed code has, once the giver approves, to
/// confirm the giver's account and claim the sign-in: a person reads a name and taps.
const ACCEPT_SECONDS: i64 = 60;
/// How long a scan holds the link taken: past the longest the link lives once scanned (the
/// giver's confirmation, then the receiver's), so no second scan can take it.
const TAKEN_SECONDS: i64 = CONFIRM_SECONDS + ACCEPT_SECONDS + CLAIM_SECONDS;
/// The longest name a device may give itself, in characters.
pub const MAX_DEVICE_NAME_CHARS: usize = 64;
/// The letters of a user code: RFC 8628's base-20 alphabet, the consonants but `Y`. Without
/// vowels a code spells no word, and has no `O` or `I` to mistake for a digit.
pub const CODE_ALPHABET: &[u8; 20] = b"BCDFGHJKLMNPQRSTVWXZ";
/// Letters in a user code: 20^8, about 34.5 bits, shown as two groups of four.
pub const CODE_LENGTH: usize = 8;
/// How many codes are drawn before giving up on one no live link holds. Even with a hundred
/// thousand links live, a draw collides once in a quarter of a million.
const CODE_DRAWS: usize = 8;

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
    /// Taken by a typed user code rather than by scanning the QR code.
    #[serde(default)]
    entered: bool,
}

impl Link {
    /// Whether the receiver must confirm the giver's account before claiming: a request whose
    /// giver typed its code, and so may have guessed it.
    fn needs_acceptance(&self) -> bool {
        self.entered && self.kind == Kind::Request
    }
}

/// A link just started.
pub struct Started {
    pub id: String,
    pub kind: Kind,
    pub expires_at: DateTime<Utc>,
    /// The user code, as shown (`XXXX-XXXX`), when one was asked for.
    pub code: Option<String>,
}

/// What the scanning device learns.
pub struct Scanned {
    pub kind: Kind,
    /// For a request, the name the receiver gave itself, for the giver to confirm.
    pub device_name: Option<String>,
    /// For an offer, the account the receiver will be signed in to.
    pub user: Option<UserId>,
}

/// A link taken by its typed user code.
pub struct Entered {
    pub id: String,
    pub scanned: Scanned,
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

/// Which side of a link a device scanning or typing its code is.
enum Side {
    Giver(Giver),
    Receiver(Receiver),
}

fn new_id() -> String {
    BASE64_URL_SAFE_NO_PAD.encode(CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 32]>()))
}

/// A new user code, as it is kept: `CODE_LENGTH` letters of `CODE_ALPHABET`.
fn new_code() -> String {
    CHACHA_RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        (0..CODE_LENGTH)
            .map(|_| char::from(CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())]))
            .collect()
    })
}

/// A typed user code as it is kept: in compatibility form (NFKC, so the full-width letters an
/// input method may type read as the letters they are), hyphens and whitespace dropped, in
/// capitals. `None` for anything that cannot be a code at all.
fn normalize_code(typed: &str) -> Option<String> {
    let code: String = typed
        .nfkc()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    (code.len() == CODE_LENGTH && code.bytes().all(|b| CODE_ALPHABET.contains(&b))).then_some(code)
}

/// A code as people read it, in two groups of four: `BCDF-GHJK`.
fn shown_code(code: &str) -> String {
    let (first, second) = code.split_at(CODE_LENGTH / 2);
    format!("{first}-{second}")
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

fn receiver(name: Option<String>, challenge: Option<String>) -> crate::Result<Receiver> {
    Ok(Receiver {
        name: device_name(name)?,
        code_challenge: code_challenge(challenge)?,
    })
}

/// Refuses a caller who cannot give a sign-in away: a bot, whose token is its sign-in, a user
/// of another deployment, whose sign-ins their home makes, and a sign-in not verified recently
/// (`reauthenticationRequired`), since giving a sign-in away is a security change: a stolen
/// session must not mint another, longer-lived one on a device of the thief's.
fn giver(state: &GlobalServerContext, caller: &Caller) -> crate::Result<Giver> {
    if caller.bot {
        return Err(crate::Error::Forbidden(t!("deviceLinkBot")));
    }
    if caller.foreign {
        return Err(crate::Error::Forbidden(t!("deviceLinkForeign")));
    }
    caller.ensure_recently_verified(&state.config.auth)?;
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

/// Writes a link back after a step on it, only while it still exists (`XX`), so a step racing
/// a `cancel` that removed it between its read and this write cannot bring it back.
async fn rewrite(
    state: &GlobalServerContext,
    id: &str,
    link: &Link,
    ttl_seconds: i64,
) -> crate::Result<()> {
    let written: Option<String> = state
        .valkey
        .set(
            token_key(LINK_PREFIX, id),
            serde_json::to_string(link)?,
            Some(Expiration::EX(ttl_seconds)),
            Some(SetOptions::XX),
            false,
        )
        .await?;
    written.map(|_| ()).ok_or(crate::Error::DeviceLinkExpired)
}

/// Gives the link `id` a user code no live link holds (`NX`), for `ttl_seconds`: the code as
/// it is kept.
async fn reserve_code(
    state: &GlobalServerContext,
    id: &str,
    ttl_seconds: i64,
) -> crate::Result<String> {
    for _ in 0..CODE_DRAWS {
        let code = new_code();
        let reserved: Option<String> = state
            .valkey
            .set(
                token_key(CODE_PREFIX, &code),
                id,
                Some(Expiration::EX(ttl_seconds)),
                Some(SetOptions::NX),
                false,
            )
            .await?;
        if reserved.is_some() {
            return Ok(code);
        }
    }
    tracing::warn!("no free device link code in {CODE_DRAWS} draws");
    Err(crate::Error::Busy)
}

/// Starts a link. With `caller` set it is an offer of the caller's account; without, a request
/// from a device naming itself `name`, which will claim with the verifier behind
/// `challenge`. `with_code` gives it a user code too.
pub async fn start(
    state: &GlobalServerContext,
    caller: Option<&Caller>,
    name: Option<String>,
    challenge: Option<String>,
    with_code: bool,
) -> crate::Result<Started> {
    let link = match caller {
        Some(caller) => Link {
            kind: Kind::Offer,
            receiver: None,
            giver: Some(giver(state, caller)?),
            scanned: false,
            approved: false,
            entered: false,
        },
        None => Link {
            kind: Kind::Request,
            receiver: Some(receiver(name, challenge)?),
            giver: None,
            scanned: false,
            approved: false,
            entered: false,
        },
    };
    let lifetime = if with_code {
        CODE_LIFETIME_SECONDS
    } else {
        LIFETIME_SECONDS
    };
    let id = new_id();
    write(state, &id, &link, lifetime).await?;
    let code = if with_code {
        match reserve_code(state, &id, lifetime).await {
            Ok(code) => Some(shown_code(&code)),
            Err(e) => {
                let _: i64 = state.valkey.del(token_key(LINK_PREFIX, &id)).await?;
                return Err(e);
            }
        }
    } else {
        None
    };
    Ok(Started {
        id,
        kind: link.kind,
        expires_at: Utc::now() + chrono::Duration::seconds(lifetime),
        code,
    })
}

/// Takes a link for `side`, once: of two devices scanning or typing at once, exactly one gets
/// it, and the other `DeviceLinkUsed`.
async fn take(
    state: &GlobalServerContext,
    id: &str,
    mut link: Link,
    side: Side,
    entered: bool,
) -> crate::Result<Scanned> {
    let taken: Option<String> = state
        .valkey
        .set(
            token_key(SCAN_PREFIX, id),
            "1",
            Some(Expiration::EX(TAKEN_SECONDS)),
            Some(SetOptions::NX),
            false,
        )
        .await?;
    if taken.is_none() {
        return Err(crate::Error::DeviceLinkUsed);
    }
    link.scanned = true;
    link.entered = entered;
    match side {
        Side::Giver(giver) => link.giver = Some(giver),
        Side::Receiver(receiver) => link.receiver = Some(receiver),
    }
    rewrite(state, id, &link, CONFIRM_SECONDS).await?;
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
    let link = read(state, id).await?;
    // Checked before the scan is taken, so a request scanned by someone signed out (or an
    // offer by a malformed client) does not use the code up.
    let side = match (link.kind, caller) {
        (Kind::Request, Some(caller)) => Side::Giver(giver(state, caller)?),
        (Kind::Request, None) => return Err(crate::Error::Unauthenticated),
        (Kind::Offer, Some(_)) => {
            return Err(crate::Error::Validation(t!("deviceLinkAlreadySignedIn")));
        }
        (Kind::Offer, None) => Side::Receiver(receiver(name, challenge)?),
    };
    take(state, id, link, side, false).await
}

/// Takes a link by its typed user code: a request, by the signed-in `caller` who will give it
/// their account; an offer, by a device that is not signed in, naming itself `name`, which will
/// claim with the verifier behind `challenge`. Whatever the caller lacks is refused before the
/// code is looked at, so a refusal says nothing of it. A code that is unknown, expired,
/// cancelled, or of a link the caller cannot take (an offer typed by someone signed in, a
/// request by someone signed out) is `DeviceLinkCodeUnknown` alike, which the API counts as a
/// failure: a guess learns nothing of other links. A code whose link is already taken is
/// `DeviceLinkUsed`, as a second scan is.
pub async fn enter(
    state: &GlobalServerContext,
    typed: &str,
    caller: Option<&Caller>,
    name: Option<String>,
    challenge: Option<String>,
) -> crate::Result<Entered> {
    let side = match caller {
        Some(caller) => Side::Giver(giver(state, caller)?),
        None => Side::Receiver(receiver(name, challenge)?),
    };
    let code = normalize_code(typed).ok_or_else(|| {
        crate::Error::Validation(t!("deviceLinkCodeMalformed", length = CODE_LENGTH))
    })?;
    let id: Option<String> = state.valkey.get(token_key(CODE_PREFIX, &code)).await?;
    let id = id.ok_or(crate::Error::DeviceLinkCodeUnknown)?;
    let link = match read(state, &id).await {
        Err(crate::Error::DeviceLinkExpired) => return Err(crate::Error::DeviceLinkCodeUnknown),
        read => read?,
    };
    let fits = match (&side, link.kind) {
        (Side::Giver(_), Kind::Request) | (Side::Receiver(_), Kind::Offer) => true,
        (Side::Giver(_), Kind::Offer) | (Side::Receiver(_), Kind::Request) => false,
    };
    if !fits {
        return Err(crate::Error::DeviceLinkCodeUnknown);
    }
    let scanned = take(state, &id, link, side, true).await?;
    Ok(Entered { id, scanned })
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
        let ttl = if link.needs_acceptance() {
            ACCEPT_SECONDS
        } else {
            CLAIM_SECONDS
        };
        rewrite(state, id, &link, ttl).await?;
    }
    Ok(())
}

/// Ends a link before it is claimed: the giver declining, or either device giving up. Holding
/// the code is enough, since it ends nothing but the link.
/// A scan or an approval under way when it ends finds it gone (`rewrite`) rather than reviving it.
/// Its user code, if it has one, then matches no link, and is kept until it expires so that no
/// other link is given it meanwhile.
pub async fn cancel(state: &GlobalServerContext, id: &str) -> crate::Result<()> {
    let _: i64 = state.valkey.del(token_key(LINK_PREFIX, id)).await?;
    Ok(())
}

/// What the receiver's claim finds.
pub enum Claim {
    Pending(Progress),
    /// The link was taken by a typed code, so its giver may have guessed it: the receiver
    /// confirms that `UserId` is the account it means to sign in to by claiming again with it.
    ConfirmAccount(UserId),
    SignedIn(Session),
}

/// The receiver asks for its sign-in with the verifier whose SHA-256 it gave: the progress
/// until the giver approves, then the sign-in, once. A request taken by a typed code also asks
/// the receiver to confirm the giver's account, which it does by naming it as `account`.
pub async fn claim(
    state: &GlobalServerContext,
    id: &str,
    code_verifier: &str,
    account: Option<UserId>,
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
    if let Some(answer) = before_taking(&link, account) {
        return Ok(answer);
    }
    let taken: Option<String> = state.valkey.getdel(token_key(LINK_PREFIX, id)).await?;
    let link: Link = taken
        .map(|raw| serde_json::from_str(&raw))
        .transpose()?
        .ok_or(crate::Error::DeviceLinkExpired)?;
    if unaccepted_giver(&link, account).is_some() {
        return Err(crate::Error::DeviceLinkExpired);
    }
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
            &giver.sign_in,
        )
        .await?,
    ))
}

/// What the receiver's claim answers while the link cannot yet be taken: its progress until the
/// giver approves, and only then, for a link whose giver the receiver must confirm, the giver's
/// account. Whoever typed a request's code learns nothing of it, and the receiver nothing of who
/// typed it, before that person has approved the receiver by its name.
fn before_taking(link: &Link, account: Option<UserId>) -> Option<Claim> {
    if !link.approved {
        return Some(Claim::Pending(progress_of(link)));
    }
    unaccepted_giver(link, account).map(Claim::ConfirmAccount)
}

/// The giver's account, when the receiver must confirm it and `account` does not name it.
fn unaccepted_giver(link: &Link, account: Option<UserId>) -> Option<UserId> {
    let giver = link.giver.as_ref()?;
    (link.needs_acceptance() && account != Some(giver.user)).then_some(giver.user)
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
            entered: false,
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

    #[test]
    fn user_codes_are_drawn_from_the_alphabet_and_shown_in_two_groups() {
        let code = new_code();
        assert_eq!(code.len(), CODE_LENGTH);
        assert!(code.bytes().all(|b| CODE_ALPHABET.contains(&b)));
        let shown = shown_code(&code);
        assert_eq!(shown.len(), CODE_LENGTH + 1);
        assert_eq!(&shown[4..5], "-");
        assert_eq!(normalize_code(&shown).as_deref(), Some(code.as_str()));
    }

    #[test]
    fn typed_codes_ignore_case_hyphens_and_whitespace() {
        assert_eq!(normalize_code("bcdf-ghjk").as_deref(), Some("BCDFGHJK"));
        assert_eq!(normalize_code(" BcDf GhJk\n").as_deref(), Some("BCDFGHJK"));
        assert_eq!(
            normalize_code("B-C-D-F-G-H-J-K").as_deref(),
            Some("BCDFGHJK")
        );
        assert!(normalize_code("BCDF-GHJ").is_none());
        assert!(normalize_code("BCDF-GHJKL").is_none());
        // Vowels, `Y`, and digits are not in the alphabet.
        assert!(normalize_code("ABCD-FGHJ").is_none());
        assert!(normalize_code("YBCD-FGHJ").is_none());
        assert!(normalize_code("0BCD-FGHJ").is_none());
        // Full-width letters and hyphens, as an input method may type them.
        assert_eq!(
            normalize_code("\u{FF22}CDF\u{FF0D}GHJK").as_deref(),
            Some("BCDFGHJK")
        );
        // Letters only like the alphabet's are not in it.
        assert!(normalize_code("\u{0392}CDF-GHJK").is_none());
    }

    #[test]
    fn a_request_taken_by_a_typed_code_waits_for_its_receiver_to_confirm_the_giver() {
        let giver = UserId(uuid::Uuid::now_v7());
        let mut link = Link {
            kind: Kind::Request,
            receiver: None,
            giver: Some(Giver {
                user: giver,
                sign_in: String::new(),
                method: SignInMethod::Password,
                verified_at: Utc::now(),
            }),
            scanned: true,
            approved: true,
            entered: false,
        };
        assert_eq!(unaccepted_giver(&link, None), None, "scanned, not typed");
        link.entered = true;
        assert_eq!(unaccepted_giver(&link, None), Some(giver));
        let stranger = UserId(uuid::Uuid::now_v7());
        assert_eq!(unaccepted_giver(&link, Some(stranger)), Some(giver));
        assert_eq!(unaccepted_giver(&link, Some(giver)), None);
        link.approved = false;
        assert!(
            matches!(before_taking(&link, None), Some(Claim::Pending(_))),
            "the giver's account is not shown before they approve"
        );
        link.approved = true;
        assert!(matches!(
            before_taking(&link, None),
            Some(Claim::ConfirmAccount(shown)) if shown == giver
        ));
        assert!(before_taking(&link, Some(giver)).is_none());
        link.kind = Kind::Offer;
        assert_eq!(
            unaccepted_giver(&link, None),
            None,
            "an offer's receiver typed the code itself"
        );
    }
}
