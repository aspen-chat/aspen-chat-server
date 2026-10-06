//! Second factors and the rules around them.
//!
//! An account has two-factor sign-in on while it holds at least one second factor: a confirmed
//! authenticator app (TOTP, RFC 6238) or a passkey. While it is on, a password alone never
//! completes a sign-in, and the account holds single-use recovery codes that stand in for a lost
//! factor. The codes are made when the first factor is added, together with signing out every
//! other session, and dropped when the last one is removed.
//!
//! Changes to security settings need the session to have proved who its user is recently
//! (`Caller::ensure_recently_verified`): at sign-in, or again through `reauthenticate`. With
//! two-factor on, re-verifying takes a second factor, so a stolen session token cannot turn it
//! off. Wrong codes are counted per user (`MAX_FAILURES` within `FAILURE_WINDOW_SECONDS`), which
//! bounds guessing however many sign-in attempts an attacker who knows the password starts.

use crate::CHACHA_RNG;
use crate::app::context::GlobalServerContext;
use crate::app::deployment_settings::DeploymentSettings;
use crate::app::{self, UserId};
use crate::aspen_config::AuthConfig;
use crate::database::schema::{passkey, recovery_code, refresh_token, totp_secret, user};
use crate::t;
use chrono::{DateTime, Duration, Utc};
use data_encoding::BASE32_NOPAD;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use fred::interfaces::{KeysInterface, LuaInterface};
use hmac::{Hmac, Mac};
use rand::RngExt;
use sha1::Sha1;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// How many recovery codes an account holds.
pub const RECOVERY_CODE_COUNT: usize = 10;
/// Wrong codes or passwords a user may enter within `FAILURE_WINDOW_SECONDS`.
const MAX_FAILURES: i64 = 10;
const FAILURE_WINDOW_SECONDS: i64 = 15 * 60;

/// RFC 6238 parameters every authenticator app supports: HMAC-SHA1, six digits, 30 seconds.
const TOTP_STEP_SECONDS: i64 = 30;
const TOTP_DIGITS: u32 = 6;
/// 160 bits, the HMAC-SHA1 block size RFC 4226 recommends.
const TOTP_SECRET_BYTES: usize = 20;

/// Crockford's base32 alphabet, lowercase: no `i`, `l`, `o`, or `u`, so a code read aloud or
/// copied by hand is hard to get wrong.
const RECOVERY_ALPHABET: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";
/// Ten symbols, 50 bits, shown as two groups of five.
const RECOVERY_CODE_LENGTH: usize = 10;

/// The authenticated session a request arrived with, as the sign-in rules see it.
#[derive(Clone, Debug)]
pub struct Caller {
    pub user: UserId,
    pub session_token: String,
    /// The refresh token the session was issued from; it stands for the sign-in.
    pub refresh_token: String,
    /// When the sign-in last proved who its user is.
    pub verified_at: DateTime<Utc>,
    pub has_second_factor: bool,
    /// Whether the caller is a bot (`app::bot`). A bot has no password or second factor, and
    /// changes no security settings.
    pub bot: bool,
    /// How the sign-in proved who its user is.
    pub method: app::login::SignInMethod,
    /// Whether the caller is a user of another deployment, whose sign-in and its security are
    /// their home's (`app::federation::abroad`).
    pub foreign: bool,
    /// Whether the account has an email address it has not verified (`app::email`).
    pub email_unverified: bool,
}

impl Caller {
    /// Until when the session counts as recently verified.
    pub fn verified_until(&self, config: &AuthConfig) -> DateTime<Utc> {
        self.verified_at + reverify_window(config)
    }

    /// Refuses a bot what only a person's sign-in has: a password, second factors, and the
    /// verifications they give.
    pub fn ensure_person(&self) -> app::Result<()> {
        if self.bot {
            Err(app::Error::Forbidden(t!("botNoSignInSecurity")))
        } else if self.foreign {
            Err(app::Error::Forbidden(t!("foreignNoSignInSecurity")))
        } else {
            Ok(())
        }
    }

