//! Messages held for their previews.
//!
//! A message sent with a picture or video whose preview (`app::attachment::preview`) is still
//! being made would reach readers showing the original inline, then switch to the preview as it
//! lands. A client that says it may (`mayHold`) has such a message held instead ([`post`]): it
//! is checked as any message is, kept in `held_message`, and answered `202 Accepted`, and the
//! author's apps show it waiting. Nobody else learns of it until it is posted.
//!
//! A held message is posted by a job of its own (`releaseHeldMessage`, keyed by it; `app::jobs`)
//! once none of its attachments holds it and no message its author sent before it is still
//! held, so an author's held messages are posted in the order they were sent. Each attachment
//! holds the messages it is in until its preview is made, or found not worth making, or fails,
//! or until its preview job's `holdUntil` passes, twenty seconds after the upload
//! (`app::attachment::preview::HOLD`). A preview job that settles sets the jobs of the messages
//! it held going at once ([`wake_holding`]); a job still held looks again when the hold runs
//! out, and at least every [`LOOK_EVERY`]. A preview made after the message is posted still
//! reaches its readers, by `attachmentPreviewed`.
//!
//! Posting it is posting the message as it was sent (`super::post`), with the author's
//! permissions as they are then, its plugins deciding it then, in the language it was sent in;
//! the held row goes in the same transaction, so it is posted once however its job is run, and
//! `heldMessagePosted` tells the author's apps which message it became. One that can no longer
//! be posted (the author lost the right to post there, the channel went), or whose posting has
//! failed [`MAX_ATTEMPTS`] times, is dropped, and `heldMessageFailed` tells them why. A first
//! reply that made its thread as it was held takes the thread with it when it is dropped, if
//! nothing else was posted or waits there.
//!
//! A message sent while its files are still being uploaded is held the same way, when its
//! client says it may be (`awaitUploads`): it names reservations not yet confirmed, is checked
//! as any message is but for that, and waits (`waiting = 'uploads'`) until each has been
//! confirmed. The request that confirms the last of them sets it going ([`uploaded`]), and
//! from there it is a message held for its previews, posted by its job as above. A slash
//! command waits for its files the same way ([`hold_command`]), and so does a message scheduled
//! for later, which then waits for its time. Its job is due when the wait gives out,
//! [`UPLOADS_WAIT`] after it was sent, and then fails it; a file of it deleted meanwhile brings
//! the job forward to fail it at once ([`wake_awaiting`]). One author has at most
//! [`MAX_AWAITING_UPLOADS`] waiting so, which bounds what one confirmation sets going.
//!
//! A message scheduled for later (`super::scheduled`) is a held message too, `send_at` saying
//! when: its job comes due then and posts it as above. One that cannot be posted is kept for
//! its author with why ([`fail_held`]) rather than dropped, while they can still see where it
//! would go. Whatever changes when it is due replaces its job (`scheduled::reschedule`), and
//! what a step does to a row is done only while the row still stands as the step read it
//! ([`take`], [`fail_held`], `queue::wait_held`), so a step whose job was replaced as it ran
//! does nothing.

use super::Message;
use crate::attachment::preview;
use crate::context::GlobalServerContext;
use crate::cursor::{List, Paged};
use crate::jobs::{self, Claimed, JobClass, JobKind, NewJob, Outcome};
use crate::mana::queue::{self, Waiting};
use crate::mana::{self, Decision};
use crate::t;
use crate::{AttachmentId, ChannelId, EventScope, HeldMessageId, MessageId, UserId};
use aspen_schema::held_message;
pub use aspen_wire::held::{HeldKind, HeldMessage as HeldRecord, HeldWaiting};
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, SubsecRound, TimeDelta, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, Bool, Nullable, Text, Timestamptz, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::time::Duration;

/// The longest a held message's job waits between looks while something holds it.
pub const LOOK_EVERY: Duration = Duration::from_secs(5);
/// How long a held message's job waits while a message its author sent before it is held.
const BEHIND_WAIT: Duration = Duration::from_secs(1);
/// How many times posting one is tried before it is dropped.
pub const MAX_ATTEMPTS: i32 = 10;
/// How long a message sent before its files were uploaded waits for them, from when it was
/// sent, before it is failed: as long as a confirmed upload waits to be sent
/// (`attachment::UNSENT_LIFETIME`).
pub const UPLOADS_WAIT: TimeDelta = TimeDelta::hours(24);
/// The most messages and commands one author may have waiting for their files at once,
/// scheduled messages apart, which have a limit of their own (`scheduled::MAX_PER_AUTHOR`).
pub const MAX_AWAITING_UPLOADS: i64 = 50;

/// A message waiting for its files to be uploaded, for its attachments' previews, or for its
/// channel's mana, and the other things that wait for mana (`app::mana::queue`).
#[derive(Debug, Clone, QueryableByName)]
pub struct HeldMessage {
    #[diesel(sql_type = PgUuid)]
    pub id: HeldMessageId,
    #[diesel(sql_type = PgUuid)]
    pub author: UserId,
    #[diesel(sql_type = PgUuid)]
    pub channel: ChannelId,
    #[diesel(sql_type = Text)]
    pub content: String,
    #[diesel(sql_type = Array<PgUuid>)]
    pub attachments: Vec<AttachmentId>,
    #[diesel(sql_type = Bool)]
    pub echo_to_parent: bool,
    /// The message it replies to (`super::reply`).
    #[diesel(sql_type = Nullable<PgUuid>)]
    pub reply_to: Option<MessageId>,
    #[diesel(sql_type = Bool)]
    pub tags_replied: bool,
    /// The language it was sent in, which a refusal to post it is told in.
    #[diesel(sql_type = Text)]
    pub locale: String,
    #[diesel(sql_type = Timestamptz)]
    pub held_at: DateTime<Utc>,
    #[diesel(sql_type = Text)]
    pub waiting: HeldWaiting,
    #[diesel(sql_type = Text)]
    pub kind: HeldKind,
    /// The message an edit changes or an echo copies.
    #[diesel(sql_type = Nullable<PgUuid>)]
    pub target: Option<MessageId>,
    /// A command's invocation, or a poll as it was asked for.
    #[diesel(sql_type = Nullable<diesel::sql_types::Jsonb>)]
    pub payload: Option<serde_json::Value>,
    /// What it spends of its channel's mana when posted, in mana.
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub cost: i32,
    /// When a message scheduled for later is due (`super::scheduled`), which stays set while
    /// it then waits for its channel's mana; `None` for everything else.
    #[diesel(sql_type = Nullable<Timestamptz>)]
    pub send_at: Option<DateTime<Utc>>,
    /// Why a scheduled message could not be posted, in the language it was scheduled in: it
    /// is kept for its author, and nothing posts it until they schedule it again.
    #[diesel(sql_type = Nullable<Text>)]
    pub failure: Option<String>,
    /// For what was sent while its files were still uploading, when waiting for them gives
    /// out; it stays set once they are all there. `None` for everything else.
    #[diesel(sql_type = Nullable<Timestamptz>)]
    pub uploads_due: Option<DateTime<Utc>>,
}

/// The columns a [`HeldMessage`] is read from.
pub(crate) const COLUMNS: &str = "id, author, channel, content, attachments, echo_to_parent, \
    reply_to, tags_replied, locale, held_at, waiting, kind, target, payload, cost, send_at, \
    failure, uploads_due";

impl From<HeldMessage> for HeldRecord {
    fn from(held: HeldMessage) -> Self {
        HeldRecord {
            id: held.id,
            channel_id: held.channel,
            content: held.content,
            attachments: held.attachments,
            echo_to_parent: held.echo_to_parent,
            reply_to: held.reply_to,
            tag_replied: held.tags_replied,
            held_at: held.held_at,
            waiting: held.waiting,
            kind: held.kind,
            target: held.target,
            awaited_uploads: held.uploads_due.is_some(),
        }
    }
}

/// Tells the author's apps how the held message `row` stands, in the transaction that made or
/// changed it: one sent before its files were uploaded, which their other devices show too.
pub(crate) async fn announce_changed(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    row: &HeldMessage,
) -> crate::Result<()> {
    crate::publish_event(
        state,
        conn,
        EventScope::User(row.author),
        &ServerEvent::HeldMessageChanged {
            held: row.clone().into(),
        },
    )
    .await
}

/// What became of a message sent: posted, or held.
pub enum Posted {
    Sent(Box<Message>),
    Held(Box<HeldMessage>),
}

/// Why a message is held as it is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// A preview of one of its attachments is being made.
    Preview,
    /// Some of its files are still being uploaded.
    Uploads,
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
}

