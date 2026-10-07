//! Two-phase attachment uploads.
//!
//! 1. [`init_upload`] reserves an `attachment` row with `ready_at = NULL`,
//!    presigns a short-lived `PUT` URL, and hands `(id, upload_url, expiry)`
//!    back to the client. Bytes never touch the API process.
//! 2. The client uploads directly to S3 / Garage using the presigned URL, which
//!    writes the object's staging key, never the one readers fetch
//!    (`app::media_store`).
//! 3. [`confirm_upload`] promotes the upload to its own key within the store,
//!    so once confirmed the attachment is what was uploaded and the URL can no
//!    longer change it, flips `ready_at` to `now()`, and returns the wire DTO
//!    including a stable `download_url`. Until that
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
//! Confirming a picture or video queues the making of its preview, a smaller copy for showing it
//! inline, which the servers that make previews do in the background ([`preview`]).
//!
//! [`delete_attachment`] removes an unsent row of the caller's, confirmed or
//! not, and best-effort deletes the S3 object. Stale `ready_at IS NULL` rows
//! whose presigned URL has expired are orphans the operator can sweep on a
//! schedule; this module intentionally does not run that sweep itself so a
//! hung confirm path can't delete an upload that's still racing toward
//! `ready_at`.

use crate::context::GlobalServerContext;
use crate::media_store::{PresignedUpload, Promotion, Served};
use crate::t;
use crate::{AttachmentId, Loadable, UserId};
use aspen_schema::{attachment, message, message_attachment};
use chrono::{DateTime, Utc};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable,
    SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
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
    /// Its preview, all four or none; see [`preview`].
    pub preview_storage_key: Option<String>,
    pub preview_mime_type: Option<String>,
    pub preview_width: Option<i32>,
    pub preview_height: Option<i32>,
}

pub mod preview;

/// The largest side, in pixels, a picture's stated size may have.
pub const MAX_PICTURE_SIDE: u32 = 100_000;

/// A picture's size as an uploader states it: both sides or neither, each from one pixel to
/// `MAX_PICTURE_SIDE`.
pub fn picture_size(width: Option<u32>, height: Option<u32>) -> crate::Result<Option<(i32, i32)>> {
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
            _ => Err(crate::Error::Validation(t!(
                "attachmentDimensions",
                max = MAX_PICTURE_SIDE
            ))),
        },
        _ => Err(crate::Error::Validation(t!(
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
pub fn description(raw: Option<String>) -> crate::Result<Option<String>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let text = raw.trim();
    if text.is_empty() {
        return Ok(None);
    }
    if text.chars().count() > DESCRIPTION_MAX_CHARS {
        return Err(crate::Error::Validation(t!(
            "attachmentDescriptionLength",
            max = DESCRIPTION_MAX_CHARS
        )));
    }
    Ok(Some(text.to_owned()))
}

/// The longest name a file may be sent with, in characters.
pub const MAX_FILE_NAME_CHARS: usize = 255;

/// The longest type a file may be declared as, in bytes; a media type is ASCII (RFC 6838
/// allows 127 characters for each half).
pub const MAX_MIME_TYPE_BYTES: usize = 255;

/// The kinds of file served as what they are, for apps and browsers to show in place: pictures,
/// video, and sound in the formats browsers play, plain text, and PDF, none of which runs script
/// in a page that opens it. Any other kind (an HTML page, an SVG, XML, a script, an archive, or
/// one not given) is uploaded and served as `application/octet-stream`, to be saved, since a
/// browser opening it from storage could run what it holds; its record keeps the kind its
/// uploader declared, which apps show.
pub const INLINE_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/bmp",
    "video/mp4",
    "video/webm",
    "video/ogg",
    "video/quicktime",
    "audio/mpeg",
    "audio/mp4",
    "audio/aac",
    "audio/ogg",
    "audio/opus",
    "audio/wav",
    "audio/webm",
    "audio/flac",
    "text/plain",
    "application/pdf",
];

/// The type of anything not among [`INLINE_TYPES`], which browsers save rather than show.
const SAVED_TYPE: &str = "application/octet-stream";

/// Whether `mime_type` may be declared for an attachment: at most [`MAX_MIME_TYPE_BYTES`] of
/// printable ASCII, without the commas and quotes that would let one header value read as
/// several types, or as parameters a browser reads differently from this server.
pub fn declarable(mime_type: &str) -> bool {
    mime_type.len() <= MAX_MIME_TYPE_BYTES
        && mime_type
            .bytes()
            .all(|b| matches!(b, b' '..=b'~') && b != b',' && b != b'"')
}

/// The longest `charset` parameter served with plain text.
const MAX_CHARSET_BYTES: usize = 40;

/// The `charset` parameter among a type's `parameters` (what follows its essence), when it
/// names a character set in the letters, digits, and punctuation the registered names use.
fn charset<'a>(parameters: impl Iterator<Item = &'a str>) -> Option<String> {
    parameters
        .filter_map(|parameter| parameter.split_once('='))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("charset"))
        .map(|(_, value)| value.trim())
        .filter(|value| {
            (1..=MAX_CHARSET_BYTES).contains(&value.len())
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
        })
        .map(str::to_ascii_lowercase)
}

