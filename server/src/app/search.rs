//! Message search: the messages a user may read that match words, who wrote them, whom they tag,
//! and what they hold, newest first.
//!
//! Words are matched by PostgreSQL full-text search in the `simple` configuration, which
//! lowercases every word and does no more, so every language is searched alike; the query takes
//! the syntax web search engines use (`websearch_to_tsquery`): words all of which must appear,
//! `"a phrase"`, `or`, and `-word` to leave one out. The index `message_search` is on the very
//! expression written here.
//!
//! What may be searched is what the user may read: the channels they may view in the
//! communities they belong to, the DMs they are in, and the threads of both. Messages by anyone
//! they blocked are left out, as they are of their unread counts.

use crate::app::context::GlobalServerContext;
use crate::app::message::MessageKind;
use crate::app::message::{Message, MessageWithRelations};
use crate::app::visibility::Visibility;
use crate::app::{self, ChannelId, CommunityId, MessageId, UserId};
use crate::database::schema::{
    attachment, channel, community_user, dm_recipient, mention, message, message_attachment,
    user_block,
};
use crate::t;
use diesel::dsl::{exists, not, sql};
use diesel::prelude::*;
use diesel::sql_types::{Bool, Text};
use diesel_async::RunQueryDsl;

/// The most messages one search returns.
pub const MAX_RESULTS: u32 = 50;
/// The longest search text accepted, in characters.
pub const MAX_TEXT_CHARS: usize = 200;

/// Something a message may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holding {
    /// Any attachment.
    Attachment,
    /// An attachment that is a picture.
    Image,
    /// A poll.
    Poll,
}

/// Where to search.
#[derive(Debug, Clone, Copy)]
pub enum SearchScope {
    /// Everything the user may read.
    Everywhere,
    /// One community the user belongs to.
    Community(CommunityId),
    /// One channel or DM the user may read, with its threads.
    Channel(ChannelId),
}

/// What to search for. At least one of `text`, `author`, `mentions`, and `holding` narrows it.
#[derive(Debug, Clone)]
pub struct MessageSearch {
    pub text: Option<String>,
    pub scope: SearchScope,
    pub author: Option<UserId>,
    /// Messages tagging this user by name.
    pub mentions: Option<UserId>,
    /// Messages holding every one of these.
    pub holding: Vec<Holding>,
    /// Only messages older than this one.
    pub before: Option<MessageId>,
    pub limit: u32,
}

/// The messages matching `search` that `caller` may read, newest first.
pub async fn search_messages(
    state: &GlobalServerContext,
    caller: UserId,
    search: MessageSearch,
) -> app::Result<Vec<MessageWithRelations>> {
    let text = search
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if text.is_none()
        && search.author.is_none()
        && search.mentions.is_none()
        && search.holding.is_empty()
    {
        return Err(app::Error::Validation(t!("searchEmpty")));
    }
    if text.is_some_and(|text| text.chars().count() > MAX_TEXT_CHARS) {
        return Err(app::Error::Validation(t!(
            "searchTooLong",
            max = MAX_TEXT_CHARS
        )));
    }
    let scope = readable_channels(state, caller, search.scope).await?;
    let mut conn = state.connection_pool.get().await?;
    if scope.is_empty() {
        return Ok(Vec::new());
    }
    let mut query = message::table
        .inner_join(channel::table.on(channel::id.eq(message::channel)))
        .select(Message::as_select())
        .filter(message::deleted_at.is_null())
        .filter(channel::deleted_at.is_null())
        .filter(
            message::channel
                .eq_any(scope.clone())
                .or(channel::parent_channel.eq_any(scope)),
        )
        // Echoes and poll announcements say nothing of their own; the reply and the poll are
        // found where they were posted.
        .filter(message::kind.ne_all([
            MessageKind::ThreadEcho,
            MessageKind::PollClosed,
            MessageKind::Call,
            MessageKind::MissedCall,
        ]))
        .filter(not(exists(
            user_block::table.filter(
                user_block::blocker
                    .eq(caller)
                    .and(user_block::blocked.eq(message::author)),
            ),
        )))
        .into_boxed();
    if let Some(text) = text {
        query = query.filter(
            sql::<Bool>(
                "to_tsvector('simple', message.content) @@ websearch_to_tsquery('simple', ",
            )
            .bind::<Text, _>(text.to_owned())
            .sql(")"),
        );
    }
    if let Some(author) = search.author {
        query = query.filter(message::author.eq(author));
    }
    if let Some(tagged) = search.mentions {
        query = query.filter(exists(
            mention::table.filter(
                mention::message
                    .eq(message::id)
                    .and(mention::target_user.eq(tagged)),
            ),
        ));
    }
    for holding in &search.holding {
        query = match holding {
            Holding::Attachment => query.filter(exists(
                message_attachment::table.filter(message_attachment::message_id.eq(message::id)),
            )),
            Holding::Image => query.filter(exists(
                message_attachment::table
                    .inner_join(
                        attachment::table.on(attachment::id.eq(message_attachment::attachment_id)),
                    )
                    .filter(message_attachment::message_id.eq(message::id))
                    .filter(attachment::mime_type.like("image/%")),
            )),
            Holding::Poll => query.filter(message::kind.eq(MessageKind::Poll)),
        };
    }
    if let Some(before) = search.before {
        query = query.filter(message::id.lt(before));
    }
    let messages: Vec<Message> = query
        .order_by(message::id.desc())
        .limit(i64::from(search.limit.clamp(1, MAX_RESULTS)))
        .load(conn.as_mut())
        .await?;
    app::channel::with_relations(state, conn.as_mut(), messages).await
}

/// The channels and DMs `scope` covers that `caller` may read. A community they do not belong
/// to, or a channel they may not read, is answered as if it did not exist.
async fn readable_channels(
    state: &GlobalServerContext,
    caller: UserId,
    scope: SearchScope,
) -> app::Result<Vec<ChannelId>> {
    match scope {
        SearchScope::Channel(channel) => {
            let mut conn = state.connection_pool.get().await?;
            let access =
                app::permissions::channel_access(state, conn.as_mut(), caller, channel).await?;
            // Searching is reading; a deployment moderator reads DMs they are not in only by
            // opening them, where the reading is logged.
            if access.dm_moderator {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            Ok(vec![channel])
        }
        SearchScope::Community(community) => {
            let mut conn = state.connection_pool.get().await?;
            let member: bool = diesel::select(exists(
                community_user::table.filter(
                    community_user::community
                        .eq(community)
                        .and(community_user::user.eq(caller)),
                ),
            ))
            .get_result(conn.as_mut())
            .await?;
            if !member {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            drop(conn);
            Ok(Visibility::load(state, caller, &[community])
                .await?
                .visible_channels())
        }
        SearchScope::Everywhere => {
            let mut conn = state.connection_pool.get().await?;
            let communities: Vec<CommunityId> = community_user::table
                .select(community_user::community)
                .filter(community_user::user.eq(caller))
                .load(conn.as_mut())
                .await?;
            let dms: Vec<ChannelId> = dm_recipient::table
                .select(dm_recipient::channel)
                .filter(dm_recipient::user.eq(caller))
                .load(conn.as_mut())
                .await?;
            drop(conn);
            let mut channels = Visibility::load(state, caller, &communities)
                .await?
                .visible_channels();
            channels.extend(dms);
            Ok(channels)
        }
    }
}
