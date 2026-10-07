//! A community's own emoji (`custom_emoji`): named pictures its members use in messages, as
//! `<:id>` in the text, and as reactions. Holders of Manage custom emoji add, rename, and
//! remove them, up to the deployment setting `custom_emoji_limit` per community. The picture is an icon
//! uploaded first (`app::icon`), which goes with the emoji.

use crate::api::message_enum::server_event::{CustomEmojiEvent, ServerEvent};
use crate::api::message_enum::{self, request::CustomEmojiUpdateRequest};
use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::events::{EventScope, publish_event};
use crate::app::permissions::{Permissions, require_member};
use crate::app::{CommunityId, CustomEmojiId, IconId, UserId};
use crate::t;
use aspen_schema::custom_emoji;
use chrono::{DateTime, Utc};
use diesel::{
    ExpressionMethods, Insertable, OptionalExtension, QueryDsl, Queryable, Selectable,
    SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use tracing::warn;

/// How long a name may be, in characters.
pub const NAME_MIN_CHARS: usize = 2;
pub const NAME_MAX_CHARS: usize = 32;

/// The largest picture an emoji may be, in bytes, as small as it is shown.
pub const MAX_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = custom_emoji)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CustomEmojiRow {
    pub id: CustomEmojiId,
    pub community: CommunityId,
    pub name: String,
    pub icon: IconId,
    pub created_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
}

impl From<&CustomEmojiRow> for message_enum::CustomEmoji {
    fn from(row: &CustomEmojiRow) -> Self {
        message_enum::CustomEmoji {
            id: row.id,
            community: row.community,
            name: row.name.clone(),
            icon: row.icon,
            created_by: row.created_by,
        }
    }
}

/// The emoji's reference, as a message's text and a reaction name it.
pub fn reference(id: CustomEmojiId) -> String {
    format!("<:{}>", id.0)
}

/// The emoji a reference names, if `s` is one.
pub fn referenced(s: &str) -> Option<CustomEmojiId> {
    let inner = s.strip_prefix("<:")?.strip_suffix('>')?;
    uuid::Uuid::parse_str(inner).ok().map(CustomEmojiId)
}

/// `name` trimmed, when it is a name: 2 to 32 characters of any script, none of them
/// whitespace or a colon, which the message box's `:name:` could not hold.
pub fn validate_name(name: &str) -> app::Result<String> {
    let name = name.trim();
    let chars = name.chars().count();
    if !(NAME_MIN_CHARS..=NAME_MAX_CHARS).contains(&chars) {
        return Err(app::Error::Validation(t!(
            "customEmojiNameLength",
            min = NAME_MIN_CHARS,
            max = NAME_MAX_CHARS
        )));
    }
    if name
        .chars()
        .any(|c| c.is_whitespace() || c == ':' || c.is_control())
    {
        return Err(app::Error::Validation(t!("customEmojiNameChars")));
    }
    Ok(name.to_string())
}

/// Every emoji of each of `communities`, by community and then by name.
pub async fn read_communities_emoji(
    state: &GlobalServerContext,
    communities: &[CommunityId],
) -> app::Result<Vec<message_enum::CustomEmoji>> {
    if communities.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<CustomEmojiRow> = custom_emoji::table
        .select(CustomEmojiRow::as_select())
        .filter(custom_emoji::community.eq_any(communities.to_vec()))
        .order((
            custom_emoji::community,
            custom_emoji::name,
            custom_emoji::id,
        ))
        .load(conn.as_mut())
        .await?;
    Ok(rows.iter().map(message_enum::CustomEmoji::from).collect())
}

/// A community's emoji, by name, for a member of it.
pub async fn read_emoji(
    state: &GlobalServerContext,
    caller: UserId,
    community_id: CommunityId,
) -> app::Result<Vec<message_enum::CustomEmoji>> {
    let mut conn = state.connection_pool.get().await?;
    require_member(conn.as_mut(), caller, community_id).await?;
    read_communities_emoji(state, &[community_id]).await
}

/// Adds an emoji to a community from an icon the caller uploaded, which must be a picture format
/// of at most [`MAX_BYTES`] and not yet another emoji's, under the community's limit. Takes
/// Manage custom emoji.
pub async fn create_emoji(
    state: &GlobalServerContext,
    caller: UserId,
    community_id: CommunityId,
    name: &str,
    icon_id: IconId,
) -> app::Result<message_enum::CustomEmoji> {
    let name = validate_name(name)?;
    let limit = i64::from(state.settings().custom_emoji_limit);
    match state
        .media_store
        .head_object(&app::icon::storage_key(icon_id))
        .await?
    {
        None => return Err(app::Error::Validation(t!("customEmojiIconMissing"))),
        Some(bytes) if bytes > MAX_BYTES => {
            return Err(app::Error::Validation(t!(
                "customEmojiTooLarge",
                max = app::attachment::size_text(MAX_BYTES)
            )));
        }
        Some(_) => {}
    }
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = require_member(conn.as_mut(), caller, community_id).await?;
            access.require(Permissions::MANAGE_CUSTOM_EMOJI)?;
            // The icon must be the caller's, uploaded and confirmed, and not already an
            // emoji's; it is deleted with the emoji, so no two may share one.
            app::icon::require_own(conn.as_mut(), caller, icon_id, t!("customEmojiIconMissing"))
                .await?;
            let taken: i64 = custom_emoji::table
                .filter(custom_emoji::icon.eq(icon_id))
                .count()
                .get_result(conn.as_mut())
                .await?;
            if taken > 0 {
                return Err(app::Error::Validation(t!("customEmojiIconUsed")));
            }
            let count: i64 = custom_emoji::table
                .filter(custom_emoji::community.eq(community_id))
                .count()
                .get_result(conn.as_mut())
                .await?;
            if count >= limit {
                return Err(app::Error::Validation(t!(
                    "customEmojiLimit",
                    limit = limit
                )));
            }
            let row = CustomEmojiRow {
                id: CustomEmojiId::new(),
                community: community_id,
                name,
                icon: icon_id,
                created_by: Some(caller),
                created_at: Utc::now(),
            };
            diesel::insert_into(custom_emoji::table)
                .values(&row)
                .execute(conn.as_mut())
                .await?;
            let record = message_enum::CustomEmoji::from(&row);
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Community(community_id),
                &ServerEvent::CustomEmoji(CustomEmojiEvent::Create(record.clone())),
            )
            .await?;
            Ok(record)
        }
        .scope_boxed()
    })
    .await
}

