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
//! [`delete_attachment`] removes the row regardless of state and best-effort
//! deletes the S3 object. Stale `ready_at IS NULL` rows whose presigned URL
//! has expired are orphans the operator can sweep on a schedule; this module
//! intentionally does not run that sweep itself so a hung confirm path can't
//! delete an upload that's still racing toward `ready_at`.

use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::media_store::PresignedUpload;
use crate::app::{AttachmentId, Loadable};
use crate::database::schema::attachment;
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
    file_name: String,
    mime_type: String,
    size: Option<(i32, i32)>,
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

/// Verify an in-flight upload landed and flip the row to `ready`.
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
    id: AttachmentId,
) -> app::Result<Attachment> {
    let mut conn = state.connection_pool.get().await?;
    let row: Attachment = attachment::table
        .select(Attachment::as_select())
        .filter(attachment::id.eq(id).and(attachment::ready_at.is_null()))
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

/// Look up an attachment that has finished uploading.
///
/// Pending (`ready_at IS NULL`) rows are deliberately invisible here: from
/// the client's perspective the upload "doesn't exist" until the confirm
/// endpoint has accepted it, and exposing the metadata would make a
/// half-uploaded blob attachable to a message via `create_message`.
pub async fn read_attachment(
    state: &GlobalServerContext,
    id: AttachmentId,
) -> app::Result<Attachment> {
    let mut conn = state.connection_pool.get().await?;
    attachment::table
        .select(Attachment::as_select())
        .filter(
            attachment::id
                .eq(id)
                .and(attachment::ready_at.is_not_null()),
        )
        .first(conn.as_mut())
        .await
        .map_err(Into::into)
}

/// Hard-delete the row and best-effort delete the S3 object.
///
/// Works on both `ready` and pending rows; the latter is what an operator
/// sweep job will call to clean up abandoned reservations.
pub async fn delete_attachment(state: &GlobalServerContext, id: AttachmentId) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let Some(deleted) = diesel::delete(attachment::table)
        .filter(attachment::id.eq(id))
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
