//! Previews: what readers' apps show inline in place of an original sized for a camera rather
//! than a message list, a smaller copy of a picture, or a video's poster.
//!
//! Confirming a picture or video queues a job making its preview ([`queue`], in the transaction
//! that confirms it): `makePicturePreview` or `makeVideoPoster`, keyed by the attachment
//! (`app::jobs`). Only servers whose `[media.previews]` has `make` on run them, and only those
//! that can run `ffmpeg` make posters; on each, at most `concurrency` run at once. Until the
//! preview is made, or found not worth making, or fails, or until [`HOLD`] has passed since the
//! upload (the job's `holdUntil`), messages holding the attachment wait for it
//! (`app::message::held`). A preview no server has made within [`GIVEN_UP_AFTER`] is not made.
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

use crate::context::GlobalServerContext;
use crate::jobs::{self, Claimed, JobClass, JobKind, NewJob};
use crate::{AttachmentId, EventScope, MessageId};
pub use aspen_previews::{Made, Outcome};
use aspen_schema::{attachment, message, message_attachment};
use aspen_wire::attachment::AttachmentPreview;
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::Uuid as PgUuid;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

pub mod picture;
pub mod video;

/// How much smaller than its original a preview must be to be kept, in percent.
pub const MIN_SAVING_PERCENT: u64 = 10;

/// How long a message holding a picture or video waits for its preview, from when it was
/// uploaded (`app::message::held`).
pub const HOLD: Duration = Duration::from_secs(20);

/// How long a preview may wait for a server to make it, as one may when no server can run
/// `ffmpeg`, before it is given up (`jobs::upkeep::prune_failed_jobs`).
pub const GIVEN_UP_AFTER: Duration = Duration::from_secs(7 * 24 * 3600);

/// How many times a preview is tried before it is given up: the waits between them, starting
/// at a minute and doubling, add up to about an hour.
pub const MAX_ATTEMPTS: i32 = 6;

/// The kinds of job that make previews, whose payloads are [`Hold`]s.
pub const KINDS: [JobKind; 2] = [JobKind::MakePicturePreview, JobKind::MakeVideoPoster];

/// Until when messages holding an attachment wait for its preview: a preview job's payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hold {
    pub hold_until: DateTime<Utc>,
}

/// The kind of job that makes a preview of an attachment of `mime_type`, as its uploader named
/// it, where a preview may be made of one.
fn kind_of(mime_type: &str) -> Option<JobKind> {
    if mime_type.starts_with("image/") {
        Some(JobKind::MakePicturePreview)
    } else if mime_type.starts_with("video/") {
        Some(JobKind::MakeVideoPoster)
    } else {
        None
    }
}

