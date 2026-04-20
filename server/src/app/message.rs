use crate::api::link_preview::LinkPreview;
use crate::api::message_enum::command::MessageUpdateCommand;
use crate::api::message_enum::server_event::{MessageEvent, ServerEvent};
use crate::api::{GlobalServerContext, message_enum};
use crate::app;
use crate::app::channel::Channel;
use crate::app::link_preview::{delete_images_for_message, load_previews, spawn_preview_fetch};
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

/// Message + everything attached to it that we serve over the wire.
///
/// Renamed from the original `MessageWithAttachments` now that link previews
/// are another first-class relation living in its own child table.
pub struct MessageWithRelations {
    pub message: Message,
    pub attachments: Vec<AttachmentId>,
    pub link_previews: Vec<LinkPreview>,
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
    // The freshly-created message goes out with an empty `link_previews`
    // list; the async fetcher spawned below is responsible for publishing a
    // follow-up `MessageLinkPreviewsReady` event once it has settled the
    // actual preview data. See `app::link_preview` for the full flow.
    let event = ServerEvent::Message(MessageEvent::Create(message_enum::Message {
        id,
        author,
        timestamp,
        content: content.clone(),
        attachments,
        channel_id,
        link_previews: Vec::new(),
    }));
    app::publish_event(state, &event).await?;
    spawn_preview_fetch(state.clone(), id, channel_id, content);
    Ok(message)
}

pub async fn read_message(
    state: &GlobalServerContext,
    id: MessageId,
) -> Result<MessageWithRelations, app::Error> {
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
    let link_previews = load_previews(conn.as_mut(), &[id])
        .await?
        .remove(&id)
        .unwrap_or_default();
    Ok(MessageWithRelations {
        message: msg,
        attachments,
        link_previews,
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
) -> Result<MessageWithRelations, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let content_changed = command.content.is_some();
    let new_content_for_refetch = command.content.clone();
    let (message, attachments, channel_id, previews_cleared) = conn
        .transaction(|conn| {
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

                // A content edit invalidates the old link previews. Clear
                // them (and schedule the S3 objects for deletion) inside the
                // same transaction as the content change so nobody reads
                // "new content + stale previews" in between.
                let mut previews_cleared = false;
                if content_changed {
                    delete_images_for_message(state, conn.as_mut(), command.id).await?;
                    previews_cleared = true;
                }

                publish_event(
                    state,
                    &ServerEvent::Message(MessageEvent::Update {
                        id: command.id,
                        content: command.content,
                        attachments: command.attachments,
                    }),
                )
                .await?;

                let channel_id = *message.channel.id();
                Ok::<_, app::Error>((message, attachments, channel_id, previews_cleared))
            }
            .scope_boxed()
        })
        .await?;

    if previews_cleared {
        // Tell clients to drop their stale cards right now, rather than
        // wait for the fetcher to finish. The subsequent
        // `MessageLinkPreviewsReady` from the spawned task will replace the
        // empty list with the actual set.
        publish_event(
            state,
            &ServerEvent::MessageLinkPreviewsReady {
                message_id: command.id,
                channel_id,
                previews: Vec::new(),
            },
        )
        .await?;
        if let Some(content) = new_content_for_refetch {
            spawn_preview_fetch(state.clone(), command.id, channel_id, content);
        }
    }

    // Re-load the current preview set for the REST response. On a content
    // edit this will be empty (we just wiped it); for attachment-only edits
    // the previous set is still current.
    let mut conn = state.connection_pool.get().await?;
    let link_previews = load_previews(conn.as_mut(), &[command.id])
        .await?
        .remove(&command.id)
        .unwrap_or_default();

    Ok(MessageWithRelations {
        message,
        attachments,
        link_previews,
    })
}

pub async fn delete_message(state: &GlobalServerContext, id: MessageId) -> Result<(), app::Error> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            // Drop link-preview image objects from S3 first — the FK cascade
            // on `deleted_at` isn't actually a hard delete, but keeping the
            // two destructions close together mirrors the attachment path
            // and keeps the "soft-delete removes visible artefacts" model
            // consistent.
            delete_images_for_message(state, conn.as_mut(), id).await?;
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
