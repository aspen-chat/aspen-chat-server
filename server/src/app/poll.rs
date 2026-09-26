//! Timed polls. A poll is opened with a message of kind `poll` in its channel and stays open
//! until `closes_at`, when the closer task marks it closed and posts a message of kind
//! `poll_closed` announcing the outcome. Votes may be added and withdrawn while it is open, and
//! every change publishes the poll's current results, so readers follow the tally live.
//!
//! Results are computed inside the same transaction that changes the votes, with the poll row
//! locked, so the `Update` events for one poll leave in the order their tallies were taken.

use crate::api::message_enum::request::PollCreateRequest;
use crate::api::message_enum::server_event::{MessageEvent, PollEvent, ServerEvent};
use crate::api::poll::{
    PollOption as PollOptionRecord, PollOptionResult, PollVote as PollVoteRecord,
};
use crate::api::{GlobalServerContext, MessageKind, message_enum};
use crate::app;
use crate::app::message::Message;
use crate::app::react::validate_emoji;
use crate::app::{ChannelId, MaybeLoaded, MessageId, PollId, UserId, publish_event};
use crate::database::schema::{message, poll, poll_option, poll_vote};
use chrono::{DateTime, Utc};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, Insertable, NullableExpressionMethods, QueryDsl,
    Queryable, Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use rust_i18n::t;
use std::collections::HashMap;
use std::time::Duration;
use tracing::error;

pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 10;
pub const MAX_QUESTION_CHARS: usize = 300;
pub const MAX_OPTION_CHARS: usize = 100;
pub const MIN_DURATION_SECONDS: u32 = 10;
/// Four weeks.
pub const MAX_DURATION_SECONDS: u32 = 4 * 7 * 24 * 60 * 60;
/// How often the closer looks for polls whose deadline has passed.
const CLOSER_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = poll)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Poll {
    pub id: PollId,
    pub channel: ChannelId,
    pub created_by: UserId,
    pub question: String,
    pub multiple_choice: bool,
    pub anonymous: bool,
    pub created_at: DateTime<Utc>,
    pub closes_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = poll_option)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct PollOption {
    poll: PollId,
    index: i32,
    label: String,
    emoji: Option<String>,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = poll_vote)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct PollVote {
    poll: PollId,
    option_index: i32,
    user: UserId,
    timestamp: DateTime<Utc>,
}

/// Checks a creation request and returns the trimmed question and options.
fn validate(request: &PollCreateRequest) -> app::Result<(String, Vec<PollOptionRecord>)> {
    let question = request.question.trim();
    if question.is_empty() || question.chars().count() > MAX_QUESTION_CHARS {
        return Err(app::Error::Validation(t!(
            "pollQuestionLength",
            max = MAX_QUESTION_CHARS
        )));
    }
    if request.options.len() < MIN_OPTIONS || request.options.len() > MAX_OPTIONS {
        return Err(app::Error::Validation(t!(
            "pollOptionCount",
            min = MIN_OPTIONS,
            max = MAX_OPTIONS
        )));
    }
    let mut options = Vec::with_capacity(request.options.len());
    for option in &request.options {
        let label = option.label.trim();
        if label.is_empty() || label.chars().count() > MAX_OPTION_CHARS {
            return Err(app::Error::Validation(t!(
                "pollOptionLength",
                max = MAX_OPTION_CHARS
            )));
        }
        if let Some(emoji) = &option.emoji {
            validate_emoji(emoji)?;
        }
        options.push(PollOptionRecord {
            label: label.to_string(),
            emoji: option.emoji.clone(),
        });
    }
    if request.duration_seconds < MIN_DURATION_SECONDS
        || request.duration_seconds > MAX_DURATION_SECONDS
    {
        return Err(app::Error::Validation(t!(
            "pollDurationRange",
            min = MIN_DURATION_SECONDS,
            max = MAX_DURATION_SECONDS
        )));
    }
    Ok((question.to_string(), options))
}

