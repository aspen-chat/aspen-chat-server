//! Messages held for their previews.
//!
//! A message sent with a picture or video whose preview (`app::attachment::preview`) is still
//! being made would reach readers showing the original inline, then switch to the preview as it
//! lands. A client that says it may (`mayHold`) has such a message held instead ([`post`]): it
//! is checked as any message is, kept in `held_message`, and answered `202 Accepted`, and the
//! author's apps show it waiting. Nobody else learns of it until it is posted.
//!
//! A held message is posted ([`spawn_releaser`], on every server) once none of its attachments
//! holds it: each attachment holds the messages it is in until its preview is made, or found
//! not worth making, or fails, or until its preview job's `hold_until` passes, twenty seconds
//! after the upload (`app::attachment::preview::HOLD`). A preview made after the message is
//! posted still reaches its readers, by `attachmentPreviewed`.
//!
//! Posting it is posting the message as it was sent (`super::post`), with the author's
//! permissions as they are then, its plugins deciding it then, in the language it was sent in;
//! the held row goes in the same transaction, so it is posted once however many servers try,
//! and `heldMessagePosted` tells the author's apps which message it became. One that can no
//! longer be posted (the author lost the right to post there, the channel went) is dropped,
//! and `heldMessageFailed` tells them why. A releaser claims a held message by pushing its
//! `not_before` on, so one that dies leaves it to another.
//!
//! Releasers look when a preview maker finishes or a message is held ([`wake`]), and otherwise
//! when the next hold runs out.

use super::Message;
use crate::api::message_enum::server_event::ServerEvent;
use crate::app::context::GlobalServerContext;
use crate::app::{self, AttachmentId, ChannelId, EventScope, HeldMessageId, UserId};
use crate::t;
use aspen_schema::{attachment_preview_job, held_message};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Bool, Integer, Text, Timestamptz, Uuid as PgUuid};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use futures_util::StreamExt;
use std::borrow::Cow;
use std::sync::Arc;
use std::time::Duration;

/// The subject a server publishes on when held messages may be ready to go.
pub const WAKE_SUBJECT: &str = "aspen.messages.held.wake";

/// The longest a releaser waits between looks.
const POLL: Duration = Duration::from_secs(5);
/// How long a claimed held message is the claimant's to post.
const CLAIM_SECONDS: i32 = 60;
/// How many held messages one look claims.
const BATCH: i64 = 32;
/// How many times posting one is tried before it is dropped.
const MAX_ATTEMPTS: i32 = 10;

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
/// posted; [`spawn_releaser`] posts it.
pub async fn post(
    state: &GlobalServerContext,
    author: UserId,
    channel: ChannelId,
    content: String,
    attachments: Vec<AttachmentId>,
    echo_to_parent: bool,
    may_hold: bool,
) -> app::Result<Posted> {
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
                locale: app::locale::current().to_string(),
                held_at: Utc::now(),
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
                    held_message::locale.eq(&held.locale),
                    held_message::held_at.eq(held.held_at),
                ))
                .execute(conn.as_mut())
                .await?;
            drop(conn);
            wake(state).await;
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

/// Whether any of `attachments` holds the messages it is in now.
async fn held_back(
    conn: &mut AsyncPgConnection,
    attachments: &[AttachmentId],
) -> app::Result<bool> {
    Ok(diesel::select(diesel::dsl::exists(
        attachment_preview_job::table
            .filter(attachment_preview_job::attachment_id.eq_any(attachments))
            .filter(attachment_preview_job::hold_until.gt(diesel::dsl::now)),
    ))
    .get_result(conn)
    .await?)
}

