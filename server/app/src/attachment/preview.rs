//! Previews: what readers' apps show inline in place of an original sized for a camera rather
//! than a message list, a smaller copy of a picture, or a video's poster.
//!
//! Confirming a picture or video queues it here ([`queue`], in the transaction that confirms it),
//! and every server whose `[media.previews]` has `make` on makes previews from the queue
//! ([`spawn_maker`]). A maker claims rows by pushing their `not_before` past the time it needs
//! for them, under `FOR UPDATE SKIP LOCKED`, as the mail outbox does, so makers never claim one
//! row twice and a maker that dies leaves its rows to be claimed again. It looks every
//! [`POLL`], and at once when a server publishes on [`WAKE_SUBJECT`], which confirming does.
//! A maker without `ffmpeg` claims pictures only. Until the preview is made, or [`HOLD`] has
//! passed since the upload, messages holding it wait for it (`app::message::held`).
//!
//! A preview is fitted within [`aspen_previews::BOX`], never enlarged, its aspect ratio kept: 960 pixels is the
//! tallest a picture is shown inline (320 CSS pixels) on a screen of three device pixels to
//! the CSS pixel, and 1920 covers the widest message column at two. A picture
//! ([`picture::make`]) is decoded in the server's own process by decoders written in Rust,
//! since the bytes are anyone's; turned upright by its EXIF orientation; brought into sRGB from
//! the colour profile it carries, so that a phone's Display P3 photo keeps its colours; resized
//! with a Lanczos filter; and encoded as lossy WebP at [`aspen_previews::picture::QUALITY`], which at that
//! density shows no loss. Animated pictures keep their original, as do those whose colours
//! cannot be brought into sRGB faithfully (CMYK, a profile that cannot be read). A video's
//! preview ([`video::make`]) is its poster: one frame, taken by the operator's `ffmpeg` and
//! made into a picture's preview, which apps show with a play button, playing the original only
//! when asked. Video is not transcoded: that would cost a server minutes of every core for each
//! minute of video, while a frame takes about a second. HDR video has no poster, since its frame
//! flattened to SDR without tone mapping would look washed out.
//!
//! A preview is kept only when it is at least [`MIN_SAVING_PERCENT`] smaller than the original
//! ([`worth_keeping`]): an original already small and web-ready is shown as it is, saving the
//! storage of a copy no smaller. A kept preview is stored at [`storage_key`], recorded on the
//! attachment row, and announced by `attachmentPreviewed` ([`announce`]); the original stays
//! what is shown at full size, opened in the gallery, and saved.
//!
//! Who may see a preview is exactly who may see its attachment: it is served on the same
//! anonymous-read path, under a key naming the same unguessable id, and it reaches readers only
//! through the attachment's record (`app::attachment::read_attachment`) and the event, which
//! goes to the channels of the messages holding the attachment, as their own events do, or to
//! its uploader alone while it is in none. Deleting the attachment deletes the preview.

use crate::aspen_config::PreviewConfig;
use crate::context::GlobalServerContext;
use crate::{AttachmentId, EventScope, MessageId};
pub use aspen_previews::{Made, Outcome};
use aspen_schema::{attachment, attachment_preview_job, message, message_attachment};
use aspen_wire::attachment::AttachmentPreview;
use aspen_wire::message_enum::server_event::ServerEvent;
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Bool, Integer, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Duration;

pub mod picture;
pub mod video;

/// How much smaller than its original a preview must be to be kept, in percent.
pub const MIN_SAVING_PERCENT: u64 = 10;

/// How long a message holding a picture or video waits for its preview, from when it was
/// uploaded (`app::message::held`).
pub const HOLD: Duration = Duration::from_secs(20);

/// The subject a server publishes on after queueing previews, which wakes every maker.
pub const WAKE_SUBJECT: &str = "aspen.previews.wake";

/// How often a maker looks for work when nothing wakes it.
const POLL: Duration = Duration::from_secs(30);
/// How long a claimed row is held for its maker: longer than the slowest preview, a large
/// video's download and its poster, may take.
const CLAIM_SECONDS: i32 = 900;

/// How many times a row is tried before it is given up: the waits between them, starting at a
/// minute and doubling, add up to about an hour.
const MAX_ATTEMPTS: i32 = 6;

/// The priority of an attachment just uploaded, ahead of those queued when previews began to be
/// made (priority 0), which someone may be waiting for less.
const UPLOADED_PRIORITY: i16 = 10;

/// Whether an attachment of `mime_type`, as its uploader named it, is one a preview may be made
/// of. The bytes decide in the end: a picture that is not one is left as it is.
pub fn wanted(mime_type: &str) -> bool {
    mime_type.starts_with("image/") || mime_type.starts_with("video/")
}