/// Opens a poll in `channel` together with the message that shows it. Returns the poll and
/// that message, both as the wire records the events carried.
pub async fn create_poll(
    state: &GlobalServerContext,
    creator: UserId,
    channel: ChannelId,
    request: PollCreateRequest,
) -> app::Result<(message_enum::Poll, message_enum::Message)> {
    let (question, options) = validate(&request)?;
    let now = Utc::now();
    let row = Poll {
        id: PollId::new(),
        channel,
        created_by: creator,
        question,
        multiple_choice: request.multiple_choice,
        anonymous: request.anonymous,
        created_at: now,
        closes_at: now + chrono::Duration::seconds(i64::from(request.duration_seconds)),
        closed_at: None,
    };
    let message_row = poll_message(&row, MessageKind::Poll, now);
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            diesel::insert_into(poll::table)
                .values(&row)
                .execute(conn.as_mut())
                .await?;
            let option_rows: Vec<PollOption> = options
                .iter()
                .enumerate()
                .map(|(index, option)| PollOption {
                    poll: row.id,
                    index: index as i32,
                    label: option.label.clone(),
                    emoji: option.emoji.clone(),
                })
                .collect();
            diesel::insert_into(poll_option::table)
                .values(&option_rows)
                .execute(conn.as_mut())
                .await?;
            diesel::insert_into(message::table)
                .values(&message_row)
                .execute(conn.as_mut())
                .await?;
            let results = empty_results(&row, options.len());
            let poll_record = to_record(&row, message_row.id, options, results);
            let message_record = message_record(&message_row);
            // The poll goes out first so a client holds it by the time its message arrives.
            publish_event(
                state,
                &ServerEvent::Poll(PollEvent::Create(poll_record.clone())),
            )
            .await?;
            publish_event(
                state,
                &ServerEvent::Message(MessageEvent::Create(message_record.clone())),
            )
            .await?;
            Ok((poll_record, message_record))
        }
        .scope_boxed()
    })
    .await
}

/// The message row of kind `kind` that refers to `poll`, authored by the poll's creator.
fn poll_message(poll: &Poll, kind: MessageKind, timestamp: DateTime<Utc>) -> Message {
    Message {
        id: MessageId::new(),
        channel: MaybeLoaded::from_id(poll.channel),
        content: String::new(),
        author: MaybeLoaded::from_id(poll.created_by),
        timestamp,
        deleted_at: None,
        edited_at: None,
        kind,
        poll: Some(poll.id),
    }
}

fn message_record(row: &Message) -> message_enum::Message {
    message_enum::Message {
        id: row.id,
        channel_id: *row.channel.id(),
        author: *row.author.id(),
        timestamp: row.timestamp,
        edited_at: row.edited_at,
        content: row.content.clone(),
        attachments: Vec::new(),
        link_previews: Vec::new(),
        kind: row.kind,
        poll: row.poll,
    }
}

fn empty_results(poll: &Poll, option_count: usize) -> Vec<PollOptionResult> {
    (0..option_count)
        .map(|_| PollOptionResult {
            count: 0,
            voters: (!poll.anonymous).then(Vec::new),
        })
        .collect()
}

fn to_record(
    row: &Poll,
    message_id: MessageId,
    options: Vec<PollOptionRecord>,
    results: Vec<PollOptionResult>,
) -> message_enum::Poll {
    message_enum::Poll {
        id: row.id,
        channel_id: row.channel,
        message_id,
        created_by: row.created_by,
        created_at: row.created_at,
        closes_at: row.closes_at,
        closed_at: row.closed_at,
        results,
        question: row.question.clone(),
        options,
        multiple_choice: row.multiple_choice,
        anonymous: row.anonymous,
    }
}

/// The options of each poll, in option order.
async fn load_options(
    conn: &mut AsyncPgConnection,
    ids: &[PollId],
) -> app::Result<HashMap<PollId, Vec<PollOptionRecord>>> {
    let rows: Vec<PollOption> = poll_option::table
        .select(PollOption::as_select())
        .filter(poll_option::poll.eq_any(ids))
        .order((poll_option::poll, poll_option::index))
        .load(conn)
        .await?;
    let mut options: HashMap<PollId, Vec<PollOptionRecord>> = HashMap::new();
    for row in rows {
        options.entry(row.poll).or_default().push(PollOptionRecord {
            label: row.label,
            emoji: row.emoji,
        });
    }
    Ok(options)
}