/// Refuses another message or command waiting for its files when `author` already has
/// [`MAX_AWAITING_UPLOADS`], holding their own row until the caller's transaction commits so
/// two of their requests at once cannot both find room: through `held_message_by_author`.
async fn ensure_room_awaiting(conn: &mut AsyncPgConnection, author: UserId) -> crate::Result<()> {
    diesel::sql_query(r#"SELECT 1 FROM "user" WHERE id = $1 FOR NO KEY UPDATE"#)
        .bind::<PgUuid, _>(author)
        .execute(conn)
        .await?;
    let awaiting: Count = diesel::sql_query(
        "SELECT count(*) AS count FROM held_message \
         WHERE author = $1 AND waiting = 'uploads' AND send_at IS NULL",
    )
    .bind::<PgUuid, _>(author)
    .get_result(conn)
    .await?;
    if awaiting.count >= MAX_AWAITING_UPLOADS {
        return Err(crate::Error::Validation(t!(
            "heldTooManyAwaitingUploads",
            max = MAX_AWAITING_UPLOADS
        )));
    }
    Ok(())
}

/// The job that posts the held message `id`: due now, or, while it waits for its files, when
/// waiting for them gives out.
fn release_job(id: HeldMessageId, awaiting_until: Option<DateTime<Utc>>) -> crate::Result<NewJob> {
    let job = NewJob::new(JobKind::ReleaseHeldMessage, JobClass::Interactive, &())?
        .keyed(id.0.to_string());
    Ok(match awaiting_until {
        Some(until) => job.not_before(until),
        None => job,
    })
}

/// Holds a message sent `to` a place for `hold`, checked as it would have been posted, what it
/// replies to included, but that one held for its uploads may name files not yet confirmed.
/// A reply that makes its thread makes it as it is held, and waits in the thread.
#[allow(clippy::too_many_arguments)]
async fn hold_message(
    state: &GlobalServerContext,
    author: UserId,
    to: super::To,
    content: String,
    attachments: Vec<AttachmentId>,
    echo_to_parent: bool,
    reply: Option<super::reply::Reply>,
    hold: Hold,
) -> crate::Result<HeldMessage> {
    let uploads = match hold {
        Hold::Preview => super::Uploads::Confirmed,
        Hold::Uploads => super::Uploads::MayBePending,
    };
    let mut conn = state.connection_pool.get().await?;
    let (channel, unmade) = super::resolve(conn.as_mut(), to).await?;
    if unmade.is_some() && reply.is_some() {
        return Err(crate::Error::Validation(t!("replyElsewhere")));
    }
    if let Some(unmade) = &unmade {
        super::check_first_reply(
            state,
            conn.as_mut(),
            author,
            unmade,
            &attachments,
            echo_to_parent,
            uploads,
        )
        .await?;
    }
    let id = HeldMessageId::new();
    let locale = crate::locale::current().to_string();
    // To the microsecond, as the database keeps it, so the answer to this request, the event
    // telling of it, and a later read of it name one time.
    let held_at = Utc::now().trunc_subsecs(6);
    let held = conn
        .transaction::<_, crate::Error, _>(|conn| {
            async move {
                if hold == Hold::Uploads {
                    ensure_room_awaiting(conn.as_mut(), author).await?;
                }
                let (channel, made_thread) = match unmade.as_ref() {
                    Some(unmade) => {
                        let (thread, made) = crate::thread::open_in(
                            state,
                            conn.as_mut(),
                            author,
                            unmade.starter,
                            unmade.id,
                        )
                        .await?;
                        (thread.id, made)
                    }
                    None => (channel, false),
                };
                let (_, _, pending) = super::check_posting_awaiting(
                    state,
                    conn.as_mut(),
                    author,
                    channel,
                    &attachments,
                    echo_to_parent,
                    uploads,
                )
                .await?;
                if let Some(reply) = reply {
                    super::reply::check(conn.as_mut(), channel, reply.to, false).await?;
                }
                // One whose last file was confirmed as this was sent waits for nothing more
                // than its previews, and its job is due at once.
                let uploads_due = (hold == Hold::Uploads).then(|| held_at + UPLOADS_WAIT);
                let held = HeldMessage {
                    id,
                    author,
                    channel,
                    content,
                    attachments,
                    echo_to_parent,
                    reply_to: reply.map(|reply| reply.to),
                    tags_replied: reply.is_some_and(|reply| reply.tag),
                    locale,
                    held_at,
                    waiting: if pending {
                        HeldWaiting::Uploads
                    } else {
                        HeldWaiting::Preview
                    },
                    kind: HeldKind::Message,
                    target: None,
                    payload: None,
                    cost: 0,
                    send_at: None,
                    failure: None,
                    uploads_due,
                };
                diesel::insert_into(held_message::table)
                    .values((
                        held_message::id.eq(held.id),
                        held_message::author.eq(held.author),
                        held_message::channel.eq(held.channel),
                        held_message::content.eq(&held.content),
                        held_message::attachments.eq(held
                            .attachments
                            .iter()
                            .copied()
                            .map(Some)
                            .collect::<Vec<_>>()),
                        held_message::echo_to_parent.eq(held.echo_to_parent),
                        held_message::reply_to.eq(held.reply_to),
                        held_message::tags_replied.eq(held.tags_replied),
                        held_message::locale.eq(&held.locale),
                        held_message::held_at.eq(held.held_at),
                        held_message::made_thread.eq(made_thread),
                        held_message::waiting.eq(held.waiting),
                        held_message::uploads_due.eq(held.uploads_due),
                    ))
                    .execute(conn.as_mut())
                    .await?;
                jobs::enqueue(
                    conn.as_mut(),
                    release_job(id, uploads_due.filter(|_| pending))?,
                )
                .await?;
                if hold == Hold::Uploads {
                    announce_changed(state, conn.as_mut(), &held).await?;
                }
                Ok(held)
            }
            .scope_boxed()
        })
        .await?;
    drop(conn);
    jobs::wake(state).await;
    Ok(held)
}

/// Holds a slash command of `author`'s in `channel` until the files it names, some still being
/// uploaded, are each confirmed (`super::invoke_command`): checked as invoking it checks it but
/// for that, kept as the text it shows with its invocation, and posted by its job as a command
/// that waited for its channel's mana is, checked again then.
pub(crate) async fn hold_command(
    state: &GlobalServerContext,
    author: UserId,
    channel: ChannelId,
    invocation: crate::bot_command::Invocation,
) -> crate::Result<HeldMessage> {
    let id = HeldMessageId::new();
    let locale = crate::locale::current().to_string();
    let held_at = Utc::now();
    let payload = serde_json::to_value(&invocation)?;
    let mut conn = state.connection_pool.get().await?;
    let held = conn
        .transaction::<_, crate::Error, _>(|conn| {
            async move {
                ensure_room_awaiting(conn.as_mut(), author).await?;
                let (_, access, pending) = super::check_posting_awaiting(
                    state,
                    conn.as_mut(),
                    author,
                    channel,
                    &invocation.attachments,
                    false,
                    super::Uploads::MayBePending,
                )
                .await?;
                let (command, arguments) = crate::bot_command::check(
                    state,
                    conn.as_mut(),
                    author,
                    &access,
                    channel,
                    &invocation,
                    super::Uploads::MayBePending,
                )
                .await?;
                let uploads_due = held_at + UPLOADS_WAIT;
                let held: HeldMessage = diesel::sql_query(format!(
                    "INSERT INTO held_message (id, author, channel, content, attachments, \
                         echo_to_parent, locale, held_at, waiting, kind, payload, uploads_due) \
                     VALUES ($1, $2, $3, $4, $5, false, $6, $7, $8, 'command', $9, $10) \
                     RETURNING {COLUMNS}"
                ))
                .bind::<PgUuid, _>(id)
                .bind::<PgUuid, _>(author)
                .bind::<PgUuid, _>(channel)
                .bind::<Text, _>(crate::bot_command::invocation_text(&command, &arguments))
                .bind::<Array<PgUuid>, _>(&invocation.attachments)
                .bind::<Text, _>(locale)
                .bind::<Timestamptz, _>(held_at)
                .bind::<Text, _>(if pending {
                    HeldWaiting::Uploads
                } else {
                    HeldWaiting::Preview
                })
                .bind::<diesel::sql_types::Jsonb, _>(payload)
                .bind::<Timestamptz, _>(uploads_due)
                .get_result(conn.as_mut())
                .await?;
                jobs::enqueue(
                    conn.as_mut(),
                    release_job(id, pending.then_some(uploads_due))?,
                )
                .await?;
                announce_changed(state, conn.as_mut(), &held).await?;
                Ok(held)
            }
            .scope_boxed()
        })
        .await?;
    drop(conn);
    jobs::wake(state).await;
    Ok(held)
}

/// Posts a message where `to` says, or holds it: with `await_uploads`, as its client said it
/// may, while any of its files is still being uploaded, and with `may_hold` while one of its
/// attachments' previews is being made. A message held has been checked as it would have been
/// posted, what it replies to included; its job posts it. A reply that makes its thread makes
/// it whether it is posted or held, and one held waits in the thread.
#[allow(clippy::too_many_arguments)]
pub async fn post(
    state: &GlobalServerContext,
    author: UserId,
    to: super::To,
    content: String,
    attachments: Vec<AttachmentId>,
    echo_to_parent: bool,
    reply: Option<super::reply::Reply>,
    may_hold: bool,
    await_uploads: bool,
) -> crate::Result<Posted> {
    super::check_content(&content)?;
    if (may_hold || await_uploads) && !attachments.is_empty() {
        let hold = {
            let mut conn = state.connection_pool.get().await?;
            if await_uploads && super::any_unconfirmed(conn.as_mut(), &attachments).await? {
                Some(Hold::Uploads)
            } else if may_hold && held_back(conn.as_mut(), &attachments).await? {
                Some(Hold::Preview)
            } else {
                None
            }
        };
        if let Some(hold) = hold {
            return hold_message(
                state,
                author,
                to,
                content,
                attachments,
                echo_to_parent,
                reply,
                hold,
            )
            .await
            .map(|held| Posted::Held(Box::new(held)));
        }
    }
    // A busy channel's mana (`app::mana`): decided once the author is known to be allowed to
    // post what they sent, with its attachments and what it replies to, so that only what
    // could be posted spends it. A reply that makes its thread goes ahead, since a new
    // thread's pool is full, though its echo spends the parent's (`post_now`).
    let mut conn = state.connection_pool.get().await?;
    let (channel, unmade) = super::resolve(conn.as_mut(), to).await?;
    if let Some(unmade) = &unmade {
        if reply.is_some() {
            return Err(crate::Error::Validation(t!("replyElsewhere")));
        }
        super::check_first_reply(
            state,
            conn.as_mut(),
            author,
            unmade,
            &attachments,
            echo_to_parent,
            super::Uploads::Confirmed,
        )
        .await?;
    } else {
        super::check_posting(
            state,
            conn.as_mut(),
            author,
            channel,
            &attachments,
            echo_to_parent,
        )
        .await?;
        if let Some(reply) = reply {
            super::reply::check(conn.as_mut(), channel, reply.to, false).await?;
        }
        let cost = mana::message_cost(&state.settings(), &content, attachments.len());
        if mana::decide_for(state, conn.as_mut(), author, channel, cost).await? == Decision::Wait {
            drop(conn);
            let held = queue::wait(
                state,
                Waiting {
                    author,
                    channel,
                    kind: HeldKind::Message,
                    content,
                    attachments,
                    echo_to_parent,
                    reply,
                    target: None,
                    payload: None,
                    cost,
                },
            )
            .await?;
            return Ok(Posted::Held(Box::new(held)));
        }
    }
    drop(conn);
    post_now(
        state,
        author,
        to,
        content,
        attachments,
        echo_to_parent,
        reply,
        super::Posting::Text,
        None,
    )
    .await
    .map(|message| Posted::Sent(Box::new(message)))
}

/// Posts a message now (`super::post`), with its echo to the thread's parent when the parent's
/// pool can pay for it; when it cannot, the reply is posted alone and its echo waits in the
/// parent's queue, as an item of its author's. The echo's mana is decided only for an author
/// who may send in the parent, as posting the echo takes, so no one else spends its mana.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn post_now(
    state: &GlobalServerContext,
    author: UserId,
    to: super::To,
    content: String,
    attachments: Vec<AttachmentId>,
    echo_to_parent: bool,
    reply: Option<super::reply::Reply>,
    posting: super::Posting,
    held: Option<HeldMessageId>,
) -> crate::Result<Message> {
    let mut echo = echo_to_parent;
    let mut echo_waits = None;
    if echo_to_parent {
        let mut conn = state.connection_pool.get().await?;
        if let Some(parent) = super::parent_of(conn.as_mut(), to).await? {
            crate::permissions::channel_access(state, conn.as_mut(), author, parent)
                .await?
                .require(crate::permissions::Permissions::SEND_MESSAGES)?;
            let cost = mana::message_cost(&state.settings(), &content, attachments.len());
            if mana::decide_for(state, conn.as_mut(), author, parent, cost).await? == Decision::Wait
            {
                echo = false;
                echo_waits = Some((parent, cost));
            }
        }
    }
    let message = super::post(
        state,
        author,
        to,
        content,
        attachments,
        echo,
        reply,
        posting,
        held,
    )
    .await?;
    if let Some((parent, cost)) = echo_waits {
        let waiting = Waiting {
            author,
            channel: parent,
            kind: HeldKind::Echo,
            content: String::new(),
            attachments: Vec::new(),
            echo_to_parent: false,
            reply: None,
            target: Some(message.id),
            payload: None,
            cost,
        };
        // The reply stands either way; an echo the parent's queue cannot take is not made, and
        // its author may echo the reply again later.
        if let Err(e) = queue::wait(state, waiting).await {
            tracing::debug!(error = %e, "a reply's echo could not wait for its channel's mana");
        }
    }
    Ok(message)
}

