//! Tokens only the API servers make and check: each is its body and an HMAC-SHA-256 tag under
//! one of the deployment's keys (`server_secret::HmacKey`), for a context that keeps one kind
//! from being presented as another, all in base64url. Any API server checks one alone. Device
//! credentials and their challenges are made this way (`app::device_credential`), and so are
//! session and bot tokens, whose tag lets a made-up one be refused without reading the database:
//! one machine sending made-up tokens then costs nothing, and needs no limit on its address that
//! everyone signed in behind that address would share.
//!
//! A session token holds its expiry too, so an expired one is refused the same way. One that
//! checks out is still looked up, since a sign-in ends before its tokens expire; one the
//! database then rejects is remembered (`Rejected`), so presenting it again costs no lookup
//! either.

use crate::CHACHA_RNG;
use crate::server_secret::HmacKey;
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::RngExt;

pub(crate) const VERSION: u8 = 1;
const TAG_BYTES: usize = 32;
const RANDOM_BYTES: usize = 32;

const SESSION_CONTEXT: &[u8] = b"aspen session token\0";
const BOT_CONTEXT: &[u8] = b"aspen bot token\0";
/// Version, expiry, and a random part.
const SESSION_BODY: usize = 1 + 8 + RANDOM_BYTES;
/// Version and a random part.
const BOT_BODY: usize = 1 + RANDOM_BYTES;

fn tag(key: &HmacKey, context: &[u8], body: &[u8]) -> aws_lc_rs::hmac::Tag {
    let mut signing = aws_lc_rs::hmac::Context::with_key(key.key());
    signing.update(context);
    signing.update(body);
    signing.sign()
}

/// `body` and its tag under `key` for `context`, in base64url without padding.
pub(crate) fn seal(key: &HmacKey, context: &[u8], body: &[u8]) -> String {
    let mut sealed = body.to_vec();
    sealed.extend_from_slice(tag(key, context, body).as_ref());
    BASE64_URL_SAFE_NO_PAD.encode(sealed)
}

/// The body of `token` when it is `len` bytes long, begins with `VERSION`, and its tag is
/// `key`'s for `context`.
pub(crate) fn open(key: &HmacKey, context: &[u8], token: &str, len: usize) -> Option<Vec<u8>> {
    let mut sealed = BASE64_URL_SAFE_NO_PAD.decode(token.trim()).ok()?;
    if sealed.len() != len + TAG_BYTES || sealed[0] != VERSION {
        return None;
    }
    let presented = sealed.split_off(len);
    let mut message = context.to_vec();
    message.extend_from_slice(&sealed);
    aws_lc_rs::hmac::verify(key.key(), &message, &presented).ok()?;
    Some(sealed)
}

/// `at` as a token holds it: whole seconds since 1970, eight big-endian bytes.
pub(crate) fn seconds(at: DateTime<Utc>) -> [u8; 8] {
    at.timestamp().to_be_bytes()
}

pub(crate) fn time_of(bytes: &[u8]) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(i64::from_be_bytes(bytes.try_into().ok()?), 0)
}

fn random_part() -> [u8; RANDOM_BYTES] {
    CHACHA_RNG.with(|rng| rng.borrow_mut().random())
}

/// A new session token, accepted until `expires`.
pub fn session_token(key: &HmacKey, expires: DateTime<Utc>) -> String {
    let mut body = Vec::with_capacity(SESSION_BODY);
    body.push(VERSION);
    body.extend_from_slice(&seconds(expires));
    body.extend_from_slice(&random_part());
    seal(key, SESSION_CONTEXT, &body)
}

/// Whether `token` is a session token this deployment made that has not expired. Only such a
/// token is worth looking up.
pub fn session_token_checks_out(key: &HmacKey, token: &str, now: DateTime<Utc>) -> bool {
    open(key, SESSION_CONTEXT, token, SESSION_BODY)
        .and_then(|body| time_of(&body[1..9]))
        .is_some_and(|expires| expires > now)
}

/// The part of a new bot token after its prefix (`bot::TOKEN_PREFIX`).
pub fn bot_token(key: &HmacKey) -> String {
    let mut body = Vec::with_capacity(BOT_BODY);
    body.push(VERSION);
    body.extend_from_slice(&random_part());
    seal(key, BOT_CONTEXT, &body)
}

