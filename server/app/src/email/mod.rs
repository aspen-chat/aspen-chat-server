//! Email: an address an account may give, at registration or later, and what the deployment
//! sends there. Nothing is sent unless `[email]` in `aspen.toml` names an SMTP server
//! ([`Mailer`]); without it, the deployment settings that need mail cannot be turned on.
//!
//! An account of this deployment holds at most one address (`user_email`), unverified until its
//! owner types the code mailed there ([`verify`]). Addresses are not unique: two accounts may
//! share one, and nothing tells anyone whether an address is in use. Giving or changing an
//! address is a change to security settings, since a verified address can reset the password
//! ([`reset`]), so it needs a recent verification of the sign-in (`app::two_factor`), except at
//! registration; it starts unverified, and the address it replaced, when verified, is told.
//! Bots, the system account, and the users of other deployments have no address here: mail for
//! them is their home's to send.
//!
//! The deployment settings `email_required` (an address to register) and
//! `email_verification_required` (a verified one to use the deployment) are the administrators'.
//! The first binds only registration. Under the second, a session of an account whose address is
//! not verified may reach only what verifies, changes, or resends it, and signing out
//! (`Caller::verification_required`), as with a second factor the deployment requires.
//!
//! Only a verified address receives anything but its verification code: the newsletter
//! ([`newsletter`]), when the deployment has one and the account subscribed, the daily digest
//! ([`digest`]), when the account asked for it, and password reset codes. An account may show its
//! verified address on its profile, where `user.public_email` carries it to every reader of the
//! user, other deployments included ([`set_public_email`]).
//!
//! Mail is not sent while a request waits: it is written to `email_outbox` in the transaction
//! that causes it, and sent by [`outbox`], password resets first, on the servers whose `[email]`
//! has `send` on.

pub mod digest;
pub mod newsletter;
pub mod outbox;
mod render;
pub mod reset;

use crate::aspen_config::EmailConfig;
use crate::context::GlobalServerContext;
use crate::two_factor::Caller;
use crate::{CHACHA_RNG, t};
use crate::{EventScope, UserId, publish_event};
use aspen_schema::{email_outbox, user, user_email};
use aspen_wire::message_enum::server_event::{ServerEvent, UserEvent};
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use fred::prelude::KeysInterface;
use fred::types::Expiration;
use lettre::message::Mailbox;
use lettre::{AsyncSmtpTransport, Tokio1Executor};
use rand::RngExt;
use sha2::{Digest, Sha256};
use std::str::FromStr;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::sync::Notify;

/// The longest address SMTP carries (RFC 5321's path limit, less its brackets).
pub const ADDRESS_MAX_LENGTH: usize = 254;

/// How long a verification code works.
const VERIFICATION_LIFETIME: Duration = Duration::from_secs(60 * 60);
/// How many wrong verification codes end the code, so a new one must be sent.
const VERIFICATION_ATTEMPTS: i64 = 5;

/// The NATS subject a server publishes to when it queues mail someone waits for, which every
/// sending server listens on (`outbox::spawn_sender`). Plain NATS, not the event stream: a
/// wake-up missed costs only the time to the sender's next look.
pub const WAKE_SUBJECT: &str = "aspen.email.wake";

/// What a server knows of mail, made when `[email]` is configured: every such server takes
/// addresses and queues mail, and those with `send` on also send it.
pub struct Mailer {
    /// The SMTP server, on a server that sends; `None` on one that only queues.
    transport: Option<AsyncSmtpTransport<Tokio1Executor>>,
    from: Mailbox,
    /// Where this deployment is reached from mail (`AspenConfig::public_url`).
    public_url: String,
    /// The deployment-wide sending rate, when `max_per_second` sets one.
    rate: Option<aspen_limits::Rate>,
    /// Woken when mail someone waits for is queued, on this server or another, so the sender
    /// need not wait for its next look.
    wake: Notify,
}

