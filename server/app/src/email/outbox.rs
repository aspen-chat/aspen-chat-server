//! Mail waiting to be sent. Whatever causes mail queues it ([`queue`]) as a job of its own
//! (`jobs::JobKind::SendEmail`) in its own transaction, so mail is sent exactly when what caused
//! it commits and survives a restart, and any server that sends mail sends it ([`send_step`]).
//!
//! Each piece's class orders it among the deployment's jobs (`Mail::class`): a password reset
//! or a verification code, which someone waits for at a screen, before notices, before digests
//! and newsletters, however many of those are waiting. A piece sent is done. One the SMTP server
//! refuses for good (a mailbox that does not exist) is given up and logged; one it refuses for
//! now is tried again later, waiting twice as long each time, up to [`MAX_ATTEMPTS`], then given
//! up too, so a piece's content is never kept once it will not be sent.
//!
//! Only servers whose `[email]` has `send` on send (`Mailer::sends`); the others queue. Mail
//! someone waits for wakes every runner once queued (`super::wake`). Where `max_per_second` is
//! set, every sender takes each piece from one GCRA bucket in Valkey ([`SEND_RATE_KEY`]) before
//! handing it over, so the deployment as a whole keeps to the provider's quota; a Valkey failure
//! lets mail through unthrottled, as the API's own limits do, and is logged.

use super::{EmailAccount, Mailer, render};
use crate::UserId;
use crate::context::GlobalServerContext;
use crate::jobs::{JobClass, JobKind};
use aspen_schema::{user, user_email};
use diesel::prelude::*;
use diesel::sql_types::Text;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use lettre::AsyncTransport;
use serde::{Deserialize, Serialize};

/// The Valkey bucket of the deployment-wide sending rate.
const SEND_RATE_KEY: &str = "email:send-rate";

/// How many times a piece is tried before it is given up: the waits between them, starting at
/// a minute and doubling, add up to about a day.
pub const MAX_ATTEMPTS: i32 = 11;

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
    /// The password was reset with a code mailed here. `removed_factors` second factors added in
    /// the week before went with it (`reset::complete`), and with `recovery_codes_gone` every
    /// recovery code too, while factors remain, so the owner is asked to make new ones.
    PasswordWasReset {
        #[serde(default)]
        removed_factors: u32,
        #[serde(default)]
        recovery_codes_gone: bool,
    },
    /// The password was changed by one of the account's sign-ins.
    PasswordChanged,
    /// A second factor was added to the account.
    SecondFactorAdded { factor: Factor },
    /// A second factor was removed from the account.
    SecondFactorRemoved { factor: Factor },
    /// So many wrong codes and passwords were given for the account that they are refused for
    /// a day (`two_factor::limited`).
    SignInLocked,
    /// What arrived for the account since its last digest.
    Digest { digest: super::digest::Digest },
    /// A newsletter post, sent to a subscriber, or to its author as a test.
    Newsletter {
        post: super::newsletter::NewsletterPostId,
        test: bool,
    },
}

/// A second factor, as mail about it names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Factor {
    AuthenticatorApp,
    /// A passkey, by the name its owner gave it.
    Passkey {
        name: String,
    },
}

