use crate::api::GlobalServerContext;
use crate::app;
use crate::app::{IconId, Loadable};
use crate::database::schema::icon;
use chrono::Utc;
use diesel::{ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable, SelectableHelper};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
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
}

impl Loadable for Icon {
    type Id = IconId;

    async fn load_from_db(
        pg_connection: &mut AsyncPgConnection,
        id: IconId,
    ) -> Result<Self, diesel::result::Error> {
        icon::table
            .select(Icon::as_select())
            .filter(icon::id.eq(id))
            .first(pg_connection)
            .await
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

pub async fn create_icon(
    state: &GlobalServerContext,
    data: Vec<u8>,
    mime_type: String,
) -> app::Result<Icon> {
    let id = IconId::new();
    let icon = Icon {
        id,
        mime_type: mime_type.clone(),
        timestamp: Utc::now(),
        storage_key: format!("icons/{id}"),
    };
    state
        .media_store
        .put_bytes(&icon.storage_key, data, &mime_type)
        .await?;
    let mut conn = state.connection_pool.get().await?;
    if let Err(e) = diesel::insert_into(icon::table)
        .values((
            icon::id.eq(icon.id),
            icon::icon_mime_type.eq(&icon.mime_type),
            icon::timestamp.eq(icon.timestamp),
            icon::storage_key.eq(&icon.storage_key),
        ))
        .execute(conn.as_mut())
        .await
    {
        // Best effort cleanup to avoid orphaning object storage data.
        if let Err(delete_err) = state.media_store.delete(&icon.storage_key).await {
            warn!(
                error = delete_err.to_string(),
                key = icon.storage_key,
                "failed to cleanup uploaded icon after db insert failure"
            );
        }
        return Err(e.into());
    }
    Ok(icon)
}

pub async fn read_icon(state: &GlobalServerContext, id: IconId) -> app::Result<(Icon, Vec<u8>)> {
    let mut conn = state.connection_pool.get().await?;
    let icon = Icon::load_from_db(conn.as_mut(), id).await?;
    let data = match state.media_store.get_bytes(&icon.storage_key).await {
        Ok(data) => data,
        Err(e) => return Err(e),
    };
    Ok((icon, data))
}

pub async fn delete_icon(state: &GlobalServerContext, id: IconId) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let Some(deleted_icon) = diesel::delete(icon::table)
        .filter(icon::id.eq(id))
        .returning(Icon::as_returning())
        .load(conn.as_mut())
        .await?
        .into_iter()
        .next()
    else {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    };
    if let Err(e) = state.media_store.delete(&deleted_icon.storage_key).await {
        warn!(
            error = e.to_string(),
            key = deleted_icon.storage_key,
            "failed to delete icon object from media store after db deletion"
        );
    }
    Ok(())
}
