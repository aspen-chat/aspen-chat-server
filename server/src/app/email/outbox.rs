//! Mail waiting to be sent. Whatever causes mail writes it to `email_outbox` in its own
//! transaction ([`queue`]), so mail is sent exactly when what caused it commits and survives a
//! restart, and every API server sends from the table ([`spawn_sender`]).
//!
//! A sender claims a batch of rows by pushing their `not_before` past the time it needs to send
//! them, under `FOR UPDATE SKIP LOCKED`, so servers sending at once never claim one row twice,
//! and a server that dies mid-batch leaves its rows to be claimed again once that time passes.
//! It takes the highest `priority` first: a password reset, which someone is waiting for at the
//! sign-in screen, before a verification code, before notices, before digests and newsletters,
//! however many of those are waiting. A row sent is deleted. A row the SMTP server refuses for
//! good (a mailbox that does not exist) is deleted and logged; one it refuses for now is tried
//! again later, waiting twice as long each time, up to [`MAX_ATTEMPTS`].
//!
//! Only servers whose `[email]` has `send` on send (`Mailer::sends`); the others queue. Each
//! sender looks for mail every few seconds, and at once when any server publishes on
//! [`super::WAKE_SUBJECT`], which it does after queueing mail someone waits for. Where
//! `max_per_second` is set, every sender takes each piece from one GCRA bucket in Valkey
//! ([`SEND_RATE_KEY`]) before handing it over, so the deployment as a whole keeps to the
//! provider's quota, and claims no more than a second's worth at a time, so a reset queued
//! meanwhile waits at most about a second per sending server; a Valkey failure lets mail through
//! unthrottled, as the API's own limits do, and is logged.

use super::{EmailAccount, Mailer, render};
use crate::app::context::GlobalServerContext;
use crate::app::{self, UserId};
use crate::database::schema::{email_outbox, user, user_email};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Integer, Jsonb, Nullable, SmallInt, Text, Uuid as PgUuid};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use futures_util::StreamExt;
use lettre::AsyncTransport;
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// How often a server looks for mail when nothing wakes it.
const POLL: Duration = Duration::from_secs(5);
/// How long a claimed row is held for its sender.
const CLAIM_SECONDS: i32 = 300;
/// How many rows one look claims, and how many of them are sent at once.
const BATCH: i64 = 64;
const CONCURRENCY: usize = 8;
/// The Valkey bucket of the deployment-wide sending rate.
const SEND_RATE_KEY: &str = "email:send-rate";

/// How many times a row is tried before it is given up: the waits between them, starting at a
/// minute and doubling, add up to about a day.
const MAX_ATTEMPTS: i32 = 11;

/// A mailing list an address can leave by the links its mail carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::EnumString)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "camelCase")]
pub enum List {
    Newsletter,
    Digest,
}

/// A piece of mail, as the outbox keeps it until it is written out in its reader's language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Mail {
    /// The code that verifies the address it is sent to.
    Verification { code: String },
    /// The account's address changed to `new_address` (masked), or was removed; sent to the
    /// address it had.
    AddressChanged { new_address: Option<String> },
    /// The code that resets the password, asked for at the sign-in screen.
    PasswordReset { code: String },
    /// The password was reset with a code mailed here.
    PasswordWasReset,
    /// What arrived for the account since its last digest.
    Digest { digest: super::digest::Digest },
    /// A newsletter post, sent to a subscriber, or to its author as a test.
    Newsletter {
        post: super::newsletter::NewsletterPostId,
        test: bool,
    },
}

impl Mail {
    /// Higher is sent first.
    fn priority(&self) -> i16 {
        match self {
            Mail::PasswordReset { .. } => 40,
            Mail::Verification { .. } => 30,
            Mail::AddressChanged { .. } | Mail::PasswordWasReset => 20,
            Mail::Newsletter { test: true, .. } => 15,
            Mail::Digest { .. } => 10,
            Mail::Newsletter { test: false, .. } => 0,
        }
    }

    /// The list its reader may leave, which its links and headers offer.
    pub(super) fn list(&self) -> Option<List> {
        match self {
            Mail::Digest { .. } => Some(List::Digest),
            Mail::Newsletter { test: false, .. } => Some(List::Newsletter),
            _ => None,
        }
    }

