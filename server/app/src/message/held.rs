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
//! failed [`MAX_ATTEMPTS`] times, is dropped, and `heldMessageFailed` tells them why.

use super::Message;
use crate::attachment::preview;
use crate::context::GlobalServerContext;
use crate::jobs::{self, Claimed, JobClass, JobKind, NewJob, Outcome};
use crate::t;
use crate::{AttachmentId, ChannelId, EventScope, HeldMessageId, UserId};
use aspen_schema::held_message;
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, Bool, Nullable, Text, Timestamptz, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::borrow::Cow;
use std::time::Duration;

/// The longest a held message's job waits between looks while something holds it.
pub const LOOK_EVERY: Duration = Duration::from_secs(5);
/// How long a held message's job waits while a message its author sent before it is held.
const BEHIND_WAIT: Duration = Duration::from_secs(1);
/// How many times posting one is tried before it is dropped.
pub const MAX_ATTEMPTS: i32 = 10;

/// A message waiting for its attachments' previews.
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
    /// The language it was sent in, which a refusal to post it is told in.
    #[diesel(sql_type = Text)]
    pub locale: String,
    #[diesel(sql_type = Timestamptz)]
    pub held_at: DateTime<Utc>,
}

/// What became of a message sent: posted, or held for its previews.
pub enum Posted {
    Sent(Box<Message>),
    Held(HeldMessage),
}

/// Posts a message, or holds it while one of its attachments' previews is being made when
/// `may_hold`, as its client said it may. A message held has been checked as it would have been
/// posted; its job posts it.
pub async fn post(
    state: &GlobalServerContext,
    author: UserId,
    channel: ChannelId,
    content: String,
    attachments: Vec<AttachmentId>,
    echo_to_parent: bool,
    may_hold: bool,
) -> crate::Result<Posted> {
    super::check_content(&content)?;
    if may_hold && !attachments.is_empty() {
        let mut conn = state.connection_pool.get().await?;
        if held_back(conn.as_mut(), &attachments).await? {
            super::check_posting(
                state,
                conn.as_mut(),
                author,
                channel,
                &attachments,
                echo_to_parent,
            )
            .await?;
            let held = HeldMessage {
                id: HeldMessageId::new(),
                author,
                channel,
                content,
                attachments,
                echo_to_parent,
                locale: crate::locale::current().to_string(),
                held_at: Utc::now(),
            };
            let values = (
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
                held_message::locale.eq(&held.locale),
                held_message::held_at.eq(held.held_at),
            );
            let job = NewJob::new(JobKind::ReleaseHeldMessage, JobClass::Interactive, &())?
                .keyed(held.id.0.to_string());
            conn.transaction::<_, crate::Error, _>(|conn| {
                async move {
                    diesel::insert_into(held_message::table)
                        .values(values)
                        .execute(conn)
                        .await?;
                    jobs::enqueue(conn, job).await?;
                    Ok(())
                }
                .scope_boxed()
            })
            .await?;
            drop(conn);
            jobs::wake(state).await;
            return Ok(Posted::Held(held));
        }
    }
    super::create_message(
        state,
        author,
        channel,
        content,
        attachments,
        echo_to_parent,
        super::Posting::Text,
    )
    .await
    .map(|message| Posted::Sent(Box::new(message)))
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

/// The held messages of `author`, oldest first, for their apps to show waiting.
pub async fn read_held(
    state: &GlobalServerContext,
    author: UserId,
) -> crate::Result<Vec<HeldMessage>> {
    Ok(diesel::sql_query(
        r#"
        SELECT id, author, channel, content, attachments, echo_to_parent, locale, held_at
        FROM held_message WHERE author = $1 ORDER BY held_at
        "#,
    )
    .bind::<PgUuid, _>(author)
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
        let held: Option<HeldMessage> = diesel::sql_query(
            r#"
            SELECT id, author, channel, content, attachments, echo_to_parent, locale, held_at
            FROM held_message WHERE id = $1
            "#,
        )
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
    release(state, held, jobs::last_attempt(job)).await
}

/// Posts one held message, or drops it with the reason when it can no longer be posted, or this
/// is its `last` attempt.
async fn release(
    state: &GlobalServerContext,
    row: HeldMessage,
    last: bool,
) -> crate::Result<Outcome> {
    let locale = crate::locale::negotiate(&row.locale);
    let (id, author, channel) = (row.id, row.author, row.channel);
    let (posted, noted) = crate::locale::scope(
        locale,
        crate::events::noting(super::post(
            state,
            row.author,
            row.channel,
            row.content,
            row.attachments,
            row.echo_to_parent,
            super::Posting::Text,
            Some(row.id),
        )),
    )
    .await;
    crate::events::settle(state, noted, posted.is_err()).await;
    let error = match posted {
        Ok(_) => return Ok(Outcome::Done),
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

/// Drops a held message that will not be posted and tells its author's apps why.
async fn drop_held(
    state: &GlobalServerContext,
    id: HeldMessageId,
    author: UserId,
    channel: ChannelId,
    detail: Cow<'static, str>,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction::<_, crate::Error, _>(|conn| {
        async move {
            if take(conn, id).await.is_err() {
                return Ok(());
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
            .await
        }
        .scope_boxed()
    })
    .await
}
