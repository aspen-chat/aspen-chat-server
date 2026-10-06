use crate::api::link_preview::LinkPreview;
use crate::api::message_enum;
use crate::api::message_enum::request::MessageUpdateRequest;
use crate::api::message_enum::server_event::{MessageEvent, PinEvent, ServerEvent};
use crate::app;
use crate::app::bot_command::{self, Invocation};
use crate::app::channel::Channel;
use crate::app::channel::ChannelType;
use crate::app::context::GlobalServerContext;
use crate::app::link_preview::{delete_images_for_message, load_previews, spawn_preview_fetch};
use crate::app::mention::{self, Mentions};
use crate::app::message_link::{self, MessageLinks};
use crate::app::moderation_log::{ModerationAction, log_moderation};
use crate::app::permissions::{
    ChannelAccess, Permissions, channel_access, channel_access_moderating, channel_access_reading,
    missing,
};
use crate::app::plugin::card::Card;
use crate::app::plugin::intercept;
use crate::app::plugin::manifest::InterceptHook;
use crate::app::report::Warning;
use crate::app::user::User;
use crate::app::{
    AttachmentId, ChannelId, CommunityId, EventScope, PollId, UserId, publish_event, read_state,
    system_account, thread,
};
use crate::app::{HeldMessageId, MaybeLoaded, MessageId};
use crate::database::schema::attachment;
use crate::database::schema::channel;
use crate::database::schema::message;
use crate::database::schema::message_attachment;
use crate::t;
use chrono::{DateTime, Utc};
use diesel::FromSqlRow;
use diesel::deserialize::FromSql;
use diesel::expression::AsExpression;
use diesel::pg::Pg;
use diesel::serialize::{IsNull, Output, ToSql};
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, JoinOnDsl,
    OptionalExtension, QueryDsl, Queryable, Selectable, SelectableHelper,
};
use diesel_async::AsyncPgConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use std::io::Write;

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
    /// For a `Call`, how long the call lasted, in seconds.
    pub call_seconds: Option<i32>,
    /// For a `Command`, the bot it was sent to.
    pub command_bot: Option<UserId>,
    /// The messages of this deployment its text links to (`app::message_link`).
    pub linked_messages: MessageLinks,
    /// For a `Warning`, what it warns about (`app::report`).
    pub warning: Option<Warning>,
    /// The plugins that rewrote its text as it was posted or last edited
    /// (`app::plugin::intercept`).
    pub altered_by: Vec<Option<String>>,
    /// For a message of a plugin's account, the card it shows (`app::plugin::card`).
    pub card: Option<Card>,
    /// For a thread reply, its live echo in the parent channel (`app::thread::echo`).
    pub echo: Option<MessageId>,
}

pub mod held;

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
        call_seconds: row.call_seconds,
        command_bot: row.command_bot,
        linked_messages: row.linked_messages.0.clone(),
        warning: row.warning.clone(),
        altered_by: row.altered_by.iter().flatten().cloned().collect(),
        card: row.card.clone(),
        echo: row.echo,
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
/// NOT NULL`) row that `author` uploaded, or that is already in `message`,
/// before linking it to a message. The `attachment` table admits
/// half-uploaded reservations, and exposing them through a message would let
/// a client publish a card pointing at bytes that may never arrive; and an
/// upload is its uploader's to send. Returns [`app::Error::Validation`] if
/// any id is missing, pending, or someone else's.
async fn ensure_attachments_ready(
    conn: &mut AsyncPgConnection,
    author: UserId,
    message: Option<MessageId>,
    attachments: &[AttachmentId],
) -> Result<(), app::Error> {
    if attachments.is_empty() {
        return Ok(());
    }
    use diesel::NullableExpressionMethods;
    let kept = message_attachment::table
        .select(message_attachment::attachment_id)
        .filter(message_attachment::message_id.nullable().eq(message));
    let ready: Vec<AttachmentId> = attachment::table
        .select(attachment::id)
        .filter(
            attachment::id
                .eq_any(attachments)
                .and(attachment::ready_at.is_not_null())
                .and(
                    attachment::uploader
                        .eq(author)
                        .or(attachment::id.eq_any(kept)),
                ),
        )
        .load(conn)
        .await?;
    if ready.len() != attachments.len() {
        return Err(app::Error::Validation(t!("attachmentNotReady")));
    }
    Ok(())
}

