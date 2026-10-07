use crate::context::GlobalServerContext;
use crate::t;
use crate::{CustomEmojiId, EventScope, MessageId, UserId, publish_event};
use aspen_schema::react;
use aspen_wire::message_enum;
use aspen_wire::message_enum::server_event::{ReactEvent, ServerEvent};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, Insertable, JoinOnDsl, QueryDsl, Queryable,
    QueryableByName, Selectable,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = react)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct React {
    pub emoji: String,
    pub author: UserId,
    /// The custom emoji, when `emoji` is one's reference (`app::custom_emoji::reference`).
    pub custom_emoji: Option<CustomEmojiId>,
    pub message: MessageId,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// The one form `s` is stored and compared in, when it is a single emoji: the fully qualified
/// sequence Unicode lists for it, so an emoji typed with or without its variation selector
/// (`❤` and `❤️`) is one reaction, not two.
pub fn canonical_emoji(s: &str) -> Option<&'static str> {
    emojis::get(s).map(|emoji| emoji.as_str())
}

/// `s` in its canonical form, or a validation error when it is not a single emoji.
pub fn validate_emoji(s: &str) -> Result<&'static str, crate::Error> {
    canonical_emoji(s).ok_or_else(|| crate::Error::Validation(t!("reactMustBeSingleEmoji")))
}

/// What a reaction is stored and compared as: a single emoji in its canonical form, or a
/// custom emoji's reference as `app::custom_emoji::reference` writes it; `None` for anything
/// else, which is never stored.
pub fn stored_key(s: &str) -> Option<String> {
    canonical_emoji(s)
        .map(str::to_string)
        .or_else(|| crate::custom_emoji::referenced(s).map(crate::custom_emoji::reference))
}

pub async fn create_react(
    state: &GlobalServerContext,
    author: UserId,
    message_id: MessageId,
    emoji: String,
) -> crate::error::Result<React> {
    let custom = crate::custom_emoji::referenced(&emoji);
    let emoji = match custom {
        Some(id) => crate::custom_emoji::reference(id),
        None => validate_emoji(&emoji)?.to_string(),
    };
    let mut conn = state.connection_pool.get().await?;
    let (channel, community): (crate::ChannelId, Option<crate::CommunityId>) =
        aspen_schema::message::table
            .inner_join(
                aspen_schema::channel::table
                    .on(aspen_schema::channel::id.eq(aspen_schema::message::channel)),
            )
            .select((
                aspen_schema::message::channel,
                aspen_schema::channel::community,
            ))
            .filter(aspen_schema::message::id.eq(message_id))
            .first(conn.as_mut())
            .await?;
    crate::permissions::channel_access(state, conn.as_mut(), author, channel)
        .await?
        .require(crate::permissions::Permissions::ADD_REACTIONS)?;
    // A custom emoji reacts only in the community that defines it; a DM has none.
    if let Some(id) = custom {
        let Some(community) = community else {
            return Err(crate::Error::Validation(t!("customEmojiNotHere")));
        };
        if crate::custom_emoji::resolve_in_community(conn.as_mut(), community, &emoji)
            .await?
            .is_none()
        {
            return Err(crate::Error::Validation(t!("customEmojiUnknown")));
        }
        debug_assert_eq!(crate::custom_emoji::referenced(&emoji), Some(id));
    }
    let react = React {
        emoji: emoji.clone(),
        author,
        custom_emoji: custom,
        message: message_id,
        timestamp: chrono::Utc::now(),
    };
    let event = ServerEvent::React(ReactEvent::Create(message_enum::React {
        message_id,
        emoji,
        user_id: author,
    }));
    // The row commits only once its event is acknowledged, so the stream and the database
    // agree on what happened, in the same order. A reaction already there fails the insert,
    // which the caller reads as "already reacted".
    conn.transaction(|conn| {
        let react = &react;
        let event = &event;
        async move {
            diesel::insert_into(react::table)
                .values(react)
                .execute(conn.as_mut())
                .await?;
            publish_event(state, conn.as_mut(), EventScope::Message(message_id), event).await
        }
        .scope_boxed()
    })
    .await?;
    Ok(react)
}

