//! Messages a user saved for themself, to come back to: a list of their own that no one else
//! sees, newest save first, of at most [`MAX_SAVED_MESSAGES`]. A save is kept on the deployment
//! that holds the message; apps signed in to several merge each one's list.
//!
//! What may be saved is what the user may read (`channel_access`), and reads list only the saves
//! of messages they may read now: one whose channel they lost stays saved, unlisted, until they
//! may read it again. Deleting the message deletes its saves. Every change is published to the
//! user's own subject as `savedMessageChanged`, so their other devices follow.

use crate::context::GlobalServerContext;
use crate::cursor::{List, Paged};
use crate::message::{Message, MessageWithRelations};
use crate::t;
use crate::{EventScope, MessageId, SavedMessageId, UserId, publish_event};
use aspen_schema::{channel, message, saved_message, user};
use aspen_wire::message_enum::server_event::ServerEvent;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

/// The most messages one user may keep saved on one deployment.
pub const MAX_SAVED_MESSAGES: usize = 1000;
/// The most saved messages one page of them holds.
pub const MAX_PAGE: u32 = 100;

/// A message the user saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Queryable, Selectable)]
#[diesel(table_name = saved_message)]
pub struct SavedMessage {
    pub id: SavedMessageId,
    pub message: MessageId,
}

/// Saves `message_id` for the user. Returns the save and whether it was saved already.
pub async fn save(
    state: &GlobalServerContext,
    user_id: UserId,
    message_id: MessageId,
) -> crate::Result<(SavedMessage, bool)> {
    let mut conn = state.connection_pool.get().await?;
    let channel_id = message::table
        .inner_join(channel::table.on(channel::id.eq(message::channel)))
        .select(message::channel)
        .filter(message::id.eq(message_id))
        .filter(message::deleted_at.is_null())
        .filter(channel::deleted_at.is_null())
        .first(conn.as_mut())
        .await?;
    crate::permissions::channel_access(state, conn.as_mut(), user_id, channel_id).await?;
    conn.transaction(|conn| {
        async move {
            // One save at a time per user, so the limit holds however many arrive at once.
            user::table
                .select(user::id)
                .filter(user::id.eq(user_id))
                .for_update()
                .first::<UserId>(conn.as_mut())
                .await?;
            let existing: Option<SavedMessage> = saved_message::table
                .select(SavedMessage::as_select())
                .filter(saved_message::user.eq(user_id))
                .filter(saved_message::message.eq(message_id))
                .first(conn.as_mut())
                .await
                .optional()?;
            if let Some(existing) = existing {
                return Ok((existing, true));
            }
            let held: i64 = saved_message::table
                .filter(saved_message::user.eq(user_id))
                .count()
                .get_result(conn.as_mut())
                .await?;
            if held >= MAX_SAVED_MESSAGES as i64 {
                return Err(crate::Error::Validation(t!(
                    "savedMessagesFull",
                    max = MAX_SAVED_MESSAGES
                )));
            }
            let saved = SavedMessage {
                id: SavedMessageId::new(),
                message: message_id,
            };
            diesel::insert_into(saved_message::table)
                .values((
                    saved_message::id.eq(saved.id),
                    saved_message::user.eq(user_id),
                    saved_message::message.eq(message_id),
                ))
                .execute(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::User(user_id),
                &ServerEvent::SavedMessageChanged {
                    message: message_id,
                    saved: Some(saved.id),
                },
            )
            .await?;
            Ok::<_, crate::Error>((saved, false))
        }
        .scope_boxed()
    })
    .await
}

/// Stops saving `message_id` for the user, if they saved it.
pub async fn unsave(
    state: &GlobalServerContext,
    user_id: UserId,
    message_id: MessageId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let removed = diesel::delete(
                saved_message::table
                    .filter(saved_message::user.eq(user_id))
                    .filter(saved_message::message.eq(message_id)),
            )
            .execute(conn.as_mut())
            .await?;
            if removed > 0 {
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::User(user_id),
                    &ServerEvent::SavedMessageChanged {
                        message: message_id,
                        saved: None,
                    },
                )
                .await?;
            }
            Ok::<_, crate::Error>(())
        }
        .scope_boxed()
    })
    .await
}