/// Checks that `author` may post in `channel_id` with `attachments`, as `create_message` and
/// `held::post` do, answering the channel and the author's access to it.
async fn check_posting(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    author: UserId,
    channel_id: ChannelId,
    attachments: &[AttachmentId],
    echo_to_parent: bool,
) -> Result<(Channel, ChannelAccess), app::Error> {
    let target: Channel = channel::table
        .select(Channel::as_select())
        .filter(
            channel::id
                .eq(channel_id)
                .and(channel::deleted_at.is_null()),
        )
        .first(conn)
        .await?;
    // A plugin's channel holds the plugin's contents, not messages.
    if target.ty == ChannelType::Plugin {
        return Err(app::Error::Validation(t!("pluginChannelHasNoMessages")));
    }
    let access = channel_access(state, conn, author, channel_id).await?;
    access.require(access.send_permission())?;
    if !attachments.is_empty() {
        access.require(Permissions::ATTACH_FILES)?;
    }
    if echo_to_parent {
        let Some(parent) = target.parent_channel else {
            return Err(app::Error::Validation(t!("echoOutsideThread")));
        };
        // An echo is posted in the parent channel, so it takes sending there.
        channel_access(state, conn, author, parent)
            .await?
            .require(Permissions::SEND_MESSAGES)?;
    }
    ensure_attachments_ready(conn, author, None, attachments).await?;
    Ok((target, access))
}

/// What `create_message` posts besides its text.
pub enum Posting {
    /// Text written by its author.
    Text,
    /// A bot command, whose text is the command as checked.
    Command(Invocation),
    /// A moderator's warning, about what it names (`app::report`).
    Warning(Box<Warning>),
    /// Text with a card beneath it, which only a plugin's account posts (`app::plugin::card`).
    Card(Card),
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
    posting: Posting,
) -> Result<Message, app::Error> {
    post(
        state,
        author,
        channel_id,
        content,
        attachments,
        echo_to_parent,
        posting,
        None,
    )
    .await
}

