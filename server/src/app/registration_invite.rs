//! Invites to create an account. With `[registration] invite_required` set, registering takes
//! one: each may be used up to `max_uses` times, until `expires_at` if it has one, and not once
//! revoked. The deployment's administrators make and revoke them from the Administration
//! Dashboard (`app::admin`), and the terminal makes the first (`aspen-chat-server invites`),
//! since an invite-only deployment has no administrator until someone registers. Each account
//! records the invite it was made with (`user.registered_with`).

use crate::app::{self, UserId};
use crate::database::schema::registration_invite;
use crate::t;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

/// The most accounts one invite may create.
pub const MAX_USES: i32 = 1000;
/// The longest an invite may last, in days.
pub const MAX_EXPIRY_DAYS: i64 = 365;
/// The longest note an invite may carry, in characters.
pub const MAX_NOTE_CHARS: usize = 200;
/// How many invites a listing returns, newest first.
pub const LIST_LIMIT: i64 = 500;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = registration_invite)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct RegistrationInvite {
    pub code: String,
    /// The administrator who made it; `None` for one made from the terminal, or whose maker's
    /// account is gone.
    pub created_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub max_uses: i32,
    pub uses: i32,
    pub revoked_at: Option<DateTime<Utc>>,
    /// What the administrator wrote to remember it by, such as who it was for.
    pub note: Option<String>,
    /// When its last use was taken; `None` while it has uses left.
    pub used_up_at: Option<DateTime<Utc>>,
}

impl RegistrationInvite {
    /// Whether it would create an account at `now`.
    pub fn usable(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none()
            && self.uses < self.max_uses
            && self.expires_at.is_none_or(|expires| expires > now)
    }

    /// When it stopped working, as of `now`: the first of being revoked, being used up, and
    /// expiring; `None` while it works.
    pub fn stopped_at(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        [
            self.revoked_at,
            self.used_up_at,
            self.expires_at.filter(|expires| *expires <= now),
        ]
        .into_iter()
        .flatten()
        .min()
    }
}

/// Makes an invite for `max_uses` accounts, lasting `expires_in` when given.
pub async fn create(
    conn: &mut AsyncPgConnection,
    created_by: Option<UserId>,
    max_uses: i32,
    expires_in: Option<chrono::Duration>,
    note: Option<String>,
) -> app::Result<RegistrationInvite> {
    if !(1..=MAX_USES).contains(&max_uses) {
        return Err(app::Error::Validation(t!(
            "registrationInviteUses",
            max = MAX_USES
        )));
    }
    if expires_in.is_some_and(|d| d <= chrono::Duration::zero() || d.num_days() > MAX_EXPIRY_DAYS) {
        return Err(app::Error::Validation(t!(
            "registrationInviteExpiry",
            max = MAX_EXPIRY_DAYS
        )));
    }
    let note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    if note
        .as_ref()
        .is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS)
    {
        return Err(app::Error::Validation(t!(
            "registrationInviteNoteLength",
            max = MAX_NOTE_CHARS
        )));
    }
    let now = Utc::now();
    let invite = RegistrationInvite {
        code: app::invite::generate_invite_code(),
        created_by,
        created_at: now,
        expires_at: expires_in.map(|d| now + d),
        max_uses,
        uses: 0,
        revoked_at: None,
        note,
        used_up_at: None,
    };
    diesel::insert_into(registration_invite::table)
        .values(&invite)
        .execute(conn)
        .await?;
    Ok(invite)
}

/// The newest `LIST_LIMIT` invites. With `include_stale` false, an invite that no longer works
/// is left out once it stopped working (`stopped_at`) more than `app::invite::STALE_AFTER_DAYS` ago, so the
/// dashboard shows what can still be used and what recently stopped; the terminal lists
/// everything.
pub async fn list(
    conn: &mut AsyncPgConnection,
    include_stale: bool,
) -> app::Result<Vec<RegistrationInvite>> {
    let mut query = registration_invite::table
        .select(RegistrationInvite::as_select())
        .into_boxed();
    if !include_stale {
        // Usable, or stopped recently: the same rule as `usable` and `stopped_at`, in SQL.
        query = query.filter(
            diesel::dsl::sql::<diesel::sql_types::Bool>(
                "(revoked_at IS NULL AND uses < max_uses \
                  AND (expires_at IS NULL OR expires_at > now())) \
                 OR LEAST(revoked_at, used_up_at, \
                          CASE WHEN expires_at <= now() THEN expires_at END) \
                    > now() - make_interval(days => ",
            )
            .bind::<diesel::sql_types::Integer, _>(app::invite::STALE_AFTER_DAYS as i32)
            .sql(")"),
        );
    }
    Ok(query
        .order((
            registration_invite::created_at.desc(),
            registration_invite::code.desc(),
        ))
        .limit(LIST_LIMIT)
        .load(conn)
        .await?)
}