impl Mailer {
    /// The mailer `config` describes, which connects only when it first sends.
    pub fn new(config: &EmailConfig, public_url: &str) -> crate::Result<Self> {
        let invalid = |detail: String| {
            crate::Error::Config(config::ConfigError::Message(format!(
                "[email] is not usable: {detail}"
            )))
        };
        let transport = match (&config.smtp_url, config.send) {
            (Some(url), true) => Some(
                AsyncSmtpTransport::<Tokio1Executor>::from_url(url)
                    .map_err(|e| invalid(format!("smtp_url: {e}")))?
                    .build(),
            ),
            _ => None,
        };
        let from = config
            .from
            .parse::<Mailbox>()
            .map_err(|e| invalid(format!("from: {e}")))?;
        // A second's worth may go back to back.
        let rate = config.max_per_second.map(|per_second| {
            aspen_limits::Limit {
                requests: per_second,
                per_seconds: 1.0,
                burst: None,
                bucket: None,
            }
            .rate()
        });
        Ok(Self {
            transport,
            from,
            public_url: public_url.to_string(),
            rate,
            wake: Notify::new(),
        })
    }

    /// Whether this server sends mail and makes digests.
    pub fn sends(&self) -> bool {
        self.transport.is_some()
    }
}

/// Whether this server can send mail.
pub fn available(state: &GlobalServerContext) -> bool {
    state.mailer.is_some()
}

/// `given` trimmed, if it is an address mail can be sent to.
pub fn parse_address(given: &str) -> crate::Result<String> {
    let address = given.trim();
    if address.len() > ADDRESS_MAX_LENGTH
        || address.chars().any(char::is_whitespace)
        || lettre::Address::from_str(address).is_err()
    {
        return Err(crate::Error::Validation(t!(
            "emailAddressInvalid",
            max = ADDRESS_MAX_LENGTH
        )));
    }
    Ok(address.to_string())
}

/// `address` as someone who does not know it may see it: the first three characters of the part
/// before the `@` (fewer when it is that short, so one is always hidden), each other character
/// there an asterisk, and the domain whole.
pub fn mask(address: &str) -> String {
    let (local, domain) = address.rsplit_once('@').unwrap_or((address, ""));
    let length = local.chars().count();
    let shown = 3.min(length.saturating_sub(1));
    let mut masked: String = local.chars().take(shown).collect();
    masked.extend(std::iter::repeat_n('*', length - shown));
    masked.push('@');
    masked.push_str(domain);
    masked
}

/// Whether `given` is `address`, ignoring case and surrounding space. The part before the `@` is
/// case-sensitive by the letter of RFC 5321, but no mail provider people use treats it so, and
/// someone proving they know their address should not fail on a capital.
pub fn same_address(given: &str, address: &str) -> bool {
    let given = given.trim().to_lowercase();
    given
        .as_bytes()
        .ct_eq(address.to_lowercase().as_bytes())
        .into()
}

/// A random code of `digits` decimal digits.
fn code(digits: u32) -> String {
    let bound = 10u32.pow(digits);
    let n = CHACHA_RNG.with(|rng| rng.borrow_mut().random_range(0..bound));
    format!("{n:0width$}", width = digits as usize)
}

/// The digest a code is kept as, so a read of Valkey does not give it away.
fn code_digest(code: &str) -> String {
    BASE64_URL_SAFE_NO_PAD.encode(Sha256::digest(code.trim().as_bytes()))
}

/// The secret an address's unsubscribe links carry.
fn unsubscribe_token() -> String {
    BASE64_URL_SAFE_NO_PAD.encode(CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 24]>()))
}

/// An account's address and what it receives there, as `user_email` holds it.
#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = user_email)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct EmailAccount {
    pub address: String,
    pub verified_at: Option<DateTime<Utc>>,
    /// Whether the account shows the address on its profile once it is verified.
    pub shown: bool,
    pub newsletter: bool,
    pub digest: bool,
    pub digest_time_zone: String,
    pub digest_hour: i16,
    pub digest_next_at: Option<DateTime<Utc>>,
    pub locale: String,
}

