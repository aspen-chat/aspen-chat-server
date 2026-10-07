//! Resetting a forgotten password by email, from the sign-in screen, in three steps:
//!
//! 1. [`start`]: someone names an account by its username and is shown its verified address
//!    masked (`app::email::mask`): the first three characters before the `@`, and the domain.
//!    The reset lasts [`LIFETIME`] and is named by a random id, kept in Valkey under its digest.
//! 2. [`send_code`]: they type the whole address. Only when it is the account's (ignoring case)
//!    is a code of [`CODE_DIGITS`] digits mailed there, ahead of all other mail (`outbox`). Each
//!    reset takes [`ATTEMPTS`] wrong addresses before it ends, and asking again replaces the code.
//!    Each address and code is counted before it is checked, so a burst of guesses sent at once
//!    is checked no more than [`ATTEMPTS`] times, whatever the rate limits allow.
//! 3. [`complete`]: they type the code and a new password. The password is replaced, every
//!    sign-in of the account ends, as do its plugin capability URLs, the second factors added and recovery codes issued in the
//!    last week go (`two_factor::remove_recent`), in case whoever took the account added them,
//!    and the address is told. The older factors stay: signing in still asks for one. Each
//!    reset takes [`ATTEMPTS`] wrong codes before it ends.
//!
//! Only an account of this deployment with a verified address can be reset this way; bots,
//! foreign users, and the system account cannot. What `start` shows confirms that the username
//! exists, which registration does already; the address stays hidden behind its mask. Each step
//! is rate limited by address and per reset, and one account is mailed at most [`MAX_SENT`]
//! codes an hour, however many resets ask.

use super::outbox::{self, Mail};
use crate::UserId;
use crate::context::GlobalServerContext;
use crate::{CHACHA_RNG, t};
use aspen_schema::{user, user_email};
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use fred::prelude::KeysInterface;
use fred::types::Expiration;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

/// How long a reset lasts from its start.
pub const LIFETIME_SECONDS: i64 = 30 * 60;
/// How many digits a reset code has.
pub const CODE_DIGITS: u32 = 8;
/// How many wrong addresses, and how many wrong codes, end a reset.
pub const ATTEMPTS: i64 = 5;
/// How many reset codes one account is mailed in [`SENT_WINDOW_SECONDS`], however many resets
/// ask, so nobody can fill someone's inbox by starting resets from many addresses.
pub const MAX_SENT: i64 = 5;
pub const SENT_WINDOW_SECONDS: i64 = 60 * 60;

/// A reset as Valkey keeps it.
#[derive(Debug, Serialize, Deserialize)]
struct Reset {
    user: UserId,
    /// The digest of the code mailed, once one is.
    code: Option<String>,
}

/// A reset begun: its id, which the next steps name, and the address it will mail, masked.
#[derive(Debug)]
pub struct Started {
    pub id: String,
    pub masked_address: String,
}

fn key(id: &str) -> String {
    format!(
        "email:reset:{}",
        BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(id.as_bytes()))
    )
}

fn misses_key(id: &str, what: &str) -> String {
    format!("{}:{what}", key(id))
}

/// Begins resetting the password of the account named `username`.
pub async fn start(state: &GlobalServerContext, username: &str) -> crate::Result<Started> {
    if !super::available(state) {
        return Err(crate::Error::PasswordResetUnavailable(t!(
            "passwordResetNoEmail"
        )));
    }
    let mut conn = state.connection_pool.get().await?;
    /// The account, whether it is a bot, and its address with whether that is verified.
    type Found = (UserId, bool, Option<(String, bool)>);
    let found: Option<Found> = user::table
        .left_join(user_email::table)
        .select((
            user::id,
            user::bot,
            (user_email::address, user_email::verified_at.is_not_null()).nullable(),
        ))
        .filter(crate::user::named(username.to_string()))
        .first(conn.as_mut())
        .await
        .optional()?;
    let Some((user_id, bot, email)) = found else {
        return Err(crate::Error::PasswordResetUnavailable(t!(
            "passwordResetNoAccount",
            name = username
        )));
    };
    let address = match email {
        Some((address, true)) if !bot => address,
        _ => {
            return Err(crate::Error::PasswordResetUnavailable(t!(
                "passwordResetNoAddress",
                name = username
            )));
        }
    };
    let id =
        BASE64_URL_SAFE_NO_PAD.encode(CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 32]>()));
    let reset = Reset {
        user: user_id,
        code: None,
    };
    let _: () = state
        .valkey
        .set(
            key(&id),
            serde_json::to_string(&reset)?,
            Some(Expiration::EX(LIFETIME_SECONDS)),
            None,
            false,
        )
        .await?;
    Ok(Started {
        id,
        masked_address: super::mask(&address),
    })
}

async fn read(state: &GlobalServerContext, id: &str) -> crate::Result<Reset> {
    let raw: Option<String> = state.valkey.get(key(id)).await?;
    let raw = raw.ok_or(crate::Error::PasswordResetExpired)?;
    Ok(serde_json::from_str(&raw)?)
}

