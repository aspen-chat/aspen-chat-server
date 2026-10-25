//! Device credentials: what a client that has not signed in is counted by, so that the many
//! people behind one address (a carrier-grade NAT, an event's network) each have limits of their
//! own, while one machine is held to a device's (`app::rate_limit`'s `device` dimension).
//!
//! A client earns one by solving a puzzle (`aspen_limits::puzzle`): given a challenge, it finds a
//! nonce for which `SHA-256(SHA-256(challenge) || nonce)`, with the nonce as eight big-endian
//! bytes, begins with the challenge's number of zero bits (`[rate_limits] device_puzzle_bits`).
//! Finding one takes about `2^bits` hashes; checking one takes two. Each challenge earns one credential, so one
//! machine pays the puzzle for every device it claims to be, while a person pays it once per
//! deployment and then every `LIFETIME` at most.
//!
//! Challenges and credentials are both kept by no one: each is a signed token
//! (`app::signed_token`) under the deployment's device key (`server_secret::DEVICES`), so every
//! API server checks them without asking another. Only a challenge's use is remembered, in
//! Valkey, until it expires.

use crate::CHACHA_RNG;
use crate::context::GlobalServerContext;
use crate::server_secret::HmacKey;
use crate::signed_token::{VERSION, open, seal, seconds, time_of};
use aspen_limits::puzzle;
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use chrono::{DateTime, TimeDelta, Utc};
use fred::interfaces::KeysInterface;
use rand::RngExt;

/// How long a credential lasts. One presented with less than half of it left is answered with a
/// fresh one (`Presented::renewal`), so a device in use never solves the puzzle again.
pub const LIFETIME: TimeDelta = TimeDelta::days(90);

/// How long a challenge may be solved and redeemed: long enough for a slow phone that is unlucky,
/// since the number of tries a puzzle takes varies widely around its average.
pub const CHALLENGE_LIFETIME: TimeDelta = TimeDelta::minutes(5);

/// The most zero bits a challenge can ask for, which `[rate_limits] device_puzzle_bits` is
/// clamped to: past it the puzzle would take a phone hours.
pub const MAX_PUZZLE_BITS: u8 = 32;

const RANDOM_BYTES: usize = 16;
/// What each kind of token is signed as, so a challenge can never be presented as a credential.
const CHALLENGE_CONTEXT: &[u8] = b"aspen device challenge\0";
const CREDENTIAL_CONTEXT: &[u8] = b"aspen device credential\0";
/// Valkey's record of a redeemed challenge, keyed by the challenge's random part.
const USED_PREFIX: &str = "device-challenge";

/// A device, as its credential names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DeviceId([u8; RANDOM_BYTES]);

impl DeviceId {
    #[cfg(test)]
    pub(crate) fn from_bytes(bytes: [u8; RANDOM_BYTES]) -> Self {
        Self(bytes)
    }
}

impl std::fmt::Display for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&data_encoding::HEXLOWER.encode(&self.0))
    }
}

/// A puzzle to solve for a credential.
#[derive(Clone, Debug)]
pub struct Challenge {
    pub token: String,
    pub difficulty: u8,
    pub expires_at: DateTime<Utc>,
}

/// A credential: the token a client sends as `Aspen-Device`, and until when it is accepted.
#[derive(Clone, Debug)]
pub struct Credential {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// A credential a request presented, once checked.
#[derive(Clone, Debug)]
pub struct Presented {
    pub device: DeviceId,
    /// A fresh credential for the same device, when the one presented has less than half its
    /// lifetime left.
    pub renewal: Option<Credential>,
}

/// Why a solution earns no credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a challenge this deployment made.
    Malformed,
    Expired,
    /// The nonce does not solve it: its hash needs this many leading zero bits.
    Unsolved {
        bits: u8,
    },
    /// It has already earned a credential.
    Used,
}

fn random_bytes() -> [u8; RANDOM_BYTES] {
    CHACHA_RNG.with(|rng| rng.borrow_mut().random())
}

/// Version, expiry, difficulty, and a random part.
const CHALLENGE_BODY: usize = 1 + 8 + 1 + RANDOM_BYTES;
/// Version, device, and expiry.
const CREDENTIAL_BODY: usize = 1 + RANDOM_BYTES + 8;

/// A new challenge asking for `difficulty` zero bits.
pub fn challenge(key: &HmacKey, difficulty: u8, now: DateTime<Utc>) -> Challenge {
    let difficulty = difficulty.min(MAX_PUZZLE_BITS);
    let expires_at = now + CHALLENGE_LIFETIME;
    let mut body = Vec::with_capacity(CHALLENGE_BODY);
    body.push(VERSION);
    body.extend_from_slice(&seconds(expires_at));
    body.push(difficulty);
    body.extend_from_slice(&random_bytes());
    Challenge {
        token: seal(key, CHALLENGE_CONTEXT, &body),
        difficulty,
        expires_at,
    }
}

/// A challenge whose solution checked out, not yet marked used.
#[derive(Debug)]
struct Solved {
    used_key: String,
    expires_at: DateTime<Utc>,
}

fn check_solution(
    key: &HmacKey,
    token: &str,
    nonce: u64,
    now: DateTime<Utc>,
) -> Result<Solved, Refusal> {
    let body = open(key, CHALLENGE_CONTEXT, token, CHALLENGE_BODY).ok_or(Refusal::Malformed)?;
    let expires_at = time_of(&body[1..9]).ok_or(Refusal::Malformed)?;
    if expires_at <= now {
        return Err(Refusal::Expired);
    }
    let bits = body[9];
    let sealed = BASE64_URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|_| Refusal::Malformed)?;
    if puzzle::work(&sealed, nonce) < u32::from(bits) {
        return Err(Refusal::Unsolved { bits });
    }
    Ok(Solved {
        used_key: format!(
            "{USED_PREFIX}:{}",
            data_encoding::HEXLOWER.encode(&body[10..])
        ),
        expires_at,
    })
}