/// The held messages of `author`, oldest first, for their apps to show waiting.
pub async fn read_held(
    state: &GlobalServerContext,
    author: UserId,
) -> app::Result<Vec<HeldMessage>> {
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
pub(super) async fn take(conn: &mut AsyncPgConnection, id: HeldMessageId) -> app::Result<()> {
    let taken = diesel::delete(held_message::table)
        .filter(held_message::id.eq(id))
        .execute(conn)
        .await?;
    if taken == 0 {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
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
) -> app::Result<()> {
    app::publish_event(
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

/// Wakes every releaser.
pub async fn wake(state: &GlobalServerContext) {
    if let Err(e) = state
        .nats_context
        .client()
        .publish(WAKE_SUBJECT, bytes::Bytes::new())
        .await
    {
        tracing::warn!(error = %e, "could not wake the held message releasers");
    }
}

/// Starts posting held messages as they become ready, for as long as the server runs.
pub fn spawn_releaser(state: GlobalServerContext) {
    let woken = Arc::new(tokio::sync::Notify::new());
    spawn_wake_listener(state.clone(), woken.clone());
    tokio::spawn(async move {
        loop {
            match release_batch(&state).await {
                Ok(claimed) if claimed as i64 == BATCH => continue,
                Ok(_) => {}
                Err(e) => tracing::error!(error = %e, "could not post held messages"),
            }
            let wait = next_release(&state).await.unwrap_or(POLL).min(POLL);
            tokio::select! {
                () = woken.notified() => {}
                () = tokio::time::sleep(wait) => {}
            }
        }
    });
}

fn spawn_wake_listener(state: GlobalServerContext, woken: Arc<tokio::sync::Notify>) {
    tokio::spawn(async move {
        loop {
            match state.nats_context.client().subscribe(WAKE_SUBJECT).await {
                Ok(mut wakes) => {
                    while wakes.next().await.is_some() {
                        woken.notify_one();
                    }
                }
                Err(e) => tracing::warn!(error = %e, "could not listen for held messages"),
            }
            tokio::time::sleep(POLL).await;
        }
    });
}

#[derive(QueryableByName)]
struct NextRelease {
    #[diesel(sql_type = diesel::sql_types::Nullable<Timestamptz>)]
    at: Option<DateTime<Utc>>,
}

/// How long until the next hold on a held message runs out, if any does.
async fn next_release(state: &GlobalServerContext) -> Option<Duration> {
    let mut conn = state.connection_pool.get().await.ok()?;
    let next: NextRelease = diesel::sql_query(
        r#"
        SELECT min(job.hold_until) AS at
        FROM held_message held
        JOIN attachment_preview_job job ON job.attachment_id = ANY (held.attachments)
        WHERE job.hold_until > now()
        "#,
    )
    .get_result(conn.as_mut())
    .await
    .ok()?;
    // A moment past it, so the look that follows finds it out.
    (next.at? - Utc::now() + chrono::Duration::milliseconds(20))
        .to_std()
        .ok()
}

#[derive(QueryableByName)]
struct Claimed {
    #[diesel(sql_type = PgUuid)]
    id: HeldMessageId,
    #[diesel(sql_type = PgUuid)]
    author: UserId,
    #[diesel(sql_type = PgUuid)]
    channel: ChannelId,
    #[diesel(sql_type = Text)]
    content: String,
    #[diesel(sql_type = Array<PgUuid>)]
    attachments: Vec<AttachmentId>,
    #[diesel(sql_type = Bool)]
    echo_to_parent: bool,
    #[diesel(sql_type = Text)]
    locale: String,
    #[diesel(sql_type = Integer)]
    attempts: i32,
}

/// Claims the held messages nothing holds any longer and posts them, oldest first, answering
/// how many it claimed.
async fn release_batch(state: &GlobalServerContext) -> app::Result<usize> {
    let claimed: Vec<Claimed> = {
        let mut conn = state.connection_pool.get().await?;
        diesel::sql_query(
            r#"
            UPDATE held_message
            SET not_before = now() + make_interval(secs => $1), attempts = attempts + 1
            WHERE id IN (
                SELECT held.id FROM held_message held
                WHERE held.not_before <= now()
                  AND NOT EXISTS (
                      SELECT 1 FROM attachment_preview_job job
                      WHERE job.attachment_id = ANY (held.attachments)
                        AND job.hold_until > now()
                  )
                ORDER BY held.held_at
                LIMIT $2
                FOR UPDATE SKIP LOCKED
            )
            RETURNING id, author, channel, content, attachments, echo_to_parent, locale, attempts,
                      held_at
            "#,
        )
        .bind::<Integer, _>(CLAIM_SECONDS)
        .bind::<BigInt, _>(BATCH)
        .load(conn.as_mut())
        .await?
    };
    let count = claimed.len();
    let mut claimed = claimed;
    claimed.sort_by_key(|row| row.id);
    // One at a time, so that one author's messages go in the order they were sent.
    for row in claimed {
        release(state, row).await;
    }
    Ok(count)
}

/// Posts one held message, or drops it with the reason when it can no longer be posted.
async fn release(state: &GlobalServerContext, row: Claimed) {
    let locale = app::locale::negotiate(&row.locale);
    let (id, author, channel, attempts) = (row.id, row.author, row.channel, row.attempts);
    let (posted, noted) = app::locale::scope(
        locale,
        app::events::noting(super::post(
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
    app::events::settle(state, noted, posted.is_err()).await;
    let error = match posted {
        Ok(_) => return,
        Err(error) => error,
    };
    // Not found may be another server's having posted it already.
    if matches!(error, app::Error::Diesel(diesel::result::Error::NotFound))
        && !still_held(state, id).await
    {
        return;
    }
    let reason = match refusal(&error) {
        Some(reason) => reason,
        None if attempts >= MAX_ATTEMPTS => {
            tracing::error!(held = %id.0, error = %error, "gave up posting a held message");
            app::locale::scope(locale, async { t!("heldMessageNotPosted") }).await
        }
        None => {
            tracing::warn!(held = %id.0, error = %error, "could not post a held message; trying later");
            return;
        }
    };
    if let Err(e) = drop_held(state, id, author, channel, reason).await {
        tracing::error!(held = %id.0, error = %e, "could not drop a held message");
    }
}

/// Why a message cannot be posted, for its author, when it never will be; `None` for a failure
/// that may pass.
fn refusal(error: &app::Error) -> Option<Cow<'static, str>> {
    match error {
        app::Error::Validation(reason)
        | app::Error::Forbidden(reason)
        | app::Error::Conflict(reason) => Some(reason.clone()),
        app::Error::Unauthorized
        | app::Error::Blocked
        | app::Error::Diesel(diesel::result::Error::NotFound) => Some(t!("heldMessageNotPosted")),
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
) -> app::Result<()> {
    use diesel_async::AsyncConnection;
    use diesel_async::scoped_futures::ScopedFutureExt;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction::<_, app::Error, _>(|conn| {
        async move {
            if take(conn, id).await.is_err() {
                return Ok(());
            }
            app::publish_event(
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
