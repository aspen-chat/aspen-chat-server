use crate::api::message_enum::command::MessageUpdateCommand;
use crate::api::message_enum::server_event::{MessageEvent, ServerEvent};
use crate::api::{GlobalServerContext, message_enum};
use crate::app;
use crate::app::channel::Channel;
use crate::app::user::User;
use crate::app::{AttachmentId, ChannelId, UserId, publish_event};
use crate::app::{MaybeLoaded, MessageId};
use crate::database::schema::channel;
use crate::database::schema::message;
use crate::database::schema::message_attachment;
use chrono::Utc;
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable,
    Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};

#[derive(Selectable, Queryable, Insertable)]
#[diesel(table_name=message)]
pub struct Message {
    pub id: MessageId,
    pub channel: MaybeLoaded<Channel>,
    pub content: String,
    pub author: MaybeLoaded<User>,
    pub timestamp: chrono::DateTime<Utc>,
    pub deleted_at: Option<chrono::DateTime<Utc>>,
}

pub struct MessageWithAttachments {
    pub message: Message,
    pub attachments: Vec<AttachmentId>,
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
    attachments: Vec<AttachmentId>,
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
        deleted_at: None,
    };
    diesel::insert_into(message::table)
        .values(&message)
        .execute(conn.as_mut())
        .await?;
    for attachment in &attachments {
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
        attachments,
        channel_id,
    }));
    app::publish_event(state, &event).await?;
    Ok(message)
}

pub async fn read_message(
    state: &GlobalServerContext,
    id: MessageId,
) -> Result<MessageWithAttachments, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let msg = message::table
        .inner_join(channel::table)
        .select(Message::as_select())
        .filter(
            message::id
                .eq(id)
                .and(message::deleted_at.is_null())
                .and(channel::deleted_at.is_null()),
        )
        .first(conn.as_mut())
        .await?;
    let attachments: Vec<AttachmentId> = message_attachment::table
        .select(message_attachment::attachment_id)
        .filter(message_attachment::message_id.eq(id))
        .load(conn.as_mut())
        .await?;
    Ok(MessageWithAttachments {
        message: msg,
        attachments,
    })
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = message)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct MessageChangeset {
    pub content: Option<String>,
}

pub async fn update_message(
    state: &GlobalServerContext,
    command: MessageUpdateCommand,
) -> Result<MessageWithAttachments, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let Some(message) = diesel::update(message::table)
                .set(MessageChangeset {
                    content: command.content.clone(),
                })
                .filter(
                    message::id
                        .eq(command.id)
                        .and(message::deleted_at.is_null()),
                )
                .returning(Message::as_select())
                .load(conn.as_mut())
                .await?
                .into_iter()
                .next()
            else {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            };

            // Replace attachments if provided
            if let Some(ref new_attachments) = command.attachments {
                diesel::delete(message_attachment::table)
                    .filter(message_attachment::message_id.eq(command.id))
                    .execute(conn.as_mut())
                    .await?;
                for attachment_id in new_attachments {
                    diesel::insert_into(message_attachment::table)
                        .values(&MessageAttachment {
                            message_id: command.id,
                            attachment_id: *attachment_id,
                        })
                        .execute(conn.as_mut())
                        .await?;
                }
            }

            let attachments: Vec<AttachmentId> = message_attachment::table
                .select(message_attachment::attachment_id)
                .filter(message_attachment::message_id.eq(command.id))
                .load(conn.as_mut())
                .await?;

            publish_event(
                state,
                &ServerEvent::Message(MessageEvent::Update {
                    id: command.id,
                    content: command.content,
                    attachments: command.attachments,
                }),
            )
            .await?;

            Ok(MessageWithAttachments {
                message,
                attachments,
            })
        }
        .scope_boxed()
    })
    .await
}

pub async fn delete_message(state: &GlobalServerContext, id: MessageId) -> Result<(), app::Error> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            diesel::delete(message_attachment::table)
                .filter(message_attachment::message_id.eq(id))
                .execute(conn.as_mut())
                .await?;
            let deleted = diesel::update(message::table)
                .set(message::deleted_at.eq(diesel::dsl::now))
                .filter(message::id.eq(id).and(message::deleted_at.is_null()))
                .execute(conn.as_mut())
                .await?;
            if deleted == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            publish_event(state, &ServerEvent::Message(MessageEvent::Delete { id })).await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}