impl Mail {
    /// How soon it must be sent, among every job of the deployment: a code someone waits for at
    /// a screen first, notices and tests next, and what goes to many last.
    pub(super) fn class(&self) -> JobClass {
        match self {
            Mail::PasswordReset { .. } | Mail::Verification { .. } => JobClass::Interactive,
            Mail::AddressChanged { .. }
            | Mail::PasswordWasReset { .. }
            | Mail::PasswordChanged
            | Mail::SecondFactorAdded { .. }
            | Mail::SecondFactorRemoved { .. }
            | Mail::SignInLocked
            | Mail::Newsletter { test: true, .. } => JobClass::Normal,
            Mail::Digest { .. } | Mail::Newsletter { test: false, .. } => JobClass::Bulk,
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

/// Queues `mail`, a notice about the account's security, for `user_id`'s verified address, in the
/// transaction that made the change it tells of. Nothing is queued where the deployment sends no
/// mail or the account has no verified address; one verified later is not told of what came
/// before.
pub async fn notify(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    mail: &Mail,
) -> crate::Result<()> {
    if !super::available(state) {
        return Ok(());
    }
    let verified: bool = diesel::select(diesel::dsl::exists(
        user_email::table
            .filter(user_email::user.eq(user_id))
            .filter(user_email::verified_at.is_not_null()),
    ))
    .get_result(conn)
    .await?;
    if verified {
        queue(conn, user_id, None, mail).await?;
    }
    Ok(())
}

/// One piece waiting to be sent, as its job holds it: whose it is, where to (an address of its
/// own, or the account's verified address as it is when sent), and what.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queued {
    pub user: UserId,
    #[serde(default)]
    pub address: Option<String>,
    pub mail: Mail,
}

/// The piece `mail` for `user_id` as a job to save.
pub(super) fn job_of(
    user_id: UserId,
    address: Option<&str>,
    mail: &Mail,
) -> crate::Result<crate::jobs::NewJob> {
    crate::jobs::NewJob::new(
        JobKind::SendEmail,
        mail.class(),
        &Queued {
            user: user_id,
            address: address.map(str::to_string),
            mail: mail.clone(),
        },
    )
}

/// Queues `mail` for `user_id`, to `address`, or to their verified address as it is when the
/// mail is sent. Call [`super::wake`] once the transaction commits for mail someone waits on.
pub async fn queue(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    address: Option<&str>,
    mail: &Mail,
) -> crate::Result<()> {
    crate::jobs::enqueue(conn, job_of(user_id, address, mail)?).await?;
    Ok(())
}

/// Deletes the mail waiting for `user_id`, to their account's address alone when
/// `account_address_only`, as the account or its address goes (`job_send_email_user`).
pub async fn forget_queued(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    account_address_only: bool,
) -> crate::Result<()> {
    diesel::sql_query(
        "DELETE FROM job WHERE kind = 'sendEmail' AND payload->>'user' = $1 \
         AND (NOT $2 OR payload->>'address' IS NULL)",
    )
    .bind::<Text, _>(user_id.0.to_string())
    .bind::<diesel::sql_types::Bool, _>(account_address_only)
    .execute(conn)
    .await?;
    Ok(())
}

/// Waits until the deployment's sending rate allows one more piece.
async fn throttle(state: &GlobalServerContext, mailer: &Mailer) {
    let Some(rate) = mailer.rate else {
        return;
    };
    loop {
        match crate::rate_limit::take(&state.valkey, SEND_RATE_KEY, rate).await {
            Ok(None) => return,
            Ok(Some(wait)) => tokio::time::sleep(wait).await,
            Err(e) => {
                tracing::error!(error = %e, "could not read the sending rate; sending anyway");
                return;
            }
        }
    }
}

/// Sends one piece (`jobs::JobKind::SendEmail`): done once sent, or once it will never be
/// (refused for good, nowhere to send it, or tried [`MAX_ATTEMPTS`] times); a failure for now
/// is tried again after [`retry_wait`].
pub async fn send_step(
    state: &GlobalServerContext,
    job: &crate::jobs::Claimed,
) -> crate::Result<crate::jobs::Outcome> {
    let Some(mailer) = state.mailer.clone().filter(|mailer| mailer.sends()) else {
        return Ok(crate::jobs::Outcome::Later(std::time::Duration::from_secs(
            60,
        )));
    };
    let queued: Queued = job.payload()?;
    match send_one(state, &mailer, queued).await {
        Outcome::Done => {}
        Outcome::Refused(reason) => {
            metrics::counter!(aspen_metrics::api::EMAILS_FAILED).increment(1);
            tracing::warn!(%reason, "mail was refused; giving it up");
        }
        Outcome::Later(reason) if job.attempts >= MAX_ATTEMPTS => {
            metrics::counter!(aspen_metrics::api::EMAILS_FAILED).increment(1);
            tracing::error!(%reason, attempts = job.attempts, "mail could not be sent; giving it up");
        }
        Outcome::Later(reason) => return Err(crate::Error::MailUnsent(reason)),
    }
    Ok(crate::jobs::Outcome::Done)
}

/// What became of one piece.
enum Outcome {
    /// Sent, or nothing left to send (the address gone or no longer verified).
    Done,
    /// Refused for good.
    Refused(String),
    /// Could not be sent now.
    Later(String),
}

async fn send_one(state: &GlobalServerContext, mailer: &Mailer, queued: Queued) -> Outcome {
    let Queued {
        user,
        address,
        mail,
    } = queued;
    let recipient = match recipient(state, user, address.as_deref(), &mail).await {
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
) -> crate::Result<Option<Recipient>> {
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
            locale: account.map_or_else(|| crate::locale::DEFAULT.to_string(), |a| a.locale),
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

/// How long to wait after the `attempts`th failure: a minute, doubling each time.
pub fn retry_wait(attempts: i32) -> chrono::Duration {
    let doublings = u32::try_from(attempts.saturating_sub(1))
        .unwrap_or(0)
        .min(16);
    chrono::Duration::minutes(1i64 << doublings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_someone_waits_for_are_sent_before_everything_else() {
        let reset = Mail::PasswordReset {
            code: "12345678".to_string(),
        };
        let code = Mail::Verification {
            code: "123456".to_string(),
        };
        assert_eq!(reset.class(), code.class());
        for other in [
            Mail::PasswordWasReset {
                removed_factors: 0,
                recovery_codes_gone: false,
            },
            Mail::AddressChanged { new_address: None },
            Mail::Newsletter {
                post: super::super::newsletter::NewsletterPostId::new(),
                test: false,
            },
        ] {
            assert!(reset.class() < other.class());
        }
    }

    fn mailer(send: bool, max_per_second: Option<u32>) -> Mailer {
        Mailer::new(
            &crate::aspen_config::EmailConfig {
                smtp_url: send.then(|| "smtp://localhost:1025".to_string()),
                send,
                max_per_second,
                from: "Aspen <noreply@example.org>".to_string(),
                tls: None,
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

    #[test]
    fn waits_double_and_add_up_to_about_a_day() {
        assert_eq!(retry_wait(1), chrono::Duration::minutes(1));
        assert_eq!(retry_wait(2), chrono::Duration::minutes(2));
        let total: i64 = (1..MAX_ATTEMPTS).map(|n| retry_wait(n).num_minutes()).sum();
        assert!((12 * 60..48 * 60).contains(&total), "{total} minutes");
    }

    #[test]
    fn a_password_reset_notice_queued_without_its_count_still_reads() {
        let queued = serde_json::json!({ "kind": "passwordWasReset" });
        assert_eq!(
            serde_json::from_value::<Mail>(queued).unwrap(),
            Mail::PasswordWasReset {
                removed_factors: 0,
                recovery_codes_gone: false,
            }
        );
        let mail = Mail::SecondFactorRemoved {
            factor: Factor::Passkey {
                name: "Phone".to_string(),
            },
        };
        let json = serde_json::to_value(&mail).unwrap();
        assert_eq!(json["factor"]["kind"], "passkey");
        assert_eq!(serde_json::from_value::<Mail>(json).unwrap(), mail);
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
