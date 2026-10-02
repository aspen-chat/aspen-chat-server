//! Writing a benchmark population straight into the database, for `bench seed`.
//!
//! Seeding skips the API on purpose: registering a hundred thousand users one request at a time
//! would take hours and trip the registration limits. It writes what the API would, in batches,
//! and tags the run's users and communities in `benchmark_user` and `benchmark_community`.

use crate::CHACHA_RNG;
use crate::app;
use crate::app::channel::ChannelType;
use crate::database::schema::{
    benchmark_community, benchmark_run, benchmark_user, channel, community, community_user,
    message, user,
};
use aspen_bench_protocol::{Manifest, SeedPlan, SeededCommunity, SeededUser, user_name};
use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use rand::RngExt;
use uuid::Uuid;

/// Rows per insert statement, well under Postgres's 65535 bind parameters.
const BATCH: usize = 2000;
/// How far back seeded history reaches.
const HISTORY_SPAN: Duration = Duration::days(7);

const HISTORY_LINES: [&str; 8] = [
    "Anyone around tonight?",
    "That patch notes thread is wild.",
    "I'll bring snacks next time.",
    "Can someone pin the schedule?",
    "Good game, everyone.",
    "Link's in the other channel.",
    "Back in five.",
    "Has anyone tried the new map yet?",
];

/// A UUIDv7 for `at`, so seeded history sorts by time as live messages do.
fn id_at(at: DateTime<Utc>) -> Uuid {
    let seconds = u64::try_from(at.timestamp()).unwrap_or(0);
    Uuid::new_v7(uuid::Timestamp::from_unix(
        uuid::NoContext,
        seconds,
        at.timestamp_subsec_nanos(),
    ))
}

fn random_below(n: usize) -> usize {
    CHACHA_RNG.with(|rng| rng.borrow_mut().random_range(0..n))
}

