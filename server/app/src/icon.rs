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
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use std::borrow::Cow;
use tracing::warn;

/// The kinds of picture an icon may be: raster formats every browser and every service that
/// unfurls links shows, none of which can carry script, as an SVG or an HTML page could when
/// opened from storage or from `api::federation`'s avatars.
pub const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// The largest icon, in bytes: room for a large photo as a profile picture.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
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
    format!("icons/{}", id.0)
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
/// [`MAX_BYTES`] is deleted and refused.
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
    let confirmed = Utc::now();
    let updated: usize = diesel::update(icon::table)
        .filter(icon::id.eq(id).and(icon::ready_at.is_null()))
        .set(icon::ready_at.eq(confirmed))
        .execute(conn.as_mut())
        .await?;
    if updated == 0 {
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    }
    Ok(Icon {
        ready_at: Some(confirmed),
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
/// deployment's profile, or a profile a report or a warning keeps as it was.
const ICON_IN_USE_SQL: &str = "SELECT EXISTS (SELECT 1 FROM \"user\" WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM community WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM custom_emoji WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM deployment_settings WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM report WHERE profile->>'icon' = $1::text) \
     OR EXISTS (SELECT 1 FROM message WHERE warning->'profile'->>'icon' = $1::text) AS in_use";

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
