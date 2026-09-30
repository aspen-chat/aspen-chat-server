//! Threads: a channel of replies to one message of a text channel, DM, or group DM. A thread is
//! made when it is first opened (`open_thread`), belongs wherever its parent does (the same
//! community, or the same DM's recipients), and cannot have threads of its own. The message it
//! started names it (`Message.thread`), and it names that message (`Channel.starterMessage`);
//! both are written in the transaction that makes it. Its `replyCount` and `lastReplyAt` are
//! kept exact under the thread row's lock as replies come and go.
//!
//! A reply may also be echoed to the parent channel: a message of kind `ThreadEcho` there that
//! names the reply (`echoOf`) and has no content of its own, so an edit to the reply shows in
//! the echo and deleting the reply deletes its echo.

use crate::api::message_enum::server_event::{ChannelEvent, MessageEvent, ServerEvent};
use crate::api::{ChannelType, MessageKind};
use crate::app::channel::{Channel, record};
use crate::app::message::Message;
use crate::app::{
    self, ChannelId, EventScope, GlobalServerContext, MaybeLoaded, MessageId, UserId, publish_event,
};
use crate::database::schema::{channel, message};
use crate::t;
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
) -> app::Result<(Channel, bool)> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            // The starter is locked so two first openings make one thread.
            let (parent_id, kind, existing): (ChannelId, MessageKind, Option<ChannelId>) =
                message::table
                    .select((message::channel, message::kind, message::thread))
                    .filter(message::id.eq(starter).and(message::deleted_at.is_null()))
                    .for_update()
                    .first(conn.as_mut())
                    .await?;
            let access =
                crate::app::permissions::channel_access(state, conn.as_mut(), caller, parent_id)
                    .await?;
            if let Some(thread) = existing {
                let thread: Channel = channel::table
                    .select(Channel::as_select())
                    .filter(channel::id.eq(thread))
                    .first(conn.as_mut())
                    .await?;
                return Ok((thread, false));
            }
            if kind == MessageKind::ThreadEcho {
                return Err(app::Error::Validation(t!("threadFromEcho")));
            }
            // Opening an existing thread is reading; making one takes Start threads.
            access.require(crate::app::permissions::Permissions::START_THREADS)?;
            let parent: Channel = channel::table
                .select(Channel::as_select())
                .filter(channel::id.eq(parent_id).and(channel::deleted_at.is_null()))
                .first(conn.as_mut())
                .await?;
            match parent.ty {
                ChannelType::Text | ChannelType::Dm | ChannelType::GroupDm => {}
                ChannelType::Thread => {
                    return Err(app::Error::Validation(t!("threadInThread")));
                }
                ChannelType::Voice => {
                    return Err(app::Error::Validation(t!("threadNotHere")));
                }
            }
            let thread = Channel {
                id: ChannelId::new(),
                // In a community the thread records it, so routing and listings need not look
                // at the parent; in a DM there is none, and routing follows the parent.
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
            };
            diesel::insert_into(channel::table)
                .values(&thread)
                .execute(conn.as_mut())
                .await?;
            diesel::update(message::table)
                .set(message::thread.eq(Some(thread.id)))
                .filter(message::id.eq(starter))
                .execute(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::ChannelDefinition {
                    channel: thread.id,
                    departed: None,
                },
                &ServerEvent::Channel(ChannelEvent::Create(record(&thread, Vec::new()))),
            )
            .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Message(starter),
                &ServerEvent::Message(MessageEvent::Update {
                    id: starter,
                    content: None,
                    attachments: None,
                    edited_at: None,
                    link_previews: None,
                    thread: Some(Some(thread.id)),
                    mentions: None,
                }),
            )
            .await?;
            Ok((thread, true))
        }
        .scope_boxed()
    })
    .await
}

/// The live threads among `ids`, as wire records.
pub async fn read_threads(
    state: &GlobalServerContext,
    ids: &[ChannelId],
) -> app::Result<Vec<crate::api::message_enum::Channel>> {
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
) -> app::Result<()> {
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
) -> app::Result<bool> {
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

/// Takes a deleted reply out of its thread's summary and announces it. The reply is already
/// marked deleted in the caller's transaction, so the latest remaining one sets `lastReplyAt`.
pub async fn record_removal(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    thread: ChannelId,
) -> app::Result<()> {
    let last: Option<DateTime<Utc>> = message::table
        .select(diesel::dsl::max(message::timestamp))
        .filter(
            message::channel
                .eq(thread)
                .and(message::deleted_at.is_null()),
        )
        .first(conn)
        .await?;
    let (count, last): (i32, Option<DateTime<Utc>>) = diesel::update(channel::table)
        .set((
            channel::reply_count.eq(channel::reply_count - 1),
            channel::last_reply_at.eq(last),
        ))
        .filter(channel::id.eq(thread))
        .returning((channel::reply_count, channel::last_reply_at))
        .get_result(conn)
        .await?;
    publish_summary(state, conn, thread, count, last).await
}

async fn publish_summary(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    thread: ChannelId,
    count: i32,
    last: Option<DateTime<Utc>>,
) -> app::Result<()> {
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

/// Shows a thread reply in the thread's parent channel as an echo naming it, and announces it
/// there.
pub async fn echo(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    parent: ChannelId,
    reply: &Message,
) -> app::Result<Message> {
    let echo = Message {
        id: MessageId::new(),
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
        mentions: crate::app::mention::Mentions::default(),
        call_seconds: None,
    };
    diesel::insert_into(message::table)
        .values(&echo)
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        EventScope::Channel(parent),
        &ServerEvent::Message(MessageEvent::Create(app::message::record(
            &echo,
            Vec::new(),
            Vec::new(),
        ))),
    )
    .await?;
    Ok(echo)
}

/// Deletes the echo of a deleted thread reply, if it has one, and announces it.
pub async fn delete_echo_of(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    reply: MessageId,
) -> app::Result<()> {
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