    /// Whether it may go to an address that is not verified: only the code that verifies it.
    fn needs_verified(&self) -> bool {
        !matches!(self, Mail::Verification { .. })
    }
}

/// Queues `mail` for `user_id`, to `address`, or to their verified address as it is when the
/// mail is sent. Call [`super::wake`] once the transaction commits for mail someone waits on.
pub async fn queue(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    address: Option<&str>,
    mail: &Mail,
) -> app::Result<()> {
    diesel::insert_into(email_outbox::table)
        .values((
            email_outbox::id.eq(uuid::Uuid::now_v7()),
            email_outbox::priority.eq(mail.priority()),
            email_outbox::user.eq(user_id),
            email_outbox::address.eq(address),
            email_outbox::mail.eq(serde_json::to_value(mail)?),
        ))
        .execute(conn)
        .await?;
    Ok(())
}

#[derive(QueryableByName)]
struct Claimed {
    #[diesel(sql_type = PgUuid)]
    id: uuid::Uuid,
    #[diesel(sql_type = PgUuid)]
    user: UserId,
    #[diesel(sql_type = Nullable<Text>)]
    address: Option<String>,
    #[diesel(sql_type = Jsonb)]
    mail: serde_json::Value,
    #[diesel(sql_type = Integer)]
    attempts: i32,
    #[diesel(sql_type = SmallInt)]
    priority: i16,
}

/// Starts sending mail from the outbox, for as long as the server runs, where it sends.
pub fn spawn_sender(state: GlobalServerContext) {
    let Some(mailer) = state.mailer.clone().filter(|mailer| mailer.sends()) else {
        return;
    };
    spawn_wake_listener(state.clone(), mailer.clone());
    tokio::spawn(async move {
        loop {
            match send_batch(&state, &mailer).await {
                // A full batch suggests more is waiting.
                Ok(sent) if sent as i64 == claim_size(&mailer) => continue,
                Ok(_) => {}
                Err(e) => tracing::error!(error = %e, "could not send mail from the outbox"),
            }
            if let Err(e) = super::newsletter::queue_some(&state).await {
                tracing::error!(error = %e, "could not queue a newsletter");
            }
            tokio::select! {
                () = mailer.wake.notified() => {}
                () = tokio::time::sleep(POLL) => {}
            }
        }
    });
}

/// Wakes this server's sender whenever a server says mail someone waits for was queued.
fn spawn_wake_listener(state: GlobalServerContext, mailer: std::sync::Arc<Mailer>) {
    tokio::spawn(async move {
        loop {
            match state
                .nats_context
                .client()
                .subscribe(super::WAKE_SUBJECT)
                .await
            {
                Ok(mut wakes) => {
                    while wakes.next().await.is_some() {
                        mailer.wake.notify_one();
                    }
                }
                Err(e) => tracing::warn!(error = %e, "could not listen for mail to send"),
            }
            tokio::time::sleep(POLL).await;
        }
    });
}

/// How many rows one look claims: [`BATCH`], or a second's worth under a sending rate.
fn claim_size(mailer: &Mailer) -> i64 {
    match mailer.rate {
        Some(rate) => (1000 / rate.emission_ms.max(1)).clamp(1, BATCH as u64) as i64,
        None => BATCH,
    }
}

/// Waits until the deployment's sending rate allows one more piece.
async fn throttle(state: &GlobalServerContext, mailer: &Mailer) {
    let Some(rate) = mailer.rate else {
        return;
    };
    loop {
        match crate::app::rate_limit::take(&state.valkey, SEND_RATE_KEY, rate).await {
            Ok(None) => return,
            Ok(Some(wait)) => tokio::time::sleep(wait).await,
            Err(e) => {
                tracing::error!(error = %e, "could not read the sending rate; sending anyway");
                return;
            }
        }
    }
}

