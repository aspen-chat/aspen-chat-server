//! Threads: a channel of replies to one message of a text channel, DM, or group DM. A thread is
//! made in the transaction that posts its first reply (`app::message::To::ThreadOf`), so a
//! thread starts with a reply in it; `open_thread` also makes one with none, for clients that
//! open a thread before replying. It belongs wherever its parent does (the same community, or
//! the same DM's recipients), and cannot have threads of its own. The message it started names
//! it (`Message.thread`), and it names that message (`Channel.starterMessage`); both are written
//! in the transaction that makes it. Its `replyCount` and `lastReplyAt` are
//! kept exact under the thread row's lock as replies come and go.
//!
//! A reply may also be echoed to the parent channel, as it is posted or later by its author: a
//! message of kind `ThreadEcho` there that names the reply (`echoOf`) and has no content of its
//! own, so an edit to the reply shows in the echo and deleting the reply deletes its echo. The
//! reply names its live echo (`Message.echo`), which deleting the echo alone clears, after which
//! the reply may be echoed again.

use crate::channel::ChannelType;
use crate::channel::{Channel, record};
use crate::context::GlobalServerContext;
use crate::message::Message;
use crate::message::MessageKind;
use crate::permissions::{ChannelAccess, Permissions, channel_access};
use crate::t;
use crate::{ChannelId, EventScope, MaybeLoaded, MessageId, UserId, publish_event};
use aspen_schema::{channel, message};
use aspen_wire::message_enum::server_event::{ChannelEvent, MessageEvent, ServerEvent};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

/// The thread a message started, making it if this is its first opening; and whether this
/// call made it.
pub async fn open_thread(
    state: &GlobalServerContext,
    caller: UserId,
    starter: MessageId,
) -> crate::Result<(Channel, bool)> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move { open_in(state, conn.as_mut(), caller, starter, ChannelId::new()).await }
            .scope_boxed()
    })
    .await
}

/// Where a reply to the thread `starter` starts goes: that thread, or, while it has none, the
/// channel the thread will be made in.
pub enum ReplyPlace {
    Thread(ChannelId),
    Unmade { parent: ChannelId },
}

/// Where a reply to `starter`'s thread goes now. Read without a lock, so the thread may be made
/// before the reply is posted; [`open_in`] answers that.
pub async fn reply_place(
    conn: &mut AsyncPgConnection,
    starter: MessageId,
) -> crate::Result<ReplyPlace> {
    let (parent, thread): (ChannelId, Option<ChannelId>) = message::table
        .select((message::channel, message::thread))
        .filter(message::id.eq(starter).and(message::deleted_at.is_null()))
        .first(conn)
        .await?;
    Ok(match thread {
        Some(thread) => ReplyPlace::Thread(thread),
        None => ReplyPlace::Unmade { parent },
    })
}

/// Checks that `caller` may start a thread from `starter` in `parent`, as [`open_in`] will when
/// it makes it, answering their access to `parent`; for checking a reply that makes its thread
/// before the transaction that makes it opens.
pub(crate) async fn check_startable(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    caller: UserId,
    starter: MessageId,
    parent: ChannelId,
) -> crate::Result<ChannelAccess> {
    let kind: MessageKind = message::table
        .select(message::kind)
        .filter(
            message::id
                .eq(starter)
                .and(message::channel.eq(parent))
                .and(message::deleted_at.is_null()),
        )
        .first(conn)
        .await?;
    let access = channel_access(state, conn, caller, parent).await?;
    startable(conn, &access, kind, parent).await?;
    Ok(access)
}

/// Checks that a thread may be started from a message of `kind` in `parent_id` by the holder of
/// `access` to it, answering the parent.
async fn startable(
    conn: &mut AsyncPgConnection,
    access: &ChannelAccess,
    kind: MessageKind,
    parent_id: ChannelId,
) -> crate::Result<Channel> {
    if kind == MessageKind::ThreadEcho {
        return Err(crate::Error::Validation(t!("threadFromEcho")));
    }
    // Opening an existing thread is reading; making one takes Start threads.
    access.require(Permissions::START_THREADS)?;
    let parent: Channel = channel::table
        .select(Channel::as_select())
        .filter(channel::id.eq(parent_id).and(channel::deleted_at.is_null()))
        .first(conn)
        .await?;
    match parent.ty {
        ChannelType::Text | ChannelType::Dm | ChannelType::GroupDm => Ok(parent),
        ChannelType::Thread => Err(crate::Error::Validation(t!("threadInThread"))),
        ChannelType::Voice | ChannelType::Plugin => {
            Err(crate::Error::Validation(t!("threadNotHere")))
        }
    }
}

