//! Two-phase attachment uploads.
//!
//! 1. [`init_upload`] reserves an `attachment` row with `ready_at = NULL`,
//!    presigns a short-lived `PUT` URL, and hands `(id, upload_url, expiry)`
//!    back to the client. Bytes never touch the API process.
//! 2. The client uploads directly to S3 / Garage using the presigned URL.
//! 3. [`confirm_upload`] HEADs the bucket, flips `ready_at` to `now()`, and
//!    returns the wire DTO including a stable `download_url`. Until that
//!    flip happens the row is invisible to readers and to message-attach
//!    validation, so a half-finished upload can't be referenced from a
//!    message.
//!
//! Each row records its uploader. Until it is in a message an attachment is
//! theirs alone: only they may confirm it, read it, put it in a message, or
//! delete it. Once it is in a message, whoever may view that message's channel
//! may read it, and it goes only with the message (or with its removal by
//! someone allowed to remove it).
//!
//! An attachment may carry a description of what it shows, in its uploader's words, which
//! readers' apps give as the picture's or video's text alternative. It is given when the upload
//! starts or set with [`describe_attachment`] until the attachment is sent; a sent attachment's
//! description is part of the message it went with and stays as it was sent.
//!
//! [`delete_attachment`] removes an unsent row of the caller's, confirmed or
//! not, and best-effort deletes the S3 object. Stale `ready_at IS NULL` rows
//! whose presigned URL has expired are orphans the operator can sweep on a
//! schedule; this module intentionally does not run that sweep itself so a
//! hung confirm path can't delete an upload that's still racing toward
//! `ready_at`.

use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::media_store::PresignedUpload;
use crate::app::{AttachmentId, Loadable, UserId};
use crate::database::schema::{attachment, message, message_attachment};
use crate::t;
use chrono::{DateTime, Utc};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable,
    SelectableHelper,
};
use diesel_async::RunQueryDsl;
use tracing::warn;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = attachment)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Attachment {
    pub id: AttachmentId,
    pub mime_type: String,
    pub file_name: String,
    pub timestamp: chrono::DateTime<Utc>,
    pub storage_key: String,
    pub ready_at: Option<chrono::DateTime<Utc>>,
    /// A picture's size in pixels, as its uploader measured it; both or neither.
    pub width: Option<i32>,
    pub height: Option<i32>,
    /// Who uploaded it; `None` for one uploaded before uploaders were recorded, or whose
    /// uploader's account is gone.
    pub uploader: Option<UserId>,
    /// What it shows, in its uploader's words; see [`description`].
    pub description: Option<String>,
}

/// The largest side, in pixels, a picture's stated size may have.
pub const MAX_PICTURE_SIDE: u32 = 100_000;

/// A picture's size as an uploader states it: both sides or neither, each from one pixel to
/// `MAX_PICTURE_SIDE`.
pub fn picture_size(width: Option<u32>, height: Option<u32>) -> app::Result<Option<(i32, i32)>> {
    let side = |value: u32| {
        (1..=MAX_PICTURE_SIDE)
            .contains(&value)
            .then(|| i32::try_from(value).ok())
            .flatten()
    };
    match (width, height) {
        (None, None) => Ok(None),
        (Some(w), Some(h)) => match (side(w), side(h)) {
            (Some(w), Some(h)) => Ok(Some((w, h))),
            _ => Err(app::Error::Validation(t!(
                "attachmentDimensions",
                max = MAX_PICTURE_SIDE
            ))),
        },
        _ => Err(app::Error::Validation(t!(
            "attachmentDimensions",
            max = MAX_PICTURE_SIDE
        ))),
    }
}

/// The longest description an attachment may have, in characters: a long paragraph, room for
/// a chart or a screenshot of text to be told in full.
pub const DESCRIPTION_MAX_CHARS: usize = 1500;

/// A description as an uploader gives it, with the space around it taken off; one with nothing
/// else in it is no description.
pub fn description(raw: Option<String>) -> app::Result<Option<String>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().count() > DESCRIPTION_MAX_CHARS {
        return Err(app::Error::Validation(t!(
            "attachmentDescriptionLength",
            max = DESCRIPTION_MAX_CHARS
        )));
    }
    Ok(Some(text.to_owned()))
}

impl Loadable for Attachment {
    type Id = AttachmentId;

