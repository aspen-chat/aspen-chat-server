//! Writing a benchmark population straight into the database, for `bench seed`.
//!
//! Seeding skips the API on purpose: registering a hundred thousand users one request at a time
//! would take hours and trip the registration limits. It writes what the API would, in batches,
//! and tags the run's users and communities in `benchmark_user` and `benchmark_community`.

use crate::CHACHA_RNG;
use crate::UserId;
use crate::channel::ChannelType;
use crate::mention::Mentions;
use crate::message::MessageKind;
use aspen_bench_protocol::{
    CommunityPlan, Manifest, SeedPlan, SeededCommunity, SeededPoll, SeededUser, user_name, words,
};
use aspen_schema::{
    benchmark_community, benchmark_run, benchmark_user, channel, community, community_user,
    mention, message, poll, poll_option, poll_vote, thread_follow, user,
};
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
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

/// How long a seeded poll that is still open stays open.
const POLL_OPEN_FOR: Duration = Duration::days(7);

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

fn random_unit() -> f64 {
    CHACHA_RNG.with(|rng| rng.borrow_mut().random::<f64>())
}

/// `share` of `span`, to the millisecond.
fn share_of(span: Duration, share: f64) -> Duration {
    Duration::milliseconds((span.num_milliseconds() as f64 * share) as i64)
}

/// Writes `plan` in one transaction and says what was made.
pub async fn seed(
    conn: &mut AsyncPgConnection,
    plan: &SeedPlan,
    max_communities_per_user: u32,
) -> crate::Result<Manifest> {
    plan.validate(max_communities_per_user)
        .map_err(|reason| crate::Error::Validation(reason.into()))?;
    let password = plan.password.clone().unwrap_or_else(|| {
        BASE64_URL_SAFE_NO_PAD.encode(CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 18]>()))
    });
    let password_hash = crate::login::hash_password(password.clone()).await?;
    // Kept without the password, which only the manifest carries.
    let plan_json = serde_json::to_value(SeedPlan {
        password: None,
        ..plan.clone()
    })?;
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
                crate::role::create_default_roles(conn, crate::CommunityId(id)).await?;
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
                let mut threads = Vec::new();
                let mut open_polls = Vec::new();
                for channel_id in &text_channels {
                    let (channel_threads, channel_polls) =
                        seed_channel(conn, id, *channel_id, planned, &users, now).await?;
                    threads.extend(channel_threads);
                    open_polls.extend(channel_polls);
                }
                communities.push(SeededCommunity {
                    id,
                    members: planned.members.clone(),
                    text_channels,
                    voice_channels,
                    threads,
                    open_polls,
                });
            }
            Ok(Manifest {
                run: plan.run.clone(),
                password: password.clone(),
                users,
                communities,
            })
        }
        .scope_boxed()
    })
    .await
}

