use crate::category::Category;
use crate::community::Community;
use crate::context::GlobalServerContext;
use crate::link_preview::load_previews;
use crate::message::{Message, MessageWithRelations};
use crate::moderation_log::{ModerationAction, log_moderation};
use crate::permissions::{Permissions, missing, require_member};
use crate::role::GrantedOverride;
use crate::t;
use crate::{
    AttachmentId, CategoryId, ChannelId, CommunityId, EventScope, Loadable, MaybeLoaded, MessageId,
    UserId, publish_event,
};
use aspen_schema::message_attachment;
use aspen_schema::{channel, dm_recipient, message};
pub use aspen_wire::channel::ChannelType;
use aspen_wire::message_enum;
use aspen_wire::message_enum::request::{ChannelCreateRequest, ChannelUpdateRequest};
use aspen_wire::message_enum::server_event::{ChannelEvent, ServerEvent};
use diesel::{
    AsChangeset, BoolExpressionMethods, CombineDsl, ExpressionMethods, Insertable, QueryDsl,
    Queryable, Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::collections::HashMap;
use vecmap::VecMap;

#[derive(Debug, Clone, Selectable, Insertable, Queryable)]
#[diesel(table_name=channel)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Channel {
    pub id: ChannelId,
    pub community: Option<MaybeLoaded<Community>>,
    pub parent_category: Option<MaybeLoaded<Category>>,
    pub name: String,
    pub ty: ChannelType,
    pub sort_index: i32,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub parent_channel: Option<ChannelId>,
    pub starter_message: Option<MessageId>,
    pub reply_count: i32,
    pub last_reply_at: Option<chrono::DateTime<chrono::Utc>>,
    /// The two people of a one-to-one DM, as `app::dm::pair_key` writes them; `None` otherwise.
    pub dm_key: Option<String>,
    /// For a channel of a kind a plugin adds, the plugin and the kind
    /// (`org.example.forums:board`); `None` otherwise.
    pub plugin_type: Option<String>,
}

/// The channel's wire record. `recipients` are a DM's or group DM's people, empty for any other
/// channel.
pub fn record(c: &Channel, recipients: Vec<UserId>) -> message_enum::Channel {
    message_enum::Channel {
        id: c.id,
        parent_category: c.parent_category.as_ref().map(MaybeLoaded::id).copied(),
        community: c.community.as_ref().map(MaybeLoaded::id).copied(),
        name: c.name.clone(),
        sort_index: c.sort_index,
        ty: c.ty,
        parent_channel: c.parent_channel,
        starter_message: c.starter_message,
        reply_count: c.reply_count,
        last_reply_at: c.last_reply_at,
        recipients,
        plugin_type: c.plugin_type.clone(),
    }
}

impl Loadable for Channel {
    type Id = ChannelId;

    async fn load_from_db(state: &GlobalServerContext, id: Self::Id) -> crate::Result<Self> {
        let channel = channel::table
            .select(Channel::as_select())
            .filter(
                channel::dsl::id
                    .eq(id)
                    .and(channel::dsl::deleted_at.is_null()),
            )
            .first(&mut state.connection_pool.get().await?)
            .await?;
        Ok(channel)
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

/// Makes a text or voice channel in a community, or one of a kind a plugin running there adds,
/// which takes Manage channels there, with the overrides it starts with, each on the terms
/// setting it afterwards would take.
pub async fn create_channel(
    state: &GlobalServerContext,
    caller: UserId,
    request: ChannelCreateRequest,
) -> super::error::Result<Channel> {
    let Some(community) = request.community else {
        return Err(crate::Error::Validation(t!("channelNeedsCommunity")));
    };
    let name = channel_name(&request.name)?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = require_member(conn.as_mut(), caller, community).await?;
            access.require(Permissions::MANAGE_CHANNELS)?;
            if let Some(category) = request.parent_category {
                ensure_category_of(conn.as_mut(), category, community).await?;
            }
            if let Some(plugin_type) = &request.plugin_type {
                crate::plugin::channel_type::check(state, conn.as_mut(), community, plugin_type)
                    .await?;
            }
            let overrides = crate::role::check_initial_overrides(
                conn.as_mut(),
                &access,
                request.overrides.as_deref().unwrap_or_default(),
            )
            .await?;
            let new = NewChannel {
                name,
                sort_index: request.sort_index,
                ty: request.ty,
                community,
                parent_category: request.parent_category,
                plugin_type: request.plugin_type,
            };
            insert_channel(state, conn.as_mut(), new, &overrides).await
        }
        .scope_boxed()
    })
    .await
}

