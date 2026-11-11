//! Identities at providers linked to accounts: one account per identity, matched by the provider,
//! its issuer, and its `sub`, and at most one identity of each provider per account. A link keeps
//! the issuer it was made at, and is found only while the provider still names that issuer, since
//! a `sub` is unique only within one issuer.

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
    provider: &Provider,
    subject: &str,
) -> crate::Result<Option<UserId>> {
    Ok(user_identity::table
        .inner_join(user::table)
        .select(user_identity::user)
        .filter(user_identity::provider.eq(provider.id))
        .filter(user_identity::issuer.eq(&provider.issuer))
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
    provider: &Provider,
) -> crate::Result<Option<UserIdentityId>> {
    Ok(user_identity::table
        .select(user_identity::id)
        .filter(user_identity::user.eq(user_id))
        .filter(user_identity::provider.eq(provider.id))
        .filter(user_identity::issuer.eq(&provider.issuer))
        .first(conn)
        .await
        .optional()?)
}

/// Records that `subject` at `provider` signed in.
pub async fn used(
    conn: &mut AsyncPgConnection,
    provider: &Provider,
    subject: &str,
) -> crate::Result<()> {
    diesel::update(
        user_identity::table
            .filter(user_identity::provider.eq(provider.id))
            .filter(user_identity::issuer.eq(&provider.issuer))
            .filter(user_identity::subject.eq(subject)),
    )
    .set(user_identity::last_used_at.eq(diesel::dsl::now))
    .execute(conn)
    .await?;
    Ok(())
}

/// Links `subject` at `provider`'s issuer to `user_id`, refusing an identity another account
/// holds and a second identity of one provider.
pub async fn insert(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    provider: &Provider,
    subject: &str,
) -> crate::Result<UserIdentityId> {
    // An identity that is held is answered by no row rather than by an error whose constraint
    // would have to be told by name, which leaves one unique violation this insert can raise:
    // the account's second identity of the provider.
    diesel::insert_into(user_identity::table)
        .values((
            user_identity::id.eq(UserIdentityId::new()),
            user_identity::user.eq(user_id),
            user_identity::provider.eq(provider.id),
            user_identity::issuer.eq(&provider.issuer),
            user_identity::subject.eq(subject),
            user_identity::last_used_at.eq(diesel::dsl::now),
        ))
        .on_conflict((user_identity::provider, user_identity::subject))
        .do_nothing()
        .returning(user_identity::id)
        .get_result(conn)
        .await
        .optional()
        .map_err(|error| match error {
            diesel::result::Error::DatabaseError(
                diesel::result::DatabaseErrorKind::UniqueViolation,
                _,
            ) => crate::Error::Conflict(t!("identityProviderAlreadyLinkedGenerally")),
            error => error.into(),
        })?
        .ok_or(crate::Error::IdentityTaken)
}

/// Links `subject` at `provider` to `user_id`, as a claimed `link` flow does.
pub async fn link(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    provider: &Provider,
    subject: &str,
) -> crate::Result<IdentitySummary> {
    let id = insert(conn, user_id, provider, subject).await?;
    Ok(IdentitySummary {
        id,
        provider: provider.id,
        provider_name: provider.name.clone(),
        created_at: Utc::now(),
        last_used_at: Some(Utc::now()),
    })
}

/// `user_id`'s linked identities at providers that are not removed and still name the issuer each
/// was linked at. An account holds at most one per provider, and a deployment at most
/// `MAX_PROVIDERS`, so the list is short.
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
        .filter(user_identity::issuer.eq(identity_provider::issuer))
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

#[cfg(test)]
mod tests {
    use super::*;
    use diesel_async::SimpleAsyncConnection;

    /// Runs against the database `DATABASE_URL` names, inside a transaction that is never
    /// committed; without one it checks nothing.
    #[tokio::test]
    async fn a_held_identity_and_a_second_of_one_provider_are_refused_apart() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        let provider = Provider {
            id: IdentityProviderId::new(),
            name: "Example".to_string(),
            issuer: "https://id.example.test".to_string(),
            client_id: "aspen".to_string(),
            client_secret: "secret".to_string(),
            enabled: true,
            skip_invite: false,
            enforces_two_factor: false,
            trust_email: false,
            created_at: Utc::now(),
            removed_at: None,
        };
        let (holder, other) = (UserId::new(), UserId::new());
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(&format!(
            r#"INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at)
                 VALUES ('{}', 'identity-holder', '', now(), now()),
                        ('{}', 'identity-other', '', now(), now());
             INSERT INTO identity_provider (id, name, issuer, client_id, client_secret)
                 VALUES ('{}', 'Example', 'https://id.example.test', 'aspen', 'secret');"#,
            holder.0, other.0, provider.id.0
        ))
        .await
        .unwrap();

        let linked = insert(&mut conn, holder, &provider, "subject-1")
            .await
            .unwrap();
        assert_eq!(
            of(&mut conn, holder, &provider).await.unwrap(),
            Some(linked)
        );
        assert!(matches!(
            insert(&mut conn, other, &provider, "subject-1").await,
            Err(crate::Error::IdentityTaken)
        ));
        // Last, since a unique violation ends the transaction.
        assert!(matches!(
            insert(&mut conn, holder, &provider, "subject-2").await,
            Err(crate::Error::Conflict(_))
        ));
    }
}