/// [`open_thread`] in the caller's transaction, making the thread as `id` when the message has
/// none; and whether this call made it. Posting the reply that makes a thread
/// (`app::message::To::ThreadOf`) makes it here, in the transaction that posts the reply.
pub(crate) async fn open_in(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    caller: UserId,
    starter: MessageId,
    id: ChannelId,
) -> crate::Result<(Channel, bool)> {
    // The starter is locked so two first openings make one thread.
    let (parent_id, kind, existing, starter_author): (
        ChannelId,
        MessageKind,
        Option<ChannelId>,
        UserId,
    ) = message::table
        .select((
            message::channel,
            message::kind,
            message::thread,
            message::author,
        ))
        .filter(message::id.eq(starter).and(message::deleted_at.is_null()))
        .for_update()
        .first(conn)
        .await?;
    let access = channel_access(state, conn, caller, parent_id).await?;
    if let Some(thread) = existing {
        let thread: Channel = channel::table
            .select(Channel::as_select())
            .filter(channel::id.eq(thread))
            .first(conn)
            .await?;
        return Ok((thread, false));
    }
    let parent = startable(conn, &access, kind, parent_id).await?;
    let thread = Channel {
        id,
        // In a community the thread records it, so routing and listings need not look at the
        // parent; in a DM there is none, and routing follows the parent.
        community: parent.community.clone(),
        parent_category: None,
        name: String::new(),
        ty: ChannelType::Thread,
        sort_index: 0,
        deleted_at: None,
        parent_channel: Some(parent_id),
        starter_message: Some(starter),
        reply_count: 0,
        last_reply_at: None,
        dm_key: None,
        plugin_type: None,
    };
    diesel::insert_into(channel::table)
        .values(&thread)
        .execute(conn)
        .await?;
    diesel::update(message::table)
        .set(message::thread.eq(Some(thread.id)))
        .filter(message::id.eq(starter))
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        EventScope::ChannelDefinition {
            channel: thread.id,
            departed: None,
        },
        &ServerEvent::Channel(ChannelEvent::Create(record(&thread, Vec::new()))),
    )
    .await?;
    publish_event(
        state,
        conn,
        EventScope::Message(starter),
        &ServerEvent::Message(MessageEvent::Update {
            id: starter,
            content: None,
            attachments: None,
            edited_at: None,
            link_previews: None,
            thread: Some(Some(thread.id)),
            mentions: None,
            linked_messages: None,
            altered_by: None,
            card: None,
            echo: None,
        }),
    )
    .await?;
    // Whoever wrote the message replies are to is told of them.
    crate::thread_follow::took_part(state, conn, &[starter_author], thread.id).await?;
    Ok((thread, true))
}

/// The live threads among `ids`, as wire records.
pub async fn read_threads(
    state: &GlobalServerContext,
    ids: &[ChannelId],
) -> crate::Result<Vec<aspen_wire::message_enum::Channel>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    Ok(channel::table
        .select(Channel::as_select())
        .filter(channel::id.eq_any(ids).and(channel::deleted_at.is_null()))
        .load::<Channel>(conn.as_mut())
        .await?
        .iter()
        .map(|thread| record(thread, Vec::new()))
        .collect())
}

/// Counts a new reply posted at `at` into its thread's summary and announces it.
pub async fn record_reply(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    thread: ChannelId,
    at: DateTime<Utc>,
) -> crate::Result<()> {
    let (count, last): (i32, Option<DateTime<Utc>>) = diesel::update(channel::table)
        .set((
            channel::reply_count.eq(channel::reply_count + 1),
            channel::last_reply_at.eq(Some(at)),
        ))
        .filter(channel::id.eq(thread))
        .returning((channel::reply_count, channel::last_reply_at))
        .get_result(conn)
        .await?;
    publish_summary(state, conn, thread, count, last).await
}

