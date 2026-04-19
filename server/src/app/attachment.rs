use crate::api::GlobalServerContext;
use crate::app;
use crate::app::{AttachmentId, Loadable};
use crate::database::schema::attachment;
use chrono::Utc;
use diesel::{ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable, SelectableHelper};
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

pub async fn create_attachment(
    state: &GlobalServerContext,
    file_name: String,
    data: Vec<u8>,
    mime_type: String,
) -> app::Result<Attachment> {
    let id = AttachmentId::new();
    let storage_key = format!("attachments/{id}");
    let attachment = Attachment {
        id,
        mime_type: mime_type.clone(),
        file_name,
        timestamp: Utc::now(),
        storage_key,
    };
    state
        .media_store
        .put_bytes(&attachment.storage_key, data, &mime_type)
        .await?;
    let mut conn = state.connection_pool.get().await?;
    if let Err(e) = diesel::insert_into(attachment::table)
        .values((
            attachment::id.eq(attachment.id),
            attachment::mime_type.eq(&attachment.mime_type),
            attachment::file_name.eq(&attachment.file_name),
            attachment::timestamp.eq(attachment.timestamp),
            attachment::storage_key.eq(&attachment.storage_key),
        ))
        .execute(conn.as_mut())
        .await
    {
        // Best effort cleanup to avoid orphaning object storage data.
        if let Err(delete_err) = state.media_store.delete(&attachment.storage_key).await {
            warn!(
                error = delete_err.to_string(),
                key = attachment.storage_key,
                "failed to cleanup uploaded attachment after db insert failure"
            );
        }
        return Err(e.into());
    }
    Ok(attachment)
}

pub async fn read_attachment(
    state: &GlobalServerContext,
    id: AttachmentId,
) -> app::Result<(Attachment, Vec<u8>)> {
    let attachment = Attachment::load_from_db(state, id).await?;
    let data = state.media_store.get_bytes(&attachment.storage_key).await?;
    Ok((attachment, data))
}

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
