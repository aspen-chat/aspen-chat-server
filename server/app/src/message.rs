use crate::bot_command::{self, Invocation};
use crate::channel::Channel;
use crate::channel::ChannelType;
use crate::context::GlobalServerContext;
use crate::link_preview::{delete_images_for_message, load_previews, spawn_preview_fetch};
use crate::mana;
use crate::mention::{self, Mentions};
use crate::message_link::{self, MessageLinks};
use crate::moderation_log::{ModerationAction, log_moderation};
use crate::permissions::{
    ChannelAccess, Permissions, channel_access, channel_access_moderating, channel_access_reading,
    missing,
};
use crate::plugin::card::Card;
use crate::plugin::intercept;
use crate::plugin::manifest::InterceptHook;
use crate::report::Warning;
use crate::t;
use crate::user::User;
use crate::visibility::Visibility;
use crate::{
    AttachmentId, ChannelId, EventScope, PollId, UserId, publish_event, read_state, system_account,
    thread,
};
use crate::{HeldMessageId, MaybeLoaded, MessageId};
use aspen_schema::attachment;
use aspen_schema::channel;
use aspen_schema::message;
use aspen_schema::message_attachment;
use aspen_wire::link_preview::LinkPreview;
pub use aspen_wire::message::MessageKind;
use aspen_wire::message_enum;
use aspen_wire::message_enum::request::MessageUpdateRequest;
use aspen_wire::message_enum::server_event::{MessageEvent, PinEvent, ServerEvent};
use chrono::{DateTime, Utc};
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, JoinOnDsl,
    OptionalExtension, QueryDsl, Queryable, Selectable, SelectableHelper,
};
use diesel_async::AsyncPgConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

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
    /// The message of its channel it replies to (`reply`).
    pub reply_to: Option<MessageId>,
    /// Whether it was posted tagging the author of `reply_to`.
    pub tags_replied: bool,
}

pub mod held;
pub mod reply;

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
        reply_to: row.reply_to,
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

/// The most attachments one message may carry, whoever sends it.
pub const MAX_ATTACHMENTS: usize = 50;

/// The most messages one channel keeps pinned.
pub const MAX_PINS: i64 = 250;

/// Verify every id in `attachments` corresponds to a confirmed (`ready_at IS
/// NOT NULL`) row that `author` uploaded, or that is already in `message`,
/// before linking it to a message. The `attachment` table admits
/// half-uploaded reservations, and exposing them through a message would let
/// a client publish a card pointing at bytes that may never arrive; and an
/// upload is its uploader's to send. Returns [`app::Error::Validation`] if
/// any id is missing, pending, or someone else's, or if there are more than
/// [`MAX_ATTACHMENTS`].
async fn ensure_attachments_ready(
    conn: &mut AsyncPgConnection,
    author: UserId,
    message: Option<MessageId>,
    attachments: &[AttachmentId],
) -> Result<(), crate::Error> {
    if attachments.is_empty() {
        return Ok(());
    }
    if attachments.len() > MAX_ATTACHMENTS {
        return Err(crate::Error::Validation(t!(
            "messageTooManyAttachments",
            max = MAX_ATTACHMENTS
        )));
    }
    use diesel::NullableExpressionMethods;
    let kept = message_attachment::table
        .select(message_attachment::attachment_id)
        .filter(message_attachment::message_id.nullable().eq(message));
    // Locked until the message is saved, so the sweep of unsent attachments
    // (`attachment::sweep_unsent`) passes them by.
    let ready: Vec<AttachmentId> = attachment::table
        .select(attachment::id)
        .filter(
            attachment::id
                .eq_any(attachments)
                .and(attachment::ready_at.is_not_null())
                // Evidence is no one's to post again (`attachment::evidence`).
                .and(attachment::evidence_at.is_null())
                .and(
                    attachment::uploader
                        .eq(author)
                        .or(attachment::id.eq_any(kept)),
                ),
        )
        .for_key_share()
        .load(conn)
        .await?;
    if ready.len() != attachments.len() {
        return Err(crate::Error::Validation(t!("attachmentNotReady")));
    }
    Ok(())
}

/// Checks that `author` may post text in `channel_id`, answering the channel and the author's
/// access to it: that it holds messages, and that they may send there. In a transaction it holds
/// the channel ([`hold_channel`]).
pub(crate) async fn may_post(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    author: UserId,
    channel_id: ChannelId,
) -> Result<(Channel, ChannelAccess), crate::Error> {
    let target = hold_channel(conn, channel_id).await?;
    let access = may_send_in(state, conn, author, &target).await?;
    Ok((target, access))
}

/// Checks that `author` may say they are typing in `channel_id` (`app::typing`), which takes what
/// posting text there takes ([`may_post`]). It reads the channel without holding it: nothing is
/// written after it, and a check that locks a row on every word would let typing frames write to
/// the database.
pub(crate) async fn may_type(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    author: UserId,
    channel_id: ChannelId,
) -> Result<(), crate::Error> {
    let target = channel::table
        .select(Channel::as_select())
        .filter(
            channel::id
                .eq(channel_id)
                .and(channel::deleted_at.is_null()),
        )
        .first(conn)
        .await?;
    may_send_in(state, conn, author, &target).await?;
    Ok(())
}