/// What a bot token's part after its prefix says of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotToken {
    /// Made by this deployment: worth looking up.
    Signed,
    /// Made before bot tokens were signed: 32 random bytes, which only a lookup can tell from a
    /// made-up one, so a lookup of one is limited per address first
    /// (`rate_limit::Stage::RejectedTokens`).
    Unsigned,
    /// Neither: refused without a lookup.
    Refused,
}

pub fn check_bot_token(key: &HmacKey, token: &str) -> BotToken {
    if open(key, BOT_CONTEXT, token, BOT_BODY).is_some() {
        BotToken::Signed
    } else if BASE64_URL_SAFE_NO_PAD
        .decode(token.trim())
        .is_ok_and(|bytes| bytes.len() == RANDOM_BYTES)
    {
        BotToken::Unsigned
    } else {
        BotToken::Refused
    }
}

/// How many signed tokens the database rejected each API server remembers.
const REJECTED_CAPACITY: u64 = 100_000;
/// How long it remembers one: a session token's lifetime, after which a session token is
/// refused by its expiry alone. A bot token rejected once is rejected for good, so forgetting
/// it costs one lookup more.
const REJECTED_FOR: std::time::Duration = std::time::Duration::from_secs(3 * 60 * 60);

/// Signed tokens the database rejected (a sign-in ended, a bot's token reset, an account
/// deleted or banned), by their digest, so presenting one again costs no lookup. A token is
/// never accepted again once its sign-in ends, so remembering a rejection is never wrong for
/// longer than `REJECTED_FOR`, and only for an account whose ban was lifted, whose app refreshes
/// its session on the refusal.
#[derive(Clone)]
pub struct Rejected(moka::sync::Cache<String, ()>);

impl Default for Rejected {
    fn default() -> Self {
        Self(
            moka::sync::Cache::builder()
                .max_capacity(REJECTED_CAPACITY)
                .time_to_live(REJECTED_FOR)
                .build(),
        )
    }
}

impl std::fmt::Debug for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Rejected(..)")
    }
}

impl Rejected {
    pub fn contains(&self, digest: &str) -> bool {
        self.0.contains_key(digest)
    }

    pub fn insert(&self, digest: String) {
        self.0.insert(digest, ());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;

    fn key() -> HmacKey {
        HmacKey::from_secret(b"test")
    }

    #[test]
    fn a_session_token_checks_out_until_it_expires() {
        let now = Utc::now();
        let token = session_token(&key(), now + TimeDelta::hours(3));
        assert!(session_token_checks_out(&key(), &token, now));
        assert!(!session_token_checks_out(
            &key(),
            &token,
            now + TimeDelta::hours(3)
        ));
        assert!(!session_token_checks_out(
            &HmacKey::from_secret(b"other"),
            &token,
            now
        ));
    }

    #[test]
    fn made_up_and_older_session_tokens_do_not_check_out() {
        let now = Utc::now();
        assert!(!session_token_checks_out(&key(), "made-up", now));
        // A token made before session tokens were signed: 32 random bytes in base64.
        let older = base64::prelude::BASE64_STANDARD.encode([7u8; 32]);
        assert!(!session_token_checks_out(&key(), &older, now));
        // A bot token is not a session token, nor the other way round.
        assert!(!session_token_checks_out(&key(), &bot_token(&key()), now));
        assert_eq!(
            check_bot_token(&key(), &session_token(&key(), now + TimeDelta::hours(1))),
            BotToken::Refused
        );
    }

    #[test]
    fn bot_tokens_are_signed_or_from_before_signing() {
        assert_eq!(
            check_bot_token(&key(), &bot_token(&key())),
            BotToken::Signed
        );
        assert_eq!(
            check_bot_token(&HmacKey::from_secret(b"other"), &bot_token(&key())),
            BotToken::Refused
        );
        let older = BASE64_URL_SAFE_NO_PAD.encode([9u8; 32]);
        assert_eq!(check_bot_token(&key(), &older), BotToken::Unsigned);
        assert_eq!(check_bot_token(&key(), "made-up"), BotToken::Refused);
    }
}
