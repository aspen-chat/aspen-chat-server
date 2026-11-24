//! Previews: what readers' apps show inline in place of an original sized for a camera rather
//! than a message list, or served to be saved rather than shown: a smaller copy of a picture, an
//! SVG drawn as one, or a video's poster.
//!
//! Confirming a picture, SVG, or video queues a job making its preview ([`queue`], in the
//! transaction that confirms it): `makePicturePreview`, `makeSvgPreview`, or `makeVideoPoster`,
//! keyed by the attachment (`app::jobs`). Only servers whose `[media.previews]` has `make` on run
//! them, only those that can run `ffmpeg` make posters, and only those whose SVG renderer passes
//! its check draw SVGs ([`svg::make`]); on each, at most `concurrency` run at once. Until the
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
//! storage of a copy no smaller. One of an original served to be saved (an SVG) is kept whatever
//! its size, being all apps can show of it. A kept preview is stored at [`storage_key`], recorded
//! on the attachment row, and announced by `attachmentPreviewed` ([`announce`]); the original
//! stays what is saved, and what is shown at full size and opened in the gallery when it is
//! served for showing in place.
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
pub mod svg;
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
pub const KINDS: [JobKind; 3] = [
    JobKind::MakePicturePreview,
    JobKind::MakeSvgPreview,
    JobKind::MakeVideoPoster,
];

/// How many previews of each kind one account's uploads may have waiting to be made at once,
/// its bots' included. An upload past it gets no preview, and is shown as its original, so no
/// one uploading many pictures that are slow to decode keeps everyone else's previews waiting
/// behind theirs. Counted by kind, since posters may wait days for a server that can run
/// `ffmpeg` while pictures are made at once.
pub const MAX_PENDING_PER_ACCOUNT: i64 = 20;

/// Until when messages holding an attachment wait for its preview, and whose uploads it counts
/// toward: a preview job's payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hold {
    pub hold_until: DateTime<Utc>,
    /// The account charged for the upload (`upload_quota::charged_account`), for
    /// [`MAX_PENDING_PER_ACCOUNT`]; indexed by `job_preview_account`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<crate::UserId>,
}

/// The kind of job that makes a preview of an attachment of `mime_type`, as its uploader named
/// it, where a preview may be made of one.
fn kind_of(mime_type: &str) -> Option<JobKind> {
    let essence = super::essence(mime_type);
    if essence == SVG {
        return Some(JobKind::MakeSvgPreview);
    }
    match essence.split_once('/')?.0 {
        "image" => Some(JobKind::MakePicturePreview),
        "video" => Some(JobKind::MakeVideoPoster),
        _ => None,
    }
}

/// The type SVGs are declared as, whose previews `aspen-svg-render` draws.
pub const SVG: &str = "image/svg+xml";

/// Whether an attachment of `mime_type`, as its uploader named it, is one a preview may be made
/// of. The bytes decide in the end: a picture that is not one is left as it is.
pub fn wanted(mime_type: &str) -> bool {
    kind_of(mime_type).is_some()
}

/// Where an attachment's preview is stored: beside the originals rather than under the
/// original's own key, which some object stores cannot hold as both an object and a prefix.
pub fn storage_key(id: AttachmentId) -> String {
    crate::media_store::PublicPrefix::AttachmentPreviews.key(id.0)
}

/// Whether a copy of `copy_bytes` saves enough over an original of `original_bytes` to be
/// shown in its place: at least [`MIN_SAVING_PERCENT`] smaller.
pub fn smaller_enough(copy_bytes: u64, original_bytes: u64) -> bool {
    u128::from(copy_bytes) * 100
        <= u128::from(original_bytes) * u128::from(100 - MIN_SAVING_PERCENT)
}