/// Posts a message as [`create_message`] does; `released` names the held message it posts
/// (`held`), which goes in the same transaction, so that it is posted once however many servers
/// try, and its author's apps learn which message it became.
#[allow(clippy::too_many_arguments)]
async fn post(
    state: &GlobalServerContext,
    author: UserId,
    channel_id: ChannelId,
    content: String,
    attachments: Vec<AttachmentId>,
    echo_to_parent: bool,
    posting: Posting,
    released: Option<HeldMessageId>,
) -> Result<Message, app::Error> {
    let (command, warning, card) = match posting {
        Posting::Text => (None, None, None),
        Posting::Command(invocation) => (Some(invocation), None, None),
        Posting::Warning(warning) => (None, Some(*warning), None),
        Posting::Card(card) => (None, None, Some(card)),
    };
    let mut conn = state.connection_pool.get().await?;
    // Plugins decide text before the transaction that saves it opens, so a slow one holds no
    // lock; what the author may not post never reaches them. Commands and warnings are not
    // theirs to decide, nor the system account's notices.
    let (content, altered_by) = if command.is_none() && warning.is_none() {
        let running = intercept::wanted(
            state,
            conn.as_mut(),
            InterceptHook::MessageCreate,
            channel_id,
        )
        .await?;
        if running.is_empty() || system_account::is(conn.as_mut(), author).await? {
            (content, Vec::new())
        } else {
            let access = channel_access(state, conn.as_mut(), author, channel_id).await?;
            access.require(access.send_permission())?;
            // The plugins' host calls take connections of their own, so this one goes back to
            // the pool while they run: a task holding one while it waits for another can leave
            // every connection waiting.
            drop(conn);
            let decided = intercept::decide(
                state,
                InterceptHook::MessageCreate,
                running,
                intercept::Draft {
                    author,
                    access: &access,
                    content,
                    attachments: &attachments,
                    editing: None,
                },
            )
            .await?;
            conn = state.connection_pool.get().await?;
            (decided.content, decided.altered_by)
        }
    } else {
        (content, Vec::new())
    };
    let message = conn
        .transaction(|conn| {
            async move {
                if let Some(held) = released {
                    held::take(conn.as_mut(), held).await?;
                }
                let (target, access) = check_posting(
                    state,
                    conn.as_mut(),
                    author,
                    channel_id,
                    &attachments,
                    echo_to_parent,
                )
                .await?;
                // The echo is made after the reply, which names it from the start.
                let echo = echo_to_parent.then(MessageId::new);
                // A command's text is the command as sent, checked here, and it tags no one.
                let invoked = match &command {
                    Some(invocation) => Some(
                        bot_command::check(
                            state,
                            conn.as_mut(),
                            author,
                            &access,
                            channel_id,
                            invocation,
                        )
                        .await?,
                    ),
                    None => None,
                };
                let (content, mentions) = match &invoked {
                    Some((command, arguments)) => (
                        bot_command::invocation_text(command, arguments),
                        mention::Mentions::default(),
                    ),
                    None => {
                        let mentions = mention::resolve(
                            state,
                            conn.as_mut(),
                            channel_id,
                            &access,
                            mention::parse(&content),
                        )
                        .await?;
                        (content, mentions)
                    }
                };
                let id = MessageId::new();
                // A command's text is the command, which links nowhere, and a warning shows
                // what it is about through `warning`.
                let linked_messages = if invoked.is_some() || warning.is_some() {
                    MessageLinks::default()
                } else {
                    message_link::parse(&content, id)
                };
                let message = Message {
                    id,
                    channel: MaybeLoaded::from_id(channel_id),
                    content,
                    author: MaybeLoaded::from_id(author),
                    timestamp: Utc::now(),
                    deleted_at: None,
                    edited_at: None,
                    kind: if invoked.is_some() {
                        MessageKind::Command
                    } else if warning.is_some() {
                        MessageKind::Warning
                    } else {
                        MessageKind::Standard
                    },
                    poll: None,
                    thread: None,
                    echo_of: None,
                    mentions,
                    call_seconds: None,
                    command_bot: command.as_ref().map(|invocation| invocation.bot),
                    linked_messages,
                    warning,
                    altered_by: altered_by.into_iter().map(Some).collect(),
                    card,
                    echo,
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
                // The bot alone hears of the command, with its arguments checked and typed.
                if let (Some((command, arguments)), Some(invocation)) = (&invoked, &command) {
                    publish_event(
                        state,
                        conn.as_mut(),
                        EventScope::User(invocation.bot),
                        &ServerEvent::BotCommandInvoked {
                            invocation: message.id,
                            channel: channel_id,
                            community: access.community.as_ref().map(|c| c.community),
                            invoker: author,
                            bot: invocation.bot,
                            command: command.name.clone(),
                            arguments: arguments.clone(),
                        },
                    )
                    .await?;
                }
                if target.ty == ChannelType::Thread {
                    thread::record_reply(state, conn.as_mut(), channel_id, message.timestamp)
                        .await?;
                    if let (Some(parent), Some(echo)) = (target.parent_channel, echo) {
                        thread::echo(state, conn.as_mut(), parent, &message, echo).await?;
                    }
                } else {
                    read_state::advance(state, conn.as_mut(), author, channel_id, message.id)
                        .await?;
                }
                if let Some(held) = released {
                    held::announce_released(state, conn.as_mut(), author, held, &message).await?;
                }
                Ok::<_, app::Error>(message)
            }
            .scope_boxed()
        })
        .await?;
    if message.kind == MessageKind::Standard {
        spawn_preview_fetch(state.clone(), message.id, message.content.clone());
    }
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
    channel_access_reading(
        state,
        conn.as_mut(),
        caller,
        *msg.channel.id(),
        Some(id.0.to_string()),
    )
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
                // A DM read by a deployment moderator is logged once, here.
                let may_see = channel_access_reading(state, conn.as_mut(), caller, channel, None)
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
    with_relations(state, conn.as_mut(), visible).await
}

/// Loads the child-table relations of `rows`, one query per relation.
pub(crate) async fn with_relations(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    rows: Vec<Message>,
) -> Result<Vec<MessageWithRelations>, app::Error> {
    let ids: Vec<MessageId> = rows.iter().map(|m| m.id).collect();
    let mut attachments: std::collections::HashMap<MessageId, Vec<AttachmentId>> =
        std::collections::HashMap::new();
    for (message_id, attachment_id) in message_attachment::table
        .select((
            message_attachment::message_id,
            message_attachment::attachment_id,
        ))
        .filter(message_attachment::message_id.eq_any(&ids))
        .load::<(MessageId, AttachmentId)>(conn)
        .await?
    {
        attachments
            .entry(message_id)
            .or_default()
            .push(attachment_id);
    }
    let mut previews = load_previews(conn, state.media_store.as_ref(), &ids).await?;
    Ok(rows
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
    pub linked_messages: Option<MessageLinks>,
    pub altered_by: Option<Vec<Option<String>>>,
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
    // A command was sent as it stands, and its bot has already answered it.
    if kind == MessageKind::Command {
        return Err(app::Error::Validation(t!("commandNotEditable")));
    }
    // Plugins decide new text as they decide a new message's, and the edit records who
    // rewrote it.
    let mut command = command;
    let mut altered_by = None;
    if let Some(content) = command.content.take() {
        let running =
            intercept::wanted(state, conn.as_mut(), InterceptHook::MessageEdit, channel_id).await?;
        let attachments = match &command.attachments {
            Some(attachments) => attachments.clone(),
            None => {
                message_attachment::table
                    .select(message_attachment::attachment_id)
                    .filter(message_attachment::message_id.eq(id))
                    .load(conn.as_mut())
                    .await?
            }
        };
        // As on create, the connection goes back to the pool while the plugins run.
        let decided = if running.is_empty() {
            intercept::Decided {
                content,
                altered_by: Vec::new(),
            }
        } else {
            drop(conn);
            let decided = intercept::decide(
                state,
                InterceptHook::MessageEdit,
                running,
                intercept::Draft {
                    author: caller,
                    access: &access,
                    content,
                    attachments: &attachments,
                    editing: Some(id),
                },
            )
            .await?;
            conn = state.connection_pool.get().await?;
            decided
        };
        command.content = Some(decided.content);
        altered_by = Some(decided.altered_by);
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
                // New text links afresh.
                let linked_messages = command
                    .content
                    .as_deref()
                    .map(|content| message_link::parse(content, id));
                let Some(message) = diesel::update(message::table)
                    .set(MessageChangeset {
                        content: command.content.clone(),
                        edited_at: content_changed.then(Utc::now),
                        mentions: mentions.clone(),
                        linked_messages: linked_messages.clone(),
                        altered_by: altered_by
                            .clone()
                            .map(|by| by.into_iter().map(Some).collect()),
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
                    ensure_attachments_ready(conn.as_mut(), caller, Some(id), new_attachments)
                        .await?;
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
                    mention::replace(conn.as_mut(), id, channel_id, mentions).await?;
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
                        linked_messages: linked_messages.map(|links| links.0),
                        altered_by,
                        card: None,
                        echo: None,
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
    let access = channel_access_moderating(state, conn.as_mut(), caller, channel_id).await?;
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
        async move { soft_delete(state, conn.as_mut(), id).await }.scope_boxed()
    })
    .await?;
    Ok(())
}

/// Marks a message deleted inside the caller's transaction, announcing it and everything that
/// goes with it: a reply's echo (first, so no client ever holds an echo whose reply is gone), a
/// poll shown in it, and a thread's count. A deleted message is hidden, not erased: its text,
/// attachments, and link previews stay, for the warnings and report reviews that show it
/// (`app::report`), and readers never see it again otherwise.
pub(crate) async fn soft_delete(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    id: MessageId,
) -> Result<(), app::Error> {
    let Some(deleted) = diesel::update(message::table)
        .set(message::deleted_at.eq(diesel::dsl::now))
        .filter(message::id.eq(id).and(message::deleted_at.is_null()))
        .returning(Message::as_select())
        .load(conn)
        .await?
        .into_iter()
        .next()
    else {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    };
    if deleted.kind != MessageKind::ThreadEcho {
        thread::delete_echo_of(state, conn, id).await?;
    }
    publish_event(
        state,
        conn,
        EventScope::Channel(*deleted.channel.id()),
        &ServerEvent::Message(MessageEvent::Delete { id }),
    )
    .await?;
    // An echo deleted alone leaves its reply free to be echoed again.
    if deleted.kind == MessageKind::ThreadEcho
        && let Some(reply) = deleted.echo_of
    {
        thread::forget_echo(state, conn, reply, id).await?;
    }
    // Deleting the message a poll is shown in ends the poll; its announcement, if any, is an
    // ordinary message and stays.
    if deleted.kind == MessageKind::Poll
        && let Some(poll) = deleted.poll
    {
        app::poll::delete_poll(state, conn, poll, *deleted.channel.id()).await?;
    }
    let ty: ChannelType = channel::table
        .select(channel::ty)
        .filter(channel::id.eq(*deleted.channel.id()))
        .first(conn)
        .await?;
    if ty == ChannelType::Thread {
        thread::record_removal(state, conn, *deleted.channel.id()).await?;
    }
    Ok(())
}

/// Deletes every message `author` posted since `since` in `community`'s channels and threads,
/// or with no community anywhere on the deployment, DMs included, inside the caller's
/// transaction, which has checked who may (a ban with a deletion window, `app::ban` and
/// `app::user_ban`). Echoes go with their replies. Returns the ids.
pub(crate) async fn delete_recent_by(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community: Option<CommunityId>,
    author: UserId,
    since: DateTime<Utc>,
) -> Result<Vec<MessageId>, app::Error> {
    let mut query = message::table
        .inner_join(channel::table.on(channel::id.eq(message::channel)))
        .select(message::id)
        .filter(message::author.eq(author))
        .filter(message::deleted_at.is_null())
        .filter(message::timestamp.ge(since))
        .filter(message::kind.ne(MessageKind::ThreadEcho))
        .order(message::id.desc())
        .into_boxed();
    if let Some(community) = community {
        query = query.filter(channel::community.eq(community));
    }
    let ids: Vec<MessageId> = query.load(conn).await?;
    for id in &ids {
        soft_delete(state, conn, *id).await?;
    }
    Ok(ids)
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
    let access = channel_access_moderating(state, conn.as_mut(), caller, channel_id).await?;
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
                    linked_messages: None,
                    altered_by: None,
                    card: None,
                    echo: None,
                }),
            )
            .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// What a message is. Both poll kinds and echoes carry no `content`; the client renders the poll
/// kinds from the poll record the message's `poll` field names, and an echo from its reply.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    Serialize,
    utoipa::ToSchema,
    schemars::JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = crate::database::schema::sql_types::MessageKind)]