/// Writes `plan` in one transaction and says what was made.
pub async fn seed(
    conn: &mut AsyncPgConnection,
    plan: &SeedPlan,
    max_communities_per_user: u32,
) -> app::Result<Manifest> {
    plan.validate(max_communities_per_user)
        .map_err(|reason| app::Error::Validation(reason.into()))?;
    let password_hash = app::login::hash_password(plan.password.clone()).await?;
    let plan_json = serde_json::to_value(plan)?;
    let now = Utc::now();
    conn.transaction(|conn| {
        async move {
            diesel::insert_into(benchmark_run::table)
                .values((
                    benchmark_run::run.eq(&plan.run),
                    benchmark_run::plan.eq(&plan_json),
                ))
                .execute(conn)
                .await?;

            let users: Vec<SeededUser> = (0..plan.users)
                .map(|index| SeededUser {
                    id: Uuid::now_v7(),
                    name: user_name(&plan.run, index),
                })
                .collect();
            for chunk in users.chunks(BATCH) {
                let rows: Vec<_> = chunk
                    .iter()
                    .map(|u| {
                        (
                            user::id.eq(u.id),
                            user::name.eq(&u.name),
                            user::password_hash.eq(&password_hash),
                            user::created_at.eq(now),
                            user::last_seen_at.eq(now),
                        )
                    })
                    .collect();
                diesel::insert_into(user::table)
                    .values(rows)
                    .execute(conn)
                    .await?;
                let tags: Vec<_> = chunk
                    .iter()
                    .map(|u| {
                        (
                            benchmark_user::run.eq(&plan.run),
                            benchmark_user::user.eq(u.id),
                        )
                    })
                    .collect();
                diesel::insert_into(benchmark_user::table)
                    .values(tags)
                    .execute(conn)
                    .await?;
            }

            let mut communities = Vec::with_capacity(plan.communities.len());
            // Each user's communities are ordered as they were joined.
            let mut next_sort_index = vec![0i32; plan.users as usize];
            for (index, planned) in plan.communities.iter().enumerate() {
                let id = Uuid::now_v7();
                // Its first member owns it; everyone holds the default everyone role.
                let owner =
                    <[u32]>::first(&planned.members).map(|member| users[*member as usize].id);
                diesel::insert_into(community::table)
                    .values((
                        community::id.eq(id),
                        community::name.eq(format!("bench {} {index}", plan.run)),
                        community::owner.eq(owner),
                    ))
                    .execute(conn)
                    .await?;
                app::role::create_default_roles(conn, crate::app::CommunityId(id)).await?;
                diesel::insert_into(benchmark_community::table)
                    .values((
                        benchmark_community::run.eq(&plan.run),
                        benchmark_community::community.eq(id),
                    ))
                    .execute(conn)
                    .await?;
                for chunk in planned.members.chunks(BATCH) {
                    let rows: Vec<_> = chunk
                        .iter()
                        .map(|member| {
                            let slot = &mut next_sort_index[*member as usize];
                            let sort_index = *slot;
                            *slot += 1;
                            (
                                community_user::user.eq(users[*member as usize].id),
                                community_user::community.eq(id),
                                community_user::sort_index.eq(sort_index),
                            )
                        })
                        .collect();
                    diesel::insert_into(community_user::table)
                        .values(rows)
                        .execute(conn)
                        .await?;
                }
                let mut text_channels = Vec::new();
                let mut voice_channels = Vec::new();
                let channels: Vec<(Uuid, String, ChannelType, i32)> = (0..planned.text_channels)
                    .map(|n| (format!("text-{n}"), ChannelType::Text))
                    .chain(
                        (0..planned.voice_channels)
                            .map(|n| (format!("voice-{n}"), ChannelType::Voice)),
                    )
                    .enumerate()
                    .map(|(sort, (name, ty))| {
                        (
                            Uuid::now_v7(),
                            name,
                            ty,
                            i32::try_from(sort).unwrap_or(i32::MAX),
                        )
                    })
                    .collect();
                for (channel_id, name, ty, sort_index) in &channels {
                    diesel::insert_into(channel::table)
                        .values((
                            channel::id.eq(channel_id),
                            channel::community.eq(Some(id)),
                            channel::name.eq(name),
                            channel::ty.eq(ty),
                            channel::sort_index.eq(sort_index),
                        ))
                        .execute(conn)
                        .await?;
                    match ty {
                        ChannelType::Text => text_channels.push(*channel_id),
                        _ => voice_channels.push(*channel_id),
                    }
                }
                for channel_id in &text_channels {
                    seed_history(
                        conn,
                        *channel_id,
                        planned.history_per_channel,
                        &planned.members,
                        &users,
                        now,
                    )
                    .await?;
                }
                communities.push(SeededCommunity {
                    id,
                    members: planned.members.clone(),
                    text_channels,
                    voice_channels,
                });
            }
            Ok(Manifest {
                run: plan.run.clone(),
                password: plan.password.clone(),
                users,
                communities,
            })
        }
        .scope_boxed()
    })
    .await
}

async fn seed_history(
    conn: &mut AsyncPgConnection,
    channel_id: Uuid,
    count: u32,
    members: &[u32],
    users: &[SeededUser],
    now: DateTime<Utc>,
) -> app::Result<()> {
    if count == 0 {
        return Ok(());
    }
    let step = HISTORY_SPAN / i32::try_from(count).unwrap_or(i32::MAX);
    let start = now - HISTORY_SPAN;
    let messages: Vec<(Uuid, Uuid, String, DateTime<Utc>)> = (0..count)
        .map(|n| {
            let at = start + step * i32::try_from(n).unwrap_or(i32::MAX);
            let author = users[members[random_below(members.len())] as usize].id;
            let line = HISTORY_LINES[random_below(HISTORY_LINES.len())];
            (id_at(at), author, line.to_string(), at)
        })
        .collect();
    for chunk in messages.chunks(BATCH) {
        let rows: Vec<_> = chunk
            .iter()
            .map(|(id, author, content, at)| {
                (
                    message::id.eq(id),
                    message::author.eq(author),
                    message::channel.eq(channel_id),
                    message::content.eq(content),
                    message::timestamp.eq(at),
                )
            })
            .collect();
        diesel::insert_into(message::table)
            .values(rows)
            .execute(conn)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_ids_sort_by_time() {
        let now = Utc::now();
        assert!(id_at(now - Duration::hours(1)) < id_at(now));
    }
}