/// Whether an attachment of `mime_type`, as its uploader named it, is one a preview may be made
/// of. The bytes decide in the end: a picture that is not one is left as it is.
pub fn wanted(mime_type: &str) -> bool {
    kind_of(mime_type).is_some()
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

/// Queues the making of the preview of attachment `id`, of `mime_type`, in the transaction that
/// confirms it. Call [`jobs::wake`] once it commits.
pub async fn queue(
    conn: &mut AsyncPgConnection,
    id: AttachmentId,
    mime_type: &str,
) -> crate::Result<()> {
    let Some(kind) = kind_of(mime_type) else {
        return Ok(());
    };
    let hold = Hold {
        hold_until: Utc::now() + chrono::Duration::from_std(HOLD).unwrap_or_default(),
    };
    jobs::enqueue(
        conn,
        NewJob::new(kind, JobClass::Interactive, &hold)?.keyed(id.0.to_string()),
    )
    .await?;
    Ok(())
}

/// The attachment a preview job makes the preview of.
fn attachment_of(job: &Claimed) -> crate::Result<AttachmentId> {
    job.key
        .as_deref()
        .and_then(|key| key.parse().ok())
        .map(AttachmentId)
        .ok_or_else(|| crate::Error::PreviewUnmade("a preview job names no attachment".into()))
}

/// One preview job: makes the attachment's preview and records what became of it.
pub async fn make_step(state: &GlobalServerContext, job: &Claimed) -> crate::Result<jobs::Outcome> {
    let id = attachment_of(job)?;
    let row: Option<super::Attachment> = attachment::table
        .select(super::Attachment::as_select())
        .filter(attachment::id.eq(id))
        .filter(attachment::ready_at.is_not_null())
        .filter(attachment::evidence_at.is_null())
        .first(state.connection_pool.get().await?.as_mut())
        .await
        .optional()?;
    let Some(row) = row.filter(|row| row.preview_storage_key.is_none()) else {
        return finish(state, job).await;
    };
    let Some(original_bytes) = state.media_store.head_object(&row.storage_key).await? else {
        return finish(state, job).await;
    };
    let config = &state.config.media.previews;
    let started = std::time::Instant::now();
    let (kind, made) = match job.kind {
        JobKind::MakeVideoPoster => (
            "video",
            video::make(state, config, &row.storage_key, original_bytes).await,
        ),
        _ => (
            "picture",
            picture::make(state, config, &row.storage_key, original_bytes).await,
        ),
    };
    metrics::histogram!(aspen_metrics::api::ATTACHMENT_PREVIEW_DURATION, "kind" => kind)
        .record(started.elapsed().as_secs_f64());
    match made {
        Outcome::Made(made) if worth_keeping(made.bytes.len() as u64, original_bytes) => {
            tracing::debug!(
                attachment = %id.0,
                original_bytes,
                preview_bytes = made.bytes.len() as u64,
                millis = started.elapsed().as_millis() as u64,
                "made a preview"
            );
            if let Err(e) = keep(state, job, &row, made).await {
                release_hold(state, job).await;
                return Err(e);
            }
            metrics::counter!(aspen_metrics::api::ATTACHMENT_PREVIEWS_MADE, "kind" => kind)
                .increment(1);
            Ok(jobs::Outcome::Done)
        }
        Outcome::Made(made) => {
            tracing::debug!(
                attachment = %id.0,
                original_bytes,
                preview_bytes = made.bytes.len() as u64,
                "a preview was not enough smaller than its original to keep"
            );
            finish(state, job).await
        }
        Outcome::NoPreview(reason) => {
            tracing::debug!(attachment = %id.0, reason, "no preview is made");
            finish(state, job).await
        }
        Outcome::Failed(reason) => {
            if jobs::last_attempt(job) {
                metrics::counter!(aspen_metrics::api::ATTACHMENT_PREVIEWS_FAILED, "kind" => kind)
                    .increment(1);
            }
            release_hold(state, job).await;
            Err(crate::Error::PreviewUnmade(reason.to_string()))
        }
    }
}

/// Stores a preview worth keeping, records it, and announces it.
async fn keep(
    state: &GlobalServerContext,
    job: &Claimed,
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
                settle(conn, job.id, id).await?;
                Ok(true)
            }
            .scope_boxed()
        })
        .await;
    match recorded {
        Ok(true) => {
            jobs::wake(state).await;
            Ok(())
        }
        // Deleted, or made evidence, while its preview was made: nothing waits for it.
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

/// Deletes preview job `job` of attachment `id` on `conn`, inside the transaction that recorded
/// its preview or found none would be made, and sets the messages it held going. The runner's
/// own deletion of it once the step is done then finds nothing, while what it held never sees
/// it after its preview is recorded.
async fn settle(conn: &mut AsyncPgConnection, job: Uuid, id: AttachmentId) -> crate::Result<()> {
    diesel::sql_query("DELETE FROM job WHERE id = $1")
        .bind::<PgUuid, _>(job)
        .execute(&mut *conn)
        .await?;
    crate::message::held::wake_holding(conn, id).await
}

/// Done with a preview job that made nothing to keep: its preview is not worth making, or its
/// attachment went.
async fn finish(state: &GlobalServerContext, job: &Claimed) -> crate::Result<jobs::Outcome> {
    let id = attachment_of(job)?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction::<_, crate::Error, _>(|conn| settle(conn, job.id, id).scope_boxed())
        .await?;
    drop(conn);
    jobs::wake(state).await;
    Ok(jobs::Outcome::Done)
}

/// Lets the messages preview job `job` holds go without its preview, as it is to be tried again
/// later.
async fn release_hold(state: &GlobalServerContext, job: &Claimed) {
    let released = async {
        let id = attachment_of(job)?;
        let mut conn = state.connection_pool.get().await?;
        conn.transaction::<_, crate::Error, _>(|conn| {
            async move {
                diesel::sql_query(
                    "UPDATE job SET payload = jsonb_set(payload, '{holdUntil}', to_jsonb(now())) \
                     WHERE id = $1",
                )
                .bind::<PgUuid, _>(job.id)
                .execute(&mut *conn)
                .await?;
                crate::message::held::wake_holding(conn, id).await
            }
            .scope_boxed()
        })
        .await
    }
    .await;
    match released {
        Ok(()) => jobs::wake(state).await,
        Err(e) => tracing::warn!(job = %job.id, error = %e, "could not let held messages go"),
    }
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