/// The tally of every poll in `polls`: one entry per option, with the voters listed for polls
/// that are not anonymous. Computed from the votes as they stand on `conn`.
async fn load_results(
    conn: &mut AsyncPgConnection,
    polls: &[(&Poll, usize)],
) -> app::Result<HashMap<PollId, Vec<PollOptionResult>>> {
    let ids: Vec<PollId> = polls.iter().map(|(poll, _)| poll.id).collect();
    let votes: Vec<PollVote> = poll_vote::table
        .select(PollVote::as_select())
        .filter(poll_vote::poll.eq_any(&ids))
        .order(poll_vote::timestamp)
        .load(conn)
        .await?;
    let mut results: HashMap<PollId, Vec<PollOptionResult>> = polls
        .iter()
        .map(|(poll, option_count)| (poll.id, empty_results(poll, *option_count)))
        .collect();
    for vote in votes {
        let Some(tally) = results.get_mut(&vote.poll).and_then(|tally| {
            usize::try_from(vote.option_index)
                .ok()
                .and_then(|i| tally.get_mut(i))
        }) else {
            continue;
        };
        tally.count += 1;
        if let Some(voters) = &mut tally.voters {
            voters.push(vote.user);
        }
    }
    Ok(results)
}

/// The wire records of the polls in `ids`, in no particular order; ids with no poll are
/// skipped.
async fn load_polls(
    conn: &mut AsyncPgConnection,
    ids: &[PollId],
) -> app::Result<Vec<message_enum::Poll>> {
    let rows: Vec<Poll> = poll::table
        .select(Poll::as_select())
        .filter(poll::id.eq_any(ids))
        .load(conn)
        .await?;
    load_records(conn, rows).await
}

async fn load_records(
    conn: &mut AsyncPgConnection,
    rows: Vec<Poll>,
) -> app::Result<Vec<message_enum::Poll>> {
    let ids: Vec<PollId> = rows.iter().map(|row| row.id).collect();
    let mut options = load_options(conn, &ids).await?;
    let counts: Vec<(&Poll, usize)> = rows
        .iter()
        .map(|row| (row, options.get(&row.id).map_or(0, Vec::len)))
        .collect();
    let mut results = load_results(conn, &counts).await?;
    let shown_by: HashMap<PollId, MessageId> = message::table
        .select((message::poll.assume_not_null(), message::id))
        .filter(
            message::poll
                .eq_any(&ids)
                .and(message::kind.eq(MessageKind::Poll)),
        )
        .load::<(PollId, MessageId)>(conn)
        .await?
        .into_iter()
        .collect();
    Ok(rows
        .iter()
        .filter_map(|row| {
            let message_id = shown_by.get(&row.id)?;
            Some(to_record(
                row,
                *message_id,
                options.remove(&row.id).unwrap_or_default(),
                results.remove(&row.id).unwrap_or_default(),
            ))
        })
        .collect())
}

pub async fn read_poll(state: &GlobalServerContext, id: PollId) -> app::Result<message_enum::Poll> {
    let mut conn = state.connection_pool.get().await?;
    load_polls(conn.as_mut(), &[id])
        .await?
        .into_iter()
        .next()
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))
}

/// Batched read for sideloading; see [`load_polls`].
pub async fn read_polls(
    state: &GlobalServerContext,
    ids: &[PollId],
) -> app::Result<Vec<message_enum::Poll>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    load_polls(conn.as_mut(), ids).await
}

/// `user`'s own votes on the polls in `ids`.
pub async fn read_votes(
    state: &GlobalServerContext,
    user: UserId,
    ids: &[PollId],
) -> app::Result<Vec<PollVoteRecord>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<PollVote> = poll_vote::table
        .select(PollVote::as_select())
        .filter(poll_vote::poll.eq_any(ids).and(poll_vote::user.eq(user)))
        .load(conn.as_mut())
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(PollVoteRecord {
                poll: row.poll,
                option: u32::try_from(row.option_index).ok()?,
            })
        })
        .collect())
}