/// Revokes an invite, so it creates no more accounts; those it already made are kept. Not
/// found for an unknown code; revoking one twice keeps the first time.
pub async fn revoke(conn: &mut AsyncPgConnection, code: &str) -> app::Result<RegistrationInvite> {
    let invite: RegistrationInvite = registration_invite::table
        .select(RegistrationInvite::as_select())
        .filter(registration_invite::code.eq(code))
        .first(conn)
        .await?;
    if invite.revoked_at.is_some() {
        return Ok(invite);
    }
    Ok(
        diesel::update(registration_invite::table.filter(registration_invite::code.eq(code)))
            .set(registration_invite::revoked_at.eq(Some(Utc::now())))
            .returning(RegistrationInvite::as_select())
            .get_result(conn)
            .await?,
    )
}

/// Uses `code` for one new account, on `conn`, which must be inside the registration's
/// transaction: the row is locked, so concurrent registrations cannot overdraw it.
pub async fn redeem(conn: &mut AsyncPgConnection, code: &str) -> app::Result<()> {
    let invite: Option<RegistrationInvite> = registration_invite::table
        .select(RegistrationInvite::as_select())
        .filter(registration_invite::code.eq(code))
        .for_update()
        .first(conn)
        .await
        .optional()?;
    match invite {
        Some(invite) if invite.usable(Utc::now()) => {
            // Taking the last use records when the invite stopped working.
            let used_up = invite.uses + 1 >= invite.max_uses;
            diesel::update(registration_invite::table.filter(registration_invite::code.eq(code)))
                .set((
                    registration_invite::uses.eq(registration_invite::uses + 1),
                    registration_invite::used_up_at.eq(used_up.then(Utc::now)),
                ))
                .execute(conn)
                .await?;
            Ok(())
        }
        _ => Err(app::Error::RegistrationInviteInvalid),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite() -> RegistrationInvite {
        RegistrationInvite {
            code: "code".to_string(),
            created_by: None,
            created_at: Utc::now(),
            expires_at: None,
            max_uses: 2,
            uses: 0,
            revoked_at: None,
            note: None,
            used_up_at: None,
        }
    }

    #[test]
    fn an_invite_is_usable_until_used_up_expired_or_revoked() {
        let now = Utc::now();
        assert!(invite().usable(now));
        assert!(
            !RegistrationInvite {
                uses: 2,
                ..invite()
            }
            .usable(now)
        );
        assert!(
            !RegistrationInvite {
                expires_at: Some(now),
                ..invite()
            }
            .usable(now)
        );
        assert!(
            RegistrationInvite {
                expires_at: Some(now + chrono::Duration::seconds(1)),
                ..invite()
            }
            .usable(now)
        );
        assert!(
            !RegistrationInvite {
                revoked_at: Some(now),
                ..invite()
            }
            .usable(now)
        );
    }

    #[test]
    fn an_invite_stops_working_at_the_first_of_revoked_used_up_and_expired() {
        let now = Utc::now();
        let hours = |n: i64| now - chrono::Duration::hours(n);
        assert_eq!(invite().stopped_at(now), None);
        let later_expiry = RegistrationInvite {
            expires_at: Some(now + chrono::Duration::hours(1)),
            ..invite()
        };
        assert_eq!(later_expiry.stopped_at(now), None);
        let spent_then_revoked = RegistrationInvite {
            uses: 2,
            used_up_at: Some(hours(5)),
            revoked_at: Some(hours(2)),
            ..invite()
        };
        assert_eq!(spent_then_revoked.stopped_at(now), Some(hours(5)));
        let expired = RegistrationInvite {
            expires_at: Some(hours(3)),
            revoked_at: Some(hours(1)),
            ..invite()
        };
        assert_eq!(expired.stopped_at(now), Some(hours(3)));
    }
}