/// Counts a message posted at `at` toward its channel's thread summary, when the channel is a
/// thread; for messages made outside `app::message::create_message`, such as polls. Returns
/// whether the channel is a thread.
pub async fn record_if_reply(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    channel_id: ChannelId,
    at: DateTime<Utc>,
) -> crate::Result<bool> {
    let ty: ChannelType = channel::table
        .select(channel::ty)
        .filter(channel::id.eq(channel_id))
        .first(conn)
        .await?;
    if ty == ChannelType::Thread {
        record_reply(state, conn, channel_id, at).await?;
        return Ok(true);
    }
    Ok(false)
}

/// Takes `removed` deleted replies out of its thread's summary at once and announces it, as
/// [`record_removal`] does for one.
pub async fn record_removals(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    thread: ChannelId,
    removed: i32,
) -> crate::Result<()> {
    let last: Option<DateTime<Utc>> = message::table
        .select(message::timestamp)
        .filter(
            message::channel
                .eq(thread)
                .and(message::deleted_at.is_null()),
        )
        .order(message::id.desc())
        .first(conn)
        .await
        .optional()?;
    let (count, last): (i32, Option<DateTime<Utc>>) = diesel::update(channel::table)
        .set((
            channel::reply_count.eq(channel::reply_count - removed),
            channel::last_reply_at.eq(last),
        ))
        .filter(channel::id.eq(thread))
        .returning((channel::reply_count, channel::last_reply_at))
        .get_result(conn)
        .await?;
    publish_summary(state, conn, thread, count, last).await
}

/// Takes a deleted reply out of its thread's summary and announces it. The reply is already
/// marked deleted in the caller's transaction, so the latest remaining one sets `lastReplyAt`:
/// the newest by id (ids are UUIDv7), found walking `message (channel, id)` back from the end,
/// which stops at the first reply not deleted.
pub async fn record_removal(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    thread: ChannelId,
) -> crate::Result<()> {
    record_removals(state, conn, thread, 1).await
}

async fn publish_summary(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    thread: ChannelId,
    count: i32,
    last: Option<DateTime<Utc>>,
) -> crate::Result<()> {
    publish_event(
        state,
        conn,
        EventScope::ChannelDefinition {
            channel: thread,
            departed: None,
        },
        &ServerEvent::Channel(ChannelEvent::Update {
            id: thread,
            parent_category: None,
            community: None,
            name: None,
            sort_index: None,
            reply_count: Some(count),
            last_reply_at: Some(last),
            recipients: None,
        }),
    )
    .await
}

/// Shows a thread reply in the thread's parent channel as the echo `id` naming it, and announces
/// it there. The reply names `id` as its echo, written in the same transaction.
pub async fn echo(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    parent: ChannelId,
    reply: &Message,
    id: MessageId,
) -> crate::Result<Message> {
    let echo = Message {
        id,
        channel: MaybeLoaded::from_id(parent),
        content: String::new(),
        author: MaybeLoaded::from_id(*reply.author.id()),
        timestamp: Utc::now(),
        deleted_at: None,
        edited_at: None,
        kind: MessageKind::ThreadEcho,
        poll: None,
        thread: None,
        echo_of: Some(reply.id),
        // An echo shows its reply, whose tags count in the thread.
        mentions: crate::mention::Mentions::default(),
        call_seconds: None,
        command_bot: None,
        linked_messages: Default::default(),
        warning: None,
        altered_by: Vec::new(),
        card: None,
        echo: None,
    };
    diesel::insert_into(message::table)
        .values(&echo)
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        EventScope::Channel(parent),
        &ServerEvent::Message(MessageEvent::Create(crate::message::record(
            &echo,
            Vec::new(),
            Vec::new(),
        ))),
    )
    .await?;
    Ok(echo)
}