/// A credential for a new device, earned by solving `challenge` with `nonce`. Each challenge
/// earns one, so a solution is remembered until its challenge expires.
pub async fn redeem(
    state: &GlobalServerContext,
    challenge: &str,
    nonce: u64,
) -> crate::Result<Result<Credential, Refusal>> {
    let now = Utc::now();
    let solved = match check_solution(&state.device_key, challenge, nonce, now) {
        Ok(solved) => solved,
        Err(refusal) => return Ok(Err(refusal)),
    };
    let first: Option<String> = state
        .valkey
        .set(
            solved.used_key,
            1,
            Some(fred::types::Expiration::PX(
                (solved.expires_at - now).num_milliseconds().max(1),
            )),
            Some(fred::types::SetOptions::NX),
            false,
        )
        .await?;
    if first.is_none() {
        return Ok(Err(Refusal::Used));
    }
    Ok(Ok(issue(&state.device_key, DeviceId(random_bytes()), now)))
}

/// A credential for `device`, lasting `LIFETIME` from `now`.
pub fn issue(key: &HmacKey, device: DeviceId, now: DateTime<Utc>) -> Credential {
    let expires_at = now + LIFETIME;
    let mut body = Vec::with_capacity(CREDENTIAL_BODY);
    body.push(VERSION);
    body.extend_from_slice(&device.0);
    body.extend_from_slice(&seconds(expires_at));
    Credential {
        token: seal(key, CREDENTIAL_CONTEXT, &body),
        expires_at,
    }
}

/// The device a presented credential names, when it is this deployment's and has not expired.
pub fn verify(key: &HmacKey, token: &str, now: DateTime<Utc>) -> Option<Presented> {
    let body = open(key, CREDENTIAL_CONTEXT, token, CREDENTIAL_BODY)?;
    let device = DeviceId(body[1..1 + RANDOM_BYTES].try_into().ok()?);
    let expires_at = time_of(&body[1 + RANDOM_BYTES..])?;
    if expires_at <= now {
        return None;
    }
    let renewal = (expires_at - now < LIFETIME / 2).then(|| issue(key, device, now));
    Some(Presented { device, renewal })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> HmacKey {
        HmacKey::from_secret(b"test")
    }

    fn solve(challenge: &Challenge) -> u64 {
        let sealed = BASE64_URL_SAFE_NO_PAD.decode(&challenge.token).unwrap();
        puzzle::solve(&sealed, challenge.difficulty)
    }

    #[test]
    fn a_solution_checks_out_and_a_wrong_one_does_not() {
        let now = Utc::now();
        let made = challenge(&key(), 8, now);
        let nonce = solve(&made);
        assert!(check_solution(&key(), &made.token, nonce, now).is_ok());
        let wrong = (0..)
            .find(|n| puzzle::work(&BASE64_URL_SAFE_NO_PAD.decode(&made.token).unwrap(), *n) < 8)
            .unwrap();
        assert_eq!(
            check_solution(&key(), &made.token, wrong, now).unwrap_err(),
            Refusal::Unsolved { bits: 8 }
        );
    }

    #[test]
    fn challenges_expire_and_belong_to_their_key() {
        let now = Utc::now();
        let made = challenge(&key(), 1, now);
        let nonce = solve(&made);
        assert_eq!(
            check_solution(&key(), &made.token, nonce, now + CHALLENGE_LIFETIME).unwrap_err(),
            Refusal::Expired
        );
        assert_eq!(
            check_solution(&HmacKey::from_secret(b"other"), &made.token, nonce, now).unwrap_err(),
            Refusal::Malformed
        );
    }

    #[test]
    fn the_difficulty_is_signed() {
        let now = Utc::now();
        let made = challenge(&key(), 20, now);
        let mut sealed = BASE64_URL_SAFE_NO_PAD.decode(&made.token).unwrap();
        sealed[9] = 0;
        let lowered = BASE64_URL_SAFE_NO_PAD.encode(sealed);
        assert_eq!(
            check_solution(&key(), &lowered, 0, now).unwrap_err(),
            Refusal::Malformed
        );
    }

    #[test]
    fn credentials_name_their_device_and_renew_past_half_their_life() {
        let now = Utc::now();
        let device = DeviceId([7; RANDOM_BYTES]);
        let credential = issue(&key(), device, now);
        let fresh = verify(&key(), &credential.token, now).unwrap();
        assert_eq!(fresh.device, device);
        assert!(fresh.renewal.is_none());
        let later = now + LIFETIME / 2 + TimeDelta::days(1);
        let old = verify(&key(), &credential.token, later).unwrap();
        assert_eq!(old.renewal.unwrap().expires_at, later + LIFETIME);
        assert!(verify(&key(), &credential.token, now + LIFETIME).is_none());
        assert!(verify(&HmacKey::from_secret(b"other"), &credential.token, now).is_none());
    }

    #[test]
    fn a_challenge_is_not_a_credential() {
        let now = Utc::now();
        let made = challenge(&key(), 1, now);
        assert!(verify(&key(), &made.token, now).is_none());
        let credential = issue(&key(), DeviceId([1; RANDOM_BYTES]), now);
        assert_eq!(
            check_solution(&key(), &credential.token, 0, now).unwrap_err(),
            Refusal::Malformed
        );
    }
}