    pub fn ensure_recently_verified(&self, config: &AuthConfig) -> app::Result<()> {
        self.ensure_person()?;
        if Utc::now() < self.verified_until(config) {
            Ok(())
        } else {
            Err(app::Error::ReauthenticationRequired)
        }
    }

    /// Whether `settings` require a second factor this account does not have yet. Such a
    /// session may only add one, or sign out.
    pub fn enrollment_required(&self, settings: &DeploymentSettings) -> bool {
        settings.require_two_factor && !self.bot && !self.foreign && !self.has_second_factor
    }

    /// Whether `settings` require a verified email address and this account's is not. Such a
    /// session may only verify, change, or resend it, or sign out.
    pub fn verification_required(&self, settings: &DeploymentSettings) -> bool {
        settings.email_verification_required && self.email_unverified
    }
}

fn reverify_window(config: &AuthConfig) -> Duration {
    Duration::seconds(i64::try_from(config.reverify_seconds).unwrap_or(i64::MAX / 1000))
}

/// Which second factors an account can use to finish signing in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SecondFactorMethods {
    pub totp: bool,
    pub passkey: bool,
    pub recovery_code: bool,
}

impl SecondFactorMethods {
    pub fn any_factor(self) -> bool {
        self.totp || self.passkey
    }
}

/// A second factor presented as text.
#[derive(Clone, Debug)]
pub enum SecondFactor {
    Totp(String),
    RecoveryCode(String),
}

pub async fn methods(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
) -> app::Result<SecondFactorMethods> {
    let totp = diesel::select(diesel::dsl::exists(
        totp_secret::table
            .filter(totp_secret::user.eq(user_id))
            .filter(totp_secret::confirmed_at.is_not_null()),
    ))
    .get_result::<bool>(conn)
    .await?;
    let passkey = diesel::select(diesel::dsl::exists(
        passkey::table.filter(passkey::user.eq(user_id)),
    ))
    .get_result::<bool>(conn)
    .await?;
    let recovery_code = diesel::select(diesel::dsl::exists(
        recovery_code::table
            .filter(recovery_code::user.eq(user_id))
            .filter(recovery_code::used_at.is_null()),
    ))
    .get_result::<bool>(conn)
    .await?;
    Ok(SecondFactorMethods {
        totp,
        passkey,
        recovery_code,
    })
}

/// The SQL condition that the `user` row in a query has an email address it has not verified,
/// answered in the per-request session lookup as [`HAS_SECOND_FACTOR_SQL`] is.
pub const EMAIL_UNVERIFIED_SQL: &str = "EXISTS (SELECT 1 FROM user_email e \
     WHERE e.\"user\" = \"user\".id AND e.verified_at IS NULL)";

/// SQL that is true when the row of `"user"` in scope holds a second factor. Kept as one
/// fragment so the per-request session lookup answers it in the same query.
pub const HAS_SECOND_FACTOR_SQL: &str = "(EXISTS (SELECT 1 FROM totp_secret t \
     WHERE t.\"user\" = \"user\".id AND t.confirmed_at IS NOT NULL) \
     OR EXISTS (SELECT 1 FROM passkey p WHERE p.\"user\" = \"user\".id))";

// ---------------------------------------------------------------------------------------------
// Failure counting
// ---------------------------------------------------------------------------------------------

fn failures_key(user_id: UserId) -> String {
    format!("auth:failures:{}", user_id.0)
}

/// Counts one attempt and answers how many the window holds now, starting the window with the
/// first. One script, so the count and its expiry are set together and a lost connection never
/// leaves a count that does not expire.
const COUNT_ATTEMPT: &str = "
local count = redis.call('INCR', KEYS[1])
if count == 1 then
  redis.call('EXPIRE', KEYS[1], ARGV[1])
end
return count
";