/// Refuses, as not found, a category that is not a live one of `community`.
async fn ensure_category_of(
    conn: &mut AsyncPgConnection,
    category_id: CategoryId,
    community: CommunityId,
) -> crate::Result<()> {
    use aspen_schema::category;
    category::table
        .select(category::id)
        .filter(
            category::id
                .eq(category_id)
                .and(category::community.eq(community))
                .and(category::deleted_at.is_null()),
        )
        .first::<CategoryId>(conn)
        .await?;
    Ok(())
}

/// A text or voice channel, or one of a plugin's kinds, to be made in a community.
pub struct NewChannel {
    pub name: String,
    pub sort_index: i32,
    pub ty: ChannelType,
    pub community: CommunityId,
    pub parent_category: Option<CategoryId>,
    /// For `ChannelType::Plugin`, the plugin's kind, which the caller has checked.
    pub plugin_type: Option<String>,
}

/// Writes a new text or voice channel with its overrides and announces it, inside the caller's
/// transaction.
pub async fn insert_channel(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    new: NewChannel,
    overrides: &[GrantedOverride],
) -> crate::Result<Channel> {
    let NewChannel {
        name,
        sort_index,
        ty,
        community,
        parent_category,
        plugin_type,
    } = new;
    // Threads, DMs, and group DMs have endpoints of their own, which set what they need; a
    // plugin's channel names its kind, and nothing else does.
    match (ty, &plugin_type) {
        (ChannelType::Text | ChannelType::Voice, None) | (ChannelType::Plugin, Some(_)) => {}
        (ChannelType::Plugin, None) => {
            return Err(crate::Error::Validation(t!("pluginTypeMissing")));
        }
        _ => return Err(crate::Error::Validation(t!("channelTypeNotCreatable"))),
    }
    let channel = Channel {
        id: ChannelId::new(),
        community: Some(MaybeLoaded::NotLoaded(community)),
        parent_category: parent_category.map(MaybeLoaded::NotLoaded),
        ty,
        sort_index,
        name,
        deleted_at: None,
        parent_channel: None,
        starter_message: None,
        reply_count: 0,
        last_reply_at: None,
        dm_key: None,
        plugin_type,
    };
    diesel::insert_into(channel::table)
        .values(&channel)
        .execute(conn)
        .await?;
    crate::role::insert_initial_overrides(state, conn, channel.id, overrides).await?;
    // Published as the channel's own event, after its overrides, so it reaches only those who
    // may view it.
    let event = ServerEvent::Channel(ChannelEvent::Create(record(&channel, Vec::new())));
    let scope = EventScope::ChannelDefinition {
        channel: channel.id,
        departed: None,
    };
    crate::publish_event(state, conn, scope, &event).await?;
    Ok(channel)
}

