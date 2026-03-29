use crate::api::channel::ChannelViewDescription;
use crate::api::message_enum::server_event::{ChannelEvent, ServerEvent};
use crate::api::{ChannelType, GlobalServerContext, message_enum};
use crate::app;
use crate::app::category::Category;
use crate::app::community::Community;
use crate::app::message::{Message, MessageWithAttachments};
use crate::app::{
    AttachmentId, CategoryId, ChannelId, CommunityId, Loadable, MaybeLoaded, MessageId,
};
use crate::database::schema::message_attachment;
use crate::database::schema::{channel, message};
use diesel::{
    CombineDsl, ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable, SelectableHelper,
};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
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
}

impl Loadable for Channel {
    type Id = ChannelId;

    async fn load_from_db(
        _pg_connection: &mut AsyncPgConnection,
        _id: Self::Id,
    ) -> Result<Self, diesel::result::Error> {
        todo!()
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
        .filter(channel::id.eq(id))
        .first(conn.as_mut())
        .await?;
    Ok(channel)
}

const MAX_MESSAGES_QUERIED: u32 = 200;

pub(crate) async fn read_channel_messages(
    state: &GlobalServerContext,
    id: ChannelId,
    channel_view_description: ChannelViewDescription,
) -> app::error::Result<Vec<MessageWithAttachments>> {
    let mut conn = state.connection_pool.get().await?;
    let query = message::table
        .select(Message::as_select())
        .filter(message::channel.eq(id));
    let messages: Vec<Message> = match channel_view_description {
        ChannelViewDescription::Before { message, count } => {
            query
                .limit(count.min(MAX_MESSAGES_QUERIED) as i64)
                .filter(message::id.le(message))
                .order_by(message::id.desc())
                .load(conn.as_mut())
                .await?
        }
        ChannelViewDescription::After { message, count } => {
            query
                .limit(count.min(MAX_MESSAGES_QUERIED) as i64)
                .filter(message::id.ge(message))
                .order_by(message::id.asc())
                .load(conn.as_mut())
                .await?
        }
        ChannelViewDescription::Around { message, radius } => {
            query
                .limit(radius.min(MAX_MESSAGES_QUERIED / 2) as i64)
                .filter(message::id.le(message))
                .order_by(message::id.desc())
                .union(
                    query
                        .limit(radius.min(MAX_MESSAGES_QUERIED / 2) as i64)
                        .filter(message::id.gt(message))
                        .order_by(message::id.asc()),
                )
                .load(conn.as_mut())
                .await?
        }
        ChannelViewDescription::Search { .. } => {
            todo!()
        }
    };
    let message_attachments: Vec<(MessageId, AttachmentId)> = message_attachment::table
        .select((
            message_attachment::message_id,
            message_attachment::attachment_id,
        ))
        .filter(message_attachment::message_id.eq_any(messages.iter().map(|m| m.id)))
        .order_by(message_attachment::message_id.asc())
        .load(conn.as_mut())
        .await?;
    let mut ret = messages
        .into_iter()
        .map(|message| {
            (
                message.id,
                MessageWithAttachments {
                    message,
                    attachments: Vec::new(),
                },
            )
        })
        .collect::<VecMap<_, _>>();
    for attachment in message_attachments {
        if let Some(message_with_attachments) = ret.get_mut(&attachment.0) {
            message_with_attachments.attachments.push(attachment.1);
        }
    }
    Ok(ret.into_values().collect())
}