/// Takes back an attempt `COUNT_ATTEMPT` counted, unless its window has ended meanwhile (which
/// would leave a count without an expiry).
const UNCOUNT_ATTEMPT: &str = "
if redis.call('EXISTS', KEYS[1]) == 1 then
  redis.call('DECR', KEYS[1])
end
return 0
";

/// Runs `check`, which answers whether a presented secret was right, under the failure limit:
/// refused outright once the limit is reached, a wrong answer counted, a right one clearing the
/// count.
pub async fn limited<F>(state: &GlobalServerContext, user_id: UserId, check: F) -> app::Result<bool>
where
    F: AsyncFnOnce() -> app::Result<bool>,
{
    limited_in(&state.valkey, &failures_key(user_id), check).await
}

/// `limited` on the count at `key`. The attempt is counted before `check` runs, so however many
/// arrive at once, no more than `MAX_FAILURES` are checked in a window without one succeeding.
/// A right answer clears the count; an attempt that could not be checked (an error, not a wrong
/// answer) is taken back off it.
async fn limited_in<F>(valkey: &fred::clients::Client, key: &str, check: F) -> app::Result<bool>
where
    F: AsyncFnOnce() -> app::Result<bool>,
{
    let attempts: i64 = valkey
        .eval(COUNT_ATTEMPT, key.to_string(), FAILURE_WINDOW_SECONDS)
        .await?;
    if attempts > MAX_FAILURES {
        return Err(app::Error::TooManyAttempts);
    }
    match check().await {
        Ok(true) => {
            let _: i64 = valkey.del(key).await?;
            Ok(true)
        }
        Ok(false) => Ok(false),
        Err(e) => {
            let _: i64 = valkey
                .eval(UNCOUNT_ATTEMPT, key.to_string(), Vec::<String>::new())
                .await?;
            Err(e)
        }
    }
}

// ---------------------------------------------------------------------------------------------
// TOTP
// ---------------------------------------------------------------------------------------------

/// The HOTP value (RFC 4226) of `secret` at counter `step`, as the zero-padded digits an
/// authenticator app shows.
fn hotp(secret: &[u8], step: u64) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC accepts keys of any length");
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = usize::from(digest[digest.len() - 1] & 0x0f);
    let binary = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    format!(
        "{:0width$}",
        binary % 10u32.pow(TOTP_DIGITS),
        width = TOTP_DIGITS as usize
    )
}

/// The time step a code shown at Unix time `unix_seconds` belongs to.
fn totp_step(unix_seconds: i64) -> i64 {
    unix_seconds.div_euclid(TOTP_STEP_SECONDS)
}

/// The step `code` is valid for, allowing one step of clock drift either way and refusing any
/// step at or before `last_used_step`, so a code is accepted once. Spaces in the code are
/// ignored, as apps often show it in two groups.
fn match_totp(
    secret: &[u8],
    code: &str,
    now_step: i64,
    last_used_step: Option<i64>,
) -> Option<i64> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if code.len() != TOTP_DIGITS as usize || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut found = None;
    // Every candidate is computed and compared, so timing does not reveal which one matched.
    for step in [now_step - 1, now_step, now_step + 1] {
        let Ok(counter) = u64::try_from(step) else {
            continue;
        };
        let matches: bool = hotp(secret, counter)
            .as_bytes()
            .ct_eq(code.as_bytes())
            .into();
        let fresh = last_used_step.is_none_or(|last| step > last);
        if matches && fresh && found.is_none() {
            found = Some(step);
        }
    }
    found
}

fn otpauth_uri(secret: &[u8], issuer: &str, account: &str) -> String {
    let mut uri = url::Url::parse("otpauth://totp/").expect("static URL parses");
    uri.set_path(&format!("{issuer}:{account}"));
    uri.query_pairs_mut()
        .append_pair("secret", &BASE32_NOPAD.encode(secret))
        .append_pair("issuer", issuer)
        .append_pair("algorithm", "SHA1")
        .append_pair("digits", &TOTP_DIGITS.to_string())
        .append_pair("period", &TOTP_STEP_SECONDS.to_string());
    uri.to_string()
}