    async fn load_from_db(state: &GlobalServerContext, id: AttachmentId) -> app::Result<Self> {
        attachment::table
            .select(Attachment::as_select())
            .filter(attachment::id.eq(id))
            .first(&mut state.connection_pool.get().await?)
            .await
            .map_err(Into::into)
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

/// Loads the confirmed attachments among `ids`, in no particular order. Reservations that were
/// never confirmed are invisible here, as they are everywhere else readers look.
pub async fn read_attachments(
    state: &GlobalServerContext,
    ids: &[AttachmentId],
) -> app::Result<Vec<Attachment>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    attachment::table
        .select(Attachment::as_select())
        .filter(
            attachment::id
                .eq_any(ids)
                .and(attachment::ready_at.is_not_null()),
        )
        .load(&mut state.connection_pool.get().await?)
        .await
        .map_err(Into::into)
}

/// Result of an [`init_upload`] call.
#[derive(Debug)]
pub struct AttachmentUpload {
    pub id: AttachmentId,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
}

pub fn storage_key(id: AttachmentId) -> String {
    format!("attachments/{}", id.0)
}

/// Reserve a row and mint a presigned `PUT` URL.
///
/// On any failure after the row insert (presign error, etc.), the
/// freshly-inserted row is rolled back so a client retry doesn't leak
/// `ready_at IS NULL` rows into the table.
pub async fn init_upload(
    state: &GlobalServerContext,
    caller: UserId,
    file_name: String,
    mime_type: String,
    size: Option<(i32, i32)>,
    description: Option<String>,
) -> app::Result<AttachmentUpload> {
    let id = AttachmentId::new();
    let key = storage_key(id);
    let row = Attachment {
        id,
        mime_type: mime_type.clone(),
        file_name,
        timestamp: Utc::now(),
        storage_key: key.clone(),
        ready_at: None,
        width: size.map(|(w, _)| w),
        height: size.map(|(_, h)| h),
        uploader: Some(caller),
        description,
    };
    let mut conn = state.connection_pool.get().await?;
    diesel::insert_into(attachment::table)
        .values(&row)
        .execute(conn.as_mut())
        .await?;
    match state.media_store.presign_put(&key, &mime_type).await {
        Ok(PresignedUpload { url, expires_at }) => Ok(AttachmentUpload {
            id,
            upload_url: url,
            expires_at,
        }),
        Err(e) => {
            if let Err(rollback_err) = diesel::delete(attachment::table)
                .filter(attachment::id.eq(id))
                .execute(conn.as_mut())
                .await
            {
                warn!(
                    error = rollback_err.to_string(),
                    id = id.0.to_string(),
                    "failed to roll back pending attachment row after presign failure"
                );
            }
            Err(e)
        }
    }
}

/// Verify an in-flight upload of the caller's landed and flip the row to `ready`.
///
/// Returns the DB row with a populated `ready_at`. The caller (the API
/// handler) maps that into the wire DTO with `media_store.public_url`.
///
/// Errors:
/// - `Diesel(NotFound)` — no row at all, or the row was already confirmed
///   and a subsequent client retry hit the row instead of a fresh one.
/// - `Validation` — the row exists but no object is present in the bucket;
///   the client most likely never completed the `PUT`.
pub async fn confirm_upload(
    state: &GlobalServerContext,
    caller: UserId,
    id: AttachmentId,
) -> app::Result<Attachment> {
    let mut conn = state.connection_pool.get().await?;
    let row: Attachment = attachment::table
        .select(Attachment::as_select())
        .filter(
            attachment::id
                .eq(id)
                .and(attachment::ready_at.is_null())
                .and(attachment::uploader.eq(caller)),
        )
        .first(conn.as_mut())
        .await?;
    if !state.media_store.head_object(&row.storage_key).await? {
        return Err(app::Error::Validation(t!("attachmentUploadNotFound")));
    }
    let confirmed = Utc::now();
    let updated: usize = diesel::update(attachment::table)
        .filter(attachment::id.eq(id).and(attachment::ready_at.is_null()))
        .set(attachment::ready_at.eq(confirmed))
        .execute(conn.as_mut())
        .await?;
    if updated == 0 {
        // Lost a race with a concurrent confirm; treat the second caller as
        // a no-op error rather than re-confirming.
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    }
    Ok(Attachment {
        ready_at: Some(confirmed),
        ..row
    })
}

/// Look up an attachment that has finished uploading, for its uploader or for someone who may
/// view a message it is in; not found for anyone else.
///
/// Pending (`ready_at IS NULL`) rows are deliberately invisible here: from
/// the client's perspective the upload "doesn't exist" until the confirm
/// endpoint has accepted it, and exposing the metadata would make a
/// half-uploaded blob attachable to a message via `create_message`.
pub async fn read_attachment(
    state: &GlobalServerContext,
    caller: UserId,
    id: AttachmentId,
) -> app::Result<Attachment> {
    let mut conn = state.connection_pool.get().await?;
    let row: Attachment = attachment::table
        .select(Attachment::as_select())
        .filter(
            attachment::id
                .eq(id)
                .and(attachment::ready_at.is_not_null()),
        )
        .first(conn.as_mut())
        .await?;
    if row.uploader == Some(caller) {
        return Ok(row);
    }
    let channels: Vec<app::ChannelId> = message_attachment::table
        .inner_join(message::table)
        .select(message::channel)
        .filter(message_attachment::attachment_id.eq(id))
        .filter(message::deleted_at.is_null())
        .load(conn.as_mut())
        .await?;
    for channel in channels {
        if app::permissions::channel_access(state, conn.as_mut(), caller, channel)
            .await
            .is_ok()
        {
            return Ok(row);
        }
    }
    Err(app::Error::Diesel(diesel::result::Error::NotFound))
}

/// Sets or clears the description of an attachment of the caller's that is in no message yet,
/// confirmed or not, and returns it; `None` leaves the description as it is. One in a message, or
/// anyone else's, is answered as not found.
pub async fn describe_attachment(
    state: &GlobalServerContext,
    caller: UserId,
    id: AttachmentId,
    description: Option<Option<String>>,
) -> app::Result<Attachment> {
    let mut conn = state.connection_pool.get().await?;
    let unsent = attachment::id
        .eq(id)
        .and(attachment::uploader.eq(caller))
        .and(diesel::dsl::not(diesel::dsl::exists(
            message_attachment::table.filter(message_attachment::attachment_id.eq(attachment::id)),
        )));
    let row = match description {
        Some(text) => {
            diesel::update(attachment::table)
                .filter(unsent)
                .set(attachment::description.eq(self::description(text)?))
                .returning(Attachment::as_returning())
                .get_result(conn.as_mut())
                .await?
        }
        None => {
            attachment::table
                .select(Attachment::as_select())
                .filter(unsent)
                .first(conn.as_mut())
                .await?
        }
    };
    Ok(row)
}

/// Hard-delete an attachment of the caller's that is in no message, and best-effort delete the
/// S3 object. Works on both `ready` and pending rows. One in a message goes with the message;
/// asking to delete it, or anyone else's, is answered as not found.
pub async fn delete_attachment(
    state: &GlobalServerContext,
    caller: UserId,
    id: AttachmentId,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let Some(deleted) = diesel::delete(attachment::table)
        .filter(
            attachment::id
                .eq(id)
                .and(attachment::uploader.eq(caller))
                .and(diesel::dsl::not(diesel::dsl::exists(
                    message_attachment::table
                        .filter(message_attachment::attachment_id.eq(attachment::id)),
                ))),
        )
        .returning(Attachment::as_returning())
        .load(conn.as_mut())
        .await?
        .into_iter()
        .next()
    else {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    };
    if let Err(e) = state.media_store.delete(&deleted.storage_key).await {
        warn!(
            error = e.to_string(),
            key = deleted.storage_key,
            "failed to delete attachment object from media store after db deletion"
        );
    }
    Ok(())
}

#[cfg(test)]
mod description_tests {
    use super::*;

