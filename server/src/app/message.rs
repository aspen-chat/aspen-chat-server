use crate::api::link_preview::LinkPreview;
use crate::api::message_enum::request::MessageUpdateRequest;
use crate::api::message_enum::server_event::{MessageEvent, PinEvent, ServerEvent};
use crate::api::{ChannelType, GlobalServerContext, MessageKind, message_enum};
use crate::app;
use crate::app::channel::Channel;
use crate::app::deployment::{ModerationAction, log_moderation};
use crate::app::link_preview::{delete_images_for_message, load_previews, spawn_preview_fetch};
use crate::app::mention::{self, Mentions};
use crate::app::permissions::{Permissions, channel_access, missing};
use crate::app::user::User;
use crate::app::{
    AttachmentId, ChannelId, EventScope, PollId, UserId, publish_event, read_state, thread,
};
use crate::app::{MaybeLoaded, MessageId};
use crate::database::schema::attachment;
use crate::database::schema::channel;
use crate::database::schema::message;
use crate::database::schema::message_attachment;
use crate::t;
use chrono::Utc;
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, JoinOnDsl,
    OptionalExtension, QueryDsl, Queryable, Selectable, SelectableHelper,
};
use diesel_async::AsyncPgConnection;
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
    pub edited_at: Option<chrono::DateTime<Utc>>,
    pub kind: MessageKind,
    pub poll: Option<PollId>,
    pub thread: Option<ChannelId>,
    pub echo_of: Option<MessageId>,
    /// Who it tags, as far as its author was allowed to (`app::mention`).
    pub mentions: Mentions,
}

/// The message's wire record, with the relations it carries from child tables.
pub fn record(
    row: &Message,
    attachments: Vec<AttachmentId>,
    link_previews: Vec<LinkPreview>,
) -> message_enum::Message {
    message_enum::Message {
        id: row.id,
        channel_id: *row.channel.id(),
        author: *row.author.id(),
        timestamp: row.timestamp,
        edited_at: row.edited_at,
        content: row.content.clone(),
        attachments,
        link_previews,
        kind: row.kind,
        poll: row.poll,
        thread: row.thread,
        echo_of: row.echo_of,
        mentions: row.mentions.clone(),
    }
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

/// Posts a message. In a thread it counts toward the thread's summary, and with
/// `echo_to_parent` it is also shown in the parent channel as a `ThreadEcho`. In a DM, or a
/// thread in one, only a recipient may post.
pub async fn create_message(
    state: &GlobalServerContext,
    author: UserId,
    channel_id: ChannelId,
    content: String,
    attachments: Vec<AttachmentId>,
    echo_to_parent: bool,
) -> Result<Message, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let message = conn
        .transaction(|conn| {
            async move {
                let target: Channel = channel::table
                    .select(Channel::as_select())
                    .filter(
                        channel::id
                            .eq(channel_id)
                            .and(channel::deleted_at.is_null()),
                    )
                    .first(conn.as_mut())
                    .await?;
                let access = channel_access(state, conn.as_mut(), author, channel_id).await?;
                access.require(access.send_permission())?;
                if !attachments.is_empty() {
                    access.require(Permissions::ATTACH_FILES)?;
                }
                if echo_to_parent {
                    let Some(parent) = target.parent_channel else {
                        return Err(app::Error::Validation(t!("echoOutsideThread")));
                    };
                    // An echo is posted in the parent channel, so it takes sending there.
                    channel_access(state, conn.as_mut(), author, parent)
                        .await?
                        .require(Permissions::SEND_MESSAGES)?;
                }
                ensure_attachments_ready(conn.as_mut(), &attachments).await?;
                let mentions = mention::resolve(
                    state,
                    conn.as_mut(),
                    channel_id,
                    &access,
                    mention::parse(&content),
                )
                .await?;
                let message = Message {
                    id: MessageId::new(),
                    channel: MaybeLoaded::from_id(channel_id),
                    content,
                    author: MaybeLoaded::from_id(author),
                    timestamp: Utc::now(),
                    deleted_at: None,
                    edited_at: None,
                    kind: MessageKind::Standard,
                    poll: None,
                    thread: None,
                    echo_of: None,
                    mentions,
                };
                diesel::insert_into(message::table)
                    .values(&message)
                    .execute(conn.as_mut())
                    .await?;
                mention::record(conn.as_mut(), message.id, channel_id, &message.mentions).await?;
                for attachment in &attachments {
                    diesel::insert_into(message_attachment::table)
                        .values(&MessageAttachment {
                            message_id: message.id,
                            attachment_id: *attachment,
                        })
                        .execute(conn.as_mut())
                        .await?;
                }
                // The new message goes out with no link previews; the fetcher spawned below
                // publishes an `Update` with them once it has settled (see `app::link_preview`).
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::Channel(channel_id),
                    &ServerEvent::Message(MessageEvent::Create(record(
                        &message,
                        attachments,
                        Vec::new(),
                    ))),
                )
                .await?;
                if target.ty == ChannelType::Thread {
                    thread::record_reply(state, conn.as_mut(), channel_id, message.timestamp)
                        .await?;
                    if echo_to_parent && let Some(parent) = target.parent_channel {
                        thread::echo(state, conn.as_mut(), parent, &message).await?;
                    }
                } else {
                    read_state::advance(state, conn.as_mut(), author, channel_id, message.id)
                        .await?;
                }
                Ok::<_, app::Error>(message)
            }
            .scope_boxed()
        })
        .await?;
    spawn_preview_fetch(state.clone(), message.id, message.content.clone());
    Ok(message)
}