/// Claims and sends one batch, answering how many rows it claimed.
async fn send_batch(state: &GlobalServerContext, mailer: &Mailer) -> app::Result<usize> {
    let claimed: Vec<Claimed> = {
        let mut conn = state.connection_pool.get().await?;
        diesel::sql_query(
            r#"
            UPDATE email_outbox
            SET not_before = now() + make_interval(secs => $1), attempts = attempts + 1
            WHERE id IN (
                SELECT id FROM email_outbox
                WHERE not_before <= now()
                ORDER BY priority DESC, not_before, id
                LIMIT $2
                FOR UPDATE SKIP LOCKED
            )
            RETURNING id, "user", address, mail, attempts, priority
            "#,
        )
        .bind::<Integer, _>(CLAIM_SECONDS)
        .bind::<diesel::sql_types::BigInt, _>(claim_size(mailer))
        .load(conn.as_mut())
        .await?
    };
    let count = claimed.len();
    // The most urgent first, however the update returned them.
    let mut claimed = claimed;
    claimed.sort_by_key(|row| std::cmp::Reverse(row.priority));
    futures_util::stream::iter(claimed)
        .for_each_concurrent(CONCURRENCY, |row| async move {
            let id = row.id;
            let attempts = row.attempts;
            let outcome = send_one(state, mailer, row).await;
            if let Err(e) = settle(state, id, attempts, outcome).await {
                tracing::error!(error = %e, "could not record a mail's outcome");
            }
        })
        .await;
    Ok(count)
}

/// What became of one row.
enum Outcome {
    /// Sent, or nothing left to send (the address gone or no longer verified).
    Done,
    /// Refused for good.
    Refused(String),
    /// Could not be sent now.
    Later(String),
}

async fn send_one(state: &GlobalServerContext, mailer: &Mailer, row: Claimed) -> Outcome {
    let mail: Mail = match serde_json::from_value(row.mail) {
        Ok(mail) => mail,
        Err(e) => return Outcome::Refused(format!("unreadable mail: {e}")),
    };
    let recipient = match recipient(state, row.user, row.address.as_deref(), &mail).await {
        Ok(Some(recipient)) => recipient,
        Ok(None) => return Outcome::Done,
        Err(e) => return Outcome::Later(e.to_string()),
    };
    let message = match render::message(state, mailer, &recipient, &mail).await {
        Ok(Some(message)) => message,
        Ok(None) => return Outcome::Done,
        Err(e) => return Outcome::Later(e.to_string()),
    };
    let Some(transport) = &mailer.transport else {
        return Outcome::Later("this server does not send mail".to_string());
    };
    throttle(state, mailer).await;
    match transport.send(message).await {
        Ok(_) => {
            metrics::counter!(aspen_metrics::api::EMAILS_SENT).increment(1);
            Outcome::Done
        }
        Err(e) if e.is_permanent() => Outcome::Refused(e.to_string()),
        Err(e) => Outcome::Later(e.to_string()),
    }
}

/// Who a row goes to, as its mail is written for them.
pub(super) struct Recipient {
    pub user: UserId,
    pub address: String,
    pub locale: String,
    /// The secret the unsubscribe links carry.
    pub unsubscribe_token: String,
}

/// Where a row goes: the address it names, or the account's verified address. `None` when there
/// is nowhere to send it any more: the account is gone, or its address is, or is not verified
/// and the mail needs it to be, or it left the list the mail is for.
async fn recipient(
    state: &GlobalServerContext,
    user_id: UserId,
    address: Option<&str>,
    mail: &Mail,
) -> app::Result<Option<Recipient>> {
    let mut conn = state.connection_pool.get().await?;
    let found: Option<(Option<EmailAccount>, Option<String>)> = user::table
        .left_join(user_email::table)
        .select((
            Option::<EmailAccount>::as_select(),
            user_email::unsubscribe_token.nullable(),
        ))
        .filter(user::id.eq(user_id))
        .filter(user::deleted_at.is_null())
        .first(conn.as_mut())
        .await
        .optional()?;
    // Nothing goes to an account that is gone.
    let Some((account, unsubscribe_token)) = found else {
        return Ok(None);
    };
    // Mail naming its own address (a code for it, or a notice to the address an account left)
    // goes there whatever the account holds now, except a code for an address it no longer has.
    if let Some(address) = address {
        if matches!(mail, Mail::Verification { .. })
            && account
                .as_ref()
                .is_none_or(|account| account.address != address)
        {
            return Ok(None);
        }
        return Ok(Some(Recipient {
            user: user_id,
            address: address.to_string(),
            locale: account.map_or_else(|| app::locale::DEFAULT.to_string(), |a| a.locale),
            unsubscribe_token: unsubscribe_token.unwrap_or_default(),
        }));
    }
    let (Some(account), Some(unsubscribe_token)) = (account, unsubscribe_token) else {
        return Ok(None);
    };
    let left = match mail.list() {
        Some(List::Newsletter) => !account.newsletter,
        Some(List::Digest) => !account.digest,
        None => false,
    };
    if left || (mail.needs_verified() && !account.verified()) {
        return Ok(None);
    }
    Ok(Some(Recipient {
        user: user_id,
        address: account.address,
        locale: account.locale,
        unsubscribe_token,
    }))
}