#[derive(QueryableByName)]
struct HeldUntil {
    #[diesel(sql_type = Nullable<Timestamptz>)]
    at: Option<DateTime<Utc>>,
}

/// Until when the attachments most holding the messages they are in hold them, if any does now:
/// their preview jobs' `holdUntil`.
async fn held_until(
    conn: &mut AsyncPgConnection,
    attachments: &[AttachmentId],
) -> crate::Result<Option<DateTime<Utc>>> {
    let held: HeldUntil = diesel::sql_query(
        r#"
        SELECT max(until) AS at FROM (
            SELECT (payload->>'holdUntil')::timestamptz AS until FROM job
            WHERE kind = ANY($1) AND key = ANY($2::uuid[]::text[]) AND failed_at IS NULL
        ) holds
        WHERE until > now()
        "#,
    )
    .bind::<Array<Text>, _>(preview::KINDS.to_vec())
    .bind::<Array<PgUuid>, _>(attachments)
    .get_result(conn)
    .await?;
    Ok(held.at)
}

/// Whether any of `attachments` holds the messages it is in now.
async fn held_back(
    conn: &mut AsyncPgConnection,
    attachments: &[AttachmentId],
) -> crate::Result<bool> {
    Ok(held_until(conn, attachments).await?.is_some())
}

/// The place a held message list's cursor holds: the id of the last held message of the page
/// before, which is all the list is ordered by.
fn held_position(cursor: &str) -> crate::Result<HeldMessageId> {
    crate::cursor::decode(List::HeldMessages, cursor)
}

/// A page of the held messages of `author` in the order of their ids, which is the order they
/// were held in, for their apps to show waiting: at most `limit` of them (up to
/// [`crate::LIST_PAGE`]), after the place `cursor` holds when given, which is the `next_cursor`
/// of the page before. Those they scheduled for later are read apart
/// (`super::scheduled::read_scheduled`).
pub async fn read_held(
    state: &GlobalServerContext,
    author: UserId,
    cursor: Option<&str>,
    limit: i64,
) -> crate::Result<Paged<HeldMessage>> {
    let after = cursor.map(held_position).transpose()?;
    read_held_page(
        state.connection_pool.get().await?.as_mut(),
        author,
        after,
        limit,
    )
    .await
}

/// One page of `author`'s held messages and the cursor of the next, by id through the index
/// `held_message_list`, after `after`. The place is compared as an id alone, so the page is
/// read whether or not the message of that id is still held; held messages are posted within
/// seconds, often between two pages.
async fn read_held_page(
    conn: &mut AsyncPgConnection,
    author: UserId,
    after: Option<HeldMessageId>,
    limit: i64,
) -> crate::Result<Paged<HeldMessage>> {
    let limit = limit.clamp(1, crate::LIST_PAGE);
    let rows: Vec<HeldMessage> = diesel::sql_query(format!(
        r#"
        SELECT {COLUMNS}
        FROM held_message
        WHERE author = $1 AND send_at IS NULL AND id > $2
        ORDER BY id
        LIMIT $3
        "#
    ))
    .bind::<PgUuid, _>(author)
    // No id sorts before the nil one, so a first page starts from it.
    .bind::<PgUuid, _>(after.map_or(uuid::Uuid::nil(), |after| after.0))
    .bind::<diesel::sql_types::BigInt, _>(limit + 1)
    .load(conn)
    .await?;
    Ok(crate::cursor::cut(
        List::HeldMessages,
        rows,
        limit as usize,
        |last| last.id,
    ))
}