/// Echoes a thread reply that was posted without one to the thread's parent channel, which only
/// its author may do, as posting it with `echo_to_parent` would have: made now (`true`), or the
/// live echo it already has (`false`). The echo is new in the parent channel, so it sits there
/// at the time it was made, not the reply's.
pub async fn echo_reply(
    state: &GlobalServerContext,
    caller: UserId,
    reply: MessageId,
) -> crate::Result<(Message, bool)> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            // The reply is locked so two echoes of it make one.
            let reply: Message = message::table
                .select(Message::as_select())
                .filter(message::id.eq(reply).and(message::deleted_at.is_null()))
                .for_update()
                .first(conn.as_mut())
                .await?;
            let thread_id = *reply.channel.id();
            channel_access(state, conn.as_mut(), caller, thread_id).await?;
            // The echo speaks for its reply's author, so the choice to show it is theirs.
            if *reply.author.id() != caller {
                return Err(crate::Error::Forbidden(t!("echoOthersMessage")));
            }
            if let Some(existing) = reply.echo {
                let existing: Message = message::table
                    .select(Message::as_select())
                    .filter(message::id.eq(existing))
                    .first(conn.as_mut())
                    .await?;
                return Ok((existing, false));
            }
            let parent: Option<ChannelId> = channel::table
                .select(channel::parent_channel)
                .filter(channel::id.eq(thread_id))
                .first(conn.as_mut())
                .await?;
            let Some(parent) = parent else {
                return Err(crate::Error::Validation(t!("echoOutsideThread")));
            };
            // Only what its author wrote is echoed: polls and their results show themselves.
            if !matches!(reply.kind, MessageKind::Standard | MessageKind::Command) {
                return Err(crate::Error::Validation(t!("echoKind")));
            }
            // An echo is posted in the parent channel, so it takes sending there.
            channel_access(state, conn.as_mut(), caller, parent)
                .await?
                .require(Permissions::SEND_MESSAGES)?;
            let id = MessageId::new();
            diesel::update(message::table)
                .set(message::echo.eq(Some(id)))
                .filter(message::id.eq(reply.id))
                .execute(conn.as_mut())
                .await?;
            let made = echo(state, conn.as_mut(), parent, &reply, id).await?;
            publish_echo(state, conn.as_mut(), reply.id, Some(id)).await?;
            Ok((made, true))
        }
        .scope_boxed()
    })
    .await
}

/// Clears a reply's echo when the echo alone is deleted, inside the caller's transaction, and
/// announces it, so the reply may be echoed again.
pub async fn forget_echo(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    reply: MessageId,
    echo: MessageId,
) -> crate::Result<()> {
    let cleared = diesel::update(message::table)
        .set(message::echo.eq(None::<MessageId>))
        .filter(
            message::id
                .eq(reply)
                .and(message::echo.eq(echo))
                .and(message::deleted_at.is_null()),
        )
        .execute(conn)
        .await?;
    if cleared > 0 {
        publish_echo(state, conn, reply, None).await?;
    }
    Ok(())
}

/// Announces the reply's echo, or that it has none.
async fn publish_echo(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    reply: MessageId,
    echo: Option<MessageId>,
) -> crate::Result<()> {
    publish_event(
        state,
        conn,
        EventScope::Message(reply),
        &ServerEvent::Message(MessageEvent::Update {
            id: reply,
            content: None,
            attachments: None,
            edited_at: None,
            link_previews: None,
            thread: None,
            mentions: None,
            linked_messages: None,
            altered_by: None,
            card: None,
            echo: Some(echo),
        }),
    )
    .await
}

/// Deletes the echo of a deleted thread reply, if it has one, and announces it.
pub async fn delete_echo_of(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    reply: MessageId,
) -> crate::Result<()> {
    let echoes: Vec<(MessageId, ChannelId)> = diesel::update(message::table)
        .set(message::deleted_at.eq(diesel::dsl::now))
        .filter(
            message::echo_of
                .eq(reply)
                .and(message::deleted_at.is_null()),
        )
        .returning((message::id, message::channel))
        .load(conn)
        .await?;
    for (id, parent) in echoes {
        publish_event(
            state,
            conn,
            EventScope::Channel(parent),
            &ServerEvent::Message(MessageEvent::Delete { id }),
        )
        .await?;
    }
    Ok(())
}
