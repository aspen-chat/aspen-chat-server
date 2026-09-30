//! Mention everyone in a growing community: when a community has gained
//! `[communities] everyone_mention_limit` members, Mention everyone is taken from its everyone
//! role, where a tag of everyone would reach more people than it likely means to, and the
//! owner is told why by the system account (`app::system_account`), free to give it back.
//!
//! It happens once per community (`community.everyone_limited_at`), whatever its membership
//! does after, so an owner who turns the permission back on keeps it. The server that adds the
//! member does it, once the membership is committed: the community's row is locked while its
//! members are counted, so of two joins that cross the limit together exactly one acts, and a
//! join that finds the limit reached acts even if the one that reached it could not.

use crate::api::GlobalServerContext;
use crate::app::permissions::Permissions;
use crate::app::{self, CommunityId, UserId};
use crate::database::schema::{community, community_user};
use crate::t;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};

/// Applies the limit to `community` if it has reached it, after a member joined. A failure is
/// logged: the member is in either way, and the next join tries again.
pub async fn after_join(state: &GlobalServerContext, community_id: CommunityId) {
    if let Err(error) = apply(state, community_id).await {
        tracing::error!(community = %community_id.0, ?error, "applying the everyone mention limit failed");
    }
}

async fn apply(state: &GlobalServerContext, community_id: CommunityId) -> app::Result<()> {
    let limit = state.config.communities.everyone_mention_limit;
    if limit == 0 {
        return Ok(());
    }
    let mut conn = state.connection_pool.get().await?;
    // Most joins are to a community limited already, which a read without a lock settles.
    let limited: Option<Option<DateTime<Utc>>> = community::table
        .select(community::everyone_limited_at)
        .filter(community::id.eq(community_id))
        .first(conn.as_mut())
        .await
        .optional()?;
    if !matches!(limited, Some(None)) {
        return Ok(());
    }
    let told: Option<(UserId, String)> = conn
        .transaction(|conn| {
            async move {
                let (limited, owner, name): (Option<DateTime<Utc>>, Option<UserId>, String) =
                    community::table
                        .select((
                            community::everyone_limited_at,
                            community::owner,
                            community::name,
                        ))
                        .filter(community::id.eq(community_id))
                        .for_update()
                        .first(conn)
                        .await?;
                if limited.is_some() {
                    return Ok(None);
                }
                let members: i64 = community_user::table
                    .filter(community_user::community.eq(community_id))
                    .count()
                    .get_result(conn)
                    .await?;
                if members < i64::from(limit) {
                    return Ok(None);
                }
                diesel::update(community::table.filter(community::id.eq(community_id)))
                    .set(community::everyone_limited_at.eq(Utc::now()))
                    .execute(conn)
                    .await?;
                let taken = app::role::take_from_everyone(
                    state,
                    conn,
                    community_id,
                    Permissions::MENTION_EVERYONE,
                )
                .await?;
                Ok::<_, app::Error>(owner.filter(|_| taken).map(|owner| (owner, name)))
            }
            .scope_boxed()
        })
        .await?;
    drop(conn);
    if let Some((owner, name)) = told {
        // Written in the deployment's default language: it is read later, by someone other
        // than whoever's request made the join.
        let notice = app::locale::scope(app::locale::DEFAULT, async {
            t!("everyoneLimitNotice", community = name, count = limit).into_owned()
        })
        .await;
        app::system_account::notify(state, owner, notice).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::app::mention;

    /// The system account holds every permission in its DM, so a tag in a notice would count:
    /// the notice names `@everyone` only as code.
    #[test]
    fn the_notice_tags_no_one() {
        let notice = rust_i18n::t!(
            "everyoneLimitNotice",
            locale = crate::app::locale::DEFAULT,
            community = "Example",
            count = 200
        );
        assert!(notice.contains("@everyone"));
        assert_eq!(mention::parse(&notice), mention::Requested::default());
    }
}