/// Takes a held message off the table as it is posted, in the posting's transaction; not found
/// when it is no longer there, posted by another server, still waits for its files, or is a
/// scheduled one no longer to be posted now: failed, or given a later time since the posting
/// began.
pub(super) async fn take(conn: &mut AsyncPgConnection, id: HeldMessageId) -> crate::Result<()> {
    let taken = diesel::delete(held_message::table)
        .filter(held_message::id.eq(id))
        .filter(held_message::failure.is_null())
        .filter(held_message::waiting.ne(HeldWaiting::Uploads))
        .filter(
            held_message::send_at
                .is_null()
                .or(held_message::send_at.le(diesel::dsl::now)),
        )
        .execute(conn)
        .await?;
    if taken == 0 {
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    }
    Ok(())
}

/// Cancels a held message of `author`'s that has not been posted, whatever it waits for, and
/// tells their apps. A first reply that made its thread as it was held (`made_thread`) takes
/// the thread with it, as a dropped one does. A scheduled one's job goes with it, and its files,
/// which its author may send another way, are kept as long from now as a new upload is
/// (`attachment::renew_unsent`); the job of one waiting for its files goes with it too. Not
/// found when it is not theirs, or already posted.
pub async fn cancel(
    state: &GlobalServerContext,
    author: UserId,
    id: HeldMessageId,
) -> crate::Result<()> {
    cancel_of(state, author, id, false).await
}

