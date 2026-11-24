//! Two-phase icon uploads.
//!
//! Same shape as [`crate::attachment`]: the server reserves a row in
//! the `icon` table with `ready_at = NULL`, hands back a presigned `PUT`
//! URL for the icon's staging key, and waits for the client to confirm,
//! promoting the upload to the icon's own key, before flipping the row to
//! ready and exposing it to readers. Icons carry no `file_name`, only a
//! mime type, so the wire surface is one field shorter; everything else
//! is symmetric.
//!
//! An icon is stored as it is shown ([`as_stored`]): a picture larger than
//! [`STORED_BOX`], or not already WebP, is fitted within it and stored as WebP
//! in place of what was uploaded, when that comes out smaller. Icons confirmed
//! before this was so are brought to it by the `shrinkIcons` job
//! ([`shrink_step`]).
//!
//! An icon is a picture of one of [`IMAGE_TYPES`], and records who uploaded
//! it (`icon.uploaded_by`). Through the API only they may confirm it, give it
//! to something ([`require_own`]: their profile or a bot's, a community, a
//! custom emoji, the deployment's profile), or delete it
//! ([`delete_own_icon`]), and that only while nothing uses it: a profile, a
//! community, a custom emoji, the deployment's profile, or a profile a report
//! or a warning keeps as it was.

use crate::context::GlobalServerContext;
use crate::media_store::{PresignedUpload, Promotion, Served};
use crate::t;
use crate::{IconId, Loadable, UserId};
use aspen_schema::icon;
use chrono::{DateTime, Utc};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, Insertable, OptionalExtension, QueryDsl, Queryable,
    Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::borrow::Cow;
use tracing::warn;

/// The kinds of picture an icon may be: raster formats every browser and every service that
/// unfurls links shows, none of which can carry script, as an SVG or an HTML page could when
/// opened from storage or from `api::federation`'s avatars.
pub const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// The largest icon, in bytes: room for a large photo as a profile picture.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// The most pixels an icon may have, as its header gives them: 4096 by 4096. A picture is
/// decoded whole wherever it is shown, so a small file declaring a huge canvas would take every
/// reader's memory; the same cap holds for link previews' pictures and copied foreign avatars.
pub const MAX_PIXELS: u64 = 4096 * 4096;

/// The box an icon is stored fitted within, in pixels: twice the largest it is shown, the
/// sign-in screen's 256 CSS pixels, for screens of two device pixels to one, and as large as
/// the web client crops pictures to (`ICON_MAX_SIZE`, `client/packages/app/src/features/media/crop.ts`).
pub const STORED_BOX: (u32, u32) = (512, 512);

/// The type an icon is stored as once fitted within [`STORED_BOX`].
pub const STORED_TYPE: &str = "image/webp";

/// How many pictures this server fits within [`STORED_BOX`] at once: half its logical CPUs, so
/// that a burst of uploads leaves the rest to requests. Each decodes at most [`MAX_PIXELS`].
static FITTING: std::sync::LazyLock<tokio::sync::Semaphore> = std::sync::LazyLock::new(|| {
    let cpus = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    tokio::sync::Semaphore::new((cpus / 2).max(1))
});

/// What an icon is stored as.
#[derive(Debug)]
pub struct Stored {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    /// Whether `bytes` are a WebP made from the picture rather than the picture itself.
    pub fitted: bool,
}

