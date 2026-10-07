//! Invites to create an account. With the deployment setting `registration_invite_required` on
//! (`app::deployment_settings`), registering takes one: each may be used up to `max_uses` times, until `expires_at` if it has one, and not once
//! revoked. The deployment's administrators make and revoke them from the Administration
//! Dashboard (`app::admin`), and the terminal makes the first (`aspen-chat-server invites`),
//! since an invite-only deployment, made one from the terminal (`aspen-chat-server settings`),
//! has no administrator until someone registers. Each account
//! records the invite it was made with (`user.registered_with`).
//!
//! A dual invite is a registration invite that also names an invite to a community
//! (`community_invite`): the account it makes joins that community in the same transaction. Its
//! maker needs Manage registration invites and, in the community, Create invites, since the
//! community invite is theirs like any other; the community lists it among its invites, and
//! revoking it there leaves a plain registration invite, while revoking the dual invite revokes
//! both.

use crate::context::GlobalServerContext;
use crate::events::Publishing;
use crate::t;
use crate::{CommunityId, UserId};
use aspen_schema::{community, invite, registration_invite};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::collections::HashMap;

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
    /// The community invite each account it makes is joined with, for a dual invite.
    pub community_invite: Option<String>,
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

/// The start of `code`, which is all a log names of it: enough to tell invites apart, too
/// little to register with, since whoever reads the logs should not be able to.
pub fn logged_code(code: &str) -> &str {
    let end = code.char_indices().nth(4).map_or(code.len(), |(at, _)| at);
    &code[..end]
}

/// How many accounts an invite makes, for how long, and what it is for.
pub struct Terms {
    pub max_uses: i32,
    /// How long it lasts; for good when `None`.
    pub expires_in: Option<chrono::Duration>,
    pub note: Option<String>,
}

impl Terms {
    /// Checks the terms against the limits, trimming the note and dropping a blank one.
    fn validated(self) -> crate::Result<Self> {
        if !(1..=MAX_USES).contains(&self.max_uses) {
            return Err(crate::Error::Validation(t!(
                "registrationInviteUses",
                max = MAX_USES
            )));
        }
        if self
            .expires_in
            .is_some_and(|d| d <= chrono::Duration::zero() || d.num_days() > MAX_EXPIRY_DAYS)
        {
            return Err(crate::Error::Validation(t!(
                "registrationInviteExpiry",
                max = MAX_EXPIRY_DAYS
            )));
        }
        let note = self
            .note
            .map(|n| n.trim().to_string())
            .filter(|n| !n.is_empty());
        if note
            .as_ref()
            .is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS)
        {
            return Err(crate::Error::Validation(t!(
                "registrationInviteNoteLength",
                max = MAX_NOTE_CHARS
            )));
        }
        Ok(Self { note, ..self })
    }
}

/// Makes an invite on `terms`.
pub async fn create(
    conn: &mut AsyncPgConnection,
    created_by: Option<UserId>,
    terms: Terms,
) -> crate::Result<RegistrationInvite> {
    insert(conn, created_by, terms.validated()?, None).await
}

/// Makes a dual invite on `terms`: a registration invite whose accounts join `community`,
/// through an invite to it from `admin`, who needs Create invites there. The community invite
/// expires with the registration invite.
pub async fn create_dual(
    state: &GlobalServerContext,
    admin: UserId,
    community: CommunityId,
    terms: Terms,
) -> crate::Result<RegistrationInvite> {
    let terms = terms.validated()?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let expires_at = terms.expires_in.map(|d| Utc::now() + d);
            let community_invite =
                crate::invite::insert(state, conn, admin, community, None, expires_at).await?;
            insert(conn, Some(admin), terms, Some(community_invite.code)).await
        }
        .scope_boxed()
    })
    .await
}

async fn insert(
    conn: &mut AsyncPgConnection,
    created_by: Option<UserId>,
    terms: Terms,
    community_invite: Option<String>,
) -> crate::Result<RegistrationInvite> {
    let now = Utc::now();
    let invite = RegistrationInvite {
        code: crate::invite::generate_invite_code(),
        created_by,
        created_at: now,
        expires_at: terms.expires_in.map(|d| now + d),
        max_uses: terms.max_uses,
        uses: 0,
        revoked_at: None,
        note: terms.note,
        used_up_at: None,
        community_invite,
    };
    diesel::insert_into(registration_invite::table)
        .values(&invite)
        .execute(conn)
        .await?;
    Ok(invite)
}

/// The community a dual invite's accounts join, as the dashboard shows it.
#[derive(Debug, Clone)]
pub struct InvitedCommunity {
    pub id: CommunityId,
    pub name: String,
    /// The community invite's code.
    pub invite: String,
    /// Whether the community invite still works: not revoked or expired, and the community not
    /// deleted.
    pub usable: bool,
}

