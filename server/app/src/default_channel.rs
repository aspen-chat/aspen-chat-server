//! A community's default channel: the text channel new members are shown first and their join
//! messages are posted in.
//!
//! Its first text channel becomes it, a holder of Manage community may name another, and when
//! it is deleted `choose` picks the next that everyone's role may view, or none, which is
//! announced as the community's `update`. Every change to it holds the
//! community's row (`hold_community`), so two channels deleted at once cannot each pick the
//! other.

use crate::channel::ChannelType;
use crate::context::GlobalServerContext;
use crate::message::Message;
use crate::visibility::CommunityModel;
use crate::{ChannelId, CommunityId, EventScope, MaybeLoaded, MessageId, UserId, publish_event, t};
use aspen_schema::{channel, community, message};
use aspen_wire::message::MessageKind;
use aspen_wire::message_enum::server_event::{CommunityEvent, MessageEvent, ServerEvent};
use chrono::Utc;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};

/// The channel `community` should default to, of its live text channels: the oldest that
/// everyone's role may view. `None` when everyone's role may view none, so new members are never
/// shown, nor joins announced in, a channel not everyone sees. A community holds at most
/// `app::community::MAX_CHANNELS_AND_CATEGORIES` channels.
pub async fn choose(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
) -> crate::Result<Option<ChannelId>> {
    // Oldest first: a UUIDv7 id begins with the moment it was made.
    let channels: Vec<ChannelId> = channel::table
        .select(channel::id)
        .filter(
            channel::community
                .eq(Some(community))
                .and(channel::ty.eq(ChannelType::Text))
                .and(channel::parent_channel.is_null())
                .and(channel::deleted_at.is_null()),
        )
        .order_by(channel::id)
        .load(conn)
        .await?;
    if channels.is_empty() {
        return Ok(None);
    }
    let model = CommunityModel::load(conn, &[community])
        .await?
        .remove(&community)
        .unwrap_or_else(|| CommunityModel::new(community));
    Ok(channels
        .into_iter()
        .find(|channel| model.everyone_views(*channel)))
}

/// Holds `community`'s row until the caller's transaction ends, for reading and changing its
/// default channel. `false` for a deleted community, which needs none.
async fn hold_community(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
) -> crate::Result<bool> {
    Ok(community::table
        .select(community::id)
        .filter(
            community::id
                .eq(community)
                .and(community::deleted_at.is_null()),
        )
        .for_no_key_update()
        .first::<CommunityId>(conn)
        .await
        .optional()?
        .is_some())
}

/// Sets `community`'s default channel and announces it to its members.
async fn set(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    channel: Option<ChannelId>,
) -> crate::Result<()> {
    diesel::update(community::table.filter(community::id.eq(community)))
        .set(community::default_channel.eq(channel))
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        EventScope::Community(community),
        &ServerEvent::Community(CommunityEvent::Update {
            id: community,
            name: None,
            icon: None,
            owner: None,
            default_channel: Some(channel),
            suppress_join_messages: None,
        }),
    )
    .await
}

/// Makes `channel`, a text channel just made in `community`, its default when it has none,
/// inside the transaction that made it. Says whether it did.
pub async fn channel_created(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    channel: ChannelId,
) -> crate::Result<bool> {
    if !hold_community(conn, community).await? {
        return Ok(false);
    }
    let current: Option<ChannelId> = community::table
        .select(community::default_channel)
        .filter(community::id.eq(community))
        .first(conn)
        .await?;
    if current.is_some() {
        return Ok(false);
    }
    set(state, conn, community, Some(channel)).await?;
    Ok(true)
}

/// Picks `community` another default channel when `channel`, just deleted inside the caller's
/// transaction, was it.
pub async fn channel_deleted(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    channel: ChannelId,
) -> crate::Result<()> {
    if !hold_community(conn, community).await? {
        return Ok(());
    }
    let current: Option<ChannelId> = community::table
        .select(community::default_channel)
        .filter(community::id.eq(community))
        .first(conn)
        .await?;
    if current != Some(channel) {
        return Ok(());
    }
    let next = choose(conn, community).await?;
    set(state, conn, community, next).await
}

/// Refuses `channel` as `community`'s default unless it is one of its live text channels,
/// holding the channel's row until the caller's transaction ends so it cannot be deleted before
/// the community names it. The caller has checked that they may view it.
pub async fn check_candidate(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    channel: ChannelId,
) -> crate::Result<()> {
    channel::table
        .select(channel::id)
        .filter(
            channel::id
                .eq(channel)
                .and(channel::community.eq(Some(community)))
                .and(channel::ty.eq(ChannelType::Text))
                .and(channel::parent_channel.is_null())
                .and(channel::deleted_at.is_null()),
        )
        .for_share()
        .first::<ChannelId>(conn)
        .await
        .optional()?
        .ok_or(crate::Error::Validation(t!("defaultChannelNotText")))?;
    Ok(())
}

