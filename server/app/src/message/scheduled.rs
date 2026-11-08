//! Messages scheduled for later.
//!
//! A scheduled message is a held message (`super::held`) that waits for the time its author
//! chose (`send_at`, `waiting = 'scheduled'`): it is checked as it would be posted when it is
//! scheduled, only its author is told of it, and its `releaseHeldMessage` job, due at that
//! time, posts it as any held message is posted, with the author's permissions as they are
//! then and its channel's plugins deciding it then. When its channel is busy at that time it
//! waits for the channel's mana with everything else (`app::mana::queue`), and is never
//! dropped for waiting.
//!
//! One that cannot be posted is kept, failed, with why (`held::fail_held`), until its author
//! gives it another time ([`reschedule`], [`send_now`]) or deletes it (`held::cancel`), or it
//! has been kept [`FAILED_KEPT`] ([`sweep_failed`]). An author who can no longer see where it
//! would go has it dropped instead, as everything they hold somewhere is dropped when they
//! leave it (`held::drop_held_by`).
//!
//! Each of an author's is theirs alone to read ([`read_scheduled`]), and every change to one
//! reaches their apps as `scheduledMessageChanged` with the whole record ([`announce`]); it
//! leaves by the events every held message leaves by.

use super::held::{self, COLUMNS, HeldMessage, HeldWaiting};
use crate::context::GlobalServerContext;
use crate::jobs::{self, JobClass, JobKind, NewJob};
use crate::t;
use crate::{AttachmentId, ChannelId, EventScope, HeldMessageId, UserId};
pub use aspen_wire::held::{ScheduledMessage, ScheduledStatus};
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, TimeDelta, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Bool, Nullable, Text, Timestamptz, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

/// The soonest a message may be scheduled for, from now.
pub const MIN_AHEAD: TimeDelta = TimeDelta::minutes(1);
/// The furthest ahead a message may be scheduled for.
pub const MAX_AHEAD: TimeDelta = TimeDelta::days(730);
/// The most scheduled messages one author may have at once, failed ones among them.
pub const MAX_PER_AUTHOR: i64 = 100;
/// How long a failed scheduled message is kept for its author before it is deleted.
pub const FAILED_KEPT: TimeDelta = TimeDelta::days(30);
/// How many failed scheduled messages one statement of [`sweep_failed`] deletes.
const SWEEP_BATCH: i64 = 100;
/// How long past its time a scheduled message keeps its author's later ones in the same
/// channel waiting behind it: longer than posting one is tried for (`held::MAX_ATTEMPTS`), and
/// no longer, so one that is never posted does not hold the rest for good.
const BEHIND_FOR: TimeDelta = TimeDelta::minutes(10);

/// `row` as its author's apps hold it; `None` for a held message that was not scheduled.
fn record(row: HeldMessage) -> Option<ScheduledMessage> {
    let send_at = row.send_at?;
    // Due but not yet in its channel's queue, for want of room there, it waits for mana all the
    // same (`queue::wait_held`).
    let status = if row.failure.is_some() {
        ScheduledStatus::Failed
    } else if row.waiting == HeldWaiting::Mana || send_at <= Utc::now() {
        ScheduledStatus::WaitingForMana
    } else {
        ScheduledStatus::Scheduled
    };
    Some(ScheduledMessage {
        id: row.id,
        channel_id: row.channel,
        content: row.content,
        attachments: row.attachments,
        reply_to: row.reply_to,
        tag_replied: row.tags_replied,
        send_at,
        status,
        failure: row.failure,
    })
}

/// Tells the author's apps how the scheduled message `row` stands, in the transaction that
/// changed it.
pub(crate) async fn announce(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    row: &HeldMessage,
) -> crate::Result<()> {
    let author = row.author;
    let Some(scheduled) = record(row.clone()) else {
        return Ok(());
    };
    crate::publish_event(
        state,
        conn,
        EventScope::User(author),
        &ServerEvent::ScheduledMessageChanged { scheduled },
    )
    .await
}