/// Counts an attempt at `what` (the address or the code) against the reset before it is checked,
/// so attempts sent all at once are checked no more than [`ATTEMPTS`] times: the attempt's
/// number, or `TooManyAttempts`, ending the reset, once that many have been checked.
async fn attempt(state: &GlobalServerContext, id: &str, what: &str) -> crate::Result<i64> {
    let attempts =
        crate::two_factor::count_attempt(&state.valkey, &misses_key(id, what), LIFETIME_SECONDS)
            .await?;
    if attempts > ATTEMPTS {
        end(state, id).await?;
        return Err(crate::Error::TooManyAttempts);
    }
    Ok(attempts)
}

/// The error for a wrong `what` that was attempt number `attempts`, ending the reset at the
/// last one [`ATTEMPTS`] allows.
async fn miss(
    state: &GlobalServerContext,
    id: &str,
    what: &str,
    attempts: i64,
) -> crate::Result<crate::Error> {
    if attempts >= ATTEMPTS {
        end(state, id).await?;
        return Ok(crate::Error::TooManyAttempts);
    }
    Ok(match what {
        "address" => crate::Error::EmailMismatch,
        _ => crate::Error::VerificationFailed,
    })
}

async fn end(state: &GlobalServerContext, id: &str) -> crate::Result<()> {
    let _: i64 = state
        .valkey
        .del(vec![
            key(id),
            misses_key(id, "address"),
            misses_key(id, "code"),
        ])
        .await?;
    Ok(())
}

/// Mails a reset code to the account's address, once `address` proves the caller knows it.
pub async fn send_code(state: &GlobalServerContext, id: &str, address: &str) -> crate::Result<()> {
    let reset = read(state, id).await?;
    let mut conn = state.connection_pool.get().await?;
    let on_file: Option<String> = user_email::table
        .select(user_email::address)
        .filter(user_email::user.eq(reset.user))
        .filter(user_email::verified_at.is_not_null())
        .first(conn.as_mut())
        .await
        .optional()?;
    let Some(on_file) = on_file else {
        end(state, id).await?;
        return Err(crate::Error::PasswordResetExpired);
    };
    let attempts = attempt(state, id, "address").await?;
    if !super::same_address(address, &on_file) {
        return Err(miss(state, id, "address", attempts).await?);
    }
    // The right address, which asking again for a code gives each time, counts against nothing.
    crate::two_factor::uncount_attempt(&state.valkey, &misses_key(id, "address")).await?;
    let sent_key = format!("email:reset-sent:{}", reset.user.0);
    let sent: i64 = state.valkey.incr(&sent_key).await?;
    if sent == 1 {
        let _: bool = state
            .valkey
            .expire(&sent_key, SENT_WINDOW_SECONDS, None)
            .await?;
    }
    if sent > MAX_SENT {
        return Err(crate::Error::TooManyAttempts);
    }
    let code = super::code(CODE_DIGITS);
    let updated = Reset {
        user: reset.user,
        code: Some(super::code_digest(&code)),
    };
    let ttl: i64 = state.valkey.ttl(key(id)).await?;
    let _: () = state
        .valkey
        .set(
            key(id),
            serde_json::to_string(&updated)?,
            Some(Expiration::EX(ttl.max(1))),
            None,
            false,
        )
        .await?;
    outbox::queue(
        conn.as_mut(),
        reset.user,
        None,
        &Mail::PasswordReset { code },
    )
    .await?;
    super::wake(state).await;
    Ok(())
}

/// Replaces the password with `new_password`, given the code mailed for the reset, ends every
/// sign-in of the account, and removes the second factors added in the last week.
pub async fn complete(
    state: &GlobalServerContext,
    id: &str,
    code: &str,
    new_password: &str,
) -> crate::Result<()> {
    let reset = read(state, id).await?;
    let Some(expected) = &reset.code else {
        return Err(crate::Error::Validation(t!("passwordResetNoCodeYet")));
    };
    // Refused before the code is tried, so a short password costs no attempt.
    if new_password.len() < crate::login::PASSWORD_MIN_LENGTH {
        return Err(crate::Error::PasswordRequirement(
            crate::PasswordRequirement::Length,
        ));
    }
    let attempts = attempt(state, id, "code").await?;
    let matches: bool = expected
        .as_bytes()
        .ct_eq(super::code_digest(code).as_bytes())
        .into();
    if !matches {
        return Err(miss(state, id, "code", attempts).await?);
    }
    let password_hash = crate::login::hash_password(new_password.to_string()).await?;
    // Used once, whatever happens next.
    end(state, id).await?;
    let user_id = reset.user;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let changed = diesel::update(
                user::table
                    .filter(user::id.eq(user_id))
                    .filter(user::deleted_at.is_null()),
            )
            .set(user::password_hash.eq(password_hash))
            .execute(conn)
            .await?;
            if changed == 0 {
                return Err(crate::Error::PasswordResetExpired);
            }
            crate::login::revoke_all_sessions(state, conn, user_id).await?;
            crate::plugin::capability::revoke_all(conn, user_id).await?;
            let removed_factors = crate::two_factor::remove_recent(conn, user_id).await?;
            outbox::queue(
                conn,
                user_id,
                None,
                &Mail::PasswordWasReset { removed_factors },
            )
            .await
        }
        .scope_boxed()
    })
    .await?;
    super::wake(state).await;
    tracing::info!(user = %user_id.0, "a password was reset by email");
    Ok(())
}