/// Deletes every save of `messages`, which the caller's transaction is deleting, and tells each
/// saver: one who may no longer read its channel hears nothing else of the deletion. Every way
/// of deleting messages comes through here (`message::soft_delete`, `soft_delete_many`).
pub async fn forget(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    messages: &[MessageId],
) -> crate::Result<()> {
    let saves: Vec<(UserId, MessageId)> =
        diesel::delete(saved_message::table.filter(saved_message::message.eq_any(messages)))
            .returning((saved_message::user, saved_message::message))
            .get_results(conn)
            .await?;
    let events = saves
        .into_iter()
        .map(|(saver, message)| {
            (
                EventScope::User(saver),
                ServerEvent::SavedMessageChanged {
                    message,
                    saved: None,
                },
            )
        })
        .collect();
    crate::events::publish_events(state, conn, events).await
}

/// The channel whose reading decides who reads a saved message's: its own, or a thread's
/// parent. `readable_everywhere` lists only channels and DMs, never threads, so one check of
/// this against it, which PostgreSQL hashes, stands for both.
fn home_channel() -> diesel::expression::SqlLiteral<diesel::sql_types::Uuid> {
    diesel::dsl::sql("COALESCE(channel.parent_channel, channel.id)")
}

/// Every save of the user's that they may read now, newest first.
pub async fn read_saves(
    state: &GlobalServerContext,
    user_id: UserId,
) -> crate::Result<Vec<SavedMessage>> {
    let readable = crate::search::readable_everywhere(state, user_id).await?;
    let mut conn = state.connection_pool.get().await?;
    Ok(saved_message::table
        .inner_join(message::table.on(message::id.eq(saved_message::message)))
        .inner_join(channel::table.on(channel::id.eq(message::channel)))
        .select(SavedMessage::as_select())
        .filter(saved_message::user.eq(user_id))
        .filter(message::deleted_at.is_null())
        .filter(channel::deleted_at.is_null())
        .filter(home_channel().eq_any(readable))
        .order_by(saved_message::id.desc())
        .load(conn.as_mut())
        .await?)
}

/// The place a saved message list's cursor holds: the id of the last save of the page before,
/// which is all the list is ordered by.
fn saved_position(cursor: &str) -> crate::Result<SavedMessageId> {
    crate::cursor::decode(List::SavedMessages, cursor)
}

/// The messages the user saved and may read now, newest save first: at most `limit` of them
/// (up to [`MAX_PAGE`]), after the place `cursor` holds when given, which is the `next_cursor`
/// of the page before.
pub async fn read_saved_messages(
    state: &GlobalServerContext,
    user_id: UserId,
    cursor: Option<&str>,
    limit: u32,
) -> crate::Result<Paged<MessageWithRelations>> {
    let after = cursor.map(saved_position).transpose()?;
    let readable = crate::search::readable_everywhere(state, user_id).await?;
    let mut conn = state.connection_pool.get().await?;
    let page = saved_page(conn.as_mut(), user_id, &readable, after, limit).await?;
    Ok(Paged {
        items: crate::message::with_relations(state, conn.as_mut(), page.items).await?,
        next_cursor: page.next_cursor,
    })
}

/// One page of the messages `user_id` saved, of those in the channels `readable`, and the
/// cursor of the next, by the saves' ids descending through `saved_message_by_user`, after
/// `after`. The place is compared as an id alone, so the page is read whether or not that save
/// still stands. What its reader may not read now is passed over before the limit.
async fn saved_page(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    readable: &[crate::ChannelId],
    after: Option<SavedMessageId>,
    limit: u32,
) -> crate::Result<Paged<Message>> {
    let limit = limit.clamp(1, MAX_PAGE);
    let mut query = saved_message::table
        .inner_join(message::table.on(message::id.eq(saved_message::message)))
        .inner_join(channel::table.on(channel::id.eq(message::channel)))
        .select((saved_message::id, Message::as_select()))
        .filter(saved_message::user.eq(user_id))
        .filter(message::deleted_at.is_null())
        .filter(channel::deleted_at.is_null())
        .filter(home_channel().eq_any(readable))
        .into_boxed();
    if let Some(after) = after {
        query = query.filter(saved_message::id.lt(after));
    }
    let rows: Vec<(SavedMessageId, Message)> = query
        .order_by(saved_message::id.desc())
        .limit(i64::from(limit) + 1)
        .load(conn)
        .await?;
    let page = crate::cursor::cut(List::SavedMessages, rows, limit as usize, |(save, _)| *save);
    Ok(Paged {
        items: page.items.into_iter().map(|(_, message)| message).collect(),
        next_cursor: page.next_cursor,
    })
}

#[cfg(test)]
mod database_tests {
    use super::*;
    use diesel_async::SimpleAsyncConnection;

    const READER: &str = "5a7e7000-0000-4000-8000-000000000001";
    const CHANNEL: &str = "5a7e7000-0000-4000-8000-000000000002";
    const HIDDEN: &str = "5a7e7000-0000-4000-8000-000000000003";