/// Refuses a time sooner than [`MIN_AHEAD`] or further than [`MAX_AHEAD`] from now.
fn check_time(send_at: DateTime<Utc>) -> crate::Result<()> {
    let ahead = send_at - Utc::now();
    if ahead < MIN_AHEAD {
        return Err(crate::Error::Validation(t!(
            "scheduledTooSoon",
            minutes = MIN_AHEAD.num_minutes()
        )));
    }
    if ahead > MAX_AHEAD {
        return Err(crate::Error::Validation(t!(
            "scheduledTooFar",
            years = MAX_AHEAD.num_days() / 365
        )));
    }
    Ok(())
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

#[derive(QueryableByName)]
struct Account {
    #[diesel(sql_type = Bool)]
    bot: bool,
}

/// Schedules a message of `author`'s for `send_at` in `channel`: a channel, a thread already
/// made, or a DM. It is checked as posting it now would be (the right to post there, its
/// attachments, what it replies to), and again when it is posted. Refused when the author
/// already has [`MAX_PER_AUTHOR`] scheduled, and for a bot, which keeps time of its own.
pub async fn schedule(
    state: &GlobalServerContext,
    author: UserId,
    channel: ChannelId,
    content: String,
    attachments: Vec<AttachmentId>,
    reply: Option<super::reply::Reply>,
    send_at: DateTime<Utc>,
) -> crate::Result<ScheduledMessage> {
    super::check_content(&content)?;
    check_time(send_at)?;
    let id = HeldMessageId::new();
    let locale = crate::locale::current().to_string();
    let mut conn = state.connection_pool.get().await?;
    let row = conn
        .transaction::<_, crate::Error, _>(|conn| {
            async move {
                // Held until this commits, so two of the author's requests at once cannot both
                // find room under the limit.
                let account: Account =
                    diesel::sql_query(r#"SELECT bot FROM "user" WHERE id = $1 FOR NO KEY UPDATE"#)
                        .bind::<PgUuid, _>(author)
                        .get_result(conn.as_mut())
                        .await?;
                if account.bot {
                    return Err(crate::Error::Forbidden(t!("scheduledNotForBots")));
                }
                let scheduled: Count = diesel::sql_query(
                    "SELECT count(*) AS count FROM held_message \
                     WHERE author = $1 AND send_at IS NOT NULL",
                )
                .bind::<PgUuid, _>(author)
                .get_result(conn.as_mut())
                .await?;
                if scheduled.count >= MAX_PER_AUTHOR {
                    return Err(crate::Error::Validation(t!(
                        "scheduledTooMany",
                        max = MAX_PER_AUTHOR
                    )));
                }
                super::check_posting(state, conn.as_mut(), author, channel, &attachments, false)
                    .await?;
                if let Some(reply) = reply {
                    super::reply::check(conn.as_mut(), channel, reply.to, false).await?;
                }
                let row: HeldMessage = diesel::sql_query(format!(
                    "INSERT INTO held_message (id, author, channel, content, attachments, \
                         echo_to_parent, reply_to, tags_replied, locale, waiting, send_at) \
                     VALUES ($1, $2, $3, $4, $5, false, $6, $7, $8, 'scheduled', $9) \
                     RETURNING {COLUMNS}"
                ))
                .bind::<PgUuid, _>(id)
                .bind::<PgUuid, _>(author)
                .bind::<PgUuid, _>(channel)
                .bind::<Text, _>(content)
                .bind::<Array<PgUuid>, _>(attachments)
                .bind::<Nullable<PgUuid>, _>(reply.map(|reply| reply.to))
                .bind::<Bool, _>(reply.is_some_and(|reply| reply.tag))
                .bind::<Text, _>(locale)
                .bind::<Timestamptz, _>(send_at)
                .get_result(conn.as_mut())
                .await?;
                jobs::enqueue(conn.as_mut(), job(id, Some(send_at))?).await?;
                announce(state, conn.as_mut(), &row).await?;
                Ok(row)
            }
            .scope_boxed()
        })
        .await?;
    record(row).ok_or(crate::Error::Diesel(diesel::result::Error::NotFound))
}

/// The job that posts the scheduled message `id`, due at `send_at`, or now.
fn job(id: HeldMessageId, send_at: Option<DateTime<Utc>>) -> crate::Result<NewJob> {
    let job = NewJob::new(JobKind::ReleaseHeldMessage, JobClass::Interactive, &())?
        .keyed(id.0.to_string());
    Ok(match send_at {
        Some(send_at) => job.not_before(send_at),
        None => job,
    })
}

/// Every message `author` has scheduled, the soonest due first, failed ones among them: at most
/// [`MAX_PER_AUTHOR`], through `held_message_scheduled`.
pub async fn read_scheduled(
    state: &GlobalServerContext,
    author: UserId,
) -> crate::Result<Vec<ScheduledMessage>> {
    let rows: Vec<HeldMessage> = diesel::sql_query(format!(
        "SELECT {COLUMNS} FROM held_message WHERE author = $1 AND send_at IS NOT NULL \
         ORDER BY send_at, id LIMIT $2"
    ))
    .bind::<PgUuid, _>(author)
    .bind::<BigInt, _>(MAX_PER_AUTHOR)
    .load(state.connection_pool.get().await?.as_mut())
    .await?;
    Ok(rows.into_iter().filter_map(record).collect())
}

/// The scheduled message `id` of `author`'s; not found when it is not theirs or is gone.
pub async fn read(
    state: &GlobalServerContext,
    author: UserId,
    id: HeldMessageId,
) -> crate::Result<ScheduledMessage> {
    let row: HeldMessage = diesel::sql_query(format!(
        "SELECT {COLUMNS} FROM held_message \
         WHERE id = $1 AND author = $2 AND send_at IS NOT NULL"
    ))
    .bind::<PgUuid, _>(id)
    .bind::<PgUuid, _>(author)
    .get_result(state.connection_pool.get().await?.as_mut())
    .await?;
    record(row).ok_or(crate::Error::Diesel(diesel::result::Error::NotFound))
}

/// Deletes the scheduled message `id` of `author`'s before it is posted, whatever it waits
/// for, as a held message is cancelled (`held::cancel`). Not found when it is not theirs, or
/// already posted.
pub async fn cancel(
    state: &GlobalServerContext,
    author: UserId,
    id: HeldMessageId,
) -> crate::Result<()> {
    held::cancel_of(state, author, id, true).await
}

/// Gives the scheduled message `id` of `author`'s another time, a failed one among them, which
/// is then tried again at that time, and one waiting for its channel's mana among them, which
/// leaves the queue. Not found when it is not theirs or is gone.
pub async fn reschedule(
    state: &GlobalServerContext,
    author: UserId,
    id: HeldMessageId,
    send_at: DateTime<Utc>,
) -> crate::Result<ScheduledMessage> {
    check_time(send_at)?;
    set_time(state, author, id, Some(send_at)).await
}

/// Posts the scheduled message `id` of `author`'s now, a failed one among them: its job comes
/// due at once, and posts it as it would have at its time. One already due, waiting for its
/// channel's mana, is left as it is. Not found as [`reschedule`].
pub async fn send_now(
    state: &GlobalServerContext,
    author: UserId,
    id: HeldMessageId,
) -> crate::Result<ScheduledMessage> {
    let scheduled = set_time(state, author, id, None).await?;
    jobs::wake(state).await;
    Ok(scheduled)
}

/// Makes the scheduled message `id` of `author`'s due at `send_at`, or now, with a job of its
/// own for that time in place of whichever it had: a step of the old job running now, or the
/// drainer about to post it from the queue it leaves, finds the message no longer as it read it,
/// and leaves it.
async fn set_time(
    state: &GlobalServerContext,
    author: UserId,
    id: HeldMessageId,
    send_at: Option<DateTime<Utc>>,
) -> crate::Result<ScheduledMessage> {
    let mut conn = state.connection_pool.get().await?;
    let row = conn
        .transaction::<_, crate::Error, _>(|conn| {
            async move {
                let held: HeldMessage = diesel::sql_query(format!(
                    "SELECT {COLUMNS} FROM held_message \
                     WHERE id = $1 AND author = $2 AND send_at IS NOT NULL FOR UPDATE"
                ))
                .bind::<PgUuid, _>(id)
                .bind::<PgUuid, _>(author)
                .get_result(conn.as_mut())
                .await?;
                if held.waiting == HeldWaiting::Mana && send_at.is_none() {
                    return Ok(held);
                }
                let row: HeldMessage = diesel::sql_query(format!(
                    "UPDATE held_message SET send_at = COALESCE($2, now()), waiting = 'scheduled', \
                         failure = NULL, failed_at = NULL, stuck_since = NULL \
                     WHERE id = $1 RETURNING {COLUMNS}"
                ))
                .bind::<PgUuid, _>(id)
                .bind::<Nullable<Timestamptz>, _>(send_at)
                .get_result(conn.as_mut())
                .await?;
                held::forget_job(conn.as_mut(), id).await?;
                jobs::enqueue(conn.as_mut(), job(id, send_at)?).await?;
                announce(state, conn.as_mut(), &row).await?;
                Ok(row)
            }
            .scope_boxed()
        })
        .await?;
    record(row).ok_or(crate::Error::Diesel(diesel::result::Error::NotFound))
}

#[derive(QueryableByName)]
struct Behind {
    #[diesel(sql_type = Bool)]
    behind: bool,
}

/// Whether a message `held`'s author scheduled for the same channel, due before it (or at the
/// same time and scheduled first), is still to be posted, so an author's are posted in the
/// order they are due: through `held_message_scheduled`, which holds at most
/// [`MAX_PER_AUTHOR`] of theirs. One that failed, waits for mana, or has not been posted
/// [`BEHIND_FOR`] past its time is not waited for.
pub(super) async fn behind_another(
    conn: &mut AsyncPgConnection,
    held: &HeldMessage,
) -> crate::Result<bool> {
    let found: Behind = diesel::sql_query(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM held_message earlier
            WHERE earlier.author = $1 AND earlier.send_at IS NOT NULL
              AND (earlier.send_at, earlier.id) < ($2, $3)
              AND earlier.send_at > now() - make_interval(secs => $5)
              AND earlier.channel = $4 AND earlier.waiting = 'scheduled'
              AND earlier.failure IS NULL
        ) AS behind
        "#,
    )
    .bind::<PgUuid, _>(held.author)
    .bind::<Nullable<Timestamptz>, _>(held.send_at)
    .bind::<PgUuid, _>(held.id)
    .bind::<PgUuid, _>(held.channel)
    .bind::<diesel::sql_types::Double, _>(BEHIND_FOR.as_seconds_f64())
    .get_result(conn)
    .await?;
    Ok(found.behind)
}

#[derive(QueryableByName)]
struct Swept {
    #[diesel(sql_type = PgUuid)]
    id: HeldMessageId,
    #[diesel(sql_type = PgUuid)]
    author: UserId,
    #[diesel(sql_type = PgUuid)]
    channel: ChannelId,
}

/// Deletes the scheduled messages that failed [`FAILED_KEPT`] ago, [`SWEEP_BATCH`] of them,
/// oldest first through `held_message_failed`, telling each author's apps as a cancellation
/// does; their files are then swept as any unsent ones are (`attachment::sweep_unsent`).
/// Answers whether a whole batch went, so there may be more. Rows another server is deleting
/// are passed by, so every server may run it at once.
pub async fn sweep_failed(state: &GlobalServerContext) -> crate::Result<bool> {
    let mut conn = state.connection_pool.get().await?;
    let swept = conn
        .transaction::<_, crate::Error, _>(|conn| {
            async move {
                let swept: Vec<Swept> = diesel::sql_query(
                    "DELETE FROM held_message WHERE id IN ( \
                         SELECT id FROM held_message \
                         WHERE failed_at < now() - make_interval(secs => $1) \
                         ORDER BY failed_at LIMIT $2 FOR UPDATE SKIP LOCKED \
                     ) RETURNING id, author, channel",
                )
                .bind::<diesel::sql_types::Double, _>(FAILED_KEPT.as_seconds_f64())
                .bind::<BigInt, _>(SWEEP_BATCH)
                .load(conn.as_mut())
                .await?;
                let events = swept
                    .iter()
                    .map(|row| {
                        (
                            EventScope::User(row.author),
                            ServerEvent::HeldMessageCancelled {
                                held: row.id,
                                channel: row.channel,
                            },
                        )
                    })
                    .collect();
                crate::events::publish_events(state, conn.as_mut(), events).await?;
                Ok(swept.len())
            }
            .scope_boxed()
        })
        .await?;
    Ok(swept as i64 == SWEEP_BATCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MessageId;
    use aspen_wire::held::HeldKind;

    fn row(waiting: HeldWaiting, failure: Option<&str>) -> HeldMessage {
        HeldMessage {
            id: HeldMessageId(uuid::Uuid::from_u128(1)),
            author: UserId(uuid::Uuid::from_u128(2)),
            channel: ChannelId(uuid::Uuid::from_u128(3)),
            content: "later".to_owned(),
            attachments: vec![AttachmentId(uuid::Uuid::from_u128(4))],
            echo_to_parent: false,
            reply_to: Some(MessageId(uuid::Uuid::from_u128(5))),
            tags_replied: true,
            locale: "en".to_owned(),
            held_at: DateTime::UNIX_EPOCH,
            waiting,
            kind: HeldKind::Message,
            target: None,
            payload: None,
            cost: 0,
            send_at: Some(Utc::now() + TimeDelta::hours(1)),
            failure: failure.map(str::to_owned),
        }
    }

    #[test]
    fn status_follows_what_it_waits_for_and_whether_it_failed() {
        let status = |waiting, failure| record(row(waiting, failure)).map(|r| r.status);
        assert_eq!(
            status(HeldWaiting::Scheduled, None),
            Some(ScheduledStatus::Scheduled)
        );
        assert_eq!(
            status(HeldWaiting::Mana, None),
            Some(ScheduledStatus::WaitingForMana)
        );
        assert_eq!(
            status(HeldWaiting::Scheduled, Some("no")),
            Some(ScheduledStatus::Failed)
        );
        // Due, it waits for mana whether or not it has found room in the queue yet.
        let mut due = row(HeldWaiting::Scheduled, None);
        due.send_at = Some(Utc::now() - TimeDelta::seconds(5));
        assert_eq!(
            record(due).map(|r| r.status),
            Some(ScheduledStatus::WaitingForMana)
        );
    }

    #[test]
    fn a_message_held_for_anything_else_is_no_scheduled_message() {
        let mut held = row(HeldWaiting::Preview, None);
        held.send_at = None;
        assert_eq!(record(held), None);
    }

    #[test]
    fn the_record_carries_what_was_scheduled() {
        let scheduled = record(row(HeldWaiting::Scheduled, None)).unwrap();
        assert_eq!(scheduled.content, "later");
        assert_eq!(scheduled.attachments.len(), 1);
        assert!(scheduled.reply_to.is_some() && scheduled.tag_replied);
        assert!(scheduled.send_at > Utc::now());
        assert_eq!(scheduled.failure, None);
    }

    #[test]
    fn a_time_must_be_a_minute_to_two_years_ahead() {
        let now = Utc::now();
        assert!(check_time(now).is_err());
        assert!(check_time(now + TimeDelta::seconds(30)).is_err());
        assert!(check_time(now + TimeDelta::minutes(2)).is_ok());
        assert!(check_time(now + TimeDelta::days(729)).is_ok());
        assert!(check_time(now + TimeDelta::days(731)).is_err());
    }
}
