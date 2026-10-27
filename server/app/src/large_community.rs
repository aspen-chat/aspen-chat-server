//! Large communities: when a community has gained as many members as the deployment setting
//! `large_community_members`, it is large from then on. Mention everyone is taken from its
//! everyone role, where a tag of everyone would reach more people than it likely means to; its
//! channels tell members without a setting of their own of tags rather than every message
//! (`app::notification_setting::default_level`); its default channel announces both with a
//! `CommunityGrew` message, posted by the system account (`app::system_account`) and tagging
//! everyone; and its owner is told privately how to give Mention everyone back.
//!
//! It happens once per community (`community.large_at`), whatever its membership does after, so
//! an owner who turns the permission back on keeps it and members' notifications do not change
//! back and forth as people come and go near the threshold. The server that adds the member
//! does it, once the membership is committed: the community's row is locked while its members
//! are counted, so of two joins that cross the threshold together exactly one acts, and a join
//! that finds the threshold reached acts even if the one that reached it could not.
//!
//! With the threshold at 0 no community crosses it, and every community counts as large
//! ([`small`]), so no channel tells everyone of every message whatever the community's size.

use crate::context::GlobalServerContext;
use crate::mention::Mentions;
use crate::message::Message;
use crate::permissions::Permissions;
use crate::t;
use crate::{CommunityId, EventScope, MaybeLoaded, MessageId, UserId, publish_event};
use aspen_schema::{community, community_user, message};
pub use aspen_wire::message::CommunityGrowth;
use aspen_wire::message::MessageKind;
use aspen_wire::message_enum::server_event::{CommunityEvent, MessageEvent, ServerEvent};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::collections::HashSet;

/// Whether a community that became large at `large_at` (`None` while it has not) is small by
/// the deployment's threshold of the moment.
pub fn small(state: &GlobalServerContext, large_at: Option<DateTime<Utc>>) -> bool {
    large_at.is_none() && state.settings().large_community_members > 0
}

/// Whether `community` is small.
pub async fn is_small(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community: CommunityId,
) -> crate::Result<bool> {
    Ok(small_among(state, conn, &[community])
        .await?
        .contains(&community))
}

/// Which of `communities` are small.
pub async fn small_among(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    communities: &[CommunityId],
) -> crate::Result<HashSet<CommunityId>> {
    if communities.is_empty() || state.settings().large_community_members == 0 {
        return Ok(HashSet::new());
    }
    Ok(community::table
        .select(community::id)
        .filter(community::id.eq_any(communities))
        .filter(community::large_at.is_null())
        .load::<CommunityId>(conn)
        .await?
        .into_iter()
        .collect())
}

/// Makes `community` large if it has reached the threshold, after a member joined. A failure
/// is logged: the member is in either way, and the next join tries again.
pub async fn after_join(state: &GlobalServerContext, community_id: CommunityId) {
    if let Err(error) = apply(state, community_id).await {
        tracing::error!(community = %community_id.0, ?error, "making a community large failed");
    }
}