    fn message(n: u8) -> MessageId {
        MessageId(
            format!("5a7e7000-0000-4000-8000-0000000001{n:02x}")
                .parse()
                .unwrap(),
        )
    }

    /// A connection to the database `DATABASE_URL` names, inside a transaction that is never
    /// committed, with someone who saved `saves` messages, the n-th before the one before it,
    /// the fifth in a channel they may not read; `None` without one, when the test checks
    /// nothing.
    async fn reader_with(saves: u8) -> Option<AsyncPgConnection> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return None;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(&format!(
            r#"INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at)
                   VALUES ('{READER}', '{READER}', '', now(), now());
               INSERT INTO channel (id, name, ty, sort_index)
                   VALUES ('{CHANNEL}', '', 'dm', 0), ('{HIDDEN}', '', 'dm', 0);"#
        ))
        .await
        .unwrap();
        for n in 1..=saves {
            let id = message(n).0;
            let channel = if n == 5 { HIDDEN } else { CHANNEL };
            // Save ids fall as n rises, so the first is the newest save.
            let save = format!("5a7e7000-0000-4000-8000-0000000002{:02x}", u8::MAX - n);
            conn.batch_execute(&format!(
                r#"INSERT INTO message (id, channel, author, content, timestamp)
                     VALUES ('{id}', '{channel}', '{READER}', 'm', now());
                 INSERT INTO saved_message (id, "user", message)
                     VALUES ('{save}', '{READER}', '{id}');"#,
            ))
            .await
            .unwrap();
        }
        Some(conn)
    }

    /// Which messages one page holds, by their numbers, and the next page's cursor.
    async fn page(
        conn: &mut AsyncPgConnection,
        cursor: Option<&str>,
        limit: u32,
    ) -> (Vec<u8>, Option<String>) {
        let page = saved_page(
            conn,
            UserId(READER.parse().unwrap()),
            &[crate::ChannelId(CHANNEL.parse().unwrap())],
            cursor.map(|c| saved_position(c).unwrap()),
            limit,
        )
        .await
        .unwrap();
        let messages = page
            .items
            .into_iter()
            .map(|m| (1..=u8::MAX).find(|n| message(*n) == m.id).unwrap())
            .collect();
        (messages, page.next_cursor)
    }

    /// The saved list's cursor is a place, not a save: the pages go on after a save removed
    /// since, past a message its reader may not read, and the last page says it is the last
    /// however many it holds.
    #[tokio::test]
    async fn the_saves_page_past_a_save_that_is_gone() {
        let Some(mut conn) = reader_with(7).await else {
            return;
        };
        let (messages, cursor) = page(&mut conn, None, 3).await;
        assert_eq!(messages, [1, 2, 3]);
        // The third, which the cursor was made from, is no longer saved.
        conn.batch_execute(&format!(
            "DELETE FROM saved_message WHERE message = '{}'",
            message(3).0
        ))
        .await
        .unwrap();
        let (messages, cursor) = page(&mut conn, cursor.as_deref(), 2).await;
        assert_eq!(messages, [4, 6]);
        assert!(cursor.is_some());
        // The sixth, the boundary again, is deleted.
        conn.batch_execute(&format!(
            "UPDATE message SET deleted_at = now() WHERE id = '{}'",
            message(6).0
        ))
        .await
        .unwrap();
        let (messages, cursor) = page(&mut conn, cursor.as_deref(), 1).await;
        assert_eq!(messages, [7]);
        assert_eq!(cursor, None);
        // A page that ends on the last save has none after it, though it is full.
        let (messages, cursor) = page(&mut conn, None, 4).await;
        assert_eq!(messages, [1, 2, 4, 7]);
        assert_eq!(cursor, None);
    }

    #[test]
    fn a_cursor_of_no_saved_list_is_refused() {
        let refused =
            |cursor: &str| matches!(saved_position(cursor), Err(crate::Error::Validation(_)));
        assert!(refused("garbage"));
        // A saved message's id as it is, which is not a cursor.
        assert!(refused(&message(1).0.to_string()));
        // A place of another shape, and a held message list's cursor, whose place is an id
        // alone too.
        assert!(refused(&crate::cursor::encode(
            List::SavedMessages,
            &(1_i64, message(1))
        )));
        assert!(refused(&crate::cursor::encode(
            List::HeldMessages,
            &message(1)
        )));
        assert!(!refused(&crate::cursor::encode(
            List::SavedMessages,
            &message(1)
        )));
    }
}
