//! Identities at providers linked to accounts: one account per identity, matched by the provider
//! and its `sub` alone, and at most one identity of each provider per account.

use super::Provider;
use crate::context::GlobalServerContext;
use crate::two_factor::Caller;
use crate::{IdentityProviderId, UserId, UserIdentityId, t};
use aspen_schema::{identity_provider, user, user_identity};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

/// A linked identity as its account sees it.
#[derive(Clone, Debug)]
pub struct IdentitySummary {
    pub id: UserIdentityId,
    pub provider: IdentityProviderId,
    pub provider_name: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
}

type SummaryRow = (
    UserIdentityId,
    IdentityProviderId,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

fn summary((id, provider, provider_name, created_at, last_used_at): SummaryRow) -> IdentitySummary {
    IdentitySummary {
        id,
        provider,
        provider_name,
        created_at,
        last_used_at,
    }
}

/// The account `subject` at `provider` is linked to, if it is live.
pub async fn account_of(
    conn: &mut AsyncPgConnection,
    provider: IdentityProviderId,
    subject: &str,
) -> crate::Result<Option<UserId>> {
    Ok(user_identity::table
        .inner_join(user::table)
        .select(user_identity::user)
        .filter(user_identity::provider.eq(provider))
        .filter(user_identity::subject.eq(subject))
        .filter(user::deleted_at.is_null())
        .first(conn)
        .await
        .optional()?)
}

/// `user`'s identity at `provider`, if they linked one.
pub async fn of(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    provider: IdentityProviderId,
) -> crate::Result<Option<UserIdentityId>> {
    Ok(user_identity::table
        .select(user_identity::id)
        .filter(user_identity::user.eq(user_id))
        .filter(user_identity::provider.eq(provider))
        .first(conn)
        .await
        .optional()?)
}

/// Records that `subject` at `provider` signed in.
pub async fn used(
    conn: &mut AsyncPgConnection,
    provider: IdentityProviderId,
    subject: &str,
) -> crate::Result<()> {
    diesel::update(
        user_identity::table
            .filter(user_identity::provider.eq(provider))
            .filter(user_identity::subject.eq(subject)),
    )
    .set(user_identity::last_used_at.eq(diesel::dsl::now))
    .execute(conn)
    .await?;
    Ok(())
}

/// Links `subject` at `provider` to `user_id`, refusing an identity another account holds and a
/// second identity of one provider.
pub async fn insert(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    provider: IdentityProviderId,
    subject: &str,
) -> crate::Result<UserIdentityId> {
    let id = UserIdentityId::new();
    diesel::insert_into(user_identity::table)
        .values((
            user_identity::id.eq(id),
            user_identity::user.eq(user_id),
            user_identity::provider.eq(provider),
            user_identity::subject.eq(subject),
            user_identity::last_used_at.eq(diesel::dsl::now),
        ))
        .execute(conn)
        .await
        .map_err(|error| match &error {
            diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                info,
            ) if info.constraint_name() == Some("user_identity_provider_subject_key") => {
                crate::Error::IdentityTaken
            }
            diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            ) => crate::Error::Conflict(t!("identityProviderAlreadyLinkedGenerally")),
            _ => error.into(),
        })?;
    Ok(id)
}

/// Links `subject` at `provider` to `user_id`, as a claimed `link` flow does.
pub async fn link(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    provider: &Provider,
    subject: &str,
) -> crate::Result<IdentitySummary> {
    let id = insert(conn, user_id, provider.id, subject).await?;
    Ok(IdentitySummary {
        id,
        provider: provider.id,
        provider_name: provider.name.clone(),
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
    })
}

/// `user_id`'s linked identities at providers that are not removed. An account holds at most
/// one per provider, and a deployment at most `MAX_PROVIDERS`, so the list is short.
pub async fn list(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
) -> crate::Result<Vec<IdentitySummary>> {
    let rows: Vec<SummaryRow> = user_identity::table
        .inner_join(identity_provider::table)
        .select((
            user_identity::id,
            user_identity::provider,
            identity_provider::name,
            user_identity::created_at,
            user_identity::last_used_at,
        ))
        .filter(user_identity::user.eq(user_id))
        .filter(identity_provider::removed_at.is_null())
        .order(user_identity::created_at.asc())
        .limit(super::MAX_PROVIDERS)
        .load(conn)
        .await?;
    Ok(rows.into_iter().map(summary).collect())
}

/// Unlinks one of the caller's identities. It needs a recent verification, and is refused when
/// the account would be left no way in (`lastSignInMethod`). The sign-ins it made stay.
pub async fn unlink(
    state: &GlobalServerContext,
    caller: &Caller,
    id: UserIdentityId,
) -> crate::Result<()> {
    caller.ensure_recently_verified(&state.config.auth)?;
    let user_id = caller.user;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let removed = diesel::delete(
                user_identity::table
                    .filter(user_identity::id.eq(id))
                    .filter(user_identity::user.eq(user_id)),
            )
            .execute(conn)
            .await?;
            if removed == 0 {
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            }
            if !super::has_way_in(conn, user_id).await? {
                return Err(crate::Error::LastSignInMethod);
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}