async fn apply(state: &GlobalServerContext, community_id: CommunityId) -> crate::Result<()> {
    let threshold = state.settings().large_community_members;
    if threshold == 0 {
        return Ok(());
    }
    let mut conn = state.connection_pool.get().await?;
    // Most joins are to a community large already, which a read without a lock settles.
    let large: Option<Option<DateTime<Utc>>> = community::table
        .select(community::large_at)
        .filter(community::id.eq(community_id))
        .first(conn.as_mut())
        .await
        .optional()?;
    if !matches!(large, Some(None)) {
        return Ok(());
    }
    let told: Option<(UserId, String)> = conn
        .transaction(|conn| {
            async move {
                let (large_at, owner, name, default_channel): (
                    Option<DateTime<Utc>>,
                    Option<UserId>,
                    String,
                    Option<crate::ChannelId>,
                ) = community::table
                    .select((
                        community::large_at,
                        community::owner,
                        community::name,
                        community::default_channel,
                    ))
                    .filter(community::id.eq(community_id))
                    .for_update()
                    .first(conn)
                    .await?;
                if large_at.is_some() {
                    return Ok(None);
                }
                let members: i64 = community_user::table
                    .filter(community_user::community.eq(community_id))
                    .count()
                    .get_result(conn)
                    .await?;
                if members < i64::from(threshold) {
                    return Ok(None);
                }
                diesel::update(community::table.filter(community::id.eq(community_id)))
                    .set(community::large_at.eq(Utc::now()))
                    .execute(conn)
                    .await?;
                publish_event(
                    state,
                    conn,
                    EventScope::Community(community_id),
                    &ServerEvent::Community(CommunityEvent::Update {
                        id: community_id,
                        name: None,
                        icon: None,
                        owner: None,
                        default_channel: None,
                        suppress_join_messages: None,
                        small: Some(false),
                    }),
                )
                .await?;
                let taken = crate::role::take_from_everyone(
                    state,
                    conn,
                    community_id,
                    Permissions::MENTION_EVERYONE,
                )
                .await?;
                if let Some(channel) = default_channel {
                    let growth = CommunityGrowth {
                        members: u32::try_from(members).unwrap_or(u32::MAX),
                        mention_everyone_taken: taken,
                    };
                    announce(state, conn, channel, growth).await?;
                }
                Ok::<_, crate::Error>(owner.filter(|_| taken).map(|owner| (owner, name)))
            }
            .scope_boxed()
        })
        .await?;
    drop(conn);
    if let Some((owner, name)) = told {
        // Written in the deployment's default language: it is read later, by someone other
        // than whoever's request made the join.
        let notice = crate::locale::scope(crate::locale::DEFAULT, async {
            t!(
                "largeCommunityOwnerNotice",
                community = name,
                count = threshold
            )
            .into_owned()
        })
        .await;
        crate::system_account::notify(state, owner, notice).await?;
    }
    Ok(())
}

/// Posts the `CommunityGrew` message in the community's default channel, from the system
/// account and tagging everyone who can see the channel, so it tells every one of them whatever
/// their level but `nothing` (`app::push`, the apps' own rule).
async fn announce(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    channel: crate::ChannelId,
    growth: CommunityGrowth,
) -> crate::Result<()> {
    let system = crate::system_account::id(state, conn).await?;
    let row = Message {
        id: MessageId::new(),
        channel: MaybeLoaded::from_id(channel),
        content: String::new(),
        search_text: None,
        author: MaybeLoaded::from_id(system),
        timestamp: Utc::now(),
        deleted_at: None,
        edited_at: None,
        kind: MessageKind::CommunityGrew,
        poll: None,
        thread: None,
        echo_of: None,
        mentions: Mentions {
            everyone: true,
            ..Mentions::default()
        },
        call_seconds: None,
        command_bot: None,
        linked_messages: Default::default(),
        warning: None,
        altered_by: Vec::new(),
        card: None,
        echo: None,
        reply_to: None,
        tags_replied: false,
        growth: Some(growth),
    };
    diesel::insert_into(message::table)
        .values(&row)
        .execute(conn)
        .await?;
    crate::mention::record(conn, row.id, channel, &row.mentions).await?;
    publish_event(
        state,
        conn,
        EventScope::Channel(channel),
        &ServerEvent::Message(MessageEvent::Create(crate::message::record(
            &row,
            Vec::new(),
            Vec::new(),
        ))),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::mention;

    /// The system account holds every permission in its DM, so a tag in a notice would count:
    /// the notice names `@everyone` only as code.
    #[test]
    fn the_owner_notice_tags_no_one() {
        // Outside a request, `t!` speaks `locale::DEFAULT`.
        let notice = crate::t!(
            "largeCommunityOwnerNotice",
            community = "Example",
            count = 200
        );
        assert!(notice.contains("@everyone"));
        assert_eq!(mention::parse(&notice), mention::Requested::default());
    }
}