/// A text channel's seeded contents: its history, some of it tagging members, the threads
/// started from it, and its polls. Returns the threads and the polls still open.
async fn seed_channel(
    conn: &mut AsyncPgConnection,
    community: Uuid,
    channel_id: Uuid,
    planned: &CommunityPlan,
    users: &[SeededUser],
    now: DateTime<Utc>,
) -> crate::Result<(Vec<Uuid>, Vec<SeededPoll>)> {
    let members = &planned.members;
    let member = || users[members[random_below(members.len())] as usize].id;
    let count = planned.history_per_channel as usize;
    let start = now - HISTORY_SPAN;
    let step = HISTORY_SPAN / i32::try_from(count.max(1)).unwrap_or(i32::MAX);
    let mut tagged = vec![false; count];
    for index in sample(count, planned.tagged_per_channel as usize) {
        tagged[index] = true;
    }
    let history: Vec<SeedMessage> = (0..count)
        .map(|n| {
            let at = start + step * i32::try_from(n).unwrap_or(i32::MAX);
            let author = member();
            let line = words::line(random_unit);
            let (content, mentions) = if tagged[n] {
                let target = member();
                (
                    format!("<@{target}> {line}"),
                    Mentions {
                        users: vec![UserId(target)],
                        ..Mentions::default()
                    },
                )
            } else {
                (line, Mentions::default())
            };
            SeedMessage {
                id: id_at(at),
                author,
                channel: channel_id,
                content,
                at,
                kind: MessageKind::Standard,
                poll: None,
                mentions,
            }
        })
        .collect();

    let starters = sample(count, planned.threads_per_channel as usize);
    let replies_per_thread = planned.replies_per_thread as usize;
    let mut threads = Vec::with_capacity(starters.len());
    let mut replies = Vec::with_capacity(starters.len() * replies_per_thread);
    for starter in starters.iter().map(|index| &history[*index]) {
        // Replies come over the day after the message, or the time since it if less.
        let span = (now - starter.at).min(Duration::days(1));
        let reply_at = |k: usize| {
            starter.at
                + span * i32::try_from(k + 1).unwrap_or(i32::MAX)
                    / i32::try_from(replies_per_thread + 1).unwrap_or(i32::MAX)
        };
        let thread = id_at(reply_at(0));
        let mut people = vec![starter.author];
        for k in 0..replies_per_thread {
            let at = reply_at(k);
            let author = member();
            people.push(author);
            replies.push(SeedMessage {
                id: id_at(at),
                author,
                channel: thread,
                content: words::line(random_unit),
                at,
                kind: MessageKind::Standard,
                poll: None,
                mentions: Mentions::default(),
            });
        }
        people.sort_unstable();
        people.dedup();
        threads.push(SeedThread {
            id: thread,
            starter: starter.id,
            replies: i32::try_from(replies_per_thread).unwrap_or(i32::MAX),
            last_reply_at: (replies_per_thread > 0).then(|| reply_at(replies_per_thread - 1)),
            people,
        });
    }

    let mut polls = Vec::new();
    let mut poll_messages = Vec::new();
    let mut open_polls = Vec::new();
    for n in 0..planned.polls_per_channel {
        let created_at = start + share_of(HISTORY_SPAN, random_unit());
        let open = n % 2 == 0;
        let closes_at = if open {
            now + POLL_OPEN_FOR
        } else {
            created_at + (now - created_at).min(Duration::days(1))
        };
        let id = id_at(created_at);
        let creator = member();
        let options = 2 + random_below(3);
        poll_messages.push(SeedMessage {
            id: id_at(created_at),
            author: creator,
            channel: channel_id,
            content: String::new(),
            at: created_at,
            kind: MessageKind::Poll,
            poll: Some(id),
            mentions: Mentions::default(),
        });
        if open {
            open_polls.push(SeededPoll {
                id,
                options: u32::try_from(options).unwrap_or(u32::MAX),
            });
        } else {
            // What the poll closer posts when a poll ends.
            poll_messages.push(SeedMessage {
                id: id_at(closes_at),
                author: creator,
                channel: channel_id,
                content: String::new(),
                at: closes_at,
                kind: MessageKind::PollClosed,
                poll: Some(id),
                mentions: Mentions::default(),
            });
        }
        let voters = (planned.votes_per_poll as usize).min(members.len());
        let first = random_below(members.len());
        let votes = (0..voters)
            .map(|k| {
                (
                    users[members[(first + k) % members.len()] as usize].id,
                    i32::try_from(random_below(options)).unwrap_or(0),
                    created_at + share_of(closes_at.min(now) - created_at, random_unit()),
                )
            })
            .collect();
        polls.push(SeedPoll {
            id,
            creator,
            created_at,
            closes_at,
            closed: !open,
            question: words::line(random_unit),
            options: (0..options).map(|_| words::word(random_unit())).collect(),
            votes,
        });
    }

    for chunk in polls.chunks(BATCH) {
        let rows: Vec<_> = chunk
            .iter()
            .map(|p| {
                (
                    poll::id.eq(p.id),
                    poll::channel.eq(channel_id),
                    poll::created_by.eq(p.creator),
                    poll::question.eq(&p.question),
                    poll::multiple_choice.eq(false),
                    poll::anonymous.eq(false),
                    poll::allow_write_ins.eq(false),
                    poll::created_at.eq(p.created_at),
                    poll::closes_at.eq(p.closes_at),
                    poll::closed_at.eq(p.closed.then_some(p.closes_at)),
                )
            })
            .collect();
        diesel::insert_into(poll::table)
            .values(rows)
            .execute(conn)
            .await?;
    }
    let options: Vec<_> = polls
        .iter()
        .flat_map(|p| {
            p.options.iter().enumerate().map(|(index, label)| {
                (
                    poll_option::poll.eq(p.id),
                    poll_option::index.eq(i32::try_from(index).unwrap_or(i32::MAX)),
                    poll_option::label.eq(*label),
                )
            })
        })
        .collect();
    for chunk in options.chunks(BATCH) {
        diesel::insert_into(poll_option::table)
            .values(chunk)
            .execute(conn)
            .await?;
    }
    insert_messages(conn, history.iter().chain(&poll_messages)).await?;

    for chunk in threads.chunks(BATCH) {
        let rows: Vec<_> = chunk
            .iter()
            .map(|t| {
                (
                    channel::id.eq(t.id),
                    channel::community.eq(Some(community)),
                    channel::name.eq(""),
                    channel::ty.eq(ChannelType::Thread),
                    channel::sort_index.eq(0),
                    channel::parent_channel.eq(Some(channel_id)),
                    channel::starter_message.eq(Some(t.starter)),
                    channel::reply_count.eq(t.replies),
                    channel::last_reply_at.eq(t.last_reply_at),
                )
            })
            .collect();
        diesel::insert_into(channel::table)
            .values(rows)
            .execute(conn)
            .await?;
    }
    // Replies go in once their threads exist, which the insert trigger reads for their
    // `home_channel`.
    insert_messages(conn, replies.iter()).await?;
    for chunk in threads.chunks(BATCH) {
        let (thread_ids, starter_ids): (Vec<Uuid>, Vec<Uuid>) =
            chunk.iter().map(|t| (t.id, t.starter)).unzip();
        diesel::sql_query(
            "UPDATE message SET thread = started.thread \
             FROM unnest($1::uuid[], $2::uuid[]) AS started(thread, starter) \
             WHERE message.id = started.starter",
        )
        .bind::<diesel::sql_types::Array<diesel::sql_types::Uuid>, _>(&thread_ids)
        .bind::<diesel::sql_types::Array<diesel::sql_types::Uuid>, _>(&starter_ids)
        .execute(conn)
        .await?;
    }
    let follows: Vec<_> = threads
        .iter()
        .flat_map(|t| {
            t.people.iter().map(|person| {
                (
                    thread_follow::user.eq(*person),
                    thread_follow::thread.eq(t.id),
                    thread_follow::followed_at.eq(now),
                )
            })
        })
        .collect();
    for chunk in follows.chunks(BATCH) {
        diesel::insert_into(thread_follow::table)
            .values(chunk)
            .execute(conn)
            .await?;
    }

    let votes: Vec<_> = polls
        .iter()
        .flat_map(|p| {
            p.votes.iter().map(|(voter, option, at)| {
                (
                    poll_vote::poll.eq(p.id),
                    poll_vote::option_index.eq(*option),
                    poll_vote::user.eq(*voter),
                    poll_vote::timestamp.eq(*at),
                )
            })
        })
        .collect();
    for chunk in votes.chunks(BATCH) {
        diesel::insert_into(poll_vote::table)
            .values(chunk)
            .execute(conn)
            .await?;
    }
    Ok((threads.into_iter().map(|t| t.id).collect(), open_polls))
}