/// Renames an emoji. Takes Manage custom emoji in its community.
pub async fn update_emoji(
    state: &GlobalServerContext,
    caller: UserId,
    id: CustomEmojiId,
    request: &CustomEmojiUpdateRequest,
) -> app::Result<message_enum::CustomEmoji> {
    let name = request.name.as_deref().map(validate_name).transpose()?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let mut row = load_for_update(conn.as_mut(), id).await?;
            let access = require_member(conn.as_mut(), caller, row.community).await?;
            access.require(Permissions::MANAGE_CUSTOM_EMOJI)?;
            let Some(name) = name else {
                return Ok(message_enum::CustomEmoji::from(&row));
            };
            if name != row.name {
                diesel::update(custom_emoji::table.filter(custom_emoji::id.eq(id)))
                    .set(custom_emoji::name.eq(&name))
                    .execute(conn.as_mut())
                    .await?;
                row.name = name.clone();
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::Community(row.community),
                    &ServerEvent::CustomEmoji(CustomEmojiEvent::Update {
                        id,
                        name: Some(name),
                    }),
                )
                .await?;
            }
            Ok(message_enum::CustomEmoji::from(&row))
        }
        .scope_boxed()
    })
    .await
}

/// Removes an emoji, and with it its reactions and its picture. Takes Manage custom emoji in
/// its community.
pub async fn delete_emoji(
    state: &GlobalServerContext,
    caller: UserId,
    id: CustomEmojiId,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let icon_id = conn
        .transaction(|conn| {
            async move {
                let row = load_for_update(conn.as_mut(), id).await?;
                let access = require_member(conn.as_mut(), caller, row.community).await?;
                access.require(Permissions::MANAGE_CUSTOM_EMOJI)?;
                // Its reactions go with it, by the table's cascade; a client drops them on
                // the emoji's deletion event.
                diesel::delete(custom_emoji::table.filter(custom_emoji::id.eq(id)))
                    .execute(conn.as_mut())
                    .await?;
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::Community(row.community),
                    &ServerEvent::CustomEmoji(CustomEmojiEvent::Delete { id }),
                )
                .await?;
                Ok::<_, app::Error>(row.icon)
            }
            .scope_boxed()
        })
        .await?;
    // The picture is nobody's now; losing it to a storage failure costs an orphaned object,
    // never the deletion.
    if let Err(e) = app::icon::delete_icon(state, icon_id).await {
        warn!(
            error = e.to_string(),
            icon = icon_id.0.to_string(),
            "failed to delete a custom emoji's picture"
        );
    }
    Ok(())
}

/// The emoji's row, locked for the rest of the transaction.
async fn load_for_update(
    conn: &mut AsyncPgConnection,
    id: CustomEmojiId,
) -> app::Result<CustomEmojiRow> {
    custom_emoji::table
        .select(CustomEmojiRow::as_select())
        .filter(custom_emoji::id.eq(id))
        .for_update()
        .first(conn)
        .await
        .map_err(Into::into)
}

/// The id of the emoji `reference` names, if it belongs to `community`.
pub async fn resolve_in_community(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    reference: &str,
) -> app::Result<Option<CustomEmojiId>> {
    let Some(id) = referenced(reference) else {
        return Ok(None);
    };
    let found: Option<CustomEmojiId> = custom_emoji::table
        .select(custom_emoji::id)
        .filter(custom_emoji::id.eq(id))
        .filter(custom_emoji::community.eq(community))
        .first(conn)
        .await
        .optional()?;
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_round_trip() {
        let id = CustomEmojiId::new();
        assert_eq!(referenced(&reference(id)), Some(id));
        assert_eq!(referenced("<:nope>"), None);
        assert_eq!(referenced("👍"), None);
    }

    #[test]
    fn names_are_short_words_of_any_script() {
        assert_eq!(validate_name(" partyparrot ").unwrap(), "partyparrot");
        assert!(validate_name("笑顔").is_ok());
        assert!(validate_name("a").is_err());
        assert!(validate_name("two words").is_err());
        assert!(validate_name("colon:here").is_err());
        assert!(validate_name(&"x".repeat(33)).is_err());
    }
}