/// A new authenticator app secret, not yet a factor.
pub struct TotpEnrollment {
    /// The secret in base32, for typing into an app by hand.
    pub secret: String,
    /// The `otpauth://` URI an app reads from a QR code.
    pub uri: String,
}

/// Makes a new authenticator app secret for the caller, replacing any that was never
/// confirmed. It becomes a factor when `confirm_totp` sees a correct code from it.
pub async fn begin_totp(
    state: &GlobalServerContext,
    caller: &Caller,
) -> app::Result<TotpEnrollment> {
    caller.ensure_recently_verified(&state.config.auth)?;
    let mut conn = state.connection_pool.get().await?;
    let secret = CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; TOTP_SECRET_BYTES]>());
    let user_id = caller.user;
    let name = conn
        .transaction(|conn| {
            async move {
                let confirmed = diesel::select(diesel::dsl::exists(
                    totp_secret::table
                        .filter(totp_secret::user.eq(user_id))
                        .filter(totp_secret::confirmed_at.is_not_null()),
                ))
                .get_result::<bool>(conn)
                .await?;
                if confirmed {
                    return Err(app::Error::Conflict(t!("totpAlreadyEnabled")));
                }
                diesel::insert_into(totp_secret::table)
                    .values((
                        totp_secret::user.eq(user_id),
                        totp_secret::secret.eq(secret.as_slice()),
                    ))
                    .on_conflict(totp_secret::user)
                    .do_update()
                    .set((
                        totp_secret::secret.eq(secret.as_slice()),
                        totp_secret::created_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)
                    .await?;
                user::table
                    .select(user::name)
                    .filter(user::id.eq(user_id))
                    .first::<String>(conn)
                    .await
                    .map_err(app::Error::from)
            }
            .scope_boxed()
        })
        .await?;
    Ok(TotpEnrollment {
        secret: BASE32_NOPAD.encode(&secret),
        uri: otpauth_uri(&secret, state.settings().name(), &name),
    })
}

/// Confirms the caller's pending authenticator app with a code from it, making it a factor.
/// Returns fresh recovery codes when this turned two-factor sign-in on.
pub async fn confirm_totp(
    state: &GlobalServerContext,
    caller: &Caller,
    code: &str,
) -> app::Result<Option<Vec<String>>> {
    caller.ensure_recently_verified(&state.config.auth)?;
    let user_id = caller.user;
    let secret: Vec<u8> = {
        let mut conn = state.connection_pool.get().await?;
        totp_secret::table
            .select(totp_secret::secret)
            .filter(totp_secret::user.eq(user_id))
            .filter(totp_secret::confirmed_at.is_null())
            .first(&mut conn)
            .await?
    };
    let step = match_totp(&secret, code, totp_step(Utc::now().timestamp()), None);
    let ok = limited(state, user_id, async || Ok(step.is_some())).await?;
    let Some(step) = step.filter(|_| ok) else {
        return Err(app::Error::VerificationFailed);
    };
    let mut conn = state.connection_pool.get().await?;
    let sign_in = app::login::sign_in_id(&caller.refresh_token);
    conn.transaction(|conn| {
        async move {
            let first = !methods(conn, user_id).await?.any_factor();
            let confirmed = diesel::update(
                totp_secret::table
                    .filter(totp_secret::user.eq(user_id))
                    .filter(totp_secret::secret.eq(&secret))
                    .filter(totp_secret::confirmed_at.is_null()),
            )
            .set((
                totp_secret::confirmed_at.eq(diesel::dsl::now),
                totp_secret::last_used_step.eq(step),
            ))
            .execute(conn)
            .await?;
            if confirmed == 0 {
                // Replaced or confirmed by another request since it was read.
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            factor_added(state, conn, user_id, &sign_in, first).await
        }
        .scope_boxed()
    })
    .await
}

/// Removes the caller's authenticator app.
pub async fn remove_totp(state: &GlobalServerContext, caller: &Caller) -> app::Result<()> {
    caller.ensure_recently_verified(&state.config.auth)?;
    let user_id = caller.user;
    let require = state.settings().require_two_factor;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let removed = diesel::delete(totp_secret::table.filter(totp_secret::user.eq(user_id)))
                .execute(conn)
                .await?;
            if removed == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            factor_removed(conn, user_id, require).await
        }
        .scope_boxed()
    })
    .await
}