/// Where an attachment's preview is stored: beside the originals rather than under the
/// original's own key, which some object stores cannot hold as both an object and a prefix.
pub fn storage_key(id: AttachmentId) -> String {
    format!("attachment-previews/{}", id.0)
}

/// Whether a preview of `preview_bytes` is worth showing in place of an original of
/// `original_bytes`: at least [`MIN_SAVING_PERCENT`] smaller.
pub fn worth_keeping(preview_bytes: u64, original_bytes: u64) -> bool {
    u128::from(preview_bytes) * 100
        <= u128::from(original_bytes) * u128::from(100 - MIN_SAVING_PERCENT)
}

/// An attachment's preview as readers receive it, where it has one.
pub fn of(state: &GlobalServerContext, row: &super::Attachment) -> Option<AttachmentPreview> {
    let (Some(key), Some(mime_type), Some(width), Some(height)) = (
        &row.preview_storage_key,
        &row.preview_mime_type,
        row.preview_width,
        row.preview_height,
    ) else {
        return None;
    };
    Some(AttachmentPreview {
        url: state.media_store.public_url(key),
        mime_type: mime_type.clone(),
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
    })
}

/// Queues the making of an attachment's preview, in the transaction that confirms it. Call
/// [`wake`] once it commits.
pub async fn queue(conn: &mut AsyncPgConnection, id: AttachmentId) -> crate::Result<()> {
    diesel::insert_into(attachment_preview_job::table)
        .values((
            attachment_preview_job::attachment_id.eq(id),
            attachment_preview_job::priority.eq(UPLOADED_PRIORITY),
            attachment_preview_job::hold_until
                .eq(chrono::Utc::now() + chrono::Duration::from_std(HOLD).unwrap_or_default()),
        ))
        .on_conflict_do_nothing()
        .execute(conn)
        .await?;
    Ok(())
}

/// Wakes every maker, after previews were queued.
pub async fn wake(state: &GlobalServerContext) {
    if let Err(e) = state
        .nats_context
        .client()
        .publish(WAKE_SUBJECT, bytes::Bytes::new())
        .await
    {
        tracing::warn!(error = %e, "could not wake the preview makers");
    }
}

/// A maker's means: this server's settings, and whether it has `ffmpeg`.
struct Maker {
    config: PreviewConfig,
    videos: bool,
    wake: tokio::sync::Notify,
}

/// Starts making previews from the queue, for as long as the server runs, where it makes them.
pub fn spawn_maker(state: GlobalServerContext) {
    let config = state.config.media.previews.clone();
    if !config.make {
        return;
    }
    tokio::spawn(async move {
        let videos = aspen_previews::video::available(&config).await;
        if !videos {
            tracing::warn!(
                ffmpeg = config.ffmpeg,
                ffprobe = config.ffprobe,
                "ffmpeg or ffprobe cannot be run, so this server makes previews of pictures only"
            );
        }
        let maker = Arc::new(Maker {
            config,
            videos,
            wake: tokio::sync::Notify::new(),
        });
        spawn_wake_listener(state.clone(), maker.clone());
        loop {
            match make_batch(&state, &maker).await {
                // A full batch suggests more is waiting.
                Ok(claimed) if claimed == maker.config.concurrency.max(1) => continue,
                Ok(_) => {}
                Err(e) => tracing::error!(error = %e, "could not make previews"),
            }
            tokio::select! {
                () = maker.wake.notified() => {}
                () = tokio::time::sleep(POLL) => {}
            }
        }
    });
}

/// Wakes this server's maker whenever a server says previews were queued.
fn spawn_wake_listener(state: GlobalServerContext, maker: Arc<Maker>) {
    tokio::spawn(async move {
        loop {
            match state.nats_context.client().subscribe(WAKE_SUBJECT).await {
                Ok(mut wakes) => {
                    while wakes.next().await.is_some() {
                        maker.wake.notify_one();
                    }
                }
                Err(e) => tracing::warn!(error = %e, "could not listen for previews to make"),
            }
            tokio::time::sleep(POLL).await;
        }
    });
}

#[derive(QueryableByName)]
struct Claimed {
    #[diesel(sql_type = PgUuid)]
    attachment_id: AttachmentId,
    #[diesel(sql_type = Integer)]
    attempts: i32,
}