/// A channel's wire record, with its recipients when it is a DM or group DM.
pub async fn read_channel(
    state: &GlobalServerContext,
    caller: UserId,
    id: ChannelId,
) -> crate::error::Result<message_enum::Channel> {
    let mut conn = state.connection_pool.get().await?;
    let channel = channel::table
        .select(Channel::as_select())
        .filter(channel::id.eq(id).and(channel::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;
    // A DM's record, which a deployment moderator's list of someone's DMs already shows them.
    crate::permissions::channel_access_moderating(state, conn.as_mut(), caller, id).await?;
    let recipients = if matches!(channel.ty, ChannelType::Dm | ChannelType::GroupDm) {
        crate::events::dm_recipients(conn.as_mut(), id).await?
    } else {
        Vec::new()
    };
    Ok(record(&channel, recipients))
}

/// The wire records of the channels named in `ids` that still exist, a DM's or group DM's with
/// its people. Two queries however many there are.
pub async fn read_channels(
    state: &GlobalServerContext,
    ids: &[ChannelId],
) -> crate::error::Result<Vec<message_enum::Channel>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let channels: Vec<Channel> = channel::table
        .select(Channel::as_select())
        .filter(channel::id.eq_any(ids).and(channel::deleted_at.is_null()))
        .load(conn.as_mut())
        .await?;
    let mut recipients: HashMap<ChannelId, Vec<UserId>> = HashMap::new();
    for (channel, user) in dm_recipient::table
        .select((dm_recipient::channel, dm_recipient::user))
        .filter(dm_recipient::channel.eq_any(ids))
        .order_by((dm_recipient::channel, dm_recipient::joined_at))
        .load::<(ChannelId, UserId)>(conn.as_mut())
        .await?
    {
        recipients.entry(channel).or_default().push(user);
    }
    Ok(channels
        .iter()
        .map(|c| record(c, recipients.remove(&c.id).unwrap_or_default()))
        .collect())
}

/// Every live channel the user of `visible` may view in its communities, including those filed
/// under a category, ordered by community and then sort index. This is the batch a client needs
/// to render the channel tree of every community it belongs to in one request.
pub async fn read_communities_channels(
    state: &GlobalServerContext,
    visible: &crate::visibility::Visibility,
) -> crate::error::Result<Vec<crate::channel::Channel>> {
    let communities = visible.communities();
    if communities.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let channels = channel::table
        .select(crate::channel::Channel::as_select())
        .filter(
            channel::community
                .eq_any(communities)
                // Threads record their community too, but belong under their parent channel.
                .and(channel::parent_channel.is_null())
                .and(channel::deleted_at.is_null()),
        )
        .order_by((channel::community.asc(), channel::sort_index.asc()))
        .load::<crate::channel::Channel>(conn.as_mut())
        .await?;
    Ok(channels
        .into_iter()
        .filter(|c| visible.can_view(c.id))
        .collect())
}

/// The top-level channels of a community the caller may view.
pub async fn read_community_channels(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
) -> crate::error::Result<Vec<crate::channel::Channel>> {
    let mut conn = state.connection_pool.get().await?;
    require_member(conn.as_mut(), caller, community).await?;
    let visibility = crate::visibility::Visibility::load(state, caller, &[community]).await?;
    let channels = channel::table
        .select(crate::channel::Channel::as_select())
        .filter(
            channel::community
                .eq(community)
                .and(channel::parent_category.is_null())
                .and(channel::parent_channel.is_null())
                .and(channel::deleted_at.is_null()),
        )
        .order_by(channel::sort_index.asc())
        .load::<crate::channel::Channel>(conn.as_mut())
        .await?
        .into_iter()
        .filter(|c| visibility.can_view(c.id))
        .collect();
    Ok(channels)
}

/// Upper bound on the number of messages a single read returns.
pub const MAX_MESSAGES_QUERIED: u32 = 200;

/// Which slice of a channel's history to read. Message ids are UUIDv7 and therefore sort
/// chronologically, so every window is a keyset range over the id column.
#[derive(Debug, Clone, Copy)]
pub enum MessageWindow {
    /// The newest `limit` messages.
    Latest { limit: u32 },
    /// Up to `limit` messages older than `anchor`, excluding `anchor` itself.
    Before { anchor: MessageId, limit: u32 },
    /// Up to `limit` messages newer than `anchor`, excluding `anchor` itself.
    After { anchor: MessageId, limit: u32 },
    /// `anchor` itself plus up to `radius` messages on either side of it.
    Around { anchor: MessageId, radius: u32 },
}

pub async fn read_channel_messages(
    state: &GlobalServerContext,
    caller: UserId,
    id: ChannelId,
    window: MessageWindow,
) -> crate::error::Result<Vec<MessageWithRelations>> {
    let mut conn = state.connection_pool.get().await?;
    channel::table
        .select(Channel::as_select())
        .filter(channel::id.eq(id).and(channel::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;
    // Reading a DM one is not in is moderation, and every such reading is logged.
    crate::permissions::channel_access_reading(state, conn.as_mut(), caller, id, None).await?;
    let query = message::table
        .select(Message::as_select())
        .filter(message::channel.eq(id).and(message::deleted_at.is_null()));
    let messages: Vec<Message> = match window {
        MessageWindow::Latest { limit } => {
            query
                .limit(limit.min(MAX_MESSAGES_QUERIED) as i64)
                .order_by(message::id.desc())
                .load(conn.as_mut())
                .await?
        }
        MessageWindow::Before { anchor, limit } => {
            query
                .limit(limit.min(MAX_MESSAGES_QUERIED) as i64)
                .filter(message::id.lt(anchor))
                .order_by(message::id.desc())
                .load(conn.as_mut())
                .await?
        }
        MessageWindow::After { anchor, limit } => {
            query
                .limit(limit.min(MAX_MESSAGES_QUERIED) as i64)
                .filter(message::id.gt(anchor))
                .order_by(message::id.asc())
                .load(conn.as_mut())
                .await?
        }
        MessageWindow::Around { anchor, radius } => {
            query
                .limit(radius.min(MAX_MESSAGES_QUERIED / 2) as i64 + 1)
                .filter(message::id.le(anchor))
                .order_by(message::id.desc())
                .union(
                    query
                        .limit(radius.min(MAX_MESSAGES_QUERIED / 2) as i64)
                        .filter(message::id.gt(anchor))
                        .order_by(message::id.asc()),
                )
                .load(conn.as_mut())
                .await?
        }
    };
    with_relations(state, conn.as_mut(), messages).await
}

/// `messages` with their attachments and link previews, in the order given.
pub async fn with_relations(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    messages: Vec<Message>,
) -> crate::error::Result<Vec<MessageWithRelations>> {
    let message_ids: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
    let message_attachments: Vec<(MessageId, AttachmentId)> = message_attachment::table
        .select((
            message_attachment::message_id,
            message_attachment::attachment_id,
        ))
        .filter(message_attachment::message_id.eq_any(&message_ids))
        .order_by(message_attachment::message_id.asc())
        .load(conn)
        .await?;
    // Link previews live in a separate child table; batch-load them by
    // message id so we don't N+1 the query for larger backfills.
    let mut previews_by_id = load_previews(conn, state.media_store.as_ref(), &message_ids).await?;
    let mut ret = messages
        .into_iter()
        .map(|message| {
            let link_previews = previews_by_id.remove(&message.id).unwrap_or_default();
            (
                message.id,
                MessageWithRelations {
                    message,
                    attachments: Vec::new(),
                    link_previews,
                },
            )
        })
        .collect::<VecMap<_, _>>();
    for attachment in message_attachments {
        if let Some(entry) = ret.get_mut(&attachment.0) {
            entry.attachments.push(attachment.1);
        }
    }
    Ok(ret.into_values().collect())
}

use aspen_schema::pin;

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = pin)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Pin {
    pub message_id: MessageId,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub sort_index: i32,
}

pub async fn read_channel_pins(
    state: &GlobalServerContext,
    caller: UserId,
    channel_id: ChannelId,
) -> crate::error::Result<Vec<Pin>> {
    let mut conn = state.connection_pool.get().await?;
    crate::permissions::channel_access_reading(state, conn.as_mut(), caller, channel_id, None)
        .await?;
    channel::table
        .select(Channel::as_select())
        .filter(
            channel::id
                .eq(channel_id)
                .and(channel::deleted_at.is_null()),
        )
        .first(conn.as_mut())
        .await?;
    let pins = pin::table
        .inner_join(message::table)
        .select(Pin::as_select())
        .filter(
            pin::channel
                .eq(channel_id)
                .and(message::deleted_at.is_null()),
        )
        .order_by(pin::sort_index.asc())
        .load(conn.as_mut())
        .await?;
    Ok(pins)
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = channel)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct ChannelChangeset {
    pub parent_category: Option<Option<CategoryId>>,
    pub community: Option<Option<CommunityId>>,
    pub name: Option<String>,
    pub sort_index: Option<i32>,
}

/// The community of the community channel `id` and what the caller may do across it, for
/// managing the channel: a channel they may not view is not found to them (`channel_access`), so
/// nobody renames, moves, or deletes a channel, or edits its overrides, from outside it, which
/// would let a holder of Manage channels undo the overrides that hide it from them. Manage
/// channels is a community permission, which no override changes, so the access across the
/// community says what they may do to it. DMs and threads are not managed this way.
pub async fn managed_channel(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    caller: UserId,
    id: ChannelId,
) -> crate::Result<(CommunityId, crate::permissions::CommunityAccess)> {
    let access = crate::permissions::channel_access(state, conn, caller, id).await?;
    match access.community {
        Some(community) if !access.thread => Ok((community.community, community)),
        _ => Err(crate::Error::Diesel(diesel::result::Error::NotFound)),
    }
}

/// A community channel's name as given, trimmed and within `app::community::MAX_NAME_CHARS`.
fn channel_name(name: &str) -> crate::Result<String> {
    crate::community::trimmed_name(name, |max| t!("channelNameLength", max = max))
}

/// Renames or moves a community channel the caller may view, which takes Manage channels; a
/// deployment moderator may rename one. A channel stays in its community, and a category it moves into must be one
/// of that community's.
pub async fn update_channel(
    state: &GlobalServerContext,
    caller: UserId,
    id: ChannelId,
    mut command: ChannelUpdateRequest,
) -> crate::error::Result<Channel> {
    command.name = command.name.as_deref().map(channel_name).transpose()?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let (community, access) = managed_channel(state, conn.as_mut(), caller, id).await?;
            // A deployment moderator may rename a channel, and do nothing else to it here.
            let rename_only = command.parent_category.is_none()
                && command.community.is_none()
                && command.sort_index.is_none();
            if !access.has(Permissions::MANAGE_CHANNELS) {
                if !(access.moderator && rename_only) {
                    return Err(missing(Permissions::MANAGE_CHANNELS));
                }
                log_moderation(
                    conn.as_mut(),
                    caller,
                    ModerationAction::RenameChannel,
                    Some(community),
                    Some(id),
                    command.name.clone(),
                )
                .await?;
            }
            if command.community.is_some_and(|c| c != Some(community)) {
                return Err(crate::Error::Validation(t!("channelCommunityFixed")));
            }
            if let Some(Some(category)) = command.parent_category {
                ensure_category_of(conn.as_mut(), category, community).await?;
            }
            let rows = diesel::update(channel::table)
                .set(ChannelChangeset {
                    parent_category: command.parent_category,
                    community: command.community,
                    name: command.name.clone(),
                    sort_index: command.sort_index,
                })
                .filter(channel::id.eq(id).and(channel::deleted_at.is_null()))
                .returning(Channel::as_select())
                .load(conn.as_mut())
                .await?;
            if rows.is_empty() {
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            }
            publish_event(
                state,
                conn.as_mut(),
                EventScope::ChannelDefinition {
                    channel: id,
                    departed: None,
                },
                &ServerEvent::Channel(ChannelEvent::Update {
                    id,
                    parent_category: command.parent_category,
                    community: command.community,
                    name: command.name,
                    sort_index: command.sort_index,
                    reply_count: None,
                    last_reply_at: None,
                    recipients: None,
                }),
            )
            .await?;
            Ok(rows.into_iter().next().unwrap())
        }
        .scope_boxed()
    })
    .await
}