impl EmailAccount {
    pub fn verified(&self) -> bool {
        self.verified_at.is_some()
    }

    /// The address its profile shows: the address, while it is verified and shown.
    fn public(&self) -> Option<String> {
        (self.shown && self.verified()).then(|| self.address.clone())
    }
}

/// Checks the address a registration gives against the deployment's settings, before anything
/// costly. Returns the address to record, if one was given.
pub fn check_registration(
    state: &GlobalServerContext,
    address: Option<&str>,
) -> crate::Result<Option<String>> {
    let given = address.map(str::trim).filter(|address| !address.is_empty());
    let Some(given) = given else {
        if state.settings().email_required && available(state) {
            return Err(crate::Error::Validation(t!("emailRequired")));
        }
        return Ok(None);
    };
    if !available(state) {
        return Err(crate::Error::Validation(t!("emailUnavailable")));
    }
    parse_address(given).map(Some)
}

/// Records a new account's address inside its registration transaction, and queues its
/// verification code.
pub async fn register(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    address: &str,
    newsletter: bool,
) -> crate::Result<()> {
    let newsletter = newsletter && state.settings().newsletter_enabled;
    diesel::insert_into(user_email::table)
        .values((
            user_email::user.eq(user_id),
            user_email::address.eq(address),
            user_email::newsletter.eq(newsletter),
            user_email::locale.eq(crate::locale::current()),
            user_email::unsubscribe_token.eq(unsubscribe_token()),
        ))
        .execute(conn)
        .await?;
    queue_verification(state, conn, user_id, address).await
}

/// The caller's account, or `None` without an address.
pub async fn read(
    state: &GlobalServerContext,
    user_id: UserId,
) -> crate::Result<Option<EmailAccount>> {
    let mut conn = state.connection_pool.get().await?;
    Ok(user_email::table
        .select(EmailAccount::as_select())
        .filter(user_email::user.eq(user_id))
        .first(conn.as_mut())
        .await
        .optional()?)
}

/// Whether `user_id` has an address it has not verified, as it is now.
pub async fn unverified(state: &GlobalServerContext, user_id: UserId) -> crate::Result<bool> {
    let mut conn = state.connection_pool.get().await?;
    Ok(diesel::select(diesel::dsl::exists(
        user_email::table
            .filter(user_email::user.eq(user_id))
            .filter(user_email::verified_at.is_null()),
    ))
    .get_result(conn.as_mut())
    .await?)
}

/// Refuses what only a person of this deployment has, and anything when no mail can be sent.
fn ensure_can_have_address(state: &GlobalServerContext, caller: &Caller) -> crate::Result<()> {
    caller.ensure_person()?;
    if !available(state) {
        return Err(crate::Error::Validation(t!("emailUnavailable")));
    }
    Ok(())
}