/// Checks that `target` holds messages and that `author` may send there, answering their access.
async fn may_send_in(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    author: UserId,
    target: &Channel,
) -> Result<ChannelAccess, crate::Error> {
    // A plugin's channel holds the plugin's contents, not messages.
    if target.ty == ChannelType::Plugin {
        return Err(crate::Error::Validation(t!("pluginChannelHasNoMessages")));
    }
    let access = channel_access(state, conn, author, target.id).await?;
    access.require(access.send_permission())?;
    Ok(access)
}

/// The live channel `channel_id`, whose row a transaction posting in it holds against removal
/// from here until it commits, as the message's reference to it would from its insert on: a
/// thread removed for being empty (`thread::remove_if_unreplied`) either waits for the posting
/// and sees it, or is gone before the posting reads it, which then finds no channel.
pub(crate) async fn hold_channel(
    conn: &mut AsyncPgConnection,
    channel_id: ChannelId,
) -> Result<Channel, crate::Error> {
    Ok(channel::table
        .select(Channel::as_select())
        .filter(
            channel::id
                .eq(channel_id)
                .and(channel::deleted_at.is_null()),
        )
        .for_key_share()
        .first(conn)
        .await?)
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
) -> Result<(Channel, ChannelAccess), crate::Error> {
    let (target, access) = may_post(state, conn, author, channel_id).await?;
    if echo_to_parent {
        let Some(parent) = target.parent_channel else {
            return Err(crate::Error::Validation(t!("echoOutsideThread")));
        };
        // An echo is posted in the parent channel, so it takes sending there.
        channel_access(state, conn, author, parent)
            .await?
            .require(Permissions::SEND_MESSAGES)?;
    }
    check_attachments(conn, author, &access, attachments).await?;
    Ok((target, access))
}

/// Checks that `author` may post the reply that makes `unmade`, as `check_posting` checks a
/// reply to a thread already made, answering their access to the thread. The transaction that
/// makes the thread checks again (`thread::open_in`, then `check_posting`); this refuses what it
/// would before the plugins see the reply, and before the thread is announced only to be rolled
/// back.
async fn check_first_reply(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    author: UserId,
    unmade: &Unmade,
    attachments: &[AttachmentId],
    echo_to_parent: bool,
) -> Result<ChannelAccess, crate::Error> {
    let parent =
        thread::check_startable(state, conn, author, unmade.starter, unmade.parent).await?;
    // An echo is posted in the parent channel, so it takes sending there.
    if echo_to_parent {
        parent.require(Permissions::SEND_MESSAGES)?;
    }
    let access = parent.into_unmade_thread(unmade.id);
    access.require(access.send_permission())?;
    check_attachments(conn, author, &access, attachments).await?;
    Ok(access)
}

/// Checks that `author` may post `attachments` where `access` is, and that each is theirs and
/// ready.
async fn check_attachments(
    conn: &mut AsyncPgConnection,
    author: UserId,
    access: &ChannelAccess,
    attachments: &[AttachmentId],
) -> Result<(), crate::Error> {
    if !attachments.is_empty() {
        access.require(Permissions::ATTACH_FILES)?;
    }
    ensure_attachments_ready(conn, author, None, attachments).await
}

/// Where a message is posted.
#[derive(Debug, Clone, Copy)]
pub enum To {
    /// A channel, threads among them.
    Channel(ChannelId),
    /// The thread a message starts, which posting the first reply to it makes, in the
    /// transaction that posts the reply, so a thread is never made by replying to it without
    /// the reply.
    ThreadOf(MessageId),
}

/// A thread not made yet, which the reply posted to it makes.
struct Unmade {
    /// Chosen before the plugins decide the reply, so they are told where it goes.
    id: ChannelId,
    /// The message it starts from.
    starter: MessageId,
    /// The channel it is made in.
    parent: ChannelId,
}

/// Where a message posted `to` goes as things stand: its channel, and, for a reply to a thread
/// not made yet, that thread, which the reply's transaction makes with the id named here, or
/// finds made since (`thread::open_in`).
async fn resolve(
    conn: &mut AsyncPgConnection,
    to: To,
) -> Result<(ChannelId, Option<Unmade>), crate::Error> {
    Ok(match to {
        To::Channel(channel) => (channel, None),
        To::ThreadOf(starter) => match thread::reply_place(conn, starter).await? {
            thread::ReplyPlace::Thread(thread) => (thread, None),
            thread::ReplyPlace::Unmade { parent } => {
                let id = ChannelId::new();
                (
                    id,
                    Some(Unmade {
                        id,
                        starter,
                        parent,
                    }),
                )
            }
        },
    })
}

/// Makes the thread `unmade` names in the caller's transaction, or finds it made since it was
/// resolved, answering its id; with none, `channel_id`.
async fn made(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    author: UserId,
    channel_id: ChannelId,
    unmade: Option<&Unmade>,
) -> Result<ChannelId, crate::Error> {
    Ok(match unmade {
        Some(unmade) => {
            thread::open_in(state, conn, author, unmade.starter, unmade.id)
                .await?
                .0
                .id
        }
        None => channel_id,
    })
}

/// The longest a message's text may be, in characters (Unicode scalar values, as Rust counts
/// `char`s, not bytes): what anyone writes, a plugin rewrites, or a command's text comes to.
/// Clients hold their composer to it too.
pub const MAX_CONTENT_CHARS: usize = 10_000;

/// Refuses text longer than [`MAX_CONTENT_CHARS`].
pub fn check_content(content: &str) -> crate::Result<()> {
    // No character is shorter than a byte, so text no longer in bytes needs no counting.
    if content.len() > MAX_CONTENT_CHARS && content.chars().count() > MAX_CONTENT_CHARS {
        return Err(crate::Error::Validation(t!(
            "messageContentLength",
            max = MAX_CONTENT_CHARS
        )));
    }
    if nesting_depth(content) > MAX_NESTING {
        return Err(crate::Error::Validation(t!(
            "messageNestingTooDeep",
            max = MAX_NESTING
        )));
    }
    Ok(())
}