/// Deletes a row that is done with, or puts it off until its next attempt.
async fn settle(
    state: &GlobalServerContext,
    id: uuid::Uuid,
    attempts: i32,
    outcome: Outcome,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let row = email_outbox::table.filter(email_outbox::id.eq(id));
    match outcome {
        Outcome::Done => {}
        Outcome::Refused(reason) => {
            metrics::counter!(aspen_metrics::api::EMAILS_FAILED).increment(1);
            tracing::warn!(%reason, "mail was refused; giving it up");
        }
        Outcome::Later(reason) if attempts >= MAX_ATTEMPTS => {
            metrics::counter!(aspen_metrics::api::EMAILS_FAILED).increment(1);
            tracing::error!(%reason, attempts, "mail could not be sent; giving it up");
        }
        Outcome::Later(reason) => {
            let wait = retry_wait(attempts);
            tracing::warn!(%reason, attempts, ?wait, "mail could not be sent; trying again later");
            let at: DateTime<Utc> = Utc::now() + wait;
            diesel::update(row)
                .set(email_outbox::not_before.eq(at))
                .execute(conn.as_mut())
                .await?;
            return Ok(());
        }
    }
    diesel::delete(row).execute(conn.as_mut()).await?;
    Ok(())
}

/// How long to wait after the `attempts`th failure: a minute, doubling each time.
fn retry_wait(attempts: i32) -> chrono::Duration {
    let doublings = u32::try_from(attempts.saturating_sub(1))
        .unwrap_or(0)
        .min(16);
    chrono::Duration::minutes(1i64 << doublings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resets_are_sent_before_everything_else() {
        let reset = Mail::PasswordReset {
            code: "12345678".to_string(),
        };
        for other in [
            Mail::Verification {
                code: "123456".to_string(),
            },
            Mail::PasswordWasReset,
            Mail::AddressChanged { new_address: None },
            Mail::Newsletter {
                post: super::super::newsletter::NewsletterPostId::new(),
                test: false,
            },
        ] {
            assert!(reset.priority() > other.priority());
        }
    }

    fn mailer(send: bool, max_per_second: Option<u32>) -> Mailer {
        Mailer::new(
            &crate::aspen_config::EmailConfig {
                smtp_url: send.then(|| "smtp://localhost:1025".to_string()),
                send,
                max_per_second,
                from: "Aspen <noreply@example.org>".to_string(),
            },
            "https://chat.example.org",
        )
        .unwrap()
    }

    // The SMTP transport's connection pool starts on the runtime it is made in.
    #[tokio::test]
    async fn only_a_server_that_sends_has_a_transport() {
        assert!(mailer(true, None).sends());
        assert!(!mailer(false, None).sends());
    }

    #[tokio::test]
    async fn a_sending_rate_claims_a_second_s_worth() {
        assert_eq!(claim_size(&mailer(true, None)), BATCH);
        assert_eq!(claim_size(&mailer(true, Some(10))), 10);
        assert_eq!(claim_size(&mailer(true, Some(1))), 1);
        assert_eq!(claim_size(&mailer(true, Some(10_000))), BATCH);
    }

    #[test]
    fn waits_double_and_add_up_to_about_a_day() {
        assert_eq!(retry_wait(1), chrono::Duration::minutes(1));
        assert_eq!(retry_wait(2), chrono::Duration::minutes(2));
        let total: i64 = (1..MAX_ATTEMPTS).map(|n| retry_wait(n).num_minutes()).sum();
        assert!((12 * 60..48 * 60).contains(&total), "{total} minutes");
    }

    #[test]
    fn mail_round_trips_through_json() {
        let mail = Mail::AddressChanged {
            new_address: Some("ale***@example.org".to_string()),
        };
        let json = serde_json::to_value(&mail).unwrap();
        assert_eq!(json["kind"], "addressChanged");
        assert_eq!(json["newAddress"], "ale***@example.org");
        assert_eq!(serde_json::from_value::<Mail>(json).unwrap(), mail);
    }
}