/// Locks `id`'s row for the rest of the transaction and returns it, or `NotFound`. The lock
/// serialises every vote change and the closer on the same poll, which is what keeps the
/// tallies in successive `Update` events consistent with each other.
async fn lock_poll(conn: &mut AsyncPgConnection, id: PollId) -> app::Result<Poll> {
    poll::table
        .select(Poll::as_select())
        .filter(poll::id.eq(id))
        .for_update()
        .first(conn)
        .await
        .map_err(app::Error::from)
}

fn ensure_open(poll: &Poll, now: DateTime<Utc>) -> app::Result<()> {
    if poll.closed_at.is_some() || poll.closes_at <= now {
        return Err(app::Error::PollClosed);
    }
    Ok(())
}

/// Publishes the poll's current tally and returns its record.
async fn publish_results(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    row: Poll,
) -> app::Result<message_enum::Poll> {
    let record = load_records(conn, vec![row])
        .await?
        .into_iter()
        .next()
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))?;
    publish_event(
        state,
        &ServerEvent::Poll(PollEvent::Update {
            id: record.id,
            closed_at: record.closed_at.map(Some),
            results: Some(record.results.clone()),
        }),
    )
    .await?;
    Ok(record)
}

/// Records `user`'s vote for `option`. On a single-choice poll any other vote of theirs is
/// withdrawn first. Returns whether the vote is new, and the poll with its updated tally.
pub async fn add_vote(
    state: &GlobalServerContext,
    user: UserId,
    id: PollId,
    option: u32,
) -> app::Result<(bool, message_enum::Poll)> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let now = Utc::now();
            let row = lock_poll(conn.as_mut(), id).await?;
            ensure_open(&row, now)?;
            let option_count: i64 = poll_option::table
                .filter(poll_option::poll.eq(id))
                .count()
                .get_result(conn.as_mut())
                .await?;
            let Ok(option_index) = i32::try_from(option) else {
                return Err(app::Error::Validation(t!("pollOptionOutOfRange")));
            };
            if i64::from(option_index) >= option_count {
                return Err(app::Error::Validation(t!("pollOptionOutOfRange")));
            }
            if !row.multiple_choice {
                diesel::delete(poll_vote::table)
                    .filter(
                        poll_vote::poll
                            .eq(id)
                            .and(poll_vote::user.eq(user))
                            .and(poll_vote::option_index.ne(option_index)),
                    )
                    .execute(conn.as_mut())
                    .await?;
            }
            let inserted = diesel::insert_into(poll_vote::table)
                .values(&PollVote {
                    poll: id,
                    option_index,
                    user,
                    timestamp: now,
                })
                .on_conflict_do_nothing()
                .execute(conn.as_mut())
                .await?;
            let record = publish_results(state, conn.as_mut(), row).await?;
            Ok((inserted > 0, record))
        }
        .scope_boxed()
    })
    .await
}

/// Withdraws `user`'s vote for `option`, if they had one.
pub async fn remove_vote(
    state: &GlobalServerContext,
    user: UserId,
    id: PollId,
    option: u32,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let row = lock_poll(conn.as_mut(), id).await?;
            ensure_open(&row, Utc::now())?;
            let Ok(option_index) = i32::try_from(option) else {
                return Ok(());
            };
            let deleted = diesel::delete(poll_vote::table)
                .filter(
                    poll_vote::poll
                        .eq(id)
                        .and(poll_vote::user.eq(user))
                        .and(poll_vote::option_index.eq(option_index)),
                )
                .execute(conn.as_mut())
                .await?;
            if deleted > 0 {
                publish_results(state, conn.as_mut(), row).await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Removes a poll whose message is being deleted, on the caller's transaction. Its votes go
/// with it, and a closer that has not run yet will not announce it.
pub async fn delete_poll(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    id: PollId,
) -> app::Result<()> {
    let deleted = diesel::delete(poll::table)
        .filter(poll::id.eq(id))
        .execute(conn)
        .await?;
    if deleted > 0 {
        publish_event(state, &ServerEvent::Poll(PollEvent::Delete { id })).await?;
    }
    Ok(())
}

/// Starts the task that closes polls once their deadline passes.
pub fn spawn_closer(state: GlobalServerContext) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(CLOSER_INTERVAL);
        loop {
            interval.tick().await;
            if let Err(e) = close_due_polls(&state).await {
                error!(error = e.to_string(), "closing due polls failed");
            }
        }
    });
}