/// The communities `invites` lead to, by registration invite code, in one query; a plain
/// registration invite has none.
pub async fn invited_communities(
    conn: &mut AsyncPgConnection,
    invites: &[RegistrationInvite],
) -> crate::Result<HashMap<String, InvitedCommunity>> {
    let wanted: Vec<&str> = invites
        .iter()
        .filter_map(|i| i.community_invite.as_deref())
        .collect();
    if wanted.is_empty() {
        return Ok(HashMap::new());
    }
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        String,
        CommunityId,
        String,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
        Option<DateTime<Utc>>,
    )> = invite::table
        .inner_join(community::table)
        .filter(invite::code.eq_any(&wanted))
        .select((
            invite::code,
            community::id,
            community::name,
            invite::expires_at,
            invite::deleted_at,
            community::deleted_at,
        ))
        .load(conn)
        .await?;
    let now = Utc::now();
    let by_invite: HashMap<String, InvitedCommunity> = rows
        .into_iter()
        .map(
            |(code, id, name, expires_at, revoked_at, community_deleted_at)| {
                let usable = revoked_at.is_none()
                    && community_deleted_at.is_none()
                    && expires_at.is_none_or(|expires| expires > now);
                (
                    code.clone(),
                    InvitedCommunity {
                        id,
                        name,
                        invite: code,
                        usable,
                    },
                )
            },
        )
        .collect();
    Ok(invites
        .iter()
        .filter_map(|i| {
            let community = by_invite.get(i.community_invite.as_deref()?)?;
            Some((i.code.clone(), community.clone()))
        })
        .collect())
}

/// A usable invite, as `GET /registration-invites/{code}` shows it to someone about to
/// register: the community invite they would join with, if any. Not found for a code that is
/// unknown or does not work.
pub async fn read_usable(
    state: &GlobalServerContext,
    code: &str,
) -> crate::Result<RegistrationInvite> {
    let mut conn = state.connection_pool.get().await?;
    let invite: RegistrationInvite = registration_invite::table
        .select(RegistrationInvite::as_select())
        .filter(registration_invite::code.eq(code))
        .first(conn.as_mut())
        .await?;
    if !invite.usable(Utc::now()) {
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    }
    Ok(invite)
}

/// The newest `LIST_LIMIT` invites. With `include_stale` false, an invite that no longer works
/// is left out once it stopped working (`stopped_at`) more than `app::invite::STALE_AFTER_DAYS` ago, so the
/// dashboard shows what can still be used and what recently stopped; the terminal lists
/// everything.
pub async fn list(
    conn: &mut AsyncPgConnection,
    include_stale: bool,
) -> crate::Result<Vec<RegistrationInvite>> {
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
            .bind::<diesel::sql_types::Integer, _>(crate::invite::STALE_AFTER_DAYS as i32)
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

/// Revokes an invite, so it creates no more accounts, and a dual invite's community invite
/// with it; the accounts it already made are kept. Not found for an unknown code; revoking one
/// twice keeps the first time.
pub async fn revoke(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    code: &str,
) -> crate::Result<RegistrationInvite> {
    conn.transaction(|conn| {
        async move {
            let invite: RegistrationInvite = registration_invite::table
                .select(RegistrationInvite::as_select())
                .filter(registration_invite::code.eq(code))
                .for_update()
                .first(conn)
                .await?;
            if invite.revoked_at.is_some() {
                return Ok(invite);
            }
            if let Some(community_invite) = &invite.community_invite {
                crate::invite::delete(state, conn, community_invite).await?;
            }
            Ok(diesel::update(
                registration_invite::table.filter(registration_invite::code.eq(code)),
            )
            .set(registration_invite::revoked_at.eq(Some(Utc::now())))
            .returning(RegistrationInvite::as_select())
            .get_result(conn)
            .await?)
        }
        .scope_boxed()
    })
    .await
}

/// Uses `code` for one new account, on `conn`, which must be inside the registration's
/// transaction: the row is locked, so concurrent registrations cannot overdraw it. Returns the
/// community invite the account joins with, for a dual invite (`join_invited`).
pub async fn redeem(conn: &mut AsyncPgConnection, code: &str) -> crate::Result<Option<String>> {
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
            Ok(invite.community_invite)
        }
        _ => Err(crate::Error::RegistrationInviteInvalid),
    }
}

/// Adds the account `user`, just made with a dual invite, to the community of
/// `community_invite`, on `conn`, inside the transaction that made it. A community invite that
/// no longer works is passed over, since the registration invite alone still made the account;
/// the community joined is returned for `app::everyone_limit::after_join` once the transaction
/// commits.
pub async fn join_invited(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user: UserId,
    community_invite: &str,
) -> crate::Result<Option<CommunityId>> {
    let community = match crate::invite::validate_invite(conn, community_invite).await {
        Ok(community) => community,
        Err(crate::Error::Validation(_)) => return Ok(None),
        Err(e) => return Err(e),
    };
    let deleted: bool = community::table
        .find(community)
        .select(community::deleted_at.is_not_null())
        .first(conn)
        .await?;
    if deleted {
        return Ok(None);
    }
    crate::community::add_member(state, conn, user, community, &[]).await?;
    Ok(Some(community))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A log names only the start of a code, whatever its letters.
    #[test]
    fn logs_name_only_the_start_of_a_code() {
        assert_eq!(logged_code("abcdefghij"), "abcd");
        assert_eq!(logged_code("ab"), "ab");
        assert_eq!(logged_code("éèêëì"), "éèêë");
    }

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
            community_invite: None,
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