/// Whether a preview of `preview_bytes` is worth showing in place of an original of
/// `original_bytes`, of `mime_type` as its uploader declared it: always when the original is
/// not served for showing in place (`super::shown_inline`), since the preview is then all apps
/// can show of it; otherwise when it is [`smaller_enough`].
pub fn worth_keeping(mime_type: &str, preview_bytes: u64, original_bytes: u64) -> bool {
    !super::shown_inline(mime_type) || smaller_enough(preview_bytes, original_bytes)
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

#[derive(QueryableByName)]
struct Pending {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    pending: i64,
}

/// Queues the making of the preview of attachment `id`, of `mime_type`, uploaded by `uploader`,
/// in the transaction that confirms it, unless the account charged for it already has
/// [`MAX_PENDING_PER_ACCOUNT`] waiting. Answers whether it queued one; call [`jobs::wake`] once
/// it commits if so.
pub async fn queue(
    conn: &mut AsyncPgConnection,
    id: AttachmentId,
    uploader: crate::UserId,
    mime_type: &str,
) -> crate::Result<bool> {
    let Some(kind) = kind_of(mime_type) else {
        return Ok(false);
    };
    // Locked until the transaction ends, so two confirmations at once cannot both take the last
    // place.
    let account = crate::upload_quota::charged_account(conn, uploader).await?;
    let Pending { pending } = diesel::sql_query(
        "SELECT count(*) AS pending FROM (SELECT 1 FROM job \
         WHERE kind IN ('makePicturePreview', 'makeVideoPoster', 'makeSvgPreview') \
         AND failed_at IS NULL \
         AND kind = $1 AND payload->>'account' = $2 LIMIT $3) AS waiting",
    )
    .bind::<diesel::sql_types::Text, _>(kind)
    .bind::<diesel::sql_types::Text, _>(account.0.to_string())
    .bind::<diesel::sql_types::BigInt, _>(MAX_PENDING_PER_ACCOUNT)
    .get_result(conn)
    .await?;
    if pending >= MAX_PENDING_PER_ACCOUNT {
        tracing::debug!(attachment = %id.0, account = %account.0, "too many previews waiting");
        return Ok(false);
    }
    let hold = Hold {
        hold_until: Utc::now() + chrono::Duration::from_std(HOLD).unwrap_or_default(),
        account: Some(account),
    };
    jobs::enqueue(
        conn,
        NewJob::new(kind, JobClass::Interactive, &hold)?.keyed(id.0.to_string()),
    )
    .await?;
    Ok(true)
}

/// The attachment a preview job makes the preview of.
fn attachment_of(job: &Claimed) -> crate::Result<AttachmentId> {
    job.key
        .as_deref()
        .and_then(|key| key.parse().ok())
        .map(AttachmentId)
        .ok_or_else(|| crate::Error::PreviewUnmade("a preview job names no attachment".into()))
}

/// How many attachments one step of `queueSvgPreviews` looks through, in id order.
const QUEUE_SVGS_BATCH: i64 = 1000;

/// How far `queueSvgPreviews` has come: every attachment up to `after`, in id order, has had
/// its preview queued if it is an SVG.
#[derive(Serialize, Deserialize)]
struct QueueSvgsProgress {
    after: AttachmentId,
}

#[derive(QueryableByName)]
struct Looked {
    #[diesel(sql_type = PgUuid)]
    id: Uuid,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    wanted: bool,
}

/// One step of the `queueSvgPreviews` job, which a migration queues once: looks through the
/// next [`QUEUE_SVGS_BATCH`] attachments in id order and queues a bulk `makeSvgPreview` for
/// each that is an SVG, confirmed, not evidence, and without a preview, as was every SVG
/// uploaded before SVGs had previews. One queued already is left as it is.
pub async fn queue_svgs_step(
    state: &GlobalServerContext,
    job: &Claimed,
) -> crate::Result<jobs::Outcome> {
    let after = job
        .progress::<QueueSvgsProgress>()?
        .map_or(Uuid::nil(), |progress| progress.after.0);
    let mut conn = state.connection_pool.get().await?;
    let looked: Vec<Looked> = diesel::sql_query(concat!(
        "SELECT id, (ready_at IS NOT NULL AND evidence_at IS NULL \
         AND preview_storage_key IS NULL AND ",
        super::essence_sql!("mime_type"),
        " = $3) AS wanted FROM attachment WHERE id > $1 ORDER BY id LIMIT $2"
    ))
    .bind::<PgUuid, _>(after)
    .bind::<diesel::sql_types::BigInt, _>(QUEUE_SVGS_BATCH)
    .bind::<diesel::sql_types::Text, _>(SVG)
    .load(conn.as_mut())
    .await?;
    let hold = Hold {
        hold_until: Utc::now(),
        account: None,
    };
    let queued = looked
        .iter()
        .filter(|row| row.wanted)
        .map(|row| {
            NewJob::new(JobKind::MakeSvgPreview, JobClass::Bulk, &hold)
                .map(|job| job.keyed(row.id.to_string()))
        })
        .collect::<crate::Result<Vec<_>>>()?;
    let any = !queued.is_empty();
    jobs::enqueue_many(conn.as_mut(), queued).await?;
    drop(conn);
    if any {
        jobs::wake(state).await;
    }
    match looked.last() {
        Some(last) if looked.len() as i64 == QUEUE_SVGS_BATCH => Ok(jobs::Outcome::Progress(
            serde_json::to_value(QueueSvgsProgress {
                after: AttachmentId(last.id),
            })?,
        )),
        _ => Ok(jobs::Outcome::Done),
    }
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
        JobKind::MakePicturePreview => (
            "picture",
            picture::make(state, config, &row.storage_key, original_bytes).await,
        ),
        JobKind::MakeSvgPreview => (
            "svg",
            svg::make(state, config, &row.storage_key, original_bytes).await,
        ),
        other => {
            return Err(crate::Error::PreviewUnmade(format!(
                "{other:?} is not a kind of preview job"
            )));
        }
    };
    metrics::histogram!(aspen_metrics::api::ATTACHMENT_PREVIEW_DURATION, "kind" => kind)
        .record(started.elapsed().as_secs_f64());
    match made {
        Outcome::Made(made)
            if worth_keeping(&row.mime_type, made.bytes.len() as u64, original_bytes) =>
        {
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
        assert!(smaller_enough(90, 100));
        assert!(!smaller_enough(91, 100));
        assert!(!smaller_enough(100, 100));
        assert!(smaller_enough(0, 1));
        assert!(!smaller_enough(1, 1));
        assert!(smaller_enough(u64::MAX / 2, u64::MAX));
        assert!(worth_keeping("image/png", 90, 100));
        assert!(!worth_keeping("image/png", 91, 100));
    }

    #[test]
    fn a_preview_of_what_is_not_shown_in_place_is_always_kept() {
        for saved in [
            SVG,
            "IMAGE/SVG+XML; charset=utf-8",
            "image/tiff",
            "image/heic",
        ] {
            assert!(worth_keeping(saved, 100, 100), "{saved}");
            assert!(worth_keeping(saved, u64::MAX, 1), "{saved}");
        }
    }

    #[test]
    fn the_kind_of_preview_follows_the_type_as_it_is_served() {
        for picture in ["image/png", "Image/PNG", " image/webp ", "IMAGE/jpeg; q=1"] {
            assert_eq!(
                kind_of(picture),
                Some(JobKind::MakePicturePreview),
                "{picture}"
            );
        }
        for video in ["video/mp4", "Video/MP4;codecs=avc1"] {
            assert_eq!(kind_of(video), Some(JobKind::MakeVideoPoster), "{video}");
        }
        for svg in [SVG, "Image/SVG+XML", "image/svg+xml; charset=utf-8"] {
            assert_eq!(kind_of(svg), Some(JobKind::MakeSvgPreview), "{svg}");
        }
        for neither in [
            "application/pdf",
            "image",
            "imagery/png",
            "text/plain; x=image/png",
            "",
        ] {
            assert_eq!(kind_of(neither), None, "{neither}");
        }
    }

    /// Runs against the database `DATABASE_URL` names, inside a transaction that is never
    /// committed; without one it checks nothing.
    #[tokio::test]
    async fn an_account_and_its_bots_have_a_bounded_number_of_previews_waiting() {
        use diesel_async::SimpleAsyncConnection;
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        let (owner, bot) = (Uuid::now_v7(), Uuid::now_v7());
        conn.batch_execute(&format!(
            r#"INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at)
                   VALUES ('{owner}', '{owner}', '', now(), now());
               INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at, bot, bot_owner)
                   VALUES ('{bot}', '{bot}', '', now(), now(), true, '{owner}');"#
        ))
        .await
        .unwrap();
        let queue_one = async |conn: &mut AsyncPgConnection, uploader: Uuid, mime: &str| {
            queue(
                conn,
                AttachmentId(Uuid::now_v7()),
                crate::UserId(uploader),
                mime,
            )
            .await
            .unwrap()
        };
        for i in 0..MAX_PENDING_PER_ACCOUNT {
            let uploader = if i % 2 == 0 { owner } else { bot };
            assert!(queue_one(&mut conn, uploader, "image/png").await);
        }
        // The bot's uploads count as its owner's: neither has room for another picture.
        assert!(!queue_one(&mut conn, owner, "image/png").await);
        assert!(!queue_one(&mut conn, bot, "image/jpeg").await);
        // Posters are counted apart.
        assert!(queue_one(&mut conn, bot, "video/mp4").await);
        // Nothing is queued for what has no preview.
        assert!(!queue_one(&mut conn, owner, "application/pdf").await);
    }
}