/// Gives the caller's account `address`, or changes it, which needs a recent verification. The
/// new address is unverified, its code is sent, and the address it replaces is told if it was
/// verified. Giving the address it already has changes nothing (a new code is
/// [`resend_verification`]).
pub async fn set_address(
    state: &GlobalServerContext,
    caller: &Caller,
    given: &str,
) -> crate::Result<EmailAccount> {
    ensure_can_have_address(state, caller)?;
    caller.ensure_recently_verified(&state.config.auth)?;
    let address = parse_address(given)?;
    let user_id = caller.user;
    let mut conn = state.connection_pool.get().await?;
    let account = conn
        .transaction(|conn| {
            async move {
                let before: Option<EmailAccount> = user_email::table
                    .select(EmailAccount::as_select())
                    .filter(user_email::user.eq(user_id))
                    .for_update()
                    .first(conn)
                    .await
                    .optional()?;
                if let Some(before) = &before
                    && before.address == address
                {
                    return Ok(before.clone());
                }
                let account: EmailAccount = diesel::insert_into(user_email::table)
                    .values((
                        user_email::user.eq(user_id),
                        user_email::address.eq(&address),
                        user_email::locale.eq(crate::locale::current()),
                        user_email::unsubscribe_token.eq(unsubscribe_token()),
                    ))
                    .on_conflict(user_email::user)
                    .do_update()
                    .set((
                        user_email::address.eq(&address),
                        user_email::verified_at.eq(None::<DateTime<Utc>>),
                        user_email::locale.eq(crate::locale::current()),
                        // A new address gets new links, so the old address's mail stops
                        // working on this account.
                        user_email::unsubscribe_token.eq(unsubscribe_token()),
                    ))
                    .returning(EmailAccount::as_returning())
                    .get_result(conn)
                    .await?;
                if let Some(before) = before.filter(EmailAccount::verified) {
                    outbox::queue(
                        conn,
                        user_id,
                        Some(&before.address),
                        &outbox::Mail::AddressChanged {
                            new_address: Some(mask(&address)),
                        },
                    )
                    .await?;
                }
                queue_verification(state, conn, user_id, &address).await?;
                set_public_email(state, conn, user_id, account.public()).await?;
                announce(state, conn, user_id, !account.verified()).await?;
                Ok::<_, crate::Error>(account)
            }
            .scope_boxed()
        })
        .await?;
    wake(state).await;
    Ok(account)
}

/// Takes the caller's address away, which needs a recent verification and is refused while the
/// deployment requires one. A verified address is told.
pub async fn remove_address(state: &GlobalServerContext, caller: &Caller) -> crate::Result<()> {
    caller.ensure_person()?;
    caller.ensure_recently_verified(&state.config.auth)?;
    if state.settings().email_required {
        return Err(crate::Error::Forbidden(t!("emailRequiredCannotRemove")));
    }
    let user_id = caller.user;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let removed: Option<EmailAccount> =
                diesel::delete(user_email::table.filter(user_email::user.eq(user_id)))
                    .returning(EmailAccount::as_returning())
                    .get_result(conn)
                    .await
                    .optional()?;
            let Some(removed) = removed else {
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            };
            // Mail waiting for the address goes with it; a code reaching it would do nothing.
            diesel::delete(
                email_outbox::table
                    .filter(email_outbox::user.eq(user_id))
                    .filter(email_outbox::address.is_null()),
            )
            .execute(conn)
            .await?;
            if removed.verified() {
                outbox::queue(
                    conn,
                    user_id,
                    Some(&removed.address),
                    &outbox::Mail::AddressChanged { new_address: None },
                )
                .await?;
            }
            set_public_email(state, conn, user_id, None).await?;
            announce(state, conn, user_id, false).await
        }
        .scope_boxed()
    })
    .await?;
    let _: i64 = state.valkey.del(verification_key(user_id)).await?;
    wake(state).await;
    Ok(())
}

/// A change to what an account receives at its address and whether its profile shows it. An
/// absent field is unchanged.
#[derive(Debug, Default)]
pub struct Preferences {
    pub shown: Option<bool>,
    pub newsletter: Option<bool>,
    pub digest: Option<bool>,
    pub digest_time_zone: Option<String>,
    pub digest_hour: Option<u8>,
}