/// Takes someone else's reaction off a message, which takes Manage messages where the message
/// is (or moderating the deployment, which is logged).
pub async fn remove_others_react(
    state: &GlobalServerContext,
    caller: UserId,
    message_id: MessageId,
    emoji: String,
    author: UserId,
) -> crate::error::Result<()> {
    if author != caller {
        let mut conn = state.connection_pool.get().await?;
        let channel: crate::ChannelId = aspen_schema::message::table
            .select(aspen_schema::message::channel)
            .filter(aspen_schema::message::id.eq(message_id))
            .first(conn.as_mut())
            .await?;
        let access =
            crate::permissions::channel_access_moderating(state, conn.as_mut(), caller, channel)
                .await?;
        let manage = crate::permissions::Permissions::MANAGE_MESSAGES;
        if !access.community_has(manage) {
            return Err(crate::permissions::missing(manage));
        }
        if access.moderating(manage) {
            crate::message::note_moderation(
                conn.as_mut(),
                caller,
                &access,
                crate::moderation_log::ModerationAction::RemoveReaction,
                Some(format!("{}/{emoji}/{}", message_id.0, author.0)),
            )
            .await?;
        }
    }
    delete_react(state, author, message_id, emoji).await
}

/// Takes `author`'s reaction off a message.
pub async fn delete_react(
    state: &GlobalServerContext,
    author: UserId,
    message_id: MessageId,
    emoji: String,
) -> crate::error::Result<()> {
    // Anything that is not an emoji or a custom emoji's reference was never stored, so there
    // is nothing to remove.
    let Some(emoji) = stored_key(&emoji) else {
        return Ok(());
    };
    let mut conn = state.connection_pool.get().await?;
    // As with adding: the removal commits only once its event is acknowledged.
    conn.transaction(|conn| {
        async move {
            let deleted = diesel::delete(react::table)
                .filter(
                    react::author
                        .eq(author)
                        .and(react::message.eq(message_id))
                        .and(react::emoji.eq(&emoji)),
                )
                .execute(conn.as_mut())
                .await?;
            if deleted > 0 {
                let event = ServerEvent::React(ReactEvent::Delete {
                    message_id,
                    emoji,
                    user_id: author,
                });
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::Message(message_id),
                    &event,
                )
                .await?;
            }
            Ok::<_, crate::Error>(())
        }
        .scope_boxed()
    })
    .await
}

/// How many of the people who reacted with an emoji a message read names: the earliest.
pub const SUMMARY_USERS: i32 = 4;
/// The most people one page of a reaction list holds.
pub const MAX_REACTORS_PAGE: u32 = 100;

/// One emoji's reactions to a message, in brief: how many, whether `caller` is among them, and
/// the first `SUMMARY_USERS` to react.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionSummary {
    pub message: MessageId,
    pub emoji: String,
    pub count: u32,
    pub me: bool,
    pub first: Vec<UserId>,
}

#[derive(QueryableByName)]
struct SummaryRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    message: MessageId,
    #[diesel(sql_type = diesel::sql_types::Text)]
    emoji: String,
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    count: i64,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    me: bool,
    #[diesel(sql_type = diesel::sql_types::Array<diesel::sql_types::Uuid>)]
    first: Vec<uuid::Uuid>,
}

/// The reactions to each of `messages`, one summary per emoji. A message's emoji come in the
/// order each was first used on it, so a client can break ties in count the same way. The
/// reactions of anyone `caller` has blocked are left out (`app::block`).
pub async fn read_summaries(
    state: &GlobalServerContext,
    caller: UserId,
    messages: &[MessageId],
) -> crate::Result<Vec<ReactionSummary>> {
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<SummaryRow> = diesel::sql_query(
        r#"
        SELECT message, emoji, count(*) AS count, bool_or(author = $1) AS me,
               (array_agg(author ORDER BY "timestamp", author))[1:$3] AS first
        FROM react
        WHERE message = ANY($2)
          AND NOT EXISTS (
              SELECT 1 FROM user_block WHERE blocker = $1 AND blocked = react.author
          )
        GROUP BY message, emoji
        ORDER BY message, min("timestamp"), emoji
        "#,
    )
    .bind::<diesel::sql_types::Uuid, _>(caller.0)
    .bind::<diesel::sql_types::Array<diesel::sql_types::Uuid>, _>(
        messages.iter().map(|m| m.0).collect::<Vec<_>>(),
    )
    .bind::<diesel::sql_types::Integer, _>(SUMMARY_USERS)
    .load(conn.as_mut())
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| ReactionSummary {
            message: row.message,
            emoji: row.emoji,
            count: u32::try_from(row.count).unwrap_or(u32::MAX),
            me: row.me,
            first: row.first.into_iter().map(UserId).collect(),
        })
        .collect())
}