pub async fn read_message(
    state: &GlobalServerContext,
    caller: UserId,
    id: MessageId,
) -> Result<MessageWithRelations, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let msg = message::table
        .inner_join(channel::table.on(channel::id.eq(message::channel)))
        .select(Message::as_select())
        .filter(
            message::id
                .eq(id)
                .and(message::deleted_at.is_null())
                .and(channel::deleted_at.is_null()),
        )
        .first(conn.as_mut())
        .await?;
    let access = channel_access(state, conn.as_mut(), caller, *msg.channel.id()).await?;
    if access.dm_moderator {
        note_moderation(
            conn.as_mut(),
            caller,
            &access,
            ModerationAction::ReadDm,
            Some(id.0.to_string()),
        )
        .await?;
    }
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

/// The live messages among `ids` that `caller` may see, with their relations: the thread
/// replies echoes name, which a window of the parent channel does not include.
pub async fn read_messages(
    state: &GlobalServerContext,
    caller: UserId,
    ids: &[MessageId],
) -> Result<Vec<MessageWithRelations>, app::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<Message> = message::table
        .select(Message::as_select())
        .filter(message::id.eq_any(ids).and(message::deleted_at.is_null()))
        .load(conn.as_mut())
        .await?;
    // Whether the caller may see a channel is asked once per channel, however many of the
    // messages are in it; a window's echoes all name replies in its own channel's threads.
    let mut allowed: std::collections::HashMap<ChannelId, bool> = std::collections::HashMap::new();
    let mut visible = Vec::with_capacity(rows.len());
    for row in rows {
        let channel = *row.channel.id();
        let may_see = match allowed.get(&channel) {
            Some(may_see) => *may_see,
            None => {
                let may_see = channel_access(state, conn.as_mut(), caller, channel)
                    .await
                    .is_ok();
                allowed.insert(channel, may_see);
                may_see
            }
        };
        if may_see {
            visible.push(row);
        }
    }
    let ids: Vec<MessageId> = visible.iter().map(|m| m.id).collect();
    let mut attachments: std::collections::HashMap<MessageId, Vec<AttachmentId>> =
        std::collections::HashMap::new();
    for (message_id, attachment_id) in message_attachment::table
        .select((
            message_attachment::message_id,
            message_attachment::attachment_id,
        ))
        .filter(message_attachment::message_id.eq_any(&ids))
        .load::<(MessageId, AttachmentId)>(conn.as_mut())
        .await?
    {
        attachments
            .entry(message_id)
            .or_default()
            .push(attachment_id);
    }
    let mut previews = load_previews(conn.as_mut(), state.media_store.as_ref(), &ids).await?;
    Ok(visible
        .into_iter()
        .map(|message| MessageWithRelations {
            attachments: attachments.remove(&message.id).unwrap_or_default(),
            link_previews: previews.remove(&message.id).unwrap_or_default(),
            message,
        })
        .collect())
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = message)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct MessageChangeset {
    pub content: Option<String>,
    pub edited_at: Option<chrono::DateTime<Utc>>,
    pub mentions: Option<Mentions>,
}