/// Changes what the caller receives and shows. Subscribing to the newsletter needs the
/// deployment to have one; a digest is sent at `digest_hour` o'clock in `digest_time_zone`, an
/// IANA name, and covers what arrived since the digest before it, or since it was turned on.
pub async fn update_preferences(
    state: &GlobalServerContext,
    caller: &Caller,
    change: Preferences,
) -> crate::Result<EmailAccount> {
    ensure_can_have_address(state, caller)?;
    if change.newsletter == Some(true) && !state.settings().newsletter_enabled {
        return Err(crate::Error::Validation(t!("newsletterOff")));
    }
    if let Some(zone) = &change.digest_time_zone
        && chrono_tz::Tz::from_str(zone).is_err()
    {
        return Err(crate::Error::Validation(t!("timeZoneUnknown", zone = zone)));
    }
    if change.digest_hour.is_some_and(|hour| hour > 23) {
        return Err(crate::Error::Validation(t!("digestHourInvalid")));
    }
    let user_id = caller.user;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let Some(before): Option<EmailAccount> = user_email::table
                .select(EmailAccount::as_select())
                .filter(user_email::user.eq(user_id))
                .for_update()
                .first(conn)
                .await
                .optional()?
            else {
                return Err(crate::Error::Validation(t!("emailAddressMissing")));
            };
            let mut after = before.clone();
            after.locale = crate::locale::current().to_string();
            if let Some(shown) = change.shown {
                after.shown = shown;
            }
            if let Some(newsletter) = change.newsletter {
                after.newsletter = newsletter;
            }
            if let Some(digest) = change.digest {
                after.digest = digest;
            }
            if let Some(zone) = change.digest_time_zone {
                after.digest_time_zone = zone;
            }
            if let Some(hour) = change.digest_hour {
                after.digest_hour = i16::from(hour);
            }
            let schedule_changed = after.digest != before.digest
                || after.digest_time_zone != before.digest_time_zone
                || after.digest_hour != before.digest_hour;
            if schedule_changed {
                after.digest_next_at = digest::next_due(&after, user_id, Utc::now());
            }
            let turned_on = after.digest && !before.digest;
            diesel::update(user_email::table.filter(user_email::user.eq(user_id)))
                .set((
                    user_email::shown.eq(after.shown),
                    user_email::newsletter.eq(after.newsletter),
                    user_email::digest.eq(after.digest),
                    user_email::digest_time_zone.eq(&after.digest_time_zone),
                    user_email::digest_hour.eq(after.digest_hour),
                    user_email::digest_next_at.eq(after.digest_next_at),
                    user_email::locale.eq(&after.locale),
                ))
                .execute(conn)
                .await?;
            if turned_on {
                diesel::update(user_email::table.filter(user_email::user.eq(user_id)))
                    .set(user_email::digest_since.eq(diesel::dsl::now))
                    .execute(conn)
                    .await?;
            }
            if after.public() != before.public() {
                set_public_email(state, conn, user_id, after.public()).await?;
            }
            announce(state, conn, user_id, !after.verified()).await?;
            Ok(after)
        }
        .scope_boxed()
    })
    .await
}

fn verification_key(user_id: UserId) -> String {
    format!("email:verify:{}", user_id.0)
}

fn verification_attempts_key(user_id: UserId) -> String {
    format!("email:verify-attempts:{}", user_id.0)
}

/// Makes a verification code for `address`, replacing any before it, and queues it.
async fn queue_verification(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    address: &str,
) -> crate::Result<()> {
    let code = code(6);
    let ttl = Some(Expiration::EX(VERIFICATION_LIFETIME.as_secs() as i64));
    let _: () = state
        .valkey
        .set(
            verification_key(user_id),
            format!("{}:{}", code_digest(&code), address),
            ttl,
            None,
            false,
        )
        .await?;
    let _: i64 = state.valkey.del(verification_attempts_key(user_id)).await?;
    // The code goes to the address it verifies, whatever the account holds when it is sent.
    outbox::queue(
        conn,
        user_id,
        Some(address),
        &outbox::Mail::Verification { code },
    )
    .await
}