/// Closes every open poll whose deadline has passed: marks it closed, publishes its final
/// tally, and posts the `poll_closed` message. Polls another server instance is closing at
/// the same moment are skipped and left to it.
async fn close_due_polls(state: &GlobalServerContext) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let now = Utc::now();
            let due: Vec<Poll> = poll::table
                .select(Poll::as_select())
                .filter(poll::closed_at.is_null().and(poll::closes_at.le(now)))
                .for_update()
                .skip_locked()
                .load(conn.as_mut())
                .await?;
            for mut row in due {
                diesel::update(poll::table)
                    .filter(poll::id.eq(row.id))
                    .set(poll::closed_at.eq(now))
                    .execute(conn.as_mut())
                    .await?;
                row.closed_at = Some(now);
                let announcement = poll_message(&row, MessageKind::PollClosed, now);
                diesel::insert_into(message::table)
                    .values(&announcement)
                    .execute(conn.as_mut())
                    .await?;
                publish_results(state, conn.as_mut(), row).await?;
                publish_event(
                    state,
                    &ServerEvent::Message(MessageEvent::Create(message_record(&announcement))),
                )
                .await?;
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

    fn request(options: &[&str], duration_seconds: u32) -> PollCreateRequest {
        PollCreateRequest {
            question: " Lunch? ".to_string(),
            options: options
                .iter()
                .map(|s| PollOptionRecord {
                    label: s.to_string(),
                    emoji: None,
                })
                .collect(),
            multiple_choice: false,
            anonymous: false,
            duration_seconds,
        }
    }

    #[test]
    fn validation_trims_and_bounds() {
        let (question, options) = validate(&request(&[" Pizza ", "Sushi"], 60)).unwrap();
        assert_eq!(question, "Lunch?");
        assert_eq!(
            options.iter().map(|o| o.label.as_str()).collect::<Vec<_>>(),
            ["Pizza", "Sushi"]
        );
        let mut with_emoji = request(&["Pizza", "Sushi"], 60);
        with_emoji.options[0].emoji = Some("🍕".to_string());
        assert_eq!(
            validate(&with_emoji).unwrap().1[0].emoji.as_deref(),
            Some("🍕")
        );
        with_emoji.options[0].emoji = Some("pizza".to_string());
        assert!(validate(&with_emoji).is_err());
        assert!(validate(&request(&["Pizza"], 60)).is_err());
        assert!(validate(&request(&["Pizza", "  "], 60)).is_err());
        assert!(validate(&request(&["Pizza", "Sushi"], 1)).is_err());
        assert!(validate(&request(&["Pizza", "Sushi"], MAX_DURATION_SECONDS + 1)).is_err());
        let too_many: Vec<&str> = std::iter::repeat_n("x", MAX_OPTIONS + 1).collect();
        assert!(validate(&request(&too_many, 60)).is_err());
    }

    #[test]
    fn open_only_before_the_deadline() {
        let now = Utc::now();
        let poll = Poll {
            id: PollId::new(),
            channel: ChannelId::new(),
            created_by: UserId::new(),
            question: "q".to_string(),
            multiple_choice: false,
            anonymous: true,
            created_at: now,
            closes_at: now + chrono::Duration::seconds(30),
            closed_at: None,
        };
        assert!(ensure_open(&poll, now).is_ok());
        assert!(matches!(
            ensure_open(&poll, poll.closes_at),
            Err(app::Error::PollClosed)
        ));
        let closed = Poll {
            closed_at: Some(now),
            ..poll
        };
        assert!(matches!(
            ensure_open(&closed, now),
            Err(app::Error::PollClosed)
        ));
        assert!(empty_results(&closed, 2).iter().all(|r| r.voters.is_none()));
    }
}