#[derive(QueryableByName)]
struct ReactorRow {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    author: UserId,
}

/// Who reacted to `message_id` with `emoji`, earliest first: at most `limit` of them, starting
/// after `after` when given, leaving out anyone `caller` has blocked. Not found for a message
/// `caller` may not see.
pub async fn read_reactors(
    state: &GlobalServerContext,
    caller: UserId,
    message_id: MessageId,
    emoji: &str,
    after: Option<UserId>,
    limit: u32,
) -> crate::Result<Vec<UserId>> {
    let mut conn = state.connection_pool.get().await?;
    let channel: crate::ChannelId = aspen_schema::message::table
        .select(aspen_schema::message::channel)
        .filter(aspen_schema::message::id.eq(message_id))
        .first(conn.as_mut())
        .await?;
    crate::permissions::channel_access_reading(
        state,
        conn.as_mut(),
        caller,
        channel,
        Some(message_id.0.to_string()),
    )
    .await?;
    let Some(emoji) = canonical_emoji(emoji) else {
        return Ok(Vec::new());
    };
    // Keyset on (timestamp, author), the list's order; the cursor is the last person read.
    let rows: Vec<ReactorRow> = diesel::sql_query(
        r#"
        SELECT author FROM react
        WHERE message = $1 AND emoji = $2
          AND NOT EXISTS (
              SELECT 1 FROM user_block WHERE blocker = $5 AND blocked = react.author
          )
          AND ($3::uuid IS NULL OR ("timestamp", author) > (
              SELECT "timestamp", author FROM react
              WHERE message = $1 AND emoji = $2 AND author = $3))
        ORDER BY "timestamp", author
        LIMIT $4
        "#,
    )
    .bind::<diesel::sql_types::Uuid, _>(message_id.0)
    .bind::<diesel::sql_types::Text, _>(emoji)
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Uuid>, _>(after.map(|a| a.0))
    .bind::<diesel::sql_types::BigInt, _>(i64::from(limit.min(MAX_REACTORS_PAGE)))
    .bind::<diesel::sql_types::Uuid, _>(caller.0)
    .load(conn.as_mut())
    .await?;
    Ok(rows.into_iter().map(|row| row.author).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_emoji_has_one_form_however_it_is_typed() {
        assert_eq!(canonical_emoji("\u{2764}"), Some("\u{2764}\u{FE0F}"));
        assert_eq!(
            canonical_emoji("\u{2764}\u{FE0F}"),
            Some("\u{2764}\u{FE0F}")
        );
        assert_eq!(canonical_emoji("\u{1F44D}"), Some("\u{1F44D}"));
        // Skin tones, sequences, flags, and keycaps keep their own forms.
        assert_eq!(
            canonical_emoji("\u{1F44D}\u{1F3FD}"),
            Some("\u{1F44D}\u{1F3FD}")
        );
        assert_eq!(
            canonical_emoji("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"),
            Some("\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}")
        );
        assert_eq!(
            canonical_emoji("\u{1F1F3}\u{1F1FF}"),
            Some("\u{1F1F3}\u{1F1FF}")
        );
        assert_eq!(
            canonical_emoji("1\u{FE0F}\u{20E3}"),
            Some("1\u{FE0F}\u{20E3}")
        );
        for text in ["foo", "a", "1", "", "\u{1F44D}\u{1F44D}", "\u{1F44D} "] {
            assert_eq!(canonical_emoji(text), None, "{text:?}");
        }
    }
}