pub async fn update_message(
    state: &GlobalServerContext,
    caller: UserId,
    id: MessageId,
    command: MessageUpdateRequest,
) -> Result<MessageWithRelations, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let (channel_id, kind, author): (ChannelId, MessageKind, UserId) = message::table
        .select((message::channel, message::kind, message::author))
        .filter(message::id.eq(id).and(message::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;
    let access = channel_access(state, conn.as_mut(), caller, channel_id).await?;
    // A message says what its author said; nobody else may put words in it.
    if author != caller {
        return Err(app::Error::Forbidden(t!("editOthersMessage")));
    }
    access.ensure_unblocked()?;
    if command.attachments.as_ref().is_some_and(|a| !a.is_empty()) {
        access.require(Permissions::ATTACH_FILES)?;
    }
    // An echo shows its reply's content; there is nothing of its own to edit.
    if kind == MessageKind::ThreadEcho {
        return Err(app::Error::Validation(t!("echoNotEditable")));
    }
    let content_changed = command.content.is_some();
    let new_content_for_refetch = command.content.clone();
    let (message, attachments, previews_cleared) = conn
        .transaction(|conn| {
            async move {
                // New text tags afresh, with the author's permissions as they are now.
                let mentions = match &command.content {
                    Some(content) => Some(
                        mention::resolve(
                            state,
                            conn.as_mut(),
                            channel_id,
                            &access,
                            mention::parse(content),
                        )
                        .await?,
                    ),
                    None => None,
                };
                let Some(message) = diesel::update(message::table)
                    .set(MessageChangeset {
                        content: command.content.clone(),
                        edited_at: content_changed.then(Utc::now),
                        mentions: mentions.clone(),
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
                if let Some(mentions) = &mentions {
                    mention::record(conn.as_mut(), id, channel_id, mentions).await?;
                }
                let mut previews_cleared = false;
                if content_changed {
                    delete_images_for_message(state, conn.as_mut(), id).await?;
                    previews_cleared = true;
                }

                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::Message(id),
                    &ServerEvent::Message(MessageEvent::Update {
                        id,
                        content: command.content,
                        attachments: command.attachments,
                        edited_at: content_changed.then_some(message.edited_at),
                        // Clients drop their stale cards with the edit itself; the
                        // fetcher's own update brings the new set.
                        link_previews: previews_cleared.then(Vec::new),
                        thread: None,
                        mentions,
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

/// Deletes a message, which its author may do and anyone with Manage messages. A thread reply
/// leaves its thread's summary and takes its echo with it; a thread's starter leaves the thread
/// in place.
pub async fn delete_message(
    state: &GlobalServerContext,
    caller: UserId,
    id: MessageId,
) -> Result<(), app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let (channel_id, author): (ChannelId, UserId) = message::table
        .select((message::channel, message::author))
        .filter(message::id.eq(id).and(message::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;
    let access = channel_access(state, conn.as_mut(), caller, channel_id).await?;
    if author != caller && !access.community_has(Permissions::MANAGE_MESSAGES) {
        return Err(missing(Permissions::MANAGE_MESSAGES));
    }
    if author != caller && access.moderating(Permissions::MANAGE_MESSAGES) {
        note_moderation(
            conn.as_mut(),
            caller,
            &access,
            ModerationAction::DeleteMessage,
            Some(id.0.to_string()),
        )
        .await?;
    }
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
            // A reply's echo goes first, so no client ever holds an echo whose reply is gone.
            if deleted.kind != MessageKind::ThreadEcho {
                thread::delete_echo_of(state, conn.as_mut(), id).await?;
            }
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Channel(*deleted.channel.id()),
                &ServerEvent::Message(MessageEvent::Delete { id }),
            )
            .await?;
            // Deleting the message a poll is shown in ends the poll; its announcement, if
            // any, is an ordinary message and stays.
            if deleted.kind == MessageKind::Poll
                && let Some(poll) = deleted.poll
            {
                app::poll::delete_poll(state, conn.as_mut(), poll, *deleted.channel.id()).await?;
            }
            let ty: ChannelType = channel::table
                .select(channel::ty)
                .filter(channel::id.eq(*deleted.channel.id()))
                .first(conn.as_mut())
                .await?;
            if ty == ChannelType::Thread {
                thread::record_removal(state, conn.as_mut(), *deleted.channel.id()).await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await?;
    delete_images_for_message(state, conn.as_mut(), id).await?;
    Ok(())
}

/// Pins a message in its channel, after every pin already there, or unpins it. In a community
/// it takes Pin messages; in a DM any recipient may. Returns the pin as it stands when pinning,
/// and whether anything changed.
pub async fn set_pinned(
    state: &GlobalServerContext,
    caller: UserId,
    id: MessageId,
    pinned: bool,
) -> Result<(Option<app::channel::Pin>, bool), app::Error> {
    use crate::database::schema::pin;
    let mut conn = state.connection_pool.get().await?;
    let channel_id: ChannelId = message::table
        .select(message::channel)
        .filter(message::id.eq(id).and(message::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;
    let access = channel_access(state, conn.as_mut(), caller, channel_id).await?;
    access.ensure_unblocked()?;
    if access.community.is_some() && !access.community_has(Permissions::PIN_MESSAGES) {
        return Err(missing(Permissions::PIN_MESSAGES));
    }
    conn.transaction(|conn| {
        async move {
            // The channel row is locked so two pins cannot take the same place.
            channel::table
                .select(channel::id)
                .filter(channel::id.eq(channel_id))
                .for_update()
                .first::<ChannelId>(conn.as_mut())
                .await?;
            let existing: Option<app::channel::Pin> = pin::table
                .select(app::channel::Pin::as_select())
                .filter(pin::message_id.eq(id))
                .first(conn.as_mut())
                .await
                .optional()?;
            if !pinned {
                if existing.is_none() {
                    return Ok((None, false));
                }
                diesel::delete(pin::table.filter(pin::message_id.eq(id)))
                    .execute(conn.as_mut())
                    .await?;
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::Channel(channel_id),
                    &ServerEvent::Pin(PinEvent::Delete { message_id: id }),
                )
                .await?;
                return Ok((None, true));
            }
            if let Some(existing) = existing {
                return Ok((Some(existing), false));
            }
            let last: Option<i32> = pin::table
                .select(diesel::dsl::max(pin::sort_index))
                .filter(pin::channel.eq(channel_id))
                .first(conn.as_mut())
                .await?;
            let row = app::channel::Pin {
                message_id: id,
                timestamp: Utc::now(),
                sort_index: last.map_or(0, |last| last + 1),
            };
            diesel::insert_into(pin::table)
                .values((
                    pin::message_id.eq(row.message_id),
                    pin::channel.eq(channel_id),
                    pin::timestamp.eq(row.timestamp),
                    pin::sort_index.eq(row.sort_index),
                ))
                .execute(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Channel(channel_id),
                &ServerEvent::Pin(PinEvent::Create(message_enum::Pin {
                    message_id: row.message_id,
                    timestamp: row.timestamp,
                    sort_index: row.sort_index,
                })),
            )
            .await?;
            Ok((Some(row), true))
        }
        .scope_boxed()
    })
    .await
}

/// Writes a moderator's action in a channel to the moderation log, with the channel's community
/// when it has one.
pub async fn note_moderation(
    conn: &mut AsyncPgConnection,
    actor: UserId,
    access: &crate::app::permissions::ChannelAccess,
    action: ModerationAction,
    subject: Option<String>,
) -> app::Result<()> {
    log_moderation(
        conn,
        actor,
        action,
        access.community.as_ref().map(|c| c.community),
        Some(access.channel),
        subject,
    )
    .await
}

/// Takes one attachment off a message, which its author may do and anyone with Manage messages
/// (a deployment moderator's use is logged). The message's update names what is left.
pub async fn remove_attachment(
    state: &GlobalServerContext,
    caller: UserId,
    id: MessageId,
    attachment_id: AttachmentId,
) -> Result<(), app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let (channel_id, author): (ChannelId, UserId) = message::table
        .select((message::channel, message::author))
        .filter(message::id.eq(id).and(message::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;
    let access = channel_access(state, conn.as_mut(), caller, channel_id).await?;
    if author != caller && !access.community_has(Permissions::MANAGE_MESSAGES) {
        return Err(missing(Permissions::MANAGE_MESSAGES));
    }
    conn.transaction(|conn| {
        async move {
            let removed = diesel::delete(
                message_attachment::table.filter(
                    message_attachment::message_id
                        .eq(id)
                        .and(message_attachment::attachment_id.eq(attachment_id)),
                ),
            )
            .execute(conn.as_mut())
            .await?;
            if removed == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            if author != caller && access.moderating(Permissions::MANAGE_MESSAGES) {
                note_moderation(
                    conn.as_mut(),
                    caller,
                    &access,
                    ModerationAction::RemoveAttachment,
                    Some(format!("{}/{}", id.0, attachment_id.0)),
                )
                .await?;
            }
            let attachments: Vec<AttachmentId> = message_attachment::table
                .select(message_attachment::attachment_id)
                .filter(message_attachment::message_id.eq(id))
                .load(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Message(id),
                &ServerEvent::Message(MessageEvent::Update {
                    id,
                    content: None,
                    attachments: Some(attachments),
                    edited_at: None,
                    link_previews: None,
                    thread: None,
                    mentions: None,
                }),
            )
            .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}