/// How deeply a message's Markdown may nest (quotes in quotes, lists in lists, and what they
/// hold), as the client's `MAX_NESTING` counts it: readers' apps render the tree recursively, so
/// a short message of thousands of levels (`>>>>…`, `1. 1. 1. …`) would overflow their stack,
/// and no message a person writes to be read comes near it.
pub const MAX_NESTING: usize = 32;

/// The deepest nesting of `content`'s Markdown elements, counted on the parser's events with no
/// recursion.
fn nesting_depth(content: &str) -> usize {
    let mut depth: usize = 0;
    let mut deepest = 0;
    for event in crate::markdown::parser(content) {
        match event {
            pulldown_cmark::Event::Start(_) => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            pulldown_cmark::Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    deepest
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
) -> Result<Message, crate::Error> {
    post(
        state,
        author,
        To::Channel(channel_id),
        content,
        attachments,
        echo_to_parent,
        None,
        posting,
        None,
    )
    .await
}

/// Posts a message as [`create_message`] does, where `to` says, replying to a message there as
/// `reply` says; `released` names the held message it posts (`held`), which goes in the same
/// transaction, so that it is posted once however many servers try, and its author's apps learn
/// which message it became. Only text is posted to a thread not made yet (`To::ThreadOf`), and it
/// replies to nothing, since the thread holds nothing yet.
#[allow(clippy::too_many_arguments)]
async fn post(
    state: &GlobalServerContext,
    author: UserId,
    to: To,
    content: String,
    attachments: Vec<AttachmentId>,
    echo_to_parent: bool,
    reply: Option<reply::Reply>,
    posting: Posting,
    released: Option<HeldMessageId>,
) -> Result<Message, crate::Error> {
    let (command, warning, card) = match posting {
        Posting::Text => (None, None, None),
        Posting::Command(invocation) => (Some(invocation), None, None),
        Posting::Warning(warning) => (None, Some(*warning), None),
        Posting::Card(card) => (None, None, Some(card)),
    };
    check_content(&content)?;
    let mut conn = state.connection_pool.get().await?;
    let (channel_id, unmade) = resolve(conn.as_mut(), to).await?;
    // Commands are checked against their channel before it is made; only `held::post`, which
    // posts text, names a thread not made yet.
    debug_assert!(unmade.is_none() || (command.is_none() && warning.is_none() && card.is_none()));
    if unmade.is_some() && reply.is_some() {
        return Err(crate::Error::Validation(t!("replyElsewhere")));
    }
    let unmade_access = match &unmade {
        Some(unmade) => Some(
            check_first_reply(
                state,
                conn.as_mut(),
                author,
                unmade,
                &attachments,
                echo_to_parent,
            )
            .await?,
        ),
        None => None,
    };
    // Plugins decide text before the transaction that saves it opens, so a slow one holds no
    // lock; what the author may not post, and attachments that are not theirs to post, never
    // reach them. A command is decided as the text it shows, with the files it takes. Warnings
    // are not theirs to decide, nor the system account's notices.
    let (content, altered_by) = if warning.is_none() {
        // A thread not made yet is where its parent is, whose plugins decide its replies.
        let running = intercept::wanted(
            state,
            conn.as_mut(),
            InterceptHook::MessageCreate,
            unmade.as_ref().map_or(channel_id, |unmade| unmade.parent),
        )
        .await?;
        if running.is_empty() || system_account::is(conn.as_mut(), author).await? {
            (content, Vec::new())
        } else {
            // Checked as the saving transaction checks again: the right to post, and that
            // every attachment is the author's own upload, so no plugin is shown another's.
            let access = match unmade_access {
                Some(access) => access,
                None => {
                    check_posting(
                        state,
                        conn.as_mut(),
                        author,
                        channel_id,
                        &attachments,
                        echo_to_parent,
                    )
                    .await?
                    .1
                }
            };
            let shown = match &command {
                Some(invocation) => {
                    let (checked, arguments) = bot_command::check(
                        state,
                        conn.as_mut(),
                        author,
                        &access,
                        channel_id,
                        invocation,
                    )
                    .await?;
                    Some(bot_command::invocation_text(&checked, &arguments))
                }
                None => None,
            };
            // The plugins' host calls take connections of their own, so this one goes back to
            // the pool while they run: a task holding one while it waits for another can leave
            // every connection waiting.
            drop(conn);
            let decided = match shown {
                // A command's arguments are what its bot receives, so its text is never
                // rewritten: a plugin that would change it refuses it.
                Some(shown) => {
                    intercept::decide_unchanged(
                        state,
                        InterceptHook::MessageCreate,
                        running,
                        intercept::Draft {
                            author,
                            access: &access,
                            content: shown,
                            attachments: &attachments,
                            retouched: None,
                            editing: None,
                            unmade_thread_of: unmade.as_ref().map(|unmade| unmade.parent),
                        },
                    )
                    .await?;
                    (content, Vec::new())
                }
                None => {
                    let decided = intercept::decide(
                        state,
                        InterceptHook::MessageCreate,
                        running,
                        intercept::Draft {
                            author,
                            access: &access,
                            content,
                            attachments: &attachments,
                            retouched: None,
                            editing: None,
                            unmade_thread_of: unmade.as_ref().map(|unmade| unmade.parent),
                        },
                    )
                    .await?;
                    check_content(&decided.content)?;
                    (decided.content, decided.altered_by)
                }
            };
            conn = state.connection_pool.get().await?;
            decided
        }
    } else {
        (content, Vec::new())
    };
    let (message, community) = conn
        .transaction(|conn| {
            async move {
                if let Some(held) = released {
                    held::take(conn.as_mut(), held).await?;
                }
                let channel_id =
                    made(state, conn.as_mut(), author, channel_id, unmade.as_ref()).await?;
                let (target, access) = check_posting(
                    state,
                    conn.as_mut(),
                    author,
                    channel_id,
                    &attachments,
                    echo_to_parent,
                )
                .await?;
                // Whom the reply tags for what it answers, if anyone.
                let replied = match reply {
                    Some(reply) => {
                        reply::check(conn.as_mut(), channel_id, reply.to, released.is_some())
                            .await?
                            .filter(|original| reply.tag && *original != author)
                    }
                    None => None,
                };
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
                    Some((command, arguments)) => {
                        let text = bot_command::invocation_text(command, arguments);
                        check_content(&text)?;
                        (text, mention::Mentions::default())
                    }
                    None => {
                        let requested = match replied {
                            Some(original) => mention::parse(&content).with_user(original),
                            None => mention::parse(&content),
                        };
                        let mentions =
                            mention::resolve(state, conn.as_mut(), channel_id, &access, requested)
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
                    reply_to: reply.map(|reply| reply.to),
                    tags_replied: reply.is_some_and(|reply| reply.tag),
                };
                diesel::insert_into(message::table)
                    .values(&message)
                    .execute(conn.as_mut())
                    .await?;
                mention::record(conn.as_mut(), message.id, channel_id, &message.mentions).await?;
                let held: Vec<MessageAttachment> = attachments
                    .iter()
                    .map(|attachment| MessageAttachment {
                        message_id: message.id,
                        attachment_id: *attachment,
                    })
                    .collect();
                if !held.is_empty() {
                    diesel::insert_into(message_attachment::table)
                        .values(&held)
                        .execute(conn.as_mut())
                        .await?;
                }
                crate::attachment::mark_sent(conn.as_mut(), &attachments).await?;
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
                read_state::advance(state, conn.as_mut(), author, channel_id, message.id).await?;
                if target.ty == ChannelType::Thread {
                    thread::record_reply(state, conn.as_mut(), channel_id, message.timestamp)
                        .await?;
                    crate::thread_follow::reply_posted(
                        state,
                        conn.as_mut(),
                        &target,
                        author,
                        &message.mentions,
                    )
                    .await?;
                    if let (Some(parent), Some(echo)) = (target.parent_channel, echo) {
                        thread::echo(state, conn.as_mut(), parent, &message, echo).await?;
                    }
                }
                if let Some(held) = released {
                    held::announce_released(state, conn.as_mut(), author, held, &message).await?;
                }
                let community = access
                    .community
                    .as_ref()
                    .filter(|community| community.member)
                    .map(|community| community.community);
                Ok::<_, crate::Error>((message, community))
            }
            .scope_boxed()
        })
        .await?;
    // Posting there shows its members the author is about.
    if let Some(community) = community {
        crate::presence_upkeep::seen_in(conn.as_mut(), author, community).await;
    }
    // The system account's notices quote names others chose, so nothing in them is fetched and
    // shown as a card under its name.
    if message.kind == MessageKind::Standard && !system_account::is(conn.as_mut(), author).await? {
        spawn_preview_fetch(state.clone(), author, message.id, &message.content);
    }
    Ok(message)
}

pub async fn read_message(
    state: &GlobalServerContext,
    caller: UserId,
    id: MessageId,
) -> Result<MessageWithRelations, crate::Error> {
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
) -> Result<Vec<MessageWithRelations>, crate::Error> {
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
pub async fn with_relations(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    rows: Vec<Message>,
) -> Result<Vec<MessageWithRelations>, crate::Error> {
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

/// An edit of a message: new text, a new description or spoiler mark for one of its files, or
/// both. Its files themselves are never changed by an edit; one is taken off by
/// [`remove_attachment`].
#[derive(Debug, Clone, Default)]
pub struct Edit {
    pub content: Option<String>,
    pub retouch: Option<Retouch>,
}

/// A new description or spoiler mark for one of a message's files; `None` leaves each as it is,
/// and `Some(None)` clears the description.
#[derive(Debug, Clone)]
pub struct Retouch {
    pub attachment: AttachmentId,
    pub description: Option<Option<String>>,
    pub spoiler: Option<bool>,
}

impl From<MessageUpdateRequest> for Edit {
    fn from(request: MessageUpdateRequest) -> Self {
        Edit {
            content: request.content,
            retouch: None,
        }
    }
}

/// A file's description and spoiler mark as they stand, with what a retouch makes of them.
struct Retouched {
    attachment: AttachmentId,
    before: (Option<String>, bool),
    after: (Option<String>, bool),
}

impl Retouched {
    fn changes(&self) -> bool {
        self.before != self.after
    }

    /// Whether it says more than before: a new or different description, or a spoiler mark
    /// taken off. Covering a file or clearing its description only takes away.
    fn adds(&self) -> bool {
        (self.after.0.is_some() && self.after.0 != self.before.0)
            || (self.before.1 && !self.after.1)
    }
}

/// Edits the caller's own message. Only text and files' descriptions and spoiler marks change;
/// plugins decide the message as it will stand, as they decide a new one.
pub async fn update_message(
    state: &GlobalServerContext,
    caller: UserId,
    id: MessageId,
    edit: Edit,
    waited: Option<HeldMessageId>,
) -> Result<Edited, crate::Error> {
    let mut conn = state.connection_pool.get().await?;
    let (channel_id, kind, author, before_content): (ChannelId, MessageKind, UserId, String) =
        message::table
            .select((
                message::channel,
                message::kind,
                message::author,
                message::content,
            ))
            .filter(message::id.eq(id).and(message::deleted_at.is_null()))
            .first(conn.as_mut())
            .await?;
    let access = channel_access(state, conn.as_mut(), caller, channel_id).await?;
    // A message says what its author said; nobody else may put words in it.
    if author != caller {
        return Err(crate::Error::Forbidden(t!("editOthersMessage")));
    }
    access.ensure_unblocked()?;
    // Only text its author wrote is theirs to edit. An echo shows its reply's content; a
    // command was sent as it stands, and its bot has already answered it; the other kinds
    // record what Aspen keeps (a poll, a call, a warning, a join) and have no text of their
    // author's.
    match kind {
        MessageKind::Standard => {}
        MessageKind::ThreadEcho => return Err(crate::Error::Validation(t!("echoNotEditable"))),
        MessageKind::Command => return Err(crate::Error::Validation(t!("commandNotEditable"))),
        MessageKind::Poll
        | MessageKind::PollClosed
        | MessageKind::Call
        | MessageKind::MissedCall
        | MessageKind::Warning
        | MessageKind::MemberJoined => {
            return Err(crate::Error::Validation(t!("messageNotEditable")));
        }
    }
    let attachments: Vec<AttachmentId> = message_attachment::table
        .select(message_attachment::attachment_id)
        .filter(message_attachment::message_id.eq(id))
        .load(conn.as_mut())
        .await?;
    let retouched = match &edit.retouch {
        None => None,
        Some(retouch) => {
            // Only a file of this message is retouched through it.
            if !attachments.contains(&retouch.attachment) {
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            }
            let before: (Option<String>, bool) = attachment::table
                .select((attachment::description, attachment::spoiler))
                .filter(attachment::id.eq(retouch.attachment))
                .first(conn.as_mut())
                .await?;
            let description = match &retouch.description {
                Some(raw) => crate::attachment::description(raw.clone())?,
                None => before.0.clone(),
            };
            let after = (description, retouch.spoiler.unwrap_or(before.1));
            Some(Retouched {
                attachment: retouch.attachment,
                before,
                after,
            })
        }
    };
    // Adding to a message is posting, so it takes what posting here takes: Send messages (Send
    // messages in threads in a thread) for new text or a new description. An edit that only
    // takes away does not: clearing the text (`content` given as empty), clearing a description,
    // or covering a file, so someone who may no longer post can still withdraw what they said.
    let adds_text = edit.content.as_deref().is_some_and(|text| !text.is_empty());
    if adds_text || retouched.as_ref().is_some_and(Retouched::adds) {
        access.require(access.send_permission())?;
    }
    // An edit that changes nothing is dropped, answered with the message as it stands; the apps
    // do not send one.
    let same_text = edit
        .content
        .as_ref()
        .is_none_or(|content| *content == before_content);
    let same_files = retouched.as_ref().is_none_or(|r| !r.changes());
    if same_text && same_files {
        if let Some(waited) = waited {
            held::take(conn.as_mut(), waited).await?;
        }
        return Ok(Edited::Done(Box::new(
            current(state, conn.as_mut(), id).await?,
        )));
    }
    let retouched = retouched.filter(Retouched::changes);
    // Lengthening a message spends its channel's mana as posting does (`app::mana`), and waits
    // its turn in a busy channel; one waited already has been paid for. Retouching a file
    // spends none, and waits for nothing.
    if waited.is_none()
        && retouched.is_none()
        && let Some(content) = &edit.content
    {
        check_content(content)?;
        let cost = mana::edit_cost(&state.settings(), &before_content, content, 0);
        if mana::decide_for(state, conn.as_mut(), caller, channel_id, cost).await?
            == mana::Decision::Wait
        {
            drop(conn);
            let change = WaitingEdit {
                content: Some(content.clone()),
            };
            let held = mana::queue::wait(
                state,
                mana::queue::Waiting {
                    author: caller,
                    channel: channel_id,
                    kind: held::HeldKind::Edit,
                    content: content.clone(),
                    attachments: Vec::new(),
                    echo_to_parent: false,
                    reply: None,
                    target: Some(id),
                    payload: Some(serde_json::to_value(change)?),
                    cost,
                },
            )
            .await?;
            return Ok(Edited::Waiting(held));
        }
    }
    // Plugins decide an edit as they decide a new message, whether it changes the text, a
    // file's description, or both, shown the message as it will stand; the edit records who
    // rewrote it.
    let mut content = edit.content;
    let mut altered_by = None;
    if let Some(content) = &content {
        check_content(content)?;
    }
    let running =
        intercept::wanted(state, conn.as_mut(), InterceptHook::MessageEdit, channel_id).await?;
    if !running.is_empty() {
        // With only a file retouched, a message keeps its text, and the plugins that rewrote it
        // before, unless one rewrites it now.
        let (shown, kept_text) = match content.take() {
            Some(content) => (content, None),
            None => {
                let by: Vec<Option<String>> = message::table
                    .select(message::altered_by)
                    .filter(message::id.eq(id))
                    .first(conn.as_mut())
                    .await?;
                (before_content.clone(), Some(by))
            }
        };
        // As on create, the connection goes back to the pool while the plugins run.
        drop(conn);
        let decided = intercept::decide(
            state,
            InterceptHook::MessageEdit,
            running,
            intercept::Draft {
                author: caller,
                access: &access,
                content: shown,
                attachments: &attachments,
                retouched: retouched.as_ref().map(|r| intercept::Retouched {
                    attachment: r.attachment,
                    description: r.after.0.clone(),
                    spoiler: r.after.1,
                }),
                editing: Some(id),
                unmade_thread_of: None,
            },
        )
        .await?;
        check_content(&decided.content)?;
        conn = state.connection_pool.get().await?;
        match kept_text {
            Some(_) if decided.content == before_content => {}
            Some(mut by) => {
                // Its earlier rewriters' changes are still in the text, so they stay named.
                for plugin in decided.altered_by {
                    if !by.contains(&Some(plugin.clone())) {
                        by.push(Some(plugin));
                    }
                }
                content = Some(decided.content);
                altered_by = Some(by.into_iter().flatten().collect());
            }
            None => {
                content = Some(decided.content);
                altered_by = Some(decided.altered_by);
            }
        }
    } else if content.is_some() {
        altered_by = Some(Vec::new());
    }
    let content_changed = content.is_some();
    let new_content_for_refetch = content.clone();
    let (message, attachments, previews_cleared) = conn
        .transaction(|conn| {
            async move {
                if let Some(waited) = waited {
                    held::take(conn.as_mut(), waited).await?;
                    held::announce_edited(state, conn.as_mut(), caller, waited, channel_id, id)
                        .await?;
                }
                // New text tags afresh, with the author's permissions as they are now, and as a
                // reply tags who it tagged when posted.
                let mentions = match &content {
                    Some(content) => {
                        let requested =
                            match reply::tagged_on_edit(conn.as_mut(), id, caller).await? {
                                Some(original) => mention::parse(content).with_user(original),
                                None => mention::parse(content),
                            };
                        Some(
                            mention::resolve(state, conn.as_mut(), channel_id, &access, requested)
                                .await?,
                        )
                    }
                    None => None,
                };
                // New text links afresh.
                let linked_messages = content
                    .as_deref()
                    .map(|content| message_link::parse(content, id));
                let Some(message) = diesel::update(message::table)
                    .set(MessageChangeset {
                        content: content.clone(),
                        edited_at: Some(Utc::now()),
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
                    return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
                };
                if let Some(retouched) = &retouched {
                    crate::attachment::retouch(
                        state,
                        conn.as_mut(),
                        id,
                        channel_id,
                        retouched.attachment,
                        retouched.after.0.clone(),
                        retouched.after.1,
                    )
                    .await?;
                }
                if let Some(mentions) = &mentions {
                    mention::replace(conn.as_mut(), id, channel_id, mentions).await?;
                }
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
                    conn.as_mut(),
                    EventScope::Message(id),
                    &ServerEvent::Message(MessageEvent::Update {
                        id,
                        content,
                        // A retouched file is named among the message's files, so whatever
                        // follows its files (plugins observing edits) reads it again.
                        attachments: retouched.is_some().then(|| attachments.clone()),
                        edited_at: Some(message.edited_at),
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

                Ok::<_, crate::Error>((message, attachments, previews_cleared))
            }
            .scope_boxed()
        })
        .await?;

    if previews_cleared && let Some(content) = new_content_for_refetch {
        spawn_preview_fetch(state.clone(), caller, id, &content);
    }

    // Re-load the current preview set for the REST response. On a content
    // edit this will be empty (we just wiped it); for a retouch the previous
    // set is still current.
    let mut conn = state.connection_pool.get().await?;
    let link_previews = load_previews(conn.as_mut(), state.media_store.as_ref(), &[id])
        .await?
        .remove(&id)
        .unwrap_or_default();

    Ok(Edited::Done(Box::new(MessageWithRelations {
        message,
        attachments,
        link_previews,
    })))
}

/// Posts a slash command where `channel` is, or has it wait for the channel's mana when the
/// channel is busy (`app::mana`). It is checked as posting checks it before it spends anything,
/// and again as it is posted.
pub async fn invoke_command(
    state: &GlobalServerContext,
    author: UserId,
    channel: ChannelId,
    invocation: Invocation,
) -> crate::Result<held::Posted> {
    let mut conn = state.connection_pool.get().await?;
    check_posting(
        state,
        conn.as_mut(),
        author,
        channel,
        &invocation.attachments,
        false,
    )
    .await?;
    let access = channel_access(state, conn.as_mut(), author, channel).await?;
    let (command, arguments) =
        bot_command::check(state, conn.as_mut(), author, &access, channel, &invocation).await?;
    let text = bot_command::invocation_text(&command, &arguments);
    let cost = mana::message_cost(&state.settings(), &text, invocation.attachments.len());
    if mana::decide_for(state, conn.as_mut(), author, channel, cost).await? == mana::Decision::Wait
    {
        drop(conn);
        let held = mana::queue::wait(
            state,
            mana::queue::Waiting {
                author,
                channel,
                kind: held::HeldKind::Command,
                content: text,
                attachments: invocation.attachments.clone(),
                echo_to_parent: false,
                reply: None,
                target: None,
                payload: Some(serde_json::to_value(&invocation)?),
                cost,
            },
        )
        .await?;
        return Ok(held::Posted::Held(held));
    }
    drop(conn);
    let attachments = invocation.attachments.clone();
    post(
        state,
        author,
        To::Channel(channel),
        String::new(),
        attachments,
        false,
        None,
        Posting::Command(invocation),
        None,
    )
    .await
    .map(|message| held::Posted::Sent(Box::new(message)))
}

/// What became of an edit: made, or waiting for its channel's mana.
pub enum Edited {
    Done(Box<MessageWithRelations>),
    Waiting(held::HeldMessage),
}

/// An edit waiting for its channel's mana, as it was asked for (`held_message.payload`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WaitingEdit {
    content: Option<String>,
}

impl From<WaitingEdit> for Edit {
    fn from(change: WaitingEdit) -> Self {
        Edit {
            content: change.content,
            retouch: None,
        }
    }
}

/// A message as it stands, with its attachments and link previews.
async fn current(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    id: MessageId,
) -> crate::Result<MessageWithRelations> {
    let message = message::table
        .select(Message::as_select())
        .filter(message::id.eq(id).and(message::deleted_at.is_null()))
        .first(conn)
        .await?;
    let attachments = message_attachment::table
        .select(message_attachment::attachment_id)
        .filter(message_attachment::message_id.eq(id))
        .load(conn)
        .await?;
    let link_previews = load_previews(conn, state.media_store.as_ref(), &[id])
        .await?
        .remove(&id)
        .unwrap_or_default();
    Ok(MessageWithRelations {
        message,
        attachments,
        link_previews,
    })
}

/// The channel a reply posted `to` would echo to: its thread's parent, or `None` outside a
/// thread.
pub(crate) async fn parent_of(
    conn: &mut AsyncPgConnection,
    to: To,
) -> crate::Result<Option<ChannelId>> {
    let (channel, unmade) = resolve(conn, to).await?;
    if let Some(unmade) = unmade {
        return Ok(Some(unmade.parent));
    }
    Ok(channel::table
        .select(channel::parent_channel)
        .filter(channel::id.eq(channel))
        .first::<Option<ChannelId>>(conn)
        .await?)
}

/// Deletes a message, which its author may do and anyone with Manage messages. A thread reply
/// leaves its thread's summary and takes its echo with it; a thread's starter leaves the thread
/// in place.
pub async fn delete_message(
    state: &GlobalServerContext,
    caller: UserId,
    id: MessageId,
) -> Result<(), crate::Error> {
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
            Some(author),
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
pub async fn soft_delete(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    id: MessageId,
) -> Result<(), crate::Error> {
    let Some(deleted) = diesel::update(message::table)
        .set(message::deleted_at.eq(diesel::dsl::now))
        .filter(message::id.eq(id).and(message::deleted_at.is_null()))
        .returning(Message::as_select())
        .load(conn)
        .await?
        .into_iter()
        .next()
    else {
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    };
    if deleted.kind != MessageKind::ThreadEcho {
        thread::delete_echo_of(state, conn, id).await?;
    }
    // Its files leave the public read path, kept for reviewing reports, and its link previews'
    // pictures are kept as long.
    crate::attachment::evidence::keep_deleted(conn, id).await?;
    crate::saved_message::forget(state, conn, id).await?;
    crate::link_preview::note_deleted(conn, &[id]).await?;
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
        crate::poll::delete_poll(state, conn, poll, *deleted.channel.id()).await?;
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

/// Deletes `ids` as [`soft_delete`] deletes each, as one set: one statement marks them deleted,
/// one their echoes, one keeps their files as evidence, their threads' summaries are taken down
/// once per thread, and every deletion is announced together. Messages already deleted are
/// passed over. Answers how many it deleted.
pub async fn soft_delete_many(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    ids: &[MessageId],
) -> Result<usize, crate::Error> {
    let deleted: Vec<Message> = diesel::update(message::table)
        .set(message::deleted_at.eq(diesel::dsl::now))
        .filter(message::id.eq_any(ids).and(message::deleted_at.is_null()))
        .returning(Message::as_select())
        .load(conn)
        .await?;
    if deleted.is_empty() {
        return Ok(0);
    }
    let gone: Vec<MessageId> = deleted.iter().map(|m| m.id).collect();
    let replies: Vec<MessageId> = deleted
        .iter()
        .filter(|m| m.kind != MessageKind::ThreadEcho)
        .map(|m| m.id)
        .collect();
    // Echoes go with their replies.
    let echoes: Vec<(MessageId, ChannelId)> = diesel::update(message::table)
        .set(message::deleted_at.eq(diesel::dsl::now))
        .filter(
            message::echo_of
                .eq_any(&replies)
                .and(message::deleted_at.is_null()),
        )
        .returning((message::id, message::channel))
        .load(conn)
        .await?;
    // Their files leave the public read path, kept for reviewing reports, and their link
    // previews' pictures are kept as long.
    crate::attachment::evidence::keep_deleted_many(conn, &gone).await?;
    crate::link_preview::note_deleted(conn, &gone).await?;
    let mut events: Vec<(EventScope, ServerEvent)> = deleted
        .iter()
        .map(|m| (*m.channel.id(), m.id))
        .chain(echoes.iter().map(|(id, channel)| (*channel, *id)))
        .map(|(channel, id)| {
            (
                EventScope::Channel(channel),
                ServerEvent::Message(MessageEvent::Delete { id }),
            )
        })
        .collect();
    crate::events::publish_events(state, conn, std::mem::take(&mut events)).await?;
    for message in &deleted {
        // An echo deleted alone leaves its reply free to be echoed again.
        if message.kind == MessageKind::ThreadEcho
            && let Some(reply) = message.echo_of
        {
            thread::forget_echo(state, conn, reply, message.id).await?;
        }
        // Deleting the message a poll is shown in ends the poll.
        if message.kind == MessageKind::Poll
            && let Some(poll) = message.poll
        {
            crate::poll::delete_poll(state, conn, poll, *message.channel.id()).await?;
        }
    }
    // Each thread's summary comes down once, by however many of its replies went.
    let channels: Vec<ChannelId> = deleted
        .iter()
        .map(|m| *m.channel.id())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let threads: std::collections::HashSet<ChannelId> = channel::table
        .select(channel::id)
        .filter(channel::id.eq_any(&channels))
        .filter(channel::ty.eq(ChannelType::Thread))
        .load::<ChannelId>(conn)
        .await?
        .into_iter()
        .collect();
    let mut removed: std::collections::BTreeMap<ChannelId, i32> = std::collections::BTreeMap::new();
    for message in &deleted {
        if threads.contains(message.channel.id()) {
            *removed.entry(*message.channel.id()).or_default() += 1;
        }
    }
    for (thread, count) in removed {
        thread::record_removals(state, conn, thread, count).await?;
    }
    Ok(deleted.len())
}

/// Starts deleting every message `author` posted since `since` in the channels `visible` covers
/// that its user may view, and their threads, as deleting each one by one would let them, or
/// with no `visible` anywhere on the deployment, DMs included: a ban with a deletion window
/// (`app::ban` and `app::user_ban`), which has checked who may. The deletion is a job
/// (`jobs::delete_messages`) saved in the caller's transaction, so it starts once the ban
/// commits and goes on a batch at a time, however many there are; where it may delete is fixed
/// now, as the banner may view now. Echoes go with their replies. Answers how many it will
/// delete, as they stand now.
pub async fn queue_deletion_of_recent(
    conn: &mut AsyncPgConnection,
    visible: Option<&Visibility>,
    author: UserId,
    since: DateTime<Utc>,
) -> Result<usize, crate::Error> {
    let places = visible.map(Visibility::visible_channels);
    let deletion = crate::jobs::delete_messages::Deletion {
        author,
        after: crate::read_state::position_at(since),
        before: MessageId::new(),
        places,
    };
    let count = crate::jobs::delete_messages::count(conn, &deletion).await?;
    if count > 0 {
        crate::jobs::enqueue(
            conn,
            crate::jobs::NewJob::new(
                crate::jobs::JobKind::DeleteMessagesBy,
                crate::jobs::JobClass::Normal,
                &deletion,
            )?,
        )
        .await?;
    }
    Ok(count)
}

/// Pins a message in its channel, after every pin already there, or unpins it. In a community
/// it takes Pin messages; in a DM any recipient may. Returns the pin as it stands when pinning,
/// and whether anything changed.
pub async fn set_pinned(
    state: &GlobalServerContext,
    caller: UserId,
    id: MessageId,
    pinned: bool,
) -> Result<(Option<crate::channel::Pin>, bool), crate::Error> {
    use aspen_schema::pin;
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
            let existing: Option<crate::channel::Pin> = pin::table
                .select(crate::channel::Pin::as_select())
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
            // The channel's row, locked above, keeps two pins at once from both passing.
            let pins: i64 = pin::table
                .filter(pin::channel.eq(channel_id))
                .count()
                .get_result(conn.as_mut())
                .await?;
            if pins >= MAX_PINS {
                return Err(crate::Error::Validation(t!("pinLimit", max = MAX_PINS)));
            }
            let last: Option<i32> = pin::table
                .select(diesel::dsl::max(pin::sort_index))
                .filter(pin::channel.eq(channel_id))
                .first(conn.as_mut())
                .await?;
            let row = crate::channel::Pin {
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
    access: &crate::permissions::ChannelAccess,
    action: ModerationAction,
    subject: Option<String>,
    target: Option<UserId>,
) -> crate::Result<()> {
    if let Some(target) = target {
        crate::deployment::require_outranks(conn, actor, target).await?;
    }
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
) -> Result<(), crate::Error> {
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
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            }
            // Kept for reviewing reports, off the public read path.
            crate::attachment::evidence::keep_removed(conn.as_mut(), id, &[attachment_id]).await?;
            if author != caller && access.moderating(Permissions::MANAGE_MESSAGES) {
                note_moderation(
                    conn.as_mut(),
                    caller,
                    &access,
                    ModerationAction::RemoveAttachment,
                    Some(format!("{}/{}", id.0, attachment_id.0)),
                    Some(author),
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

impl From<MessageWithRelations> for aspen_wire::message_enum::Message {
    fn from(m: MessageWithRelations) -> Self {
        record(&m.message, m.attachments, m.link_previews)
    }
}

#[cfg(test)]
mod nesting_tests {
    use super::*;

    #[test]
    fn deep_quotes_and_lists_are_refused() {
        assert!(check_content("> a quote\n\n- a\n  - list").is_ok());
        assert!(check_content(&format!("{} deep", ">".repeat(20))).is_ok());
        assert!(check_content(&format!("{} deep", ">".repeat(40))).is_err());
        assert!(check_content(&format!("{}deep", "1. ".repeat(40))).is_err());
        assert!(check_content(&format!("{}x", ">".repeat(MAX_CONTENT_CHARS - 1))).is_err());
    }
}