    #[test]
    fn a_description_is_trimmed_and_bounded() {
        assert_eq!(description(None).unwrap(), None);
        assert_eq!(description(Some("   \n".into())).unwrap(), None);
        assert_eq!(
            description(Some("  A cat asleep on a keyboard. ".into())).unwrap(),
            Some("A cat asleep on a keyboard.".into())
        );
        let longest = "é".repeat(DESCRIPTION_MAX_CHARS);
        assert_eq!(description(Some(longest.clone())).unwrap(), Some(longest));
        assert!(description(Some("é".repeat(DESCRIPTION_MAX_CHARS + 1))).is_err());
    }
}

#[cfg(test)]
mod picture_size_tests {
    use super::*;

    #[test]
    fn a_picture_states_both_sides_or_neither() {
        assert_eq!(picture_size(None, None).unwrap(), None);
        assert_eq!(
            picture_size(Some(640), Some(480)).unwrap(),
            Some((640, 480))
        );
        assert_eq!(
            picture_size(Some(MAX_PICTURE_SIDE), Some(1)).unwrap(),
            Some((100_000, 1))
        );
        assert!(picture_size(Some(640), None).is_err());
        assert!(picture_size(None, Some(480)).is_err());
        assert!(picture_size(Some(0), Some(480)).is_err());
        assert!(picture_size(Some(MAX_PICTURE_SIDE + 1), Some(480)).is_err());
    }
}
