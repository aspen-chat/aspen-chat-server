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

use super::Message;
use crate::attachment::preview;
use crate::context::GlobalServerContext;
use crate::jobs::{self, Claimed, JobClass, JobKind, NewJob, Outcome};
use crate::mana::queue::{self, Waiting};
use crate::mana::{self, Decision};
use crate::t;
use crate::{AttachmentId, ChannelId, EventScope, HeldMessageId, MessageId, UserId};
use aspen_schema::held_message;
pub use aspen_wire::held::{HeldKind, HeldWaiting};
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, Utc};
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

/// A message waiting for its attachments' previews, or for its channel's mana, and the other
/// things that wait for mana (`app::mana::queue`).
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
}

/// The columns a [`HeldMessage`] is read from.
pub(crate) const COLUMNS: &str = "id, author, channel, content, attachments, echo_to_parent, \
    reply_to, tags_replied, locale, held_at, waiting, kind, target, payload, cost";

/// What became of a message sent: posted, or held for its previews.
pub enum Posted {
    Sent(Box<Message>),
    Held(HeldMessage),
}

/// Posts a message where `to` says, or holds it while one of its attachments' previews is being
/// made when `may_hold`, as its client said it may. A message held has been checked as it would
/// have been posted, what it replies to included; its job posts it. A reply that makes its
/// thread makes it whether it is posted or held, and one held waits in the thread.
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
) -> crate::Result<Posted> {
    super::check_content(&content)?;
    if may_hold && !attachments.is_empty() {
        let mut conn = state.connection_pool.get().await?;
        if held_back(conn.as_mut(), &attachments).await? {
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
                )
                .await?;
            }
            let id = HeldMessageId::new();
            let locale = crate::locale::current().to_string();
            let held_at = Utc::now();
            let job = NewJob::new(JobKind::ReleaseHeldMessage, JobClass::Interactive, &())?
                .keyed(id.0.to_string());
            let held = conn
                .transaction::<_, crate::Error, _>(|conn| {
                    async move {
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
                            waiting: HeldWaiting::Preview,
                            kind: HeldKind::Message,
                            target: None,
                            payload: None,
                            cost: 0,
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
                            ))
                            .execute(conn.as_mut())
                            .await?;
                        jobs::enqueue(conn.as_mut(), job).await?;
                        Ok(held)
                    }
                    .scope_boxed()
                })
                .await?;
            drop(conn);
            jobs::wake(state).await;
            return Ok(Posted::Held(held));
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
            return Ok(Posted::Held(held));
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

/// A page of the held messages of `author`, oldest first, for their apps to show waiting: `limit`
/// of them (at most [`crate::LIST_PAGE`]) after `after`, through `held_message_by_author`.
pub async fn read_held(
    state: &GlobalServerContext,
    author: UserId,
    after: Option<HeldMessageId>,
    limit: i64,
) -> crate::Result<Vec<HeldMessage>> {
    Ok(diesel::sql_query(format!(
        r#"
        SELECT {COLUMNS}
        FROM held_message
        WHERE author = $1
          AND ($2::uuid IS NULL OR (held_at, id) > (
              SELECT held_at, id FROM held_message WHERE author = $1 AND id = $2))
        ORDER BY held_at, id
        LIMIT $3
        "#
    ))
    .bind::<PgUuid, _>(author)
    .bind::<diesel::sql_types::Nullable<PgUuid>, _>(after)
    .bind::<diesel::sql_types::BigInt, _>(limit.clamp(1, crate::LIST_PAGE))
    .load(state.connection_pool.get().await?.as_mut())
    .await?)
}

/// Takes a held message off the table as it is posted, in the posting's transaction; not found
/// when it is no longer there, posted by another server.
pub(super) async fn take(conn: &mut AsyncPgConnection, id: HeldMessageId) -> crate::Result<()> {
    let taken = diesel::delete(held_message::table)
        .filter(held_message::id.eq(id))
        .execute(conn)
        .await?;
    if taken == 0 {
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    }
    Ok(())
}