/// Posts the message announcing that `user` joined `community` in its default channel, inside
/// the transaction that added them, unless the community posts none or has no default channel.
/// It is posted in their name, and is read by them already.
pub async fn announce_join(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user: UserId,
    community: CommunityId,
) -> crate::Result<()> {
    let (channel, suppressed): (Option<ChannelId>, bool) = community::table
        .select((
            community::default_channel,
            community::suppress_join_messages,
        ))
        .filter(community::id.eq(community))
        .first(conn)
        .await?;
    let Some(channel) = channel.filter(|_| !suppressed) else {
        return Ok(());
    };
    let row = Message {
        id: MessageId::new(),
        channel: MaybeLoaded::from_id(channel),
        content: String::new(),
        author: MaybeLoaded::from_id(user),
        timestamp: Utc::now(),
        deleted_at: None,
        edited_at: None,
        kind: MessageKind::MemberJoined,
        poll: None,
        thread: None,
        echo_of: None,
        mentions: crate::mention::Mentions::default(),
        call_seconds: None,
        command_bot: None,
        linked_messages: Default::default(),
        warning: None,
        altered_by: Vec::new(),
        card: None,
        echo: None,
        reply_to: None,
        tags_replied: false,
    };
    diesel::insert_into(message::table)
        .values(&row)
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        EventScope::Channel(channel),
        &ServerEvent::Message(MessageEvent::Create(crate::message::record(
            &row,
            Vec::new(),
            Vec::new(),
        ))),
    )
    .await?;
    crate::read_state::advance(state, conn, user, channel, row.id).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::Permissions;
    use diesel_async::{AsyncConnection, SimpleAsyncConnection};

    /// `choose` against the database `DATABASE_URL` names, inside a transaction that is never
    /// committed; without one it checks nothing.
    #[tokio::test]
    async fn a_default_is_the_oldest_seen_by_everyone() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        const USER: &str = "01900000-0000-7000-8000-000000000001";
        const COMMUNITY: &str = "01900000-0000-7000-8000-000000000002";
        const EVERYONE: &str = "01900000-0000-7000-8000-000000000003";
        const CATEGORY: &str = "01900000-0000-7000-8000-000000000004";
        // The oldest text channel, which everyone's role is denied.
        const HIDDEN: &str = "01800000-0000-7000-8000-000000000010";
        // An older voice channel, never a default.
        const VOICE: &str = "01700000-0000-7000-8000-000000000011";
        // Seen by everyone, the older first.
        const OLDER: &str = "01900000-0000-7000-8000-000000000012";
        const NEWER: &str = "01910000-0000-7000-8000-000000000013";
        // Younger, in a category everyone's role is allowed into.
        const YOUNG: &str = "01a00000-0000-7000-8000-000000000014";
        let view = Permissions::VIEW_CHANNEL.bits();
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(&format!(
            r#"INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at)
                 VALUES ('{USER}', 'u', '', now(), now());
             INSERT INTO community (id, name) VALUES ('{COMMUNITY}', 'c');
             INSERT INTO community_role (id, community, name, position, permissions, everyone)
                 VALUES ('{EVERYONE}', '{COMMUNITY}', 'everyone', 0, {view}, true);
             INSERT INTO category (id, community, name, sort_index)
                 VALUES ('{CATEGORY}', '{COMMUNITY}', 'k', 0);
             INSERT INTO channel (id, name, ty, sort_index, community, parent_category)
                 VALUES ('{HIDDEN}', 'hidden', 'text', 0, '{COMMUNITY}', NULL),
                        ('{VOICE}', 'voice', 'voice', 1, '{COMMUNITY}', NULL),
                        ('{OLDER}', 'older', 'text', 2, '{COMMUNITY}', NULL),
                        ('{NEWER}', 'newer', 'text', 3, '{COMMUNITY}', NULL),
                        ('{YOUNG}', 'young', 'text', 4, '{COMMUNITY}', '{CATEGORY}');
             INSERT INTO channel_override (channel, role, allow, deny)
                 VALUES ('{HIDDEN}', '{EVERYONE}', 0, {view});"#
        ))
        .await
        .unwrap();
        let community = CommunityId(uuid::Uuid::parse_str(COMMUNITY).unwrap());
        let id = |id: &str| Some(ChannelId(uuid::Uuid::parse_str(id).unwrap()));
        assert_eq!(
            choose(&mut conn, community).await.unwrap(),
            id(OLDER),
            "the oldest seen by everyone"
        );
        conn.batch_execute(&format!(
            "UPDATE channel SET deleted_at = now() WHERE id IN ('{OLDER}', '{NEWER}');
             INSERT INTO category_override (category, role, allow, deny)
                 VALUES ('{CATEGORY}', '{EVERYONE}', 0, {view});"
        ))
        .await
        .unwrap();
        assert_eq!(
            choose(&mut conn, community).await.unwrap(),
            None,
            "none is seen by everyone, so none"
        );
        conn.batch_execute(&format!(
            "UPDATE category_override SET allow = {view}, deny = 0 WHERE category = '{CATEGORY}'"
        ))
        .await
        .unwrap();
        assert_eq!(
            choose(&mut conn, community).await.unwrap(),
            id(YOUNG),
            "its category lets everyone in"
        );
        conn.batch_execute(&format!(
            "UPDATE channel SET deleted_at = now() WHERE community = '{COMMUNITY}'"
        ))
        .await
        .unwrap();
        assert_eq!(choose(&mut conn, community).await.unwrap(), None);
    }
}
