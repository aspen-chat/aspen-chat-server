use crate::api::message_enum::server_event::{MessageEvent, ServerEvent};
use crate::api::{GlobalServerContext, message_enum};
use crate::app;
use crate::app::channel::Channel;
use crate::app::user::User;
use crate::app::{ASPEN_NATS_STREAM_NAME, AttachmentId, ChannelId, UserId};
use crate::app::{MaybeLoaded, MessageId};
use crate::database::schema::message;
use crate::database::schema::message_attachment;
use chrono::Utc;
use diesel::{Insertable, Queryable, Selectable};
use diesel_async::{AnsiTransactionManager, RunQueryDsl, TransactionManager};

#[derive(Selectable, Queryable, Insertable)]
#[diesel(table_name=message)]
pub struct Message {
    pub id: MessageId,
    pub channel: MaybeLoaded<Channel>,
    pub content: String,
    pub author: MaybeLoaded<User>,
    pub timestamp: chrono::DateTime<Utc>,
}

#[derive(Selectable, Queryable, Insertable)]
#[diesel(table_name=message_attachment)]
pub struct MessageAttachment {
    message_id: MessageId,
    attachment_id: AttachmentId,
}

pub async fn create_message(
    state: &GlobalServerContext,
    author: UserId,
    channel_id: ChannelId,
    content: String,
    attachment: Vec<AttachmentId>,
) -> Result<Message, app::Error> {
    let id = MessageId::new();
    let timestamp = Utc::now();
    let mut conn = state.connection_pool.get().await?;
    let message = Message {
        id,
        channel: MaybeLoaded::from_id(channel_id),
        content: content.clone(),
        author: MaybeLoaded::from_id(author),
        timestamp,
    };
    AnsiTransactionManager::begin_transaction(conn.as_mut()).await?;
    diesel::insert_into(message::table)
        .values(&message)
        .execute(conn.as_mut())
        .await?;
    for attachment in &attachment {
        diesel::insert_into(message_attachment::table)
            .values(&MessageAttachment {
                message_id: id,
                attachment_id: *attachment,
            })
            .execute(conn.as_mut())
            .await?;
    }
    let event = ServerEvent::Message(MessageEvent::Create(message_enum::Message {
        id,
        author,
        timestamp,
        content,
        attachments: attachment,
        channel_id,
    }));
    state
        .nats_context
        .publish(
            ASPEN_NATS_STREAM_NAME,
            serde_json::to_string(&event)?.into_bytes().into(),
        )
        .await?
        .await?;
    AnsiTransactionManager::commit_transaction(conn.as_mut()).await?;
    Ok(message)
}