/// How an attachment of the declared `mime_type` named `file_name` is uploaded and served: as
/// its essence (the type without parameters) when that is among [`INLINE_TYPES`], shown in
/// place, with plain text keeping a `charset` it names; and otherwise as [`SAVED_TYPE`], to be
/// saved; either way under its own name. Nothing else the uploader wrote reaches the header, so
/// what a browser reads as the type is always the type checked here.
pub fn served(mime_type: &str, file_name: &str) -> Served {
    let mut parts = mime_type.split(';');
    let essence = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    let (content_type, disposition) = if INLINE_TYPES.contains(&essence.as_str()) {
        let content_type = match charset(parts).filter(|_| essence == "text/plain") {
            Some(charset) => format!("{essence}; charset={charset}"),
            None => essence,
        };
        (content_type, "inline")
    } else {
        (SAVED_TYPE.to_owned(), "attachment")
    };
    Served {
        content_type,
        disposition: Some(format!("{disposition}; {}", filename_parameters(file_name))),
    }
}

/// `file_name` as `Content-Disposition` gives it (RFC 6266): in full as `filename*`, and as
/// `filename` with anything beyond printable ASCII, and quotes and backslashes, made `_`, for
/// the few that read only that.
fn filename_parameters(file_name: &str) -> String {
    let fallback: String = file_name
        .chars()
        .map(|c| match c {
            ' '..='~' if c != '"' && c != '\\' => c,
            _ => '_',
        })
        .collect();
    let mut encoded = String::with_capacity(file_name.len());
    for byte in file_name.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

/// `bytes` as people read a file's size: in MiB, or KiB below one.
pub fn size_text(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    if bytes >= MIB {
        format!("{} MiB", bytes.div_ceil(MIB))
    } else {
        format!("{} KiB", bytes.div_ceil(KIB))
    }
}

impl Loadable for Attachment {
    type Id = AttachmentId;

    async fn load_from_db(state: &GlobalServerContext, id: AttachmentId) -> crate::Result<Self> {
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
) -> crate::Result<Vec<Attachment>> {
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
    /// The `Content-Type` the upload must be sent with ([`served`]).
    pub content_type: String,
}

pub fn storage_key(id: AttachmentId) -> String {
    format!("attachments/{}", id.0)
}

/// An upload's size as its client declares it, which every upload URL is signed for, so storage
/// takes no more; a client that does not say is refused.
pub fn declared_size(byte_size: Option<u64>) -> crate::Result<u64> {
    byte_size.ok_or_else(|| crate::Error::Validation(t!("uploadSizeRequired")))
}

/// Reserve a row and mint a presigned `PUT` URL, for the type the file is served as
/// ([`served`]) and for exactly `byte_size` bytes, which the client must declare and which may
/// be at most `[media] max_attachment_bytes`.
///
/// On any failure after the row insert (presign error, etc.), the
/// freshly-inserted row is rolled back so a client retry doesn't leak
/// `ready_at IS NULL` rows into the table.
pub async fn init_upload(
    state: &GlobalServerContext,
    caller: UserId,
    file_name: String,
    mime_type: String,
    byte_size: Option<u64>,
    size: Option<(i32, i32)>,
    description: Option<String>,
) -> crate::Result<AttachmentUpload> {
    let max_bytes = state.config.media.max_attachment_bytes;
    let byte_size = declared_size(byte_size)?;
    if byte_size > max_bytes {
        return Err(too_large(max_bytes));
    }
    if file_name.chars().count() > MAX_FILE_NAME_CHARS {
        return Err(crate::Error::Validation(t!(
            "attachmentFileNameLength",
            max = MAX_FILE_NAME_CHARS
        )));
    }
    if !declarable(&mime_type) {
        return Err(crate::Error::Validation(t!(
            "attachmentMimeType",
            max = MAX_MIME_TYPE_BYTES
        )));
    }
    let served = served(&mime_type, &file_name);
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
        preview_storage_key: None,
        preview_mime_type: None,
        preview_width: None,
        preview_height: None,
    };
    let mut conn = state.connection_pool.get().await?;
    diesel::insert_into(attachment::table)
        .values(&row)
        .execute(conn.as_mut())
        .await?;
    match state
        .media_store
        .presign_upload(&key, &served.content_type, byte_size)
        .await
    {
        Ok(PresignedUpload { url, expires_at }) => Ok(AttachmentUpload {
            id,
            upload_url: url,
            expires_at,
            content_type: served.content_type,
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

fn too_large(max_bytes: u64) -> crate::Error {
    crate::Error::Validation(t!("attachmentTooLarge", max = size_text(max_bytes)))
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
///   the client most likely never completed the `PUT`; or the object holds
///   more than `[media] max_attachment_bytes`, and is deleted.
pub async fn confirm_upload(
    state: &GlobalServerContext,
    caller: UserId,
    id: AttachmentId,
) -> crate::Result<Attachment> {
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
    let max_bytes = state.config.media.max_attachment_bytes;
    let served = served(&row.mime_type, &row.file_name);
    match state
        .media_store
        .promote(&row.storage_key, max_bytes, &served)
        .await?
    {
        Promotion::Promoted(_) => {}
        Promotion::NotUploaded => {
            return Err(crate::Error::Validation(t!("attachmentUploadNotFound")));
        }
        Promotion::TooLarge => return Err(too_large(max_bytes)),
    }
    let confirmed = Utc::now();
    let wants_preview = preview::wanted(&row.mime_type);
    let updated: usize = conn
        .transaction::<_, crate::Error, _>(|conn| {
            async move {
                let updated = diesel::update(attachment::table)
                    .filter(attachment::id.eq(id).and(attachment::ready_at.is_null()))
                    .set(attachment::ready_at.eq(confirmed))
                    .execute(conn)
                    .await?;
                if updated > 0 && wants_preview {
                    preview::queue(conn, id).await?;
                }
                Ok(updated)
            }
            .scope_boxed()
        })
        .await?;
    if updated == 0 {
        // Lost a race with a concurrent confirm; treat the second caller as
        // a no-op error rather than re-confirming.
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    }
    if wants_preview {
        preview::wake(state).await;
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
) -> crate::Result<Attachment> {
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
    let channels: Vec<crate::ChannelId> = message_attachment::table
        .inner_join(message::table)
        .select(message::channel)
        .filter(message_attachment::attachment_id.eq(id))
        .filter(message::deleted_at.is_null())
        .load(conn.as_mut())
        .await?;
    for channel in channels {
        if crate::permissions::channel_access_reading(
            state,
            conn.as_mut(),
            caller,
            channel,
            Some(id.0.to_string()),
        )
        .await
        .is_ok()
        {
            return Ok(row);
        }
    }
    Err(crate::Error::Diesel(diesel::result::Error::NotFound))
}

/// Sets or clears the description of an attachment of the caller's that is in no message yet,
/// confirmed or not, and returns it; `None` leaves the description as it is. One in a message, or
/// anyone else's, is answered as not found.
pub async fn describe_attachment(
    state: &GlobalServerContext,
    caller: UserId,
    id: AttachmentId,
    description: Option<Option<String>>,
) -> crate::Result<Attachment> {
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
) -> crate::Result<()> {
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
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    };
    if let Err(e) = state.media_store.delete_upload(&deleted.storage_key).await {
        warn!(
            error = e.to_string(),
            key = deleted.storage_key,
            "failed to delete attachment object from media store after db deletion"
        );
    }
    // A preview being made as it is deleted is deleted by its maker, which finds the row gone.
    if let Some(key) = deleted.preview_storage_key
        && let Err(e) = state.media_store.delete(&key).await
    {
        warn!(
            error = e.to_string(),
            key, "failed to delete attachment preview from media store after db deletion"
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

#[cfg(test)]
mod served_tests {
    use super::*;

    #[test]
    fn what_could_run_in_a_browser_is_saved_rather_than_shown() {
        for declared in [
            "text/html",
            "application/xhtml+xml",
            "image/svg+xml",
            "text/xml",
            "application/xml",
            "text/javascript",
            "application/zip",
            "",
            "TEXT/HTML",
            "text/html; charset=utf-8",
        ] {
            let served = served(declared, "page.html");
            assert_eq!(served.content_type, SAVED_TYPE, "{declared}");
            assert!(
                served.disposition.unwrap().starts_with("attachment; "),
                "{declared}"
            );
        }
        for declared in ["image/png", "video/mp4", "text/plain; charset=utf-8"] {
            let served = served(declared, "a");
            assert_eq!(served.content_type, declared);
            assert!(served.disposition.unwrap().starts_with("inline; "));
        }
    }

    #[test]
    fn only_the_checked_type_reaches_the_header() {
        for (declared, sent) in [
            ("text/plain;x=,text/html", "text/plain"),
            ("Image/PNG; name=a.html", "image/png"),
            (
                "text/plain; Charset=UTF-8; x=y",
                "text/plain; charset=utf-8",
            ),
            ("text/plain; charset=utf-8\r\nX-Evil: 1", "text/plain"),
            ("video/mp4; charset=utf-8", "video/mp4"),
        ] {
            assert_eq!(served(declared, "a").content_type, sent, "{declared}");
        }
    }

    #[test]
    fn a_type_that_could_read_as_several_is_refused() {
        assert!(declarable("text/plain; charset=utf-8"));
        assert!(declarable(""));
        assert!(!declarable("text/plain;x=,text/html"));
        assert!(!declarable("text/plain; charset=\"utf-8\""));
        assert!(!declarable("text/plain\r\nX: y"));
        assert!(!declarable("text/plain\u{7f}"));
        assert!(!declarable("text/pläin"));
        assert!(!declarable(&"a".repeat(MAX_MIME_TYPE_BYTES + 1)));
    }

    #[test]
    fn a_name_is_given_whole_and_safely() {
        assert_eq!(
            filename_parameters("naïve \"plan\".txt"),
            "filename=\"na_ve _plan_.txt\"; filename*=UTF-8''na%C3%AFve%20%22plan%22.txt"
        );
        assert_eq!(
            filename_parameters("a\r\nb;c"),
            "filename=\"a__b;c\"; filename*=UTF-8''a%0D%0Ab%3Bc"
        );
    }

    #[test]
    fn sizes_read_as_people_read_them() {
        assert_eq!(size_text(256 * 1024 * 1024), "256 MiB");
        assert_eq!(size_text(256 * 1024), "256 KiB");
    }
}