/// Deletes a community channel the caller may view, which takes Manage channels or moderating the
/// deployment.
pub async fn delete_channel(
    state: &GlobalServerContext,
    caller: UserId,
    id: ChannelId,
) -> crate::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let (community, access) = managed_channel(state, conn.as_mut(), caller, id).await?;
            if !access.has(Permissions::MANAGE_CHANNELS) {
                if !access.moderator {
                    return Err(missing(Permissions::MANAGE_CHANNELS));
                }
                log_moderation(
                    conn.as_mut(),
                    caller,
                    ModerationAction::DeleteChannel,
                    Some(community),
                    Some(id),
                    None,
                )
                .await?;
            }
            let deleted = diesel::update(channel::table)
                .set(channel::deleted_at.eq(diesel::dsl::now))
                .filter(channel::id.eq(id).and(channel::deleted_at.is_null()))
                .execute(conn.as_mut())
                .await?;
            if deleted == 0 {
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            }
            // What plugins kept about it goes with it.
            crate::plugin::storage::forget(
                conn.as_mut(),
                crate::plugin::storage::Scope::Channel(id),
            )
            .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::ChannelDefinition {
                    channel: id,
                    departed: None,
                },
                &ServerEvent::Channel(ChannelEvent::Delete { id }),
            )
            .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}
