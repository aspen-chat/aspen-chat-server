use crate::api::link_preview::LinkPreview;
use crate::api::message_enum::request::MessageUpdateRequest;
use crate::api::message_enum::server_event::{MessageEvent, ServerEvent};
use crate::api::{GlobalServerContext, MessageKind, message_enum};
use crate::app;
use crate::app::channel::Channel;
use crate::app::link_preview::{delete_images_for_message, load_previews, spawn_preview_fetch};
use crate::app::user::User;
use crate::app::{AttachmentId, ChannelId, PollId, UserId, publish_event};
use crate::app::{MaybeLoaded, MessageId};
use crate::database::schema::attachment;
use crate::database::schema::channel;
use crate::database::schema::message;
use crate::database::schema::message_attachment;
use chrono::Utc;
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable,
    Selectable, SelectableHelper,
};
use diesel_async::AsyncPgConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use rust_i18n::t;

#[derive(Selectable, Queryable, Insertable)]
#[diesel(table_name=message)]
pub struct Message {
    pub id: MessageId,
    pub channel: MaybeLoaded<Channel>,
    pub content: String,
    pub author: MaybeLoaded<User>,
    pub timestamp: chrono::DateTime<Utc>,
    pub deleted_at: Option<chrono::DateTime<Utc>>,
    pub edited_at: Option<chrono::DateTime<Utc>>,
    pub kind: MessageKind,
    pub poll: Option<PollId>,
}

/// A message together with the child-table relations its wire record carries: attachment ids
/// and link previews.
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

/// Verify every id in `attachments` corresponds to a confirmed (`ready_at IS
/// NOT NULL`) row before linking it to a message. The `attachment` table
/// admits half-uploaded reservations, and exposing them through a message
/// would let a client publish a card pointing at bytes that may never
/// arrive. Returns [`app::Error::Validation`] if any id is missing or
/// pending.
async fn ensure_attachments_ready(
    conn: &mut AsyncPgConnection,
    attachments: &[AttachmentId],
) -> Result<(), app::Error> {
    if attachments.is_empty() {
        return Ok(());
    }
    let ready: Vec<AttachmentId> = attachment::table
        .select(attachment::id)
        .filter(
            attachment::id
                .eq_any(attachments)
                .and(attachment::ready_at.is_not_null()),
        )
        .load(conn)
        .await?;
    if ready.len() != attachments.len() {
        return Err(app::Error::Validation(t!("attachmentNotReady")));
    }
    Ok(())
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
        edited_at: None,
        kind: MessageKind::Standard,
        poll: None,
    };
    ensure_attachments_ready(conn.as_mut(), &attachments).await?;
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
    // list; the async fetcher spawned below publishes an `Update` carrying
    // the actual preview data once it has settled. See `app::link_preview`
    // for the full flow.
    let event = ServerEvent::Message(MessageEvent::Create(message_enum::Message {
        id,
        author,
        timestamp,
        edited_at: None,
        content: content.clone(),
        attachments,
        channel_id,
        link_previews: Vec::new(),
        kind: MessageKind::Standard,
        poll: None,
    }));
    app::publish_event(state, &event).await?;
    spawn_preview_fetch(state.clone(), id, content);
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
    let link_previews = load_previews(conn.as_mut(), state.media_store.as_ref(), &[id])
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
    pub edited_at: Option<chrono::DateTime<Utc>>,
}

pub async fn update_message(
    state: &GlobalServerContext,
    id: MessageId,
    command: MessageUpdateRequest,
) -> Result<MessageWithRelations, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let content_changed = command.content.is_some();
    let new_content_for_refetch = command.content.clone();
    let (message, attachments, previews_cleared) = conn
        .transaction(|conn| {
            async move {
                let Some(message) = diesel::update(message::table)
                    .set(MessageChangeset {
                        content: command.content.clone(),
                        edited_at: content_changed.then(Utc::now),
                    })
                    .filter(message::id.eq(id).and(message::deleted_at.is_null()))
                    .returning(Message::as_select())
                    .load(conn.as_mut())
                    .await?
                    .into_iter()
                    .next()
                else {
                    return Err(app::Error::Diesel(diesel::result::Error::NotFound));
                };

                if let Some(ref new_attachments) = command.attachments {
                    ensure_attachments_ready(conn.as_mut(), new_attachments).await?;
                    diesel::delete(message_attachment::table)
                        .filter(message_attachment::message_id.eq(id))
                        .execute(conn.as_mut())
                        .await?;
                    for attachment_id in new_attachments {
                        diesel::insert_into(message_attachment::table)
                            .values(&MessageAttachment {
                                message_id: id,
                                attachment_id: *attachment_id,
                            })
                            .execute(conn.as_mut())
                            .await?;
                    }
                }

                let attachments: Vec<AttachmentId> = message_attachment::table
                    .select(message_attachment::attachment_id)
                    .filter(message_attachment::message_id.eq(id))
                    .load(conn.as_mut())
                    .await?;

                // A content edit invalidates the old link previews. Clear
                // them (and schedule the S3 objects for deletion) inside the
                // same transaction as the content change so nobody reads
                // "new content + stale previews" in between.
                let mut previews_cleared = false;
                if content_changed {
                    delete_images_for_message(state, conn.as_mut(), id).await?;
                    previews_cleared = true;
                }

                publish_event(
                    state,
                    &ServerEvent::Message(MessageEvent::Update {
                        id,
                        content: command.content,
                        attachments: command.attachments,
                        edited_at: content_changed.then_some(message.edited_at),
                        // Clients drop their stale cards with the edit itself; the
                        // fetcher's own update brings the new set.
                        link_previews: previews_cleared.then(Vec::new),
                    }),
                )
                .await?;

                Ok::<_, app::Error>((message, attachments, previews_cleared))
            }
            .scope_boxed()
        })
        .await?;

    if previews_cleared && let Some(content) = new_content_for_refetch {
        spawn_preview_fetch(state.clone(), id, content);
    }

    // Re-load the current preview set for the REST response. On a content
    // edit this will be empty (we just wiped it); for attachment-only edits
    // the previous set is still current.
    let mut conn = state.connection_pool.get().await?;
    let link_previews = load_previews(conn.as_mut(), state.media_store.as_ref(), &[id])
        .await?
        .remove(&id)
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
            diesel::delete(message_attachment::table)
                .filter(message_attachment::message_id.eq(id))
                .execute(conn.as_mut())
                .await?;
            let Some(deleted) = diesel::update(message::table)
                .set(message::deleted_at.eq(diesel::dsl::now))
                .filter(message::id.eq(id).and(message::deleted_at.is_null()))
                .returning(Message::as_select())
                .load(conn.as_mut())
                .await?
                .into_iter()
                .next()
            else {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            };
            publish_event(state, &ServerEvent::Message(MessageEvent::Delete { id })).await?;
            // Deleting the message a poll is shown in ends the poll; its announcement, if
            // any, is an ordinary message and stays.
            if deleted.kind == MessageKind::Poll
                && let Some(poll) = deleted.poll
            {
                app::poll::delete_poll(state, conn.as_mut(), poll).await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await?;
    delete_images_for_message(state, conn.as_mut(), id).await?;
    Ok(())
}