/// Checks a code against the user's confirmed authenticator app and, when it is right, records
/// its step so it cannot be used again.
async fn use_totp(conn: &mut AsyncPgConnection, user_id: UserId, code: &str) -> app::Result<bool> {
    let code = code.to_string();
    conn.transaction(|conn| {
        async move {
            let row: Option<(Vec<u8>, Option<i64>)> = totp_secret::table
                .select((totp_secret::secret, totp_secret::last_used_step))
                .filter(totp_secret::user.eq(user_id))
                .filter(totp_secret::confirmed_at.is_not_null())
                .for_update()
                .first(conn)
                .await
                .optional()?;
            let Some((secret, last_used_step)) = row else {
                return Ok(false);
            };
            let Some(step) = match_totp(
                &secret,
                &code,
                totp_step(Utc::now().timestamp()),
                last_used_step,
            ) else {
                return Ok(false);
            };
            diesel::update(totp_secret::table.filter(totp_secret::user.eq(user_id)))
                .set(totp_secret::last_used_step.eq(step))
                .execute(conn)
                .await?;
            Ok(true)
        }
        .scope_boxed()
    })
    .await
}

// ---------------------------------------------------------------------------------------------
// Recovery codes
// ---------------------------------------------------------------------------------------------

fn new_recovery_code() -> String {
    let symbols: [u8; RECOVERY_CODE_LENGTH] = CHACHA_RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        std::array::from_fn(|_| RECOVERY_ALPHABET[rng.random_range(0..RECOVERY_ALPHABET.len())])
    });
    let (first, second) = symbols.split_at(RECOVERY_CODE_LENGTH / 2);
    format!(
        "{}-{}",
        std::str::from_utf8(first).expect("alphabet is ASCII"),
        std::str::from_utf8(second).expect("alphabet is ASCII")
    )
}

/// The code as stored: lowercase, separators dropped, and the letters Crockford's base32 reads
/// as digits (`i`, `l` as 1, `o` as 0) mapped to them.
fn normalize_recovery_code(code: &str) -> String {
    code.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| match c.to_ascii_lowercase() {
            'i' | 'l' => '1',
            'o' => '0',
            other => other,
        })
        .collect()
}

fn recovery_code_digest(code: &str) -> Vec<u8> {
    Sha256::digest(normalize_recovery_code(code).as_bytes()).to_vec()
}

async fn replace_recovery_codes(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
) -> app::Result<Vec<String>> {
    diesel::delete(recovery_code::table.filter(recovery_code::user.eq(user_id)))
        .execute(conn)
        .await?;
    let codes: Vec<String> = (0..RECOVERY_CODE_COUNT)
        .map(|_| new_recovery_code())
        .collect();
    let rows: Vec<_> = codes
        .iter()
        .map(|code| {
            (
                recovery_code::user.eq(user_id),
                recovery_code::code_hash.eq(recovery_code_digest(code)),
            )
        })
        .collect();
    diesel::insert_into(recovery_code::table)
        .values(rows)
        .execute(conn)
        .await?;
    Ok(codes)
}

async fn use_recovery_code(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    code: &str,
) -> app::Result<bool> {
    let used = diesel::update(
        recovery_code::table
            .filter(recovery_code::user.eq(user_id))
            .filter(recovery_code::code_hash.eq(recovery_code_digest(code)))
            .filter(recovery_code::used_at.is_null()),
    )
    .set(recovery_code::used_at.eq(diesel::dsl::now))
    .execute(conn)
    .await?;
    Ok(used == 1)
}