pub enum MessageKind {
    /// Text written by its author.
    Standard,
    /// The message a poll was opened with.
    Poll,
    /// The system message announcing a poll's outcome; its `author` is the poll's creator.
    PollClosed,
    /// A thread reply shown in the thread's parent channel, by reference: `echoOf` names the
    /// reply, and the echo has no content of its own. Its `author` is the reply's.
    ThreadEcho,
    /// The system message recording that a DM's call ended; `callSeconds` says how long it
    /// lasted and its `author` is who started it. It has no content of its own.
    Call,
    /// The system message recording that a DM's call ended without anyone joining whoever
    /// started it, its `author`. It has no content of its own.
    MissedCall,
    /// A bot command its `author` invoked, as it was sent (`/name` and its arguments), to the
    /// bot `commandBot` names (see `app::bot_command`).
    Command,
    /// A moderator's warning to the person `warning` names, sent for the deployment's
    /// moderators by the system account in its DM with them: the reviewer's words as
    /// `content`, about what `warning` holds (see `app::report`).
    Warning,
}

impl ToSql<crate::database::schema::sql_types::MessageKind, Pg> for MessageKind {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        out.write_all(match self {
            MessageKind::Standard => b"standard",
            MessageKind::Poll => b"poll",
            MessageKind::PollClosed => b"poll_closed",
            MessageKind::ThreadEcho => b"thread_echo",
            MessageKind::Call => b"call",
            MessageKind::MissedCall => b"missed_call",
            MessageKind::Command => b"command",
            MessageKind::Warning => b"warning",
        })?;
        Ok(IsNull::No)
    }
}

impl FromSql<crate::database::schema::sql_types::MessageKind, Pg> for MessageKind {
    fn from_sql(
        bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
    ) -> diesel::deserialize::Result<Self> {
        match bytes.as_bytes() {
            b"standard" => Ok(MessageKind::Standard),
            b"poll" => Ok(MessageKind::Poll),
            b"poll_closed" => Ok(MessageKind::PollClosed),
            b"thread_echo" => Ok(MessageKind::ThreadEcho),
            b"call" => Ok(MessageKind::Call),
            b"command" => Ok(MessageKind::Command),
            b"missed_call" => Ok(MessageKind::MissedCall),
            b"warning" => Ok(MessageKind::Warning),
            _ => Err(format!(
                "Unrecognized enum variant: {:?}",
                String::from_utf8_lossy(bytes.as_bytes())
            )
            .into()),
        }
    }
}