/// What the picture `bytes` of `mime_type` is stored as: a WebP fitted within [`STORED_BOX`]
/// when the picture is larger than it, or is not WebP, and the WebP is at least
/// `MIN_SAVING_PERCENT` smaller (`attachment::preview::smaller_enough`, the rule previews of
/// what is shown in place keep to); the picture itself otherwise. A WebP that already fits is never encoded again, which
/// would lose more of it for nothing. A picture that moves, or whose colours cannot be brought
/// into sRGB, is kept as it is (`aspen_previews::picture::fitted`).
pub async fn as_stored(bytes: Vec<u8>, mime_type: String) -> Stored {
    let kept = |bytes, mime_type| Stored {
        bytes,
        mime_type,
        fitted: false,
    };
    let fits = size_within(&bytes, MAX_PIXELS)
        .is_some_and(|size| aspen_previews::fit(size, STORED_BOX) == size);
    if fits && mime_type == STORED_TYPE {
        return kept(bytes, mime_type);
    }
    let Ok(_place) = FITTING.acquire().await else {
        return kept(bytes, mime_type);
    };
    let bytes = std::sync::Arc::new(bytes);
    let picture = bytes.clone();
    let made = tokio::task::spawn_blocking(move || {
        aspen_previews::picture::fitted(&picture, MAX_PIXELS, STORED_BOX)
    })
    .await;
    let bytes = std::sync::Arc::unwrap_or_clone(bytes);
    match made {
        Ok(Ok(made))
            if crate::attachment::preview::smaller_enough(
                made.webp.len() as u64,
                bytes.len() as u64,
            ) =>
        {
            Stored {
                bytes: made.webp,
                mime_type: STORED_TYPE.to_owned(),
                fitted: true,
            }
        }
        Ok(Ok(_)) => kept(bytes, mime_type),
        Ok(Err(reason)) => {
            tracing::debug!(reason, "an icon is stored as it was uploaded");
            kept(bytes, mime_type)
        }
        Err(error) => {
            tracing::error!(%error, "fitting an icon stopped; it is stored as it was uploaded");
            kept(bytes, mime_type)
        }
    }
}

/// The width and height `bytes`' header gives, when it is a picture of one of [`IMAGE_TYPES`]
/// that has some pixels and at most `max_pixels` of them.
pub fn size_within(bytes: &[u8], max_pixels: u64) -> Option<(u32, u32)> {
    let size = imagesize::blob_size(bytes).ok()?;
    let width = u32::try_from(size.width).ok().filter(|w| *w > 0)?;
    let height = u32::try_from(size.height).ok().filter(|h| *h > 0)?;
    (u64::from(width) * u64::from(height) <= max_pixels).then_some((width, height))
}

/// Refuses a picture whose header gives no size, or more than `max_pixels`.
pub fn require_pixels(bytes: &[u8], max_pixels: u64) -> crate::Result<()> {
    match size_within(bytes, max_pixels) {
        Some(_) => Ok(()),
        None if imagesize::blob_size(bytes).is_ok() => Err(crate::Error::Validation(t!(
            "iconTooManyPixels",
            max = max_pixels.isqrt()
        ))),
        None => Err(crate::Error::Validation(t!("iconImageType"))),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Queryable, Selectable, Insertable)]
#[diesel(table_name = icon)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Icon {
    pub id: IconId,
    #[diesel(column_name = icon_mime_type)]
    pub mime_type: String,
    pub timestamp: chrono::DateTime<Utc>,
    pub storage_key: String,
    pub ready_at: Option<chrono::DateTime<Utc>>,
}

impl Loadable for Icon {
    type Id = IconId;