/// Replaces the caller's recovery codes with a new set, which is returned once and never
/// again.
pub async fn regenerate_recovery_codes(
    state: &GlobalServerContext,
    caller: &Caller,
) -> app::Result<Vec<String>> {
    caller.ensure_recently_verified(&state.config.auth)?;
    let user_id = caller.user;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            if !methods(conn, user_id).await?.any_factor() {
                return Err(app::Error::Conflict(t!("twoFactorNotEnabled")));
            }
            replace_recovery_codes(conn, user_id).await
        }
        .scope_boxed()
    })
    .await
}

// ---------------------------------------------------------------------------------------------
// Adding and removing factors
// ---------------------------------------------------------------------------------------------

/// Called in the transaction that added a factor, by the sign-in named `sign_in`
/// (`login::sign_in_id`). `first` says whether the account had none before it; if so,
/// two-factor sign-in has just turned on, so the account gets recovery codes and every other
/// sign-in is signed out.
pub async fn factor_added(
    state: &impl crate::app::events::Publishing,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    sign_in: &str,
    first: bool,
) -> app::Result<Option<Vec<String>>> {
    if !first {
        return Ok(None);
    }
    let codes = replace_recovery_codes(conn, user_id).await?;
    app::login::revoke_other_sign_ins(state, conn, user_id, sign_in).await?;
    Ok(Some(codes))
}

/// Called in the transaction that removed a factor. When it was the last, two-factor sign-in
/// is off and the recovery codes go with it, unless the server requires a second factor, in
/// which case the removal is refused and the transaction rolls back.
pub async fn factor_removed(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    require_two_factor: bool,
) -> app::Result<()> {
    if methods(conn, user_id).await?.any_factor() {
        return Ok(());
    }
    if require_two_factor {
        return Err(app::Error::LastSecondFactor);
    }
    diesel::delete(recovery_code::table.filter(recovery_code::user.eq(user_id)))
        .execute(conn)
        .await?;
    Ok(())
}

