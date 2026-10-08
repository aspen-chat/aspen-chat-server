//! Messages a user saved for themself, to come back to: a list of their own that no one else
//! sees, newest save first, of at most [`MAX_SAVED_MESSAGES`]. A save is kept on the deployment
//! that holds the message; apps signed in to several merge each one's list.
//!
//! What may be saved is what the user may read (`channel_access`), and reads list only the saves
//! of messages they may read now: one whose channel they lost stays saved, unlisted, until they
//! may read it again. Deleting the message deletes its saves. Every change is published to the
//! user's own subject as `savedMessageChanged`, so their other devices follow.

use crate::context::GlobalServerContext;
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

/// Deletes every save of `message_id`, which the caller's transaction is deleting, and tells each
/// saver: one who may no longer read its channel hears nothing else of the deletion.
pub async fn forget(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    message_id: MessageId,
) -> crate::Result<()> {
    let savers: Vec<UserId> =
        diesel::delete(saved_message::table.filter(saved_message::message.eq(message_id)))
            .returning(saved_message::user)
            .get_results(conn)
            .await?;
    for saver in savers {
        publish_event(
            state,
            conn,
            EventScope::User(saver),
            &ServerEvent::SavedMessageChanged {
                message: message_id,
                saved: None,
            },
        )
        .await?;
    }
    Ok(())
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
        .filter(
            channel::id
                .eq_any(readable.clone())
                .or(channel::parent_channel.eq_any(readable)),
        )
        .order_by(saved_message::id.desc())
        .load(conn.as_mut())
        .await?)
}

/// The messages the user saved and may read now, newest save first: `limit` of them (at most
/// [`MAX_PAGE`]), from the save after that of `before`, a message they saved, when it is given.
pub async fn read_saved_messages(
    state: &GlobalServerContext,
    user_id: UserId,
    before: Option<MessageId>,
    limit: u32,
) -> crate::Result<Vec<MessageWithRelations>> {
    let readable = crate::search::readable_everywhere(state, user_id).await?;
    let mut conn = state.connection_pool.get().await?;
    let mut query = saved_message::table
        .inner_join(message::table.on(message::id.eq(saved_message::message)))
        .inner_join(channel::table.on(channel::id.eq(message::channel)))
        .select(Message::as_select())
        .filter(saved_message::user.eq(user_id))
        .filter(message::deleted_at.is_null())
        .filter(channel::deleted_at.is_null())
        .filter(
            channel::id
                .eq_any(readable.clone())
                .or(channel::parent_channel.eq_any(readable)),
        )
        .into_boxed();
    if let Some(before) = before {
        // A page after a save since removed is empty: the reader's app heard of the removal
        // and pages on from a save it still holds.
        let Some(after) = saved_message::table
            .select(saved_message::id)
            .filter(saved_message::user.eq(user_id))
            .filter(saved_message::message.eq(before))
            .first::<SavedMessageId>(conn.as_mut())
            .await
            .optional()?
        else {
            return Ok(Vec::new());
        };
        query = query.filter(saved_message::id.lt(after));
    }
    let messages: Vec<Message> = query
        .order_by(saved_message::id.desc())
        .limit(i64::from(limit.clamp(1, MAX_PAGE)))
        .load(conn.as_mut())
        .await?;
    crate::message::with_relations(state, conn.as_mut(), messages).await
}