    async fn load_from_db(state: &GlobalServerContext, id: IconId) -> crate::Result<Self> {
        icon::table
            .select(Icon::as_select())
            .filter(icon::id.eq(id))
            .first(&mut state.connection_pool.get().await?)
            .await
            .map_err(Into::into)
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

#[derive(Debug)]
pub struct IconUpload {
    pub id: IconId,
    pub upload_url: String,
    pub expires_at: DateTime<Utc>,
    /// The `Content-Type` the upload must be sent with: its declared type.
    pub content_type: String,
}

pub fn storage_key(id: IconId) -> String {
    crate::media_store::PublicPrefix::Icons.key(id.0)
}

/// How an icon is served: as its type, shown in place.
fn served(mime_type: &str) -> Served {
    Served {
        content_type: mime_type.to_owned(),
        disposition: None,
    }
}

/// Reserves an icon of `mime_type`, one of [`IMAGE_TYPES`], and mints a presigned `PUT` URL for
/// it, for exactly `byte_size` bytes, which the client must declare, at most [`MAX_BYTES`].
pub async fn init_upload(
    state: &GlobalServerContext,
    uploader: UserId,
    mime_type: String,
    byte_size: Option<u64>,
) -> crate::Result<IconUpload> {
    if !IMAGE_TYPES.contains(&mime_type.as_str()) {
        return Err(crate::Error::Validation(t!("iconImageType")));
    }
    let byte_size = crate::attachment::declared_size(byte_size)?;
    if byte_size > MAX_BYTES {
        return Err(too_large());
    }
    let id = IconId::new();
    let key = storage_key(id);
    let row = Icon {
        id,
        mime_type: mime_type.clone(),
        timestamp: Utc::now(),
        storage_key: key.clone(),
        ready_at: None,
    };
    let mut conn = state.connection_pool.get().await?;
    crate::upload_quota::reserve(state, conn.as_mut(), uploader, byte_size).await?;
    diesel::insert_into(icon::table)
        .values((&row, icon::uploaded_by.eq(Some(uploader))))
        .execute(conn.as_mut())
        .await?;
    match state
        .media_store
        .presign_upload(&key, &mime_type, byte_size)
        .await
    {
        Ok(PresignedUpload { url, expires_at }) => Ok(IconUpload {
            id,
            upload_url: url,
            expires_at,
            content_type: mime_type,
        }),
        Err(e) => {
            if let Err(rollback_err) = diesel::delete(icon::table)
                .filter(icon::id.eq(id))
                .execute(conn.as_mut())
                .await
            {
                warn!(
                    error = rollback_err.to_string(),
                    id = id.0.to_string(),
                    "failed to roll back pending icon row after presign failure"
                );
            }
            Err(e)
        }
    }
}

fn too_large() -> crate::Error {
    crate::Error::Validation(t!(
        "iconTooLarge",
        max = crate::attachment::size_text(MAX_BYTES)
    ))
}

/// Confirms an upload `caller` started; anyone else's is not found. One of more than
/// [`MAX_BYTES`], or whose header gives no size or more than [`MAX_PIXELS`], is deleted and
/// refused. The picture is stored as [`as_stored`] says before the icon is ready, so no reader
/// ever fetches the upload itself.
pub async fn confirm_upload(
    state: &GlobalServerContext,
    caller: UserId,
    id: IconId,
) -> crate::Result<Icon> {
    let mut conn = state.connection_pool.get().await?;
    let row: Icon = icon::table
        .select(Icon::as_select())
        .filter(
            icon::id
                .eq(id)
                .and(icon::ready_at.is_null())
                .and(icon::uploaded_by.eq(caller)),
        )
        .first(conn.as_mut())
        .await?;
    match state
        .media_store
        .promote(&row.storage_key, MAX_BYTES, &served(&row.mime_type))
        .await?
    {
        Promotion::Promoted(_) => {}
        Promotion::NotUploaded => return Err(crate::Error::Validation(t!("iconUploadNotFound"))),
        Promotion::TooLarge => return Err(too_large()),
    }
    let bytes = state
        .media_store
        .get_bytes(&row.storage_key, MAX_BYTES)
        .await?;
    let checked = match bytes {
        Some(bytes) => require_pixels(&bytes, MAX_PIXELS).map(|()| bytes),
        None => Err(too_large()),
    };
    let bytes = match checked {
        Ok(bytes) => bytes,
        Err(e) => {
            state.media_store.delete(&row.storage_key).await?;
            return Err(e);
        }
    };
    // Given back while the picture is fitted, which takes no database.
    drop(conn);
    let stored = as_stored(bytes, row.mime_type.clone()).await;
    if stored.fitted {
        state
            .media_store
            .put_bytes(&row.storage_key, stored.bytes, &stored.mime_type)
            .await?;
    }
    let confirmed = Utc::now();
    let updated: usize = diesel::update(icon::table)
        .filter(icon::id.eq(id).and(icon::ready_at.is_null()))
        .set((
            icon::ready_at.eq(confirmed),
            icon::icon_mime_type.eq(&stored.mime_type),
        ))
        .execute(state.connection_pool.get().await?.as_mut())
        .await?;
    if updated == 0 {
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    }
    Ok(Icon {
        ready_at: Some(confirmed),
        mime_type: stored.mime_type,
        ..row
    })
}

pub async fn read_icon(state: &GlobalServerContext, id: IconId) -> crate::Result<Icon> {
    let mut conn = state.connection_pool.get().await?;
    icon::table
        .select(Icon::as_select())
        .filter(icon::id.eq(id).and(icon::ready_at.is_not_null()))
        .first(conn.as_mut())
        .await
        .map_err(Into::into)
}

/// Checks that `caller` may give something the icon `id`: one they uploaded and confirmed, of one
/// of [`IMAGE_TYPES`]. Anything else is refused with `missing`, which tells them to upload it,
/// or, for a picture of another kind, with `iconImageType`. Callers skip it for the icon the
/// thing already has, so a community's managers may keep an icon another of them gave it.
pub async fn require_own(
    conn: &mut AsyncPgConnection,
    caller: UserId,
    id: IconId,
    missing: Cow<'static, str>,
) -> crate::Result<()> {
    let found: Option<(Option<UserId>, String)> = icon::table
        .select((icon::uploaded_by, icon::icon_mime_type))
        .filter(icon::id.eq(id).and(icon::ready_at.is_not_null()))
        .first(conn)
        .await
        .optional()?;
    match found {
        Some((uploader, _)) if uploader != Some(caller) => Err(crate::Error::Validation(missing)),
        None => Err(crate::Error::Validation(missing)),
        Some((_, mime_type)) if !IMAGE_TYPES.contains(&mime_type.as_str()) => {
            Err(crate::Error::Validation(t!("iconImageType")))
        }
        Some(_) => Ok(()),
    }
}

/// Whether anything uses the icon: a user's or community's picture, a custom emoji, the
/// deployment's profile, or a profile a report or a warning keeps as it was. Each is looked up
/// through an index on exactly the expression it compares (`user_icon`, `community_icon`,
/// `custom_emoji_icon`, `report_profile_icon`, `message_warning_icon`), whose conditions the
/// `IS NOT NULL`s repeat.
const ICON_IN_USE_SQL: &str = "SELECT EXISTS (SELECT 1 FROM \"user\" WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM community WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM custom_emoji WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM deployment_settings WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM report \
                WHERE profile IS NOT NULL AND profile->>'icon' = $1::text) \
     OR EXISTS (SELECT 1 FROM message \
                WHERE warning IS NOT NULL AND warning->'profile'->>'icon' = $1::text) AS in_use";

/// Whether anything uses the icon `id` ([`ICON_IN_USE_SQL`]).
pub async fn in_use(conn: &mut AsyncPgConnection, id: IconId) -> crate::Result<bool> {
    #[derive(diesel::QueryableByName)]
    struct InUse {
        #[diesel(sql_type = diesel::sql_types::Bool)]
        in_use: bool,
    }
    let InUse { in_use } = diesel::sql_query(ICON_IN_USE_SQL)
        .bind::<diesel::sql_types::Uuid, _>(id)
        .get_result(conn)
        .await?;
    Ok(in_use)
}

/// Deletes an icon `caller` uploaded that nothing uses. Anyone else's, or one whose uploader is
/// not recorded, is not found; one in use is refused as a conflict.
pub async fn delete_own_icon(
    state: &GlobalServerContext,
    caller: UserId,
    id: IconId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let uploader: Option<UserId> = icon::table
        .select(icon::uploaded_by)
        .filter(icon::id.eq(id))
        .first(conn.as_mut())
        .await?;
    if uploader != Some(caller) {
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    }
    drop(conn);
    if !delete_if_unused(state, id).await? {
        return Err(crate::Error::Conflict(t!("iconInUse")));
    }
    Ok(())
}

/// Deletes an icon and its stored picture if nothing uses it ([`ICON_IN_USE_SQL`]), checked
/// in the statement that deletes it, so nothing that takes it up meanwhile loses it: a picture
/// the server replaced (the copy of a foreign user's avatar their home changed), a removed
/// custom emoji's, or one its uploader deletes. Returns whether it was deleted.
pub async fn delete_if_unused(state: &GlobalServerContext, id: IconId) -> crate::Result<bool> {
    #[derive(diesel::QueryableByName)]
    struct Deleted {
        #[diesel(sql_type = diesel::sql_types::Text)]
        storage_key: String,
    }
    let mut conn = state.connection_pool.get().await?;
    let deleted: Option<Deleted> = diesel::sql_query(format!(
        "DELETE FROM icon WHERE id = $1 AND NOT ({ICON_IN_USE_SQL}) RETURNING storage_key"
    ))
    .bind::<diesel::sql_types::Uuid, _>(id)
    .get_result(conn.as_mut())
    .await
    .optional()?;
    drop(conn);
    let Some(Deleted { storage_key }) = deleted else {
        return Ok(false);
    };
    if let Err(e) = state.media_store.delete_upload(&storage_key).await {
        warn!(
            error = e.to_string(),
            key = storage_key,
            "failed to delete icon object from media store after db deletion"
        );
    }
    Ok(true)
}

/// One `forgetIcon` job: deletes its icon if nothing uses it. The database queues one a day
/// after an icon's upload is confirmed, and one whenever a user, a community, or the deployment
/// lets go of its icon or is deleted (`aspen_icon_let_go`), so an icon never put to use and one
/// replaced both go, without a sweep reading every icon.
pub async fn forget_step(
    state: &GlobalServerContext,
    job: &crate::jobs::Claimed,
) -> crate::Result<crate::jobs::Outcome> {
    if let Some(id) = job.key.as_deref().and_then(|key| key.parse().ok()) {
        delete_if_unused(state, IconId(id)).await?;
    }
    Ok(crate::jobs::Outcome::Done)
}

/// How many icons one step of `shrinkIcons` fits within [`STORED_BOX`]: each may decode up to
/// [`MAX_PIXELS`].
const SHRINK_BATCH: i64 = 8;

/// How far `shrinkIcons` has come: every icon up to `after`, in id order, is stored as
/// [`as_stored`] would store it.
#[derive(serde::Serialize, serde::Deserialize)]
struct ShrinkProgress {
    after: IconId,
}

/// One step of the `shrinkIcons` job, which a migration queues once: stores up to
/// [`SHRINK_BATCH`] confirmed icons as [`as_stored`] would, in id order, so icons confirmed
/// before every upload was stored that way come to be. Icons confirmed since are stored so
/// already and are passed over without being encoded again.
///
/// A new picture is stored over the old under the icon's own key, so every link to it holds;
/// its type is then written to the row. `Icon` records carry no events: their type
/// (`mimeType`) is what the picture was stored as when the record was read, and clients show
/// the picture as storage serves it, by its own type. The deployment's copy of its own icon is
/// read again on every server (`deployment_settings::icon_stored_anew`). An icon deleted while
/// its picture was fitted has the picture deleted too.
pub async fn shrink_step(
    state: &GlobalServerContext,
    job: &crate::jobs::Claimed,
) -> crate::Result<crate::jobs::Outcome> {
    let after = job
        .progress::<ShrinkProgress>()?
        .map(|progress| progress.after);
    let mut query = icon::table
        .select(Icon::as_select())
        .filter(icon::ready_at.is_not_null())
        .order(icon::id)
        .limit(SHRINK_BATCH)
        .into_boxed();
    if let Some(after) = after {
        query = query.filter(icon::id.gt(after));
    }
    let batch: Vec<Icon> = query
        .load(state.connection_pool.get().await?.as_mut())
        .await?;
    for row in &batch {
        shrink_one(state, row).await?;
    }
    match batch.last() {
        Some(last) if batch.len() as i64 == SHRINK_BATCH => Ok(crate::jobs::Outcome::Progress(
            serde_json::to_value(ShrinkProgress { after: last.id })?,
        )),
        _ => Ok(crate::jobs::Outcome::Done),
    }
}

async fn shrink_one(state: &GlobalServerContext, row: &Icon) -> crate::Result<()> {
    let Some(bytes) = state
        .media_store
        .get_bytes(&row.storage_key, MAX_BYTES)
        .await?
    else {
        return Ok(());
    };
    let stored = as_stored(bytes, row.mime_type.clone()).await;
    if !stored.fitted {
        return Ok(());
    }
    state
        .media_store
        .put_bytes(&row.storage_key, stored.bytes, &stored.mime_type)
        .await?;
    let mut conn = state.connection_pool.get().await?;
    let id = row.id;
    let mime_type = &stored.mime_type;
    let updated = conn
        .transaction(|conn| {
            async move {
                let updated = diesel::update(icon::table.filter(icon::id.eq(id)))
                    .set(icon::icon_mime_type.eq(mime_type))
                    .execute(conn)
                    .await?;
                if updated == 1 {
                    crate::deployment_settings::icon_stored_anew(state, conn, id).await?;
                }
                Ok::<_, crate::Error>(updated)
            }
            .scope_boxed()
        })
        .await?;
    drop(conn);
    if updated == 0 {
        state.media_store.delete_upload(&row.storage_key).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The start of a PNG declaring `width` by `height`: its signature and header chunk.
    fn png_header(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        bytes
    }

    mod stored {
        use super::super::*;
        use image::{DynamicImage, ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
        use std::io::Cursor;

        /// A picture with detail everywhere, as a photo has.
        fn photo(width: u32, height: u32) -> DynamicImage {
            DynamicImage::ImageRgb8(RgbImage::from_fn(width, height, |x, y| {
                Rgb([
                    (x * 255 / width) as u8,
                    (y * 255 / height) as u8,
                    ((x ^ y) & 0xff) as u8,
                ])
            }))
        }

        fn encoded(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
            let mut bytes = Vec::new();
            image
                .write_to(&mut Cursor::new(&mut bytes), format)
                .unwrap();
            bytes
        }

        fn size(bytes: &[u8]) -> (u32, u32) {
            size_within(bytes, MAX_PIXELS).unwrap()
        }

        #[tokio::test]
        async fn a_large_picture_is_fitted_within_the_box_as_webp() {
            let png = encoded(&photo(2048, 1024), ImageFormat::Png);
            let stored = as_stored(png.clone(), "image/png".to_owned()).await;
            assert!(stored.fitted);
            assert_eq!(stored.mime_type, STORED_TYPE);
            assert_eq!(size(&stored.bytes), (512, 256));
            assert!(stored.bytes.len() < png.len());
            assert_eq!(
                image::guess_format(&stored.bytes).unwrap(),
                ImageFormat::WebP
            );
        }

        #[tokio::test]
        async fn a_picture_that_fits_is_made_webp_when_that_is_smaller() {
            let png = encoded(&photo(512, 512), ImageFormat::Png);
            let stored = as_stored(png, "image/png".to_owned()).await;
            assert!(stored.fitted);
            assert_eq!(stored.mime_type, STORED_TYPE);
            assert_eq!(size(&stored.bytes), (512, 512));
        }

        #[tokio::test]
        async fn a_picture_whose_webp_is_no_smaller_is_kept() {
            // A few pixels, whose WebP's headers take as much as the PNG does.
            let png = encoded(&photo(4, 4), ImageFormat::Png);
            let stored = as_stored(png.clone(), "image/png".to_owned()).await;
            assert!(!stored.fitted);
            assert_eq!(stored.mime_type, "image/png");
            assert_eq!(stored.bytes, png);
        }

        #[tokio::test]
        async fn a_webp_that_fits_is_not_encoded_again() {
            let webp = aspen_previews::picture::fitted(
                &encoded(&photo(300, 300), ImageFormat::Png),
                MAX_PIXELS,
                STORED_BOX,
            )
            .unwrap()
            .webp;
            let stored = as_stored(webp.clone(), STORED_TYPE.to_owned()).await;
            assert!(!stored.fitted);
            assert_eq!(stored.bytes, webp);
        }

        #[tokio::test]
        async fn a_large_webp_is_fitted() {
            let webp = aspen_previews::picture::fitted(
                &encoded(&photo(1500, 1500), ImageFormat::Png),
                MAX_PIXELS,
                (2000, 2000),
            )
            .unwrap()
            .webp;
            assert_eq!(size(&webp), (1500, 1500));
            let stored = as_stored(webp, STORED_TYPE.to_owned()).await;
            assert!(stored.fitted);
            assert_eq!(size(&stored.bytes), (512, 512));
        }

        #[tokio::test]
        async fn a_picture_that_moves_is_kept() {
            let mut gif = Vec::new();
            {
                let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
                for shade in [0, 255] {
                    let frame = RgbaImage::from_fn(1024, 1024, |x, y| {
                        Rgba([shade, (x & 0xff) as u8, (y & 0xff) as u8, 255])
                    });
                    encoder.encode_frame(image::Frame::new(frame)).unwrap();
                }
            }
            let stored = as_stored(gif.clone(), "image/gif".to_owned()).await;
            assert!(!stored.fitted);
            assert_eq!(stored.mime_type, "image/gif");
            assert_eq!(stored.bytes, gif);
        }
    }

    #[test]
    fn pixels_are_read_from_the_header_and_capped() {
        assert_eq!(
            size_within(&png_header(640, 480), MAX_PIXELS),
            Some((640, 480))
        );
        assert_eq!(
            size_within(&png_header(4096, 4096), MAX_PIXELS),
            Some((4096, 4096))
        );
        assert_eq!(size_within(&png_header(65_535, 65_535), MAX_PIXELS), None);
        assert_eq!(size_within(&png_header(0, 10), MAX_PIXELS), None);
        assert_eq!(size_within(b"not a picture", MAX_PIXELS), None);
        assert!(require_pixels(&png_header(100_000, 2), MAX_PIXELS).is_ok());
        assert!(require_pixels(&png_header(100_000, 1000), MAX_PIXELS).is_err());
    }
}