/// Claims and makes one batch, as many as this server makes at once, answering how many rows
/// it claimed.
async fn make_batch(state: &GlobalServerContext, maker: &Maker) -> crate::Result<usize> {
    let size = maker.config.concurrency.max(1);
    let claimed: Vec<Claimed> = {
        let mut conn = state.connection_pool.get().await?;
        diesel::sql_query(
            r#"
            UPDATE attachment_preview_job
            SET not_before = now() + make_interval(secs => $1), attempts = attempts + 1
            WHERE attachment_id IN (
                SELECT job.attachment_id FROM attachment_preview_job job
                JOIN attachment ON attachment.id = job.attachment_id
                WHERE job.not_before <= now()
                  AND ($3 OR attachment.mime_type NOT LIKE 'video/%')
                ORDER BY job.priority DESC, job.not_before, job.attachment_id DESC
                LIMIT $2
                FOR UPDATE OF job SKIP LOCKED
            )
            RETURNING attachment_id, attempts
            "#,
        )
        .bind::<Integer, _>(CLAIM_SECONDS)
        .bind::<BigInt, _>(size as i64)
        .bind::<Bool, _>(maker.videos)
        .load(conn.as_mut())
        .await?
    };
    let count = claimed.len();
    futures_util::stream::iter(claimed)
        .for_each_concurrent(size, |row| async move {
            let id = row.attachment_id;
            if let Err(e) = make_one(state, maker, id, row.attempts).await {
                tracing::error!(attachment = %id.0, error = %e, "could not record a preview");
            }
        })
        .await;
    Ok(count)
}

/// Makes one attachment's preview and records what became of it.
async fn make_one(
    state: &GlobalServerContext,
    maker: &Maker,
    id: AttachmentId,
    attempts: i32,
) -> crate::Result<()> {
    let row: Option<super::Attachment> = attachment::table
        .select(super::Attachment::as_select())
        .filter(attachment::id.eq(id))
        .filter(attachment::ready_at.is_not_null())
        .filter(attachment::evidence_at.is_null())
        .first(state.connection_pool.get().await?.as_mut())
        .await
        .optional()?;
    let Some(row) = row.filter(|row| row.preview_storage_key.is_none()) else {
        return finish(state, id).await;
    };
    let Some(original_bytes) = state.media_store.head_object(&row.storage_key).await? else {
        return finish(state, id).await;
    };
    let kind = if row.mime_type.starts_with("image/") {
        "picture"
    } else if row.mime_type.starts_with("video/") {
        "video"
    } else {
        return finish(state, id).await;
    };
    let started = std::time::Instant::now();
    let outcome = match kind {
        "picture" => picture::make(state, &maker.config, &row.storage_key, original_bytes).await,
        _ => video::make(state, &maker.config, &row.storage_key, original_bytes).await,
    };
    metrics::histogram!(aspen_metrics::api::ATTACHMENT_PREVIEW_DURATION, "kind" => kind)
        .record(started.elapsed().as_secs_f64());
    match outcome {
        Outcome::Made(made) if worth_keeping(made.bytes.len() as u64, original_bytes) => {
            tracing::debug!(
                attachment = %id.0,
                original_bytes,
                preview_bytes = made.bytes.len() as u64,
                millis = started.elapsed().as_millis() as u64,
                "made a preview"
            );
            match keep(state, &row, made).await {
                Ok(()) => {
                    metrics::counter!(aspen_metrics::api::ATTACHMENT_PREVIEWS_MADE, "kind" => kind)
                        .increment(1);
                    Ok(())
                }
                Err(e) if attempts >= MAX_ATTEMPTS => {
                    tracing::error!(attachment = %id.0, error = %e, "gave up storing a preview");
                    finish(state, id).await
                }
                Err(e) => {
                    tracing::warn!(attachment = %id.0, error = %e, "could not store a preview; trying later");
                    retry_later(state, id, attempts).await
                }
            }
        }
        Outcome::Made(made) => {
            tracing::debug!(
                attachment = %id.0,
                original_bytes,
                preview_bytes = made.bytes.len() as u64,
                "a preview was not enough smaller than its original to keep"
            );
            finish(state, id).await
        }
        Outcome::NoPreview(reason) => {
            tracing::debug!(attachment = %id.0, reason, "no preview is made");
            finish(state, id).await
        }
        Outcome::Failed(reason) if attempts >= MAX_ATTEMPTS => {
            tracing::error!(attachment = %id.0, reason, "gave up making a preview");
            metrics::counter!(aspen_metrics::api::ATTACHMENT_PREVIEWS_FAILED, "kind" => kind)
                .increment(1);
            finish(state, id).await
        }
        Outcome::Failed(reason) => {
            tracing::warn!(attachment = %id.0, reason, "could not make a preview; trying later");
            retry_later(state, id, attempts).await
        }
    }
}

