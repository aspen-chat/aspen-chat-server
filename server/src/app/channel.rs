use crate::api::message_enum::request::ChannelUpdateRequest;
use crate::api::message_enum::server_event::{ChannelEvent, ServerEvent};
use crate::api::{ChannelType, GlobalServerContext, message_enum};
use crate::app;
use crate::app::category::Category;
use crate::app::community::Community;
use crate::app::link_preview::load_previews;
use crate::app::message::{Message, MessageWithRelations};
use crate::app::{
    AttachmentId, CategoryId, ChannelId, CommunityId, Loadable, MaybeLoaded, MessageId,
    publish_event,
};
use crate::database::schema::message_attachment;
use crate::database::schema::{channel, message};
use diesel::{
    AsChangeset, BoolExpressionMethods, CombineDsl, ExpressionMethods, Insertable, QueryDsl,
    Queryable, Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
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
}

impl Loadable for Channel {
    type Id = ChannelId;

    async fn load_from_db(state: &GlobalServerContext, id: Self::Id) -> crate::app::Result<Self> {
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

pub async fn create_channel(
    state: &GlobalServerContext,
    name: String,
    sort_index: i32,
    ty: ChannelType,
    community: Option<CommunityId>,
    parent_category: Option<CategoryId>,
) -> super::error::Result<Channel> {
    let id = ChannelId::new();
    let mut conn = state.connection_pool.get().await?;
    let channel = Channel {
        id,
        community: community.map(MaybeLoaded::NotLoaded),
        parent_category: parent_category.map(MaybeLoaded::NotLoaded),
        ty,
        sort_index,
        name: name.clone(),
        deleted_at: None,
    };
    diesel::insert_into(channel::table)
        .values(&channel)
        .execute(conn.as_mut())
        .await?;
    let event = ServerEvent::Channel(ChannelEvent::Create(message_enum::Channel {
        id,
        parent_category,
        community,
        name,
        sort_index,
        ty,
    }));
    app::publish_event(state, &event).await?;
    Ok(channel)
}

pub(crate) async fn read_channel(
    state: &GlobalServerContext,
    id: ChannelId,
) -> app::error::Result<Channel> {
    let mut conn = state.connection_pool.get().await?;
    let channel = channel::table
        .select(Channel::as_select())
        .filter(channel::id.eq(id).and(channel::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;
    Ok(channel)
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

pub(crate) async fn read_channel_messages(
    state: &GlobalServerContext,
    id: ChannelId,
    window: MessageWindow,
) -> app::error::Result<Vec<MessageWithRelations>> {
    let mut conn = state.connection_pool.get().await?;
    channel::table
        .select(Channel::as_select())
        .filter(channel::id.eq(id).and(channel::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;
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
    let message_ids: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
    let message_attachments: Vec<(MessageId, AttachmentId)> = message_attachment::table
        .select((
            message_attachment::message_id,
            message_attachment::attachment_id,
        ))
        .filter(message_attachment::message_id.eq_any(&message_ids))
        .order_by(message_attachment::message_id.asc())
        .load(conn.as_mut())
        .await?;
    // Link previews live in a separate child table; batch-load them by
    // message id so we don't N+1 the query for larger backfills.
    let mut previews_by_id =
        load_previews(conn.as_mut(), state.media_store.as_ref(), &message_ids).await?;
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

use crate::database::schema::pin;

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = pin)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Pin {
    pub message_id: MessageId,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub sort_index: i32,
}

pub(crate) async fn read_channel_pins(
    state: &GlobalServerContext,
    channel_id: ChannelId,
) -> app::error::Result<Vec<Pin>> {
    let mut conn = state.connection_pool.get().await?;
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

pub(crate) async fn update_channel(
    state: &GlobalServerContext,
    id: ChannelId,
    command: ChannelUpdateRequest,
) -> app::error::Result<Channel> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
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
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            publish_event(
                state,
                &ServerEvent::Channel(ChannelEvent::Update {
                    id,
                    parent_category: command.parent_category,
                    community: command.community,
                    name: command.name,
                    sort_index: command.sort_index,
                }),
            )
            .await?;
            Ok(rows.into_iter().next().unwrap())
        }
        .scope_boxed()
    })
    .await
}

pub(crate) async fn delete_channel(
    state: &GlobalServerContext,
    id: ChannelId,
) -> app::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let deleted = diesel::update(channel::table)
                .set(channel::deleted_at.eq(diesel::dsl::now))
                .filter(channel::id.eq(id).and(channel::deleted_at.is_null()))
                .execute(conn.as_mut())
                .await?;
            if deleted == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            publish_event(state, &ServerEvent::Channel(ChannelEvent::Delete { id })).await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}