/// Cancels a held message of `author`'s that has not been posted, whatever it waits for, and
/// tells their apps. A first reply that made its thread as it was held (`made_thread`) takes
/// the thread with it, as a dropped one does. Not found when it is not theirs, or already
/// posted.
pub async fn cancel(
    state: &GlobalServerContext,
    author: UserId,
    id: HeldMessageId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction::<_, crate::Error, _>(|conn| {
        async move {
            let (channel, made_thread): (ChannelId, bool) = diesel::delete(
                held_message::table
                    .filter(held_message::id.eq(id).and(held_message::author.eq(author))),
            )
            .returning((held_message::channel, held_message::made_thread))
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
            Ok(())
        }
        .scope_boxed()
    })
    .await
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
struct Behind {
    #[diesel(sql_type = Bool)]
    behind: bool,
}

/// Whether a message `held`'s author sent before it is still held, its own job not given up:
/// through `held_message_by_author`.
async fn behind_another(conn: &mut AsyncPgConnection, held: &HeldMessage) -> crate::Result<bool> {
    let found: Behind = diesel::sql_query(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM held_message earlier
            JOIN job ON job.kind = $4 AND job.key = earlier.id::text AND job.failed_at IS NULL
            WHERE earlier.author = $1 AND (earlier.held_at, earlier.id) < ($2, $3)
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

/// One held message's job: posts it once nothing holds it, or waits.
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
        let held: Option<HeldMessage> =
            diesel::sql_query(format!("SELECT {COLUMNS} FROM held_message WHERE id = $1"))
                .bind::<PgUuid, _>(id)
                .get_result(conn.as_mut())
                .await
                .optional()?;
        // Posted, or gone with its author or channel.
        let Some(held) = held else {
            return Ok(Outcome::Done);
        };
        if behind_another(conn.as_mut(), &held).await? {
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
    if held.waiting == HeldWaiting::Mana {
        // Released already: it waits for its channel's mana, whose drainer posts it.
        return Ok(Outcome::Done);
    }
    // Released, it is posted as any message is, and so waits for its channel's mana when the
    // channel is busy.
    let cost = mana::message_cost(&state.settings(), &held.content, held.attachments.len());
    let decided = {
        let mut conn = state.connection_pool.get().await?;
        mana::decide_for(state, conn.as_mut(), held.author, held.channel, cost).await
    };
    if let Ok(Decision::Wait) = decided {
        queue::wait_held(state, held.id, held.author, held.channel, cost).await?;
        return Ok(Outcome::Done);
    }
    release(state, held, jobs::last_attempt(job)).await
}

/// Posts what waited for its channel's mana, once its drainer has paid for it, by what it is.
/// One that can no longer be posted is dropped, its author told why; one that fails for a
/// reason that may pass is left to wait, and is dropped if it waits too long.
pub(crate) async fn post_waiting(state: &GlobalServerContext, row: HeldMessage) {
    let id = row.id;
    // Boxed: posting by any of the kinds is a large future, which would otherwise sit inside the
    // drainer's on the stack of the thread running it.
    if let Err(e) = Box::pin(release(state, row, false)).await {
        tracing::warn!(held = %id.0, error = %e, "could not post what waited for a channel's mana");
    }
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

/// Posts one held row, or drops it with the reason when it can no longer be posted, or this
/// is its `last` attempt.
async fn release(
    state: &GlobalServerContext,
    row: HeldMessage,
    last: bool,
) -> crate::Result<Outcome> {
    let locale = crate::locale::negotiate(&row.locale);
    let (id, author, channel) = (row.id, row.author, row.channel);
    let (posted, noted) =
        crate::locale::scope(locale, crate::events::noting(post_row(state, row))).await;
    crate::events::settle(state, noted, posted.is_err()).await;
    let error = match posted {
        Ok(()) => return Ok(Outcome::Done),
        Err(error) => error,
    };
    // Not found may be its having been posted already, by a run of its job whose lease ran out.
    if matches!(error, crate::Error::Diesel(diesel::result::Error::NotFound))
        && !still_held(state, id).await
    {
        return Ok(Outcome::Done);
    }
    let reason = match refusal(&error) {
        Some(reason) => reason,
        None if last => {
            tracing::error!(held = %id.0, error = %error, "gave up posting a held message");
            crate::locale::scope(locale, async { t!("heldMessageNotPosted") }).await
        }
        None => return Err(error),
    };
    drop_held(state, id, author, channel, reason).await?;
    Ok(Outcome::Done)
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

async fn still_held(state: &GlobalServerContext, id: HeldMessageId) -> bool {
    let Ok(mut conn) = state.connection_pool.get().await else {
        return true;
    };
    diesel::select(diesel::dsl::exists(
        held_message::table.filter(held_message::id.eq(id)),
    ))
    .get_result(conn.as_mut())
    .await
    .unwrap_or(true)
}

/// Drops `row`, telling its author why, when they can no longer post it where it would go: as
/// posting would refuse it (`may_post`; an edit, their access to the channel), checked before
/// the drainer pays for it, so what will be refused spends nothing. Whether it was dropped.
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
    drop_held(state, row.id, row.author, row.channel, reason).await?;
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
/// waiting for its previews or for its channel's mana, telling them why: they can no longer
/// post there, so none of it ever will be, and what waits for mana would take up the queue
/// until then. Saves nothing when they have nothing held there.
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

/// Drops a held message that will not be posted and tells its author's apps why. When it was the
/// first reply of a thread, which it made as it was held (`post`, `made_thread`), and nothing
/// else was posted or waits there, the thread goes with it (`thread::remove_if_unreplied`), in
/// the same transaction.
pub(crate) async fn drop_held(
    state: &GlobalServerContext,
    id: HeldMessageId,
    author: UserId,
    channel: ChannelId,
    detail: Cow<'static, str>,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction::<_, crate::Error, _>(|conn| {
        async move {
            let made_thread: Option<bool> = diesel::delete(held_message::table)
                .filter(held_message::id.eq(id))
                .returning(held_message::made_thread)
                .get_result(conn)
                .await
                .optional()?;
            let Some(made_thread) = made_thread else {
                return Ok(());
            };
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
        .scope_boxed()
    })
    .await
}