/// Stores a preview worth keeping, records it, and announces it.
async fn keep(
    state: &GlobalServerContext,
    row: &super::Attachment,
    made: Made,
) -> crate::Result<()> {
    let key = storage_key(row.id);
    state
        .media_store
        .put_bytes(&key, made.bytes, made.mime_type)
        .await?;
    let (width, height) = (
        i32::try_from(made.width).unwrap_or(i32::MAX),
        i32::try_from(made.height).unwrap_or(i32::MAX),
    );
    let id = row.id;
    let mut conn = state.connection_pool.get().await?;
    let recorded = conn
        .transaction::<_, crate::Error, _>(|conn| {
            let key = key.clone();
            async move {
                // Locked before the messages holding it are read, so a message taking it up at
                // the same time either is seen here or waits to be written until this commits
                // (its row in `message_attachment` locks this one), and its readers find the
                // preview on the attachment.
                // Evidence gets no preview on the public read path (`super::evidence`).
                let Some(uploader) = attachment::table
                    .select(attachment::uploader)
                    .filter(attachment::id.eq(id))
                    .filter(attachment::evidence_at.is_null())
                    .for_update()
                    .first::<Option<crate::UserId>>(conn)
                    .await
                    .optional()?
                else {
                    return Ok(false);
                };
                let updated = diesel::update(attachment::table)
                    .filter(attachment::id.eq(id))
                    .set((
                        attachment::preview_storage_key.eq(&key),
                        attachment::preview_mime_type.eq(made.mime_type),
                        attachment::preview_width.eq(width),
                        attachment::preview_height.eq(height),
                    ))
                    .returning(super::Attachment::as_returning())
                    .get_result::<super::Attachment>(conn)
                    .await?;
                if let Some(preview) = of(state, &updated) {
                    announce(state, conn, id, uploader, preview).await?;
                }
                diesel::delete(attachment_preview_job::table)
                    .filter(attachment_preview_job::attachment_id.eq(id))
                    .execute(conn)
                    .await?;
                Ok(true)
            }
            .scope_boxed()
        })
        .await;
    match recorded {
        Ok(true) => {
            crate::message::held::wake(state).await;
            Ok(())
        }
        // Deleted, or made evidence, while its preview was made; the job went with it.
        Ok(false) => {
            state.media_store.delete(&key).await?;
            Ok(())
        }
        Err(e) => Err(e),
    }
}

/// Publishes `attachmentPreviewed` to the channel of each message holding the attachment, or
/// to its uploader while it is in none.
async fn announce(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    id: AttachmentId,
    uploader: Option<crate::UserId>,
    preview: AttachmentPreview,
) -> crate::Result<()> {
    let messages: Vec<MessageId> = message_attachment::table
        .inner_join(message::table)
        .select(message::id)
        .filter(message_attachment::attachment_id.eq(id))
        .filter(message::deleted_at.is_null())
        .load(conn)
        .await?;
    let in_messages: bool = diesel::select(diesel::dsl::exists(
        message_attachment::table.filter(message_attachment::attachment_id.eq(id)),
    ))
    .get_result(conn)
    .await?;
    for message in messages {
        let event = ServerEvent::AttachmentPreviewed {
            attachment: id,
            message: Some(message),
            preview: preview.clone(),
        };
        crate::publish_event(state, conn, EventScope::Message(message), &event).await?;
    }
    // One in a deleted message only is seen by nobody, and announced to nobody.
    if !in_messages && let Some(uploader) = uploader {
        let event = ServerEvent::AttachmentPreviewed {
            attachment: id,
            message: None,
            preview,
        };
        crate::publish_event(state, conn, EventScope::User(uploader), &event).await?;
    }
    Ok(())
}

/// Takes a row off the queue: its preview is made, or none will be. Messages it held go.
async fn finish(state: &GlobalServerContext, id: AttachmentId) -> crate::Result<()> {
    diesel::delete(attachment_preview_job::table)
        .filter(attachment_preview_job::attachment_id.eq(id))
        .execute(state.connection_pool.get().await?.as_mut())
        .await?;
    crate::message::held::wake(state).await;
    Ok(())
}

/// Leaves a row to be tried again, waiting twice as long after each attempt. Messages it held
/// go now, without its preview.
async fn retry_later(
    state: &GlobalServerContext,
    id: AttachmentId,
    attempts: i32,
) -> crate::Result<()> {
    let wait = 60i32.saturating_mul(1 << attempts.clamp(0, 16));
    diesel::update(attachment_preview_job::table)
        .filter(attachment_preview_job::attachment_id.eq(id))
        .set((
            attachment_preview_job::not_before
                .eq(chrono::Utc::now() + chrono::Duration::seconds(i64::from(wait))),
            attachment_preview_job::hold_until.eq(chrono::Utc::now()),
        ))
        .execute(state.connection_pool.get().await?.as_mut())
        .await?;
    crate::message::held::wake(state).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_is_kept_when_a_tenth_smaller() {
        assert!(worth_keeping(90, 100));
        assert!(!worth_keeping(91, 100));
        assert!(!worth_keeping(100, 100));
        assert!(worth_keeping(0, 1));
        assert!(!worth_keeping(1, 1));
        assert!(worth_keeping(u64::MAX / 2, u64::MAX));
    }
}