struct SeedMessage {
    id: Uuid,
    author: Uuid,
    channel: Uuid,
    content: String,
    at: DateTime<Utc>,
    kind: MessageKind,
    poll: Option<Uuid>,
    mentions: Mentions,
}

struct SeedThread {
    id: Uuid,
    starter: Uuid,
    replies: i32,
    last_reply_at: Option<DateTime<Utc>>,
    /// Who follows it: its starter's author and everyone who replied.
    people: Vec<Uuid>,
}

struct SeedPoll {
    id: Uuid,
    creator: Uuid,
    created_at: DateTime<Utc>,
    closes_at: DateTime<Utc>,
    closed: bool,
    question: String,
    options: Vec<&'static str>,
    /// Voter, option, and when.
    votes: Vec<(Uuid, i32, DateTime<Utc>)>,
}

/// Inserts messages, with a `mention` row for each tag, as posting them would write.
async fn insert_messages(
    conn: &mut AsyncPgConnection,
    messages: impl Iterator<Item = &SeedMessage>,
) -> crate::Result<()> {
    let messages: Vec<&SeedMessage> = messages.collect();
    for chunk in messages.chunks(BATCH) {
        let rows: Vec<_> = chunk
            .iter()
            .map(|m| {
                (
                    message::id.eq(m.id),
                    message::author.eq(m.author),
                    message::channel.eq(m.channel),
                    message::content.eq(&m.content),
                    message::timestamp.eq(m.at),
                    message::kind.eq(m.kind),
                    message::poll.eq(m.poll),
                    message::mentions.eq(&m.mentions),
                )
            })
            .collect();
        diesel::insert_into(message::table)
            .values(rows)
            .execute(conn)
            .await?;
    }
    let tags: Vec<_> = messages
        .iter()
        .flat_map(|m| {
            m.mentions.users.iter().map(|user| {
                (
                    mention::message.eq(m.id),
                    mention::channel.eq(m.channel),
                    mention::target_user.eq(Some(user.0)),
                    mention::everyone.eq(false),
                )
            })
        })
        .collect();
    for chunk in tags.chunks(BATCH) {
        diesel::insert_into(mention::table)
            .values(chunk)
            .execute(conn)
            .await?;
    }
    Ok(())
}

/// `k` distinct indices below `n`, at most `n` of them.
fn sample(n: usize, k: usize) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..n).collect();
    let k = k.min(n);
    for i in 0..k {
        let j = i + random_below(n - i);
        indices.swap(i, j);
    }
    indices.truncate(k);
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_ids_sort_by_time() {
        let now = Utc::now();
        assert!(id_at(now - Duration::hours(1)) < id_at(now));
    }

    #[test]
    fn samples_are_distinct_and_in_range() {
        let picked = sample(10, 4);
        assert_eq!(picked.len(), 4);
        assert!(picked.iter().all(|i| *i < 10));
        let distinct: std::collections::HashSet<_> = picked.iter().collect();
        assert_eq!(distinct.len(), 4);
        assert_eq!(sample(3, 5).len(), 3);
        assert!(sample(0, 2).is_empty());
    }
}