/// Sends the caller a new verification code for their address.
pub async fn resend_verification(
    state: &GlobalServerContext,
    caller: &Caller,
) -> crate::Result<()> {
    ensure_can_have_address(state, caller)?;
    let user_id = caller.user;
    let mut conn = state.connection_pool.get().await?;
    let account: Option<EmailAccount> = user_email::table
        .select(EmailAccount::as_select())
        .filter(user_email::user.eq(user_id))
        .first(conn.as_mut())
        .await
        .optional()?;
    let Some(account) = account else {
        return Err(crate::Error::Validation(t!("emailAddressMissing")));
    };
    if account.verified() {
        return Err(crate::Error::Conflict(t!("emailAlreadyVerified")));
    }
    queue_verification(state, conn.as_mut(), user_id, &account.address).await?;
    wake(state).await;
    Ok(())
}

/// Verifies the caller's address with the code mailed to it. Five wrong codes end it, and a new
/// one must be sent.
pub async fn verify(
    state: &GlobalServerContext,
    caller: &Caller,
    given: &str,
) -> crate::Result<EmailAccount> {
    ensure_can_have_address(state, caller)?;
    let user_id = caller.user;
    let stored: Option<String> = state.valkey.get(verification_key(user_id)).await?;
    let Some((digest, address)) = stored.as_deref().and_then(|s| s.split_once(':')) else {
        return Err(crate::Error::Validation(t!("emailCodeExpired")));
    };
    // Counted before it is checked, so codes sent all at once are checked no more than
    // `VERIFICATION_ATTEMPTS` times.
    let attempts = crate::two_factor::count_attempt(
        &state.valkey,
        &verification_attempts_key(user_id),
        VERIFICATION_LIFETIME.as_secs() as i64,
    )
    .await?;
    if attempts > VERIFICATION_ATTEMPTS {
        let _: i64 = state.valkey.del(verification_key(user_id)).await?;
        return Err(crate::Error::TooManyAttempts);
    }
    let matches: bool = digest
        .as_bytes()
        .ct_eq(code_digest(given).as_bytes())
        .into();
    if !matches {
        if attempts >= VERIFICATION_ATTEMPTS {
            let _: i64 = state.valkey.del(verification_key(user_id)).await?;
            return Err(crate::Error::TooManyAttempts);
        }
        return Err(crate::Error::VerificationFailed);
    }
    let address = address.to_string();
    let mut conn = state.connection_pool.get().await?;
    let account = conn
        .transaction(|conn| {
            async move {
                let now = Utc::now();
                let verified: Option<EmailAccount> = diesel::update(
                    user_email::table
                        .filter(user_email::user.eq(user_id))
                        .filter(user_email::address.eq(&address)),
                )
                .set(user_email::verified_at.eq(now))
                .returning(EmailAccount::as_returning())
                .get_result(conn)
                .await
                .optional()?;
                // The code was for an address the account no longer has.
                let Some(mut verified) = verified else {
                    return Err(crate::Error::Validation(t!("emailCodeExpired")));
                };
                if verified.digest {
                    // A digest waiting on the address starts now, not at a time that passed.
                    verified.digest_next_at = digest::next_due(&verified, user_id, now);
                    diesel::update(user_email::table.filter(user_email::user.eq(user_id)))
                        .set(user_email::digest_next_at.eq(verified.digest_next_at))
                        .execute(conn)
                        .await?;
                }
                set_public_email(state, conn, user_id, verified.public()).await?;
                announce(state, conn, user_id, false).await?;
                Ok::<_, crate::Error>(verified)
            }
            .scope_boxed()
        })
        .await?;
    let _: i64 = state
        .valkey
        .del(vec![
            verification_key(user_id),
            verification_attempts_key(user_id),
        ])
        .await?;
    Ok(account)
}