/// Removes every credential of a user whose account is being deleted.
pub async fn remove_all(conn: &mut AsyncPgConnection, user_id: UserId) -> app::Result<()> {
    diesel::delete(recovery_code::table.filter(recovery_code::user.eq(user_id)))
        .execute(conn)
        .await?;
    diesel::delete(totp_secret::table.filter(totp_secret::user.eq(user_id)))
        .execute(conn)
        .await?;
    diesel::delete(passkey::table.filter(passkey::user.eq(user_id)))
        .execute(conn)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Verifying
// ---------------------------------------------------------------------------------------------

/// Checks a second factor presented as text, under the failure limit.
pub async fn verify(
    state: &GlobalServerContext,
    user_id: UserId,
    factor: &SecondFactor,
) -> app::Result<bool> {
    limited(state, user_id, async || {
        let mut conn = state.connection_pool.get().await?;
        match factor {
            SecondFactor::Totp(code) => use_totp(&mut conn, user_id, code).await,
            SecondFactor::RecoveryCode(code) => use_recovery_code(&mut conn, user_id, code).await,
        }
    })
    .await
}

/// Records that `user_id`'s live sign-in named `sign_in` (`login::sign_in_id`) has just proved
/// who its user is. Returns until when the session counts as recently verified, or
/// `Unauthenticated` when that sign-in has ended.
pub async fn mark_verified(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    sign_in: &str,
    config: &AuthConfig,
) -> app::Result<DateTime<Utc>> {
    let now = Utc::now();
    let marked = diesel::update(
        refresh_token::table
            .filter(refresh_token::user.eq(user_id))
            .filter(refresh_token::expires.gt(now.naive_utc()))
            .filter(
                diesel::dsl::sql::<diesel::sql_types::Bool>(app::login::SIGN_IN_ID_IS_SQL)
                    .bind::<diesel::sql_types::Text, _>(sign_in),
            ),
    )
    .set(refresh_token::verified_at.eq(now))
    .execute(conn)
    .await?;
    if marked == 0 {
        return Err(app::Error::Unauthenticated);
    }
    Ok(now + reverify_window(config))
}

/// What a session presents to prove again who its user is.
pub enum Proof {
    Password(String),
    SecondFactor(SecondFactor),
}

/// Re-verifies the caller. An account with two-factor sign-in on must present a second factor
/// (or a passkey, through `app::passkey`); one without presents its password. Returns until when
/// the session counts as recently verified.
pub async fn reauthenticate(
    state: &GlobalServerContext,
    caller: &Caller,
    proof: Proof,
) -> app::Result<DateTime<Utc>> {
    caller.ensure_person()?;
    let user_id = caller.user;
    let ok = match proof {
        Proof::SecondFactor(factor) => {
            if !caller.has_second_factor {
                return Err(app::Error::Validation(t!("twoFactorNotEnabled")));
            }
            verify(state, user_id, &factor).await?
        }
        Proof::Password(password) => {
            if caller.has_second_factor {
                return Err(app::Error::Validation(t!("reauthenticateWithSecondFactor")));
            }
            limited(state, user_id, async || {
                let mut conn = state.connection_pool.get().await?;
                let hash: String = user::table
                    .select(user::password_hash)
                    .filter(user::id.eq(user_id))
                    .filter(user::deleted_at.is_null())
                    .first(&mut conn)
                    .await?;
                app::login::check_password(password, hash).await
            })
            .await?
        }
    };
    if !ok {
        return Err(app::Error::VerificationFailed);
    }
    let mut conn = state.connection_pool.get().await?;
    mark_verified(
        &mut conn,
        user_id,
        &app::login::sign_in_id(&caller.refresh_token),
        &state.config.auth,
    )
    .await
}

/// A passkey as its owner sees it.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PasskeySummary {
    pub id: app::PasskeyId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

/// Everything the caller's security settings show.
pub struct SecurityOverview {
    pub totp: bool,
    pub passkeys: Vec<PasskeySummary>,
    pub recovery_codes_remaining: i64,
    pub required: bool,
    pub verified_until: DateTime<Utc>,
}

pub async fn overview(
    state: &GlobalServerContext,
    caller: &Caller,
) -> app::Result<SecurityOverview> {
    let user_id = caller.user;
    let mut conn = state.connection_pool.get().await?;
    let found = methods(&mut conn, user_id).await?;
    let passkeys = app::passkey::list(&mut conn, user_id).await?;
    let recovery_codes_remaining = recovery_code::table
        .filter(recovery_code::user.eq(user_id))
        .filter(recovery_code::used_at.is_null())
        .count()
        .get_result::<i64>(&mut conn)
        .await?;
    Ok(SecurityOverview {
        totp: found.totp,
        passkeys,
        recovery_codes_remaining,
        required: state.settings().require_two_factor,
        verified_until: caller.verified_until(&state.config.auth),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses the Valkey of `docker-compose.yaml`, or `VALKEY_URL`.
    async fn test_valkey() -> fred::clients::Client {
        use fred::interfaces::ClientLike;
        let url = std::env::var("VALKEY_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        let client = fred::clients::Client::new(
            fred::prelude::Config::from_url(&url).unwrap(),
            None,
            None,
            None,
        );
        client
            .init()
            .await
            .expect("Valkey from docker-compose.yaml");
        client
    }

    #[tokio::test]
    async fn attempts_arriving_at_once_are_checked_no_more_than_the_limit_allows() {
        let valkey = test_valkey().await;
        let key = format!("auth:failures:test:{}", uuid::Uuid::now_v7());
        let checked = std::sync::atomic::AtomicI64::new(0);
        let attempts = (0..3 * MAX_FAILURES).map(|_| {
            limited_in(&valkey, &key, async || {
                checked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                Ok(false)
            })
        });
        let results = futures_util::future::join_all(attempts).await;
        let refused = results
            .iter()
            .filter(|r| matches!(r, Err(app::Error::TooManyAttempts)))
            .count();
        assert_eq!(
            std::sync::atomic::AtomicI64::load(&checked, std::sync::atomic::Ordering::SeqCst),
            MAX_FAILURES
        );
        assert_eq!(refused as i64, 2 * MAX_FAILURES);
        let ttl: i64 = valkey.ttl(&key).await.unwrap();
        assert!(ttl > 0 && ttl <= FAILURE_WINDOW_SECONDS);
        let _: i64 = valkey.del(&key).await.unwrap();
    }

    #[tokio::test]
    async fn a_right_answer_clears_the_count_and_an_error_is_not_counted() {
        let valkey = test_valkey().await;
        let key = format!("auth:failures:test:{}", uuid::Uuid::now_v7());
        assert!(!limited_in(&valkey, &key, async || Ok(false)).await.unwrap());
        assert!(
            limited_in(&valkey, &key, async || Err(app::Error::Busy))
                .await
                .is_err()
        );
        let count: Option<i64> = valkey.get(&key).await.unwrap();
        assert_eq!(count, Some(1));
        assert!(limited_in(&valkey, &key, async || Ok(true)).await.unwrap());
        let count: Option<i64> = valkey.get(&key).await.unwrap();
        assert_eq!(count, None);
    }

    /// RFC 6238 appendix B, SHA-1 rows, truncated to six digits.
    #[test]
    fn totp_matches_rfc_6238_vectors() {
        let secret = b"12345678901234567890";
        for (time, expected) in [
            (59, "287082"),
            (1_111_111_109, "081804"),
            (1_111_111_111, "050471"),
            (1_234_567_890, "005924"),
            (2_000_000_000, "279037"),
            (20_000_000_000, "353130"),
        ] {
            assert_eq!(hotp(secret, totp_step(time) as u64), expected, "at {time}");
        }
    }

    #[test]
    fn totp_allows_one_step_of_drift_and_refuses_replays() {
        let secret = b"12345678901234567890";
        let now = totp_step(1_111_111_111);
        let current = hotp(secret, now as u64);
        let previous = hotp(secret, (now - 1) as u64);
        let stale = hotp(secret, (now - 2) as u64);
        assert_eq!(match_totp(secret, &current, now, None), Some(now));
        assert_eq!(match_totp(secret, &previous, now, None), Some(now - 1));
        assert_eq!(match_totp(secret, &stale, now, None), None);
        assert_eq!(match_totp(secret, &current, now, Some(now)), None);
        assert_eq!(match_totp(secret, &current, now, Some(now - 1)), Some(now));
    }

    #[test]
    fn totp_ignores_spaces_and_refuses_malformed_codes() {
        let secret = b"12345678901234567890";
        let now = totp_step(59);
        assert_eq!(match_totp(secret, "287 082", now, None), Some(now));
        assert_eq!(match_totp(secret, "28708", now, None), None);
        assert_eq!(match_totp(secret, "28708a", now, None), None);
    }

    #[test]
    fn recovery_codes_normalize_as_crockford_base32() {
        let code = new_recovery_code();
        assert_eq!(code.len(), RECOVERY_CODE_LENGTH + 1);
        assert_eq!(
            recovery_code_digest(&code),
            recovery_code_digest(&code.to_uppercase())
        );
        assert_eq!(
            recovery_code_digest(&code),
            recovery_code_digest(&code.replace('-', " "))
        );
        assert_eq!(normalize_recovery_code("Il0O-abc"), "1100abc");
    }

    #[test]
    fn otpauth_uri_names_the_issuer_and_account() {
        let uri = otpauth_uri(b"12345678901234567890", "Aspen", "kate");
        assert!(uri.starts_with("otpauth://totp/Aspen:kate?"), "{uri}");
        assert!(
            uri.contains("secret=GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ"),
            "{uri}"
        );
        assert!(uri.contains("issuer=Aspen"), "{uri}");
    }
}
