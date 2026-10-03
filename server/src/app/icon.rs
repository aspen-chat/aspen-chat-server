//! Two-phase icon uploads.
//!
//! Same shape as [`crate::app::attachment`]: the server reserves a row in
//! the `icon` table with `ready_at = NULL`, hands back a presigned `PUT`
//! URL, and waits for the client to confirm before flipping the row to
//! ready and exposing it to readers. Icons carry no `file_name`, only a
//! mime type, so the wire surface is one field shorter; everything else
//! is symmetric.
//!
//! An icon records who uploaded it (`icon.uploaded_by`). Through the API
//! only they may delete it ([`delete_own_icon`]), and only while nothing
//! uses it: a profile, a community, a custom emoji, the deployment's
//! profile, or a profile a report or a warning keeps as it was.

use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::media_store::PresignedUpload;
use crate::app::{IconId, Loadable, UserId};
use crate::database::schema::icon;
use crate::t;
use chrono::{DateTime, Utc};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable,
    SelectableHelper,
};
use diesel_async::RunQueryDsl;
use tracing::warn;

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

    async fn load_from_db(state: &GlobalServerContext, id: IconId) -> crate::app::Result<Self> {
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
}

pub fn storage_key(id: IconId) -> String {
    format!("icons/{}", id.0)
}

pub async fn init_upload(
    state: &GlobalServerContext,
    uploader: UserId,
    mime_type: String,
) -> app::Result<IconUpload> {
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
    match state.media_store.presign_put(&key, &mime_type).await {
        Ok(PresignedUpload { url, expires_at }) => Ok(IconUpload {
            id,
            upload_url: url,
            expires_at,
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

pub async fn confirm_upload(state: &GlobalServerContext, id: IconId) -> app::Result<Icon> {
    let mut conn = state.connection_pool.get().await?;
    let row: Icon = icon::table
        .select(Icon::as_select())
        .filter(icon::id.eq(id).and(icon::ready_at.is_null()))
        .first(conn.as_mut())
        .await?;
    if !state.media_store.head_object(&row.storage_key).await? {
        return Err(app::Error::Validation(t!("iconUploadNotFound")));
    }
    let confirmed = Utc::now();
    let updated: usize = diesel::update(icon::table)
        .filter(icon::id.eq(id).and(icon::ready_at.is_null()))
        .set(icon::ready_at.eq(confirmed))
        .execute(conn.as_mut())
        .await?;
    if updated == 0 {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    }
    Ok(Icon {
        ready_at: Some(confirmed),
        ..row
    })
}

pub async fn read_icon(state: &GlobalServerContext, id: IconId) -> app::Result<Icon> {
    let mut conn = state.connection_pool.get().await?;
    icon::table
        .select(Icon::as_select())
        .filter(icon::id.eq(id).and(icon::ready_at.is_not_null()))
        .first(conn.as_mut())
        .await
        .map_err(Into::into)
}

/// Whether anything uses the icon: a user's or community's picture, a custom emoji, the
/// deployment's profile, or a profile a report or a warning keeps as it was.
const ICON_IN_USE_SQL: &str = "SELECT EXISTS (SELECT 1 FROM \"user\" WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM community WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM custom_emoji WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM deployment_profile WHERE icon = $1) \
     OR EXISTS (SELECT 1 FROM report WHERE profile->>'icon' = $1::text) \
     OR EXISTS (SELECT 1 FROM message WHERE warning->'profile'->>'icon' = $1::text) AS in_use";

/// Deletes an icon `caller` uploaded that nothing uses. Anyone else's, or one whose uploader is
/// not recorded, is not found; one in use is refused as a conflict.
pub async fn delete_own_icon(
    state: &GlobalServerContext,
    caller: UserId,
    id: IconId,
) -> app::Result<()> {
    #[derive(diesel::QueryableByName)]
    struct InUse {
        #[diesel(sql_type = diesel::sql_types::Bool)]
        in_use: bool,
    }
    let mut conn = state.connection_pool.get().await?;
    let uploader: Option<UserId> = icon::table
        .select(icon::uploaded_by)
        .filter(icon::id.eq(id))
        .first(conn.as_mut())
        .await?;
    if uploader != Some(caller) {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    }
    let InUse { in_use } = diesel::sql_query(ICON_IN_USE_SQL)
        .bind::<diesel::sql_types::Uuid, _>(id)
        .get_result(conn.as_mut())
        .await?;
    if in_use {
        return Err(app::Error::Conflict(t!("iconInUse")));
    }
    drop(conn);
    delete_icon(state, id).await
}

/// Deletes an icon and its stored picture, for the server's own use: the caller has decided it
/// may go, as when its custom emoji is removed.
pub(crate) async fn delete_icon(state: &GlobalServerContext, id: IconId) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let Some(deleted) = diesel::delete(icon::table)
        .filter(icon::id.eq(id))
        .returning(Icon::as_returning())
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
            "failed to delete icon object from media store after db deletion"
        );
    }
    Ok(())
}