/// Writes the address `user_id`'s profile shows, announcing it to everyone who sees them when it
/// changes.
pub async fn set_public_email(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    public_email: Option<String>,
) -> crate::Result<()> {
    let before: Option<String> = user::table
        .select(user::public_email)
        .filter(user::id.eq(user_id))
        .for_update()
        .first(conn)
        .await?;
    if before == public_email {
        return Ok(());
    }
    diesel::update(user::table.filter(user::id.eq(user_id)))
        .set(user::public_email.eq(&public_email))
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        EventScope::UserEverywhere(user_id),
        &ServerEvent::User(UserEvent::Update {
            id: user_id,
            name: None,
            icon: None,
            display_name: None,
            pronouns: None,
            bio: None,
            status: None,
            bot_owner: None,
            bot_public: None,
            name_hue: None,
            public_email: Some(public_email),
        }),
    )
    .await
}

/// Tells the user's other devices that their email account changed, and their event streams
/// whether it now holds an address it has not verified.
async fn announce(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    unverified: bool,
) -> crate::Result<()> {
    publish_event(
        state,
        conn,
        EventScope::User(user_id),
        &ServerEvent::EmailAccountChanged {
            user: user_id,
            unverified,
        },
    )
    .await
}

/// Wakes every sending server, once mail someone waits for has been queued and its transaction
/// committed. A failure is logged: the senders find the mail at their next look regardless.
pub async fn wake(state: &GlobalServerContext) {
    if state.mailer.is_none() {
        return;
    }
    if let Err(e) = state
        .nats_context
        .client()
        .publish(WAKE_SUBJECT, bytes::Bytes::new())
        .await
    {
        tracing::warn!(error = %e, "could not wake the mail senders");
    }
}

/// Unsubscribes the address whose links carry `token` from `list`. `false` when the token names
/// no address, which an address changed since the mail was sent no longer has.
pub async fn unsubscribe(
    state: &GlobalServerContext,
    token: &str,
    list: outbox::List,
) -> crate::Result<bool> {
    let mut conn = state.connection_pool.get().await?;
    let found = conn
        .transaction(|conn| {
            async move {
                let target = user_email::table.filter(user_email::unsubscribe_token.eq(token));
                let changed: Option<(UserId, bool)> = match list {
                    outbox::List::Newsletter => diesel::update(target)
                        .set(user_email::newsletter.eq(false))
                        .returning((user_email::user, user_email::verified_at.is_null()))
                        .get_result(conn)
                        .await
                        .optional()?,
                    outbox::List::Digest => diesel::update(target)
                        .set((
                            user_email::digest.eq(false),
                            user_email::digest_next_at.eq(None::<DateTime<Utc>>),
                        ))
                        .returning((user_email::user, user_email::verified_at.is_null()))
                        .get_result(conn)
                        .await
                        .optional()?,
                };
                if let Some((user_id, unverified)) = changed {
                    announce(state, conn, user_id, unverified).await?;
                }
                Ok::<_, crate::Error>(changed.is_some())
            }
            .scope_boxed()
        })
        .await?;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masking_shows_three_characters_and_the_domain() {
        assert_eq!(mask("alexandra@example.org"), "ale******@example.org");
        assert_eq!(mask("bob@example.org"), "bo*@example.org");
        assert_eq!(mask("j@example.org"), "*@example.org");
        assert_eq!(mask("журавль@пример.рф"), "жур****@пример.рф");
    }

    #[test]
    fn addresses_are_compared_without_case() {
        assert!(same_address(" Alex@Example.org ", "alex@example.org"));
        assert!(!same_address("alex@example.com", "alex@example.org"));
    }

    #[test]
    fn codes_have_their_digits() {
        for _ in 0..100 {
            let code = code(6);
            assert_eq!(code.len(), 6);
            assert!(code.chars().all(|c| c.is_ascii_digit()));
        }
    }

    #[test]
    fn addresses_are_checked() {
        assert_eq!(
            parse_address("  someone@example.org ").unwrap(),
            "someone@example.org"
        );
        assert!(parse_address("someone").is_err());
        assert!(parse_address("some one@example.org").is_err());
        assert!(parse_address(&format!("{}@example.org", "a".repeat(250))).is_err());
    }
}