/// [`cancel`], of a scheduled message alone when `scheduled_only`.
pub(super) async fn cancel_of(
    state: &GlobalServerContext,
    author: UserId,
    id: HeldMessageId,
    scheduled_only: bool,
) -> crate::Result<()> {
    use diesel::IntoSql;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction::<_, crate::Error, _>(|conn| {
        async move {
            let (channel, made_thread, send_at, attachments, waiting): (
                ChannelId,
                bool,
                Option<DateTime<Utc>>,
                Vec<Option<AttachmentId>>,
                HeldWaiting,
            ) = diesel::delete(
                held_message::table
                    .filter(held_message::id.eq(id).and(held_message::author.eq(author)))
                    .filter(
                        held_message::send_at
                            .is_not_null()
                            .or((!scheduled_only).into_sql::<Bool>()),
                    ),
            )
            .returning((
                held_message::channel,
                held_message::made_thread,
                held_message::send_at,
                held_message::attachments,
                held_message::waiting,
            ))
            .get_result(conn)
            .await?;
            crate::publish_event(
                state,
                conn,
                EventScope::User(author),
                &ServerEvent::HeldMessageCancelled { held: id, channel },
            )
            .await?;
            if made_thread {
                crate::thread::remove_if_unreplied(state, conn, channel).await?;
            }
            if send_at.is_some() {
                let attachments: Vec<AttachmentId> = attachments.into_iter().flatten().collect();
                crate::attachment::renew_unsent(conn, &attachments).await?;
            }
            if send_at.is_some() || waiting == HeldWaiting::Uploads {
                forget_job(conn, id).await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Deletes the job of the held message `id`, running or not, in the transaction that removes
/// the message or gives it another job: a scheduled message's job may be due years from now,
/// and that of one waiting for its files a day from now, and would otherwise be kept until
/// then to find nothing. A step of it running now does
/// nothing more to the row, and what the runner then records of that step finds no job.
pub(crate) async fn forget_job(
    conn: &mut AsyncPgConnection,
    id: HeldMessageId,
) -> crate::Result<()> {
    diesel::sql_query("DELETE FROM job WHERE kind = $1 AND key = $2")
        .bind::<Text, _>(JobKind::ReleaseHeldMessage)
        .bind::<Text, _>(id.0.to_string())
        .execute(conn)
        .await?;
    Ok(())
}

/// [`take`] for what waits outside `app::message` (a poll), in its posting's transaction.
pub(crate) async fn take_waited(
    conn: &mut AsyncPgConnection,
    id: HeldMessageId,
) -> crate::Result<()> {
    take(conn, id).await
}

/// Tells the author's apps which message a held one became, in the posting's transaction.
pub(super) async fn announce_released(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    author: UserId,
    held: HeldMessageId,
    message: &Message,
) -> crate::Result<()> {
    crate::publish_event(
        state,
        conn,
        EventScope::User(author),
        &ServerEvent::HeldMessagePosted {
            held,
            channel: *message.channel.id(),
            message: message.id,
        },
    )
    .await
}

/// Tells the author's apps that a waiting edit (or poll, or echo) was made: the message it
/// changed or made, in the transaction that made it where there is one.
pub(crate) async fn announce_edited(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    author: UserId,
    held: HeldMessageId,
    channel: ChannelId,
    message: MessageId,
) -> crate::Result<()> {
    crate::publish_event(
        state,
        conn,
        EventScope::User(author),
        &ServerEvent::HeldMessagePosted {
            held,
            channel,
            message,
        },
    )
    .await
}

/// Sets the jobs of the messages `attachment` holds going at once, inside the transaction that
/// ends its hold. Call [`jobs::wake`] once it commits.
pub async fn wake_holding(
    conn: &mut AsyncPgConnection,
    attachment: AttachmentId,
) -> crate::Result<()> {
    wake_holding_any(conn, &[attachment]).await
}

/// [`wake_holding`] for each of `attachments`, in one statement, through
/// `held_message_attachments_idx`.
pub async fn wake_holding_any(
    conn: &mut AsyncPgConnection,
    attachments: &[AttachmentId],
) -> crate::Result<()> {
    diesel::sql_query(
        r#"
        UPDATE job SET not_before = now()
        WHERE kind = $1 AND running_since IS NULL AND failed_at IS NULL
          AND key IN (SELECT id::text FROM held_message WHERE attachments && $2::uuid[])
        "#,
    )
    .bind::<Text, _>(JobKind::ReleaseHeldMessage)
    .bind::<Array<PgUuid>, _>(attachments)
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(QueryableByName)]
struct Awaiting {
    #[diesel(sql_type = PgUuid)]
    id: HeldMessageId,
}

/// Sets going what waited only for `attachment`, in the transaction that confirms it
/// (`attachment::confirm_upload`): each held message naming it whose files are now all
/// confirmed ([`advance`]), through `held_message_attachments_idx`. Answers whether any was, for
/// the caller to wake the job runners once it commits.
///
/// Each row is locked before its files are looked at, in a statement of its own, so of two
/// requests confirming the last two files of one message at once, the second to take the row
/// looks only after the first has committed, and sees its file confirmed.
pub(crate) async fn uploaded(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    attachment: AttachmentId,
) -> crate::Result<bool> {
    let awaiting: Vec<Awaiting> = diesel::sql_query(
        "SELECT id FROM held_message \
         WHERE waiting = 'uploads' AND attachments && ARRAY[$1]::uuid[] \
         ORDER BY id FOR UPDATE",
    )
    .bind::<PgUuid, _>(attachment)
    .load(conn)
    .await?;
    if awaiting.is_empty() {
        return Ok(false);
    }
    let ids: Vec<HeldMessageId> = awaiting.into_iter().map(|row| row.id).collect();
    Ok(!advance(state, conn, &ids).await?.is_empty())
}

/// Of the held messages `ids`, which wait for their files and which the caller's transaction
/// has locked, sets going each whose files are all confirmed, answering them: it then waits
/// for its previews as a message sent with its files ready does, or, scheduled, for its time;
/// its job comes due then (one whose step is running finds the row changed, and looks again),
/// and its author's apps are told.
async fn advance(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    ids: &[HeldMessageId],
) -> crate::Result<Vec<HeldMessage>> {
    let ready: Vec<HeldMessage> = diesel::sql_query(format!(
        "UPDATE held_message h \
         SET waiting = CASE WHEN h.send_at IS NULL THEN 'preview' ELSE 'scheduled' END \
         WHERE h.id = ANY($1) AND h.waiting = 'uploads' AND NOT EXISTS ( \
             SELECT 1 FROM unnest(h.attachments) AS named(id) \
             LEFT JOIN attachment a ON a.id = named.id WHERE a.ready_at IS NULL) \
         RETURNING {COLUMNS}"
    ))
    .bind::<Array<PgUuid>, _>(ids)
    .load(conn)
    .await?;
    if ready.is_empty() {
        return Ok(ready);
    }
    diesel::sql_query(
        "UPDATE job SET due = COALESCE(h.send_at, now()), not_before = COALESCE(h.send_at, now()) \
         FROM held_message h \
         WHERE job.kind = $1 AND job.key = h.id::text AND h.id = ANY($2) \
           AND job.running_since IS NULL AND job.failed_at IS NULL",
    )
    .bind::<Text, _>(JobKind::ReleaseHeldMessage)
    .bind::<Array<PgUuid>, _>(ready.iter().map(|row| row.id).collect::<Vec<_>>())
    .execute(conn)
    .await?;
    for row in &ready {
        if row.send_at.is_some() {
            super::scheduled::announce(state, conn, row).await?;
        } else {
            announce_changed(state, conn, row).await?;
        }
    }
    Ok(ready)
}

/// Brings forward the jobs of the held messages that wait for their uploads and name any of
/// `attachments`, which have been deleted, so each fails now rather than when its wait gives
/// out ([`await_uploads`]). Answers whether there was one, for the caller to wake the runners.
pub(crate) async fn wake_awaiting(
    conn: &mut AsyncPgConnection,
    attachments: &[AttachmentId],
) -> crate::Result<bool> {
    let woken = diesel::sql_query(
        r#"
        UPDATE job SET not_before = now()
        WHERE kind = $1 AND running_since IS NULL AND failed_at IS NULL
          AND key IN (SELECT id::text FROM held_message
                      WHERE waiting = 'uploads' AND attachments && $2::uuid[])
        "#,
    )
    .bind::<Text, _>(JobKind::ReleaseHeldMessage)
    .bind::<Array<PgUuid>, _>(attachments)
    .execute(conn)
    .await?;
    Ok(woken > 0)
}

/// One look of the job of `held`, which waits for its files. It comes due when waiting for
/// them gives out, or sooner when one of them was deleted, and then fails the message, its
/// author told why: dropped as any held message is, or, scheduled, kept failed for them
/// (`super::scheduled`) while they can still see where it would go. A look that finds every
/// file confirmed after all sets it going, as the confirmation would have ([`advance`]); one
/// that finds it no longer waiting for them looks again at once, to post it.
async fn await_uploads(state: &GlobalServerContext, held: HeldMessage) -> crate::Result<Outcome> {
    let locale = crate::locale::negotiate(&held.locale);
    let mut conn = state.connection_pool.get().await?;
    conn.transaction::<_, crate::Error, _>(|conn| {
        async move {
            let waiting: Option<HeldMessage> = diesel::sql_query(format!(
                "SELECT {COLUMNS} FROM held_message \
                 WHERE id = $1 AND waiting = 'uploads' FOR UPDATE"
            ))
            .bind::<PgUuid, _>(held.id)
            .get_result(conn.as_mut())
            .await
            .optional()?;
            let Some(held) = waiting else {
                return Ok(Outcome::Later(Duration::ZERO));
            };
            let advanced = advance(state, conn.as_mut(), &[held.id]).await?;
            if let Some(ready) = advanced.as_slice().first() {
                let left = ready
                    .send_at
                    .and_then(|send_at| (send_at - Utc::now()).to_std().ok())
                    .unwrap_or_default();
                return Ok(Outcome::Later(left));
            }
            let there: Count =
                diesel::sql_query("SELECT count(*) AS count FROM attachment WHERE id = ANY($1)")
                    .bind::<Array<PgUuid>, _>(&held.attachments)
                    .get_result(conn.as_mut())
                    .await?;
            let removed = there.count < held.attachments.len() as i64;
            let left = held
                .uploads_due
                .and_then(|due| (due - Utc::now()).to_std().ok())
                .filter(|left| !left.is_zero());
            if let Some(left) = left
                && !removed
            {
                return Ok(Outcome::Later(left));
            }
            let reason = crate::locale::scope(locale, async {
                if removed {
                    t!("heldUploadRemoved")
                } else {
                    t!("heldUploadsNotFinished", hours = UPLOADS_WAIT.num_hours())
                }
            })
            .await;
            // A scheduled message is kept for its author while they can see where it would
            // go, as `fail_held` keeps one.
            let kept = held.send_at.is_some()
                && match crate::permissions::channel_access(
                    state,
                    conn.as_mut(),
                    held.author,
                    held.channel,
                )
                .await
                {
                    Ok(_) => true,
                    Err(error) if refusal(&error).is_some() => false,
                    Err(error) => return Err(error),
                };
            if kept {
                let failed: HeldMessage = diesel::sql_query(format!(
                    "UPDATE held_message SET failure = $2, failed_at = now(), \
                         waiting = 'scheduled' \
                     WHERE id = $1 RETURNING {COLUMNS}"
                ))
                .bind::<PgUuid, _>(held.id)
                .bind::<Text, _>(reason.as_ref())
                .get_result(conn.as_mut())
                .await?;
                super::scheduled::announce(state, conn.as_mut(), &failed).await?;
            } else {
                drop_held_in(
                    state,
                    conn.as_mut(),
                    held.id,
                    held.author,
                    held.channel,
                    reason,
                )
                .await?;
            }
            Ok(Outcome::Done)
        }
        .scope_boxed()
    })
    .await
}

#[derive(QueryableByName)]
struct Behind {
    #[diesel(sql_type = Bool)]
    behind: bool,
}

/// Whether a message `held`'s author sent before it is still held, its own job not given up:
/// through `held_message_by_author`. Those scheduled for later are not waited for; they keep an
/// order of their own (`scheduled::behind_another`). Nor are those still waiting for their
/// files, which may take as long as they take while what was sent after them goes ahead.
async fn behind_another(conn: &mut AsyncPgConnection, held: &HeldMessage) -> crate::Result<bool> {
    let found: Behind = diesel::sql_query(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM held_message earlier
            JOIN job ON job.kind = $4 AND job.key = earlier.id::text AND job.failed_at IS NULL
            WHERE earlier.author = $1 AND (earlier.held_at, earlier.id) < ($2, $3)
              AND earlier.send_at IS NULL AND earlier.waiting <> 'uploads'
        ) AS behind
        "#,
    )
    .bind::<PgUuid, _>(held.author)
    .bind::<Timestamptz, _>(held.held_at)
    .bind::<PgUuid, _>(held.id)
    .bind::<Text, _>(JobKind::ReleaseHeldMessage)
    .get_result(conn)
    .await?;
    Ok(found.behind)
}

/// A held row as its job looks at it.
#[derive(QueryableByName)]
struct Looked {
    #[diesel(embed)]
    held: HeldMessage,
    /// For a scheduled one, the milliseconds until `send_at` by the database's clock, rounded
    /// up so that waiting that long reaches it; zero or less once it is due.
    #[diesel(sql_type = Nullable<diesel::sql_types::BigInt>)]
    due_in_millis: Option<i64>,
}

/// The held row `id` and how long until it is due. That is asked of the database, whose clock
/// every statement that then posts or fails the row judges `send_at` by ([`take`],
/// [`fail_held`]) and a job's next look is set by, so that a row found due here is due there.
async fn look(conn: &mut AsyncPgConnection, id: HeldMessageId) -> crate::Result<Option<Looked>> {
    Ok(diesel::sql_query(format!(
        "SELECT {COLUMNS}, \
             CEIL(EXTRACT(EPOCH FROM send_at - now()) * 1000)::bigint AS due_in_millis \
         FROM held_message WHERE id = $1"
    ))
    .bind::<PgUuid, _>(id)
    .get_result(conn)
    .await
    .optional()?)
}

/// One held message's job: posts it once nothing holds it, or waits; while it waits for its
/// files, [`await_uploads`].
pub async fn release_step(state: &GlobalServerContext, job: &Claimed) -> crate::Result<Outcome> {
    let Some(id) = job
        .key
        .as_deref()
        .and_then(|key| key.parse().ok())
        .map(HeldMessageId)
    else {
        return Ok(Outcome::Done);
    };
    let held = {
        let mut conn = state.connection_pool.get().await?;
        // Posted, or gone with its author or channel.
        let Some(Looked {
            held,
            due_in_millis,
        }) = look(conn.as_mut(), id).await?
        else {
            return Ok(Outcome::Done);
        };
        // Failed, it waits for its author; released already, for its channel's mana, whose
        // drainer posts it.
        if held.failure.is_some() || held.waiting == HeldWaiting::Mana {
            return Ok(Outcome::Done);
        }
        if held.waiting == HeldWaiting::Uploads {
            drop(conn);
            return await_uploads(state, held).await;
        }
        let behind = match due_in_millis {
            Some(left) => {
                if let Some(left) = u64::try_from(left).ok().filter(|left| *left > 0) {
                    return Ok(Outcome::Later(Duration::from_millis(left)));
                }
                super::scheduled::behind_another(conn.as_mut(), &held).await?
            }
            None => behind_another(conn.as_mut(), &held).await?,
        };
        if behind {
            return Ok(Outcome::Later(BEHIND_WAIT));
        }
        if let Some(until) = held_until(conn.as_mut(), &held.attachments).await? {
            // A moment past it, so the look that follows finds it out.
            let left = (until - Utc::now() + chrono::Duration::milliseconds(20))
                .to_std()
                .unwrap_or_default();
            return Ok(Outcome::Later(left.min(LOOK_EVERY)));
        }
        held
    };
    // Released, it is posted as any message is, and so waits for its channel's mana when the
    // channel is busy.
    let cost = mana::message_cost(&state.settings(), &held.content, held.attachments.len());
    let decided = {
        let mut conn = state.connection_pool.get().await?;
        mana::decide_for(state, conn.as_mut(), held.author, held.channel, cost).await
    };
    if let Ok(Decision::Wait) = decided {
        return Ok(match queue::wait_held(state, &held, cost).await? {
            queue::Waited::Queued => Outcome::Done,
            // No longer as it was read: the next look finds how it stands now.
            queue::Waited::Changed => Outcome::Later(BEHIND_WAIT),
            queue::Waited::NoRoom => Outcome::Later(queue::BUSY_RETRY),
        });
    }
    release(state, held, jobs::last_attempt(job)).await
}

/// Posts what waited for its channel's mana, once its drainer has paid for it, by what it is.
/// One that can no longer be posted is dropped, its author told why; one that fails for a
/// reason that may pass is left to wait, and is dropped if it waits too long. A scheduled
/// message is never dropped for waiting, so one whose posting goes on failing is failed instead
/// ([`note_failing`]).
pub(crate) async fn post_waiting(state: &GlobalServerContext, row: HeldMessage) {
    let id = row.id;
    let scheduled = row
        .send_at
        .is_some()
        .then(|| (row.author, row.channel, row.locale.clone()));
    // Boxed: posting by any of the kinds is a large future, which would otherwise sit inside the
    // drainer's on the stack of the thread running it.
    if let Err(e) = Box::pin(release(state, row, false)).await {
        tracing::warn!(held = %id.0, error = %e, "could not post what waited for a channel's mana");
        if let Some((author, channel, locale)) = scheduled
            && let Err(e) = note_failing(state, id, author, channel, &locale).await
        {
            tracing::warn!(held = %id.0, error = %e, "could not record that posting a scheduled message failed");
        }
    }
}

#[derive(QueryableByName)]
struct Spent {
    #[diesel(sql_type = Bool)]
    spent: bool,
}

/// Records that the due scheduled message `id` found no room in its channel's queue, telling
/// its author's apps that it waits for mana the first time (`stuck_since`): its job tries
/// again every `queue::BUSY_RETRY`, and this says nothing more meanwhile.
pub(crate) async fn note_stuck(
    state: &GlobalServerContext,
    id: HeldMessageId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction::<_, crate::Error, _>(|conn| {
        async move {
            let stuck: Option<HeldMessage> = diesel::sql_query(format!(
                "UPDATE held_message SET stuck_since = now() \
                 WHERE id = $1 AND stuck_since IS NULL AND waiting = 'scheduled' \
                   AND failure IS NULL AND send_at <= now() RETURNING {COLUMNS}"
            ))
            .bind::<PgUuid, _>(id)
            .get_result(conn)
            .await
            .optional()?;
            if let Some(stuck) = stuck {
                super::scheduled::announce(state, conn, &stuck).await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Records that posting the scheduled message `id`, which waits for its channel's mana, failed
/// for a reason that may pass, and fails it once that has gone on as long as anything else may
/// wait there (`queue::MAX_WAIT`): the drainer tries it again every tick until then.
async fn note_failing(
    state: &GlobalServerContext,
    id: HeldMessageId,
    author: UserId,
    channel: ChannelId,
    locale: &str,
) -> crate::Result<()> {
    let failing: Option<Spent> = diesel::sql_query(
        "UPDATE held_message SET stuck_since = COALESCE(stuck_since, now()) \
         WHERE id = $1 AND waiting = 'mana' \
         RETURNING stuck_since < now() - make_interval(secs => $2) AS spent",
    )
    .bind::<PgUuid, _>(id)
    .bind::<diesel::sql_types::Double, _>(queue::MAX_WAIT.as_seconds_f64())
    .get_result(state.connection_pool.get().await?.as_mut())
    .await
    .optional()?;
    if failing.is_some_and(|failing| failing.spent) {
        tracing::error!(held = %id.0, "gave up posting a scheduled message");
        let reason = crate::locale::scope(crate::locale::negotiate(locale), async {
            t!("heldMessageNotPosted")
        })
        .await;
        fail_held(state, id, author, channel, true, reason).await?;
    }
    Ok(())
}

/// Posts one held row by what it is, taking the row in the posting's transaction where the
/// posting has one.
async fn post_row(state: &GlobalServerContext, row: HeldMessage) -> crate::Result<()> {
    let to = super::To::Channel(row.channel);
    match row.kind {
        HeldKind::Message => post_now(
            state,
            row.author,
            to,
            row.content,
            row.attachments,
            row.echo_to_parent,
            row.reply_to.map(|to| super::reply::Reply {
                to,
                tag: row.tags_replied,
            }),
            super::Posting::Text,
            Some(row.id),
        )
        .await
        .map(drop),
        HeldKind::Command => {
            let invocation = serde_json::from_value(row.payload.unwrap_or_default())?;
            super::post(
                state,
                row.author,
                to,
                String::new(),
                row.attachments,
                false,
                None,
                super::Posting::Command(invocation),
                Some(row.id),
            )
            .await
            .map(drop)
        }
        HeldKind::Edit => {
            let change: super::WaitingEdit =
                serde_json::from_value(row.payload.unwrap_or_default())?;
            let Some(target) = row.target else {
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            };
            super::update_message(state, row.author, target, change.into(), Some(row.id))
                .await
                .map(drop)
        }
        HeldKind::Poll => {
            let request = serde_json::from_value(row.payload.unwrap_or_default())?;
            crate::poll::create_poll(state, row.author, row.channel, request, Some(row.id))
                .await
                .map(drop)
        }
        HeldKind::Echo => {
            let Some(target) = row.target else {
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            };
            // Echoing a reply twice makes one echo, so its row goes after.
            let (echo, _) = crate::thread::echo_reply(state, row.author, target).await?;
            let mut conn = state.connection_pool.get().await?;
            take(conn.as_mut(), row.id).await?;
            announce_edited(
                state,
                conn.as_mut(),
                row.author,
                row.id,
                row.channel,
                echo.id,
            )
            .await
        }
    }
}

/// Posts one held row, or fails it with the reason ([`fail_held`]) when it can no longer be
/// posted, or this is its `last` attempt.
async fn release(
    state: &GlobalServerContext,
    row: HeldMessage,
    last: bool,
) -> crate::Result<Outcome> {
    let locale = crate::locale::negotiate(&row.locale);
    let (id, author, channel) = (row.id, row.author, row.channel);
    let read = Standing::of(&row);
    let (posted, noted) =
        crate::locale::scope(locale, crate::events::noting(post_row(state, row))).await;
    crate::events::settle(state, noted, posted.is_err()).await;
    let error = match posted {
        Ok(()) => return Ok(Outcome::Done),
        Err(error) => error,
    };
    // Not found may be its having been posted already, by a run of its job whose lease ran out,
    // or its no longer standing as it was read, so that this was not to post it ([`take`]).
    if matches!(error, crate::Error::Diesel(diesel::result::Error::NotFound)) {
        match standing(state, id).await {
            Ok(None) => return Ok(Outcome::Done),
            Ok(Some(now)) if now != read => return Ok(Outcome::Later(BEHIND_WAIT)),
            _ => {}
        }
    }
    let reason = match refusal(&error) {
        Some(reason) => reason,
        None if last => {
            tracing::error!(held = %id.0, error = %error, "gave up posting a held message");
            crate::locale::scope(locale, async { t!("heldMessageNotPosted") }).await
        }
        None => return Err(error),
    };
    fail_held(state, id, author, channel, read.send_at.is_some(), reason).await?;
    // `fail_held` leaves a scheduled row as it is when it is not due or was given another
    // time; this job is all that will post one still waiting for its time, so it looks again.
    Ok(match standing(state, id).await? {
        Some(now) if now.waiting == HeldWaiting::Scheduled && !now.failed => {
            Outcome::Later(BEHIND_WAIT)
        }
        _ => Outcome::Done,
    })
}

/// Why a message cannot be posted, for its author, when it never will be; `None` for a failure
/// that may pass.
fn refusal(error: &crate::Error) -> Option<Cow<'static, str>> {
    match error {
        crate::Error::Validation(reason)
        | crate::Error::Forbidden(reason)
        | crate::Error::Conflict(reason) => Some(reason.clone()),
        crate::Error::Unauthorized
        | crate::Error::Blocked
        | crate::Error::Diesel(diesel::result::Error::NotFound) => Some(t!("heldMessageNotPosted")),
        _ => None,
    }
}

/// What decides who is to post a held row, and when: what a step read of it is compared with
/// how it stands once posting it found it not to be taken.
#[derive(Debug, PartialEq, Eq, Queryable)]
struct Standing {
    waiting: HeldWaiting,
    send_at: Option<DateTime<Utc>>,
    failed: bool,
}

impl Standing {
    fn of(row: &HeldMessage) -> Self {
        Standing {
            waiting: row.waiting,
            send_at: row.send_at,
            failed: row.failure.is_some(),
        }
    }
}

/// How the held row `id` stands now; `None` when it is gone.
async fn standing(
    state: &GlobalServerContext,
    id: HeldMessageId,
) -> crate::Result<Option<Standing>> {
    Ok(held_message::table
        .select((
            held_message::waiting,
            held_message::send_at,
            held_message::failure.is_not_null(),
        ))
        .filter(held_message::id.eq(id))
        .first(state.connection_pool.get().await?.as_mut())
        .await
        .optional()?)
}

/// Fails `row` ([`fail_held`]), telling its author why, when they can no longer post it where
/// it would go: as posting would refuse it (`may_post`; an edit, their access to the channel),
/// checked before the drainer pays for it, so what will be refused spends nothing. Whether it
/// was failed.
pub(crate) async fn drop_if_unpostable(
    state: &GlobalServerContext,
    row: &HeldMessage,
) -> crate::Result<bool> {
    let locale = crate::locale::negotiate(&row.locale);
    let checked = crate::locale::scope(locale, async {
        let mut conn = state.connection_pool.get().await?;
        match row.kind {
            HeldKind::Edit => {
                crate::permissions::channel_access(state, conn.as_mut(), row.author, row.channel)
                    .await
                    .map(drop)
            }
            _ => super::may_post(state, conn.as_mut(), row.author, row.channel)
                .await
                .map(drop),
        }
    })
    .await;
    let Err(error) = checked else {
        return Ok(false);
    };
    let Some(reason) = crate::locale::scope(locale, async { refusal(&error) }).await else {
        return Err(error);
    };
    fail_held(
        state,
        row.id,
        row.author,
        row.channel,
        row.send_at.is_some(),
        reason,
    )
    .await?;
    Ok(true)
}

/// Where someone can no longer post, so what they have held there will never be
/// (`JobKind::DropHeldBy`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HeldScope {
    /// A community's channels and their threads: they left, were removed, or were banned.
    Community { community: crate::CommunityId },
    /// A group DM and its threads: they left it.
    Dm { channel: ChannelId },
    /// Everywhere: their account was banned from the deployment, or deleted.
    Everywhere,
}

impl HeldScope {
    fn mode(self) -> (&'static str, uuid::Uuid) {
        match self {
            HeldScope::Community { community } => ("community", community.0),
            HeldScope::Dm { channel } => ("dm", channel.0),
            HeldScope::Everywhere => ("everywhere", uuid::Uuid::nil()),
        }
    }
}

/// What a `dropHeldBy` job drops: whose, and where.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeldDrop {
    author: UserId,
    scope: HeldScope,
}

/// How many held messages one step of a `dropHeldBy` job drops.
const DROP_BATCH: i64 = 100;

#[derive(QueryableByName)]
struct HeldIn {
    #[diesel(sql_type = PgUuid)]
    id: HeldMessageId,
    #[diesel(sql_type = PgUuid)]
    channel: ChannelId,
    #[diesel(sql_type = Text)]
    locale: String,
}

/// The first `limit` of what `author` has held in `scope`, oldest first, through
/// `held_message_by_author`: a thread's are in its parent's community or group DM.
async fn held_in(
    conn: &mut AsyncPgConnection,
    author: UserId,
    scope: HeldScope,
    limit: i64,
) -> crate::Result<Vec<HeldIn>> {
    let (mode, place) = scope.mode();
    Ok(diesel::sql_query(
        "SELECT h.id, h.channel, h.locale FROM held_message h \
         JOIN channel c ON c.id = h.channel \
         LEFT JOIN channel parent ON parent.id = c.parent_channel \
         WHERE h.author = $1 AND ($3 = 'everywhere' \
             OR ($3 = 'community' AND COALESCE(c.community, parent.community) = $2) \
             OR ($3 = 'dm' AND COALESCE(c.parent_channel, c.id) = $2)) \
         ORDER BY h.held_at, h.id LIMIT $4",
    )
    .bind::<PgUuid, _>(author)
    .bind::<PgUuid, _>(place)
    .bind::<Text, _>(mode)
    .bind::<diesel::sql_types::BigInt, _>(limit)
    .load(conn)
    .await?)
}

/// Saves, in the caller's transaction, a job dropping everything `author` has held in `scope`,
/// waiting for its previews, for its channel's mana, or for the time it was scheduled for,
/// telling them why: they can no longer post there, so none of it ever will be, and what waits
/// for mana would take up the queue until then. Saves nothing when they have nothing held there.
pub async fn drop_held_by(
    conn: &mut AsyncPgConnection,
    author: UserId,
    scope: HeldScope,
) -> crate::Result<()> {
    if held_in(conn, author, scope, 1).await?.is_empty() {
        return Ok(());
    }
    jobs::enqueue(
        conn,
        NewJob::new(
            JobKind::DropHeldBy,
            JobClass::Normal,
            &HeldDrop { author, scope },
        )?,
    )
    .await?;
    Ok(())
}

/// One step of a `dropHeldBy` job: drops a batch, each told to its author in the language it
/// was sent in, and goes on while batches come back full.
pub async fn drop_held_by_step(
    state: &GlobalServerContext,
    job: &Claimed,
) -> crate::Result<Outcome> {
    let HeldDrop { author, scope } = job.payload()?;
    let rows = {
        let mut conn = state.connection_pool.get().await?;
        held_in(conn.as_mut(), author, scope, DROP_BATCH).await?
    };
    let full = rows.len() as i64 == DROP_BATCH;
    for row in rows {
        let reason = crate::locale::scope(crate::locale::negotiate(&row.locale), async {
            t!("heldNoLongerPermitted")
        })
        .await;
        drop_held(state, row.id, author, row.channel, reason).await?;
    }
    Ok(if full {
        Outcome::Continue
    } else {
        Outcome::Done
    })
}

/// Fails a held message that cannot be posted, telling its author's apps why. Anything but a
/// `scheduled` message is dropped ([`drop_held`]): its author sent it moments ago and is told
/// at once. A scheduled message's author may be away, so it is kept with `detail` as its
/// failure, off the mana queue if it waited there, until they schedule it again or delete it
/// (`super::scheduled`), and their apps are told by `scheduledMessageChanged`; it is dropped
/// all the same when its author can no longer see where it would go, where they could never
/// find it. One already failed, gone, or given another time since it was read is left as it is.
pub(crate) async fn fail_held(
    state: &GlobalServerContext,
    id: HeldMessageId,
    author: UserId,
    channel: ChannelId,
    scheduled: bool,
    detail: Cow<'static, str>,
) -> crate::Result<()> {
    if !scheduled {
        return drop_held(state, id, author, channel, detail).await;
    }
    let mut conn = state.connection_pool.get().await?;
    match crate::permissions::channel_access(state, conn.as_mut(), author, channel).await {
        Ok(_) => {}
        Err(error) if refusal(&error).is_some() => {
            drop(conn);
            return drop_held(state, id, author, channel, detail).await;
        }
        Err(error) => return Err(error),
    }
    conn.transaction::<_, crate::Error, _>(|conn| {
        async move {
            let failed: Option<HeldMessage> = diesel::sql_query(format!(
                "UPDATE held_message SET failure = $2, failed_at = now(), stuck_since = NULL, \
                     waiting = 'scheduled' \
                 WHERE id = $1 AND failure IS NULL AND send_at <= now() RETURNING {COLUMNS}"
            ))
            .bind::<PgUuid, _>(id)
            .bind::<Text, _>(detail.as_ref())
            .get_result(conn)
            .await
            .optional()?;
            if let Some(failed) = failed {
                super::scheduled::announce(state, conn, &failed).await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Drops a held message that will not be posted and tells its author's apps why. When it was the
/// first reply of a thread, which it made as it was held (`post`, `made_thread`), and nothing
/// else was posted or waits there, the thread goes with it (`thread::remove_if_unreplied`), in
/// the same transaction; so does the job of a scheduled message, or of one waiting for its
/// files.
pub(crate) async fn drop_held(
    state: &GlobalServerContext,
    id: HeldMessageId,
    author: UserId,
    channel: ChannelId,
    detail: Cow<'static, str>,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction::<_, crate::Error, _>(|conn| {
        async move { drop_held_in(state, conn, id, author, channel, detail).await }.scope_boxed()
    })
    .await
}

/// [`drop_held`], in the caller's transaction.
async fn drop_held_in(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    id: HeldMessageId,
    author: UserId,
    channel: ChannelId,
    detail: Cow<'static, str>,
) -> crate::Result<()> {
    let dropped: Option<(bool, Option<DateTime<Utc>>, HeldWaiting)> =
        diesel::delete(held_message::table)
            .filter(held_message::id.eq(id))
            .returning((
                held_message::made_thread,
                held_message::send_at,
                held_message::waiting,
            ))
            .get_result(conn)
            .await
            .optional()?;
    let Some((made_thread, send_at, waiting)) = dropped else {
        return Ok(());
    };
    if send_at.is_some() || waiting == HeldWaiting::Uploads {
        forget_job(conn, id).await?;
    }
    crate::publish_event(
        state,
        conn,
        EventScope::User(author),
        &ServerEvent::HeldMessageFailed {
            held: id,
            channel,
            detail: detail.into_owned(),
        },
    )
    .await?;
    if made_thread {
        crate::thread::remove_if_unreplied(state, conn, channel).await?;
    }
    Ok(())
}

#[cfg(test)]
mod database_tests {
    use super::*;
    use diesel_async::{AsyncConnection, SimpleAsyncConnection};

    const ANN: &str = "4e1d0000-0000-4000-8000-000000000001";
    const CHANNEL: &str = "4e1d0000-0000-4000-8000-000000000010";
    const SOON: &str = "4e1d0000-0000-4000-8000-000000000020";
    const DUE: &str = "4e1d0000-0000-4000-8000-000000000021";
    const UNSCHEDULED: &str = "4e1d0000-0000-4000-8000-000000000022";

    fn held(text: &str) -> HeldMessageId {
        HeldMessageId(text.parse().unwrap())
    }

    /// A connection to the database `DATABASE_URL` names, inside a transaction that is never
    /// committed, with an author and a channel; `None` without one, when the test checks
    /// nothing.
    async fn connection() -> Option<AsyncPgConnection> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return None;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(&format!(
            r#"INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at)
                 VALUES ('{ANN}', '{ANN}', '', now(), now());
             INSERT INTO channel (id, name, ty, sort_index) VALUES ('{CHANNEL}', '', 'dm', 0);"#,
        ))
        .await
        .unwrap();
        Some(conn)
    }

    #[tokio::test]
    async fn a_scheduled_row_is_due_by_the_databases_clock() {
        let Some(mut conn) = connection().await else {
            return;
        };
        conn.batch_execute(&format!(
            r#"INSERT INTO held_message
                   (id, author, channel, content, attachments, echo_to_parent, locale, waiting,
                    send_at)
               VALUES
                   ('{SOON}', '{ANN}', '{CHANNEL}', '', '{{}}', false, 'en', 'scheduled',
                    now() + interval '1 hour'),
                   ('{DUE}', '{ANN}', '{CHANNEL}', '', '{{}}', false, 'en', 'scheduled', now()),
                   ('{UNSCHEDULED}', '{ANN}', '{CHANNEL}', '', '{{}}', false, 'en', 'preview',
                    NULL);"#,
        ))
        .await
        .unwrap();
        // `now()` is the transaction's start, so each is exact.
        let soon = look(&mut conn, held(SOON)).await.unwrap().unwrap();
        assert_eq!(soon.held.id, held(SOON));
        assert_eq!(soon.due_in_millis, Some(3_600_000));
        let due = look(&mut conn, held(DUE)).await.unwrap().unwrap();
        assert_eq!(due.due_in_millis, Some(0));
        // What `look` calls due, `take` takes.
        take(&mut conn, held(DUE)).await.unwrap();
        assert!(take(&mut conn, held(SOON)).await.is_err());
        let unscheduled = look(&mut conn, held(UNSCHEDULED)).await.unwrap().unwrap();
        assert_eq!(unscheduled.due_in_millis, None);
        assert!(look(&mut conn, held(DUE)).await.unwrap().is_none());
    }
    /// The held list's cursor is a place, not a held message: when the message it was made
    /// from has been posted since the page before, the rest are still listed, in the order of
    /// their ids whatever `held_at` says, never those scheduled for later, and the last page
    /// says it is the last however many it holds.
    #[tokio::test]
    async fn the_list_pages_past_a_cursor_that_is_gone() {
        const FIRST: &str = "4e1d0000-0000-4000-8000-000000000030";
        const SECOND: &str = "4e1d0000-0000-4000-8000-000000000031";
        const THIRD: &str = "4e1d0000-0000-4000-8000-000000000032";
        const LATER: &str = "4e1d0000-0000-4000-8000-000000000033";
        const FOURTH: &str = "4e1d0000-0000-4000-8000-000000000034";
        let Some(mut conn) = connection().await else {
            return;
        };
        conn.batch_execute(&format!(
            r#"INSERT INTO held_message
                   (id, author, channel, content, attachments, echo_to_parent, locale, waiting,
                    held_at, send_at)
               VALUES
                   ('{FIRST}', '{ANN}', '{CHANNEL}', '', '{{}}', false, 'en', 'mana',
                    now(), NULL),
                   ('{SECOND}', '{ANN}', '{CHANNEL}', '', '{{}}', false, 'en', 'mana',
                    now() - interval '1 minute', NULL),
                   ('{THIRD}', '{ANN}', '{CHANNEL}', '', '{{}}', false, 'en', 'mana',
                    now() - interval '2 minutes', NULL),
                   ('{LATER}', '{ANN}', '{CHANNEL}', '', '{{}}', false, 'en', 'scheduled',
                    now(), now() + interval '1 hour'),
                   ('{FOURTH}', '{ANN}', '{CHANNEL}', '', '{{}}', false, 'en', 'mana',
                    now() - interval '3 minutes', NULL);"#,
        ))
        .await
        .unwrap();
        let author = UserId(ANN.parse().unwrap());
        async fn page(
            conn: &mut AsyncPgConnection,
            author: UserId,
            cursor: Option<&str>,
            limit: i64,
        ) -> (Vec<HeldMessageId>, Option<String>) {
            let after = cursor.map(|c| held_position(c).unwrap());
            let page = read_held_page(conn, author, after, limit).await.unwrap();
            (
                page.items.into_iter().map(|h| h.id).collect(),
                page.next_cursor,
            )
        }
        let (ids, cursor) = page(&mut conn, author, None, 2).await;
        assert_eq!(ids, vec![held(FIRST), held(SECOND)]);
        // The second, which the cursor was made from, is posted.
        conn.batch_execute(&format!("DELETE FROM held_message WHERE id = '{SECOND}'"))
            .await
            .unwrap();
        let (ids, cursor) = page(&mut conn, author, cursor.as_deref(), 1).await;
        assert_eq!(ids, vec![held(THIRD)]);
        assert!(cursor.is_some());
        let (ids, cursor) = page(&mut conn, author, cursor.as_deref(), 1).await;
        assert_eq!(ids, vec![held(FOURTH)]);
        assert_eq!(cursor, None);
        // A page that ends on the last held message has none after it, though it is full.
        let (ids, cursor) = page(&mut conn, author, None, 3).await;
        assert_eq!(ids, vec![held(FIRST), held(THIRD), held(FOURTH)]);
        assert_eq!(cursor, None);
    }

    #[test]
    fn a_cursor_of_no_held_list_is_refused() {
        let refused =
            |cursor: &str| matches!(held_position(cursor), Err(crate::Error::Validation(_)));
        assert!(refused("garbage"));
        // A held message's id as it is, which is what a cursor holds but not a cursor.
        assert!(refused(ANN));
        // A place of another shape, and a saved message list's cursor, whose place is an id
        // alone too.
        assert!(refused(&crate::cursor::encode(
            List::HeldMessages,
            &(1_i64, held(ANN))
        )));
        assert!(refused(&crate::cursor::encode(
            List::SavedMessages,
            &held(ANN)
        )));
        assert!(!refused(&crate::cursor::encode(
            List::HeldMessages,
            &held(ANN)
        )));
    }
}
