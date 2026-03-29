use crate::api::message_enum::command::CommunityCreateCommand;
use crate::api::message_enum::server_event::{ServerEvent, UserCommunityEvent};
use crate::api::{GlobalServerContext, message_enum};
use crate::app;
use crate::app::icon::Icon;
use crate::app::{CommunityId, Loadable, MaybeLoaded, UserId};
use crate::database::schema::channel;
use crate::database::schema::community;
use crate::database::schema::community_user;
use diesel::{
    BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable,
    SelectableHelper,
};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = community)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Community {
    pub id: CommunityId,
    pub name: String,
    pub icon: Option<MaybeLoaded<Icon>>,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = community_user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CommunityUser {
    pub user: UserId,
    pub community: CommunityId,
}

impl Loadable for Community {
    type Id = CommunityId;

    async fn load_from_db(
        pg_connection: &mut AsyncPgConnection,
        id: CommunityId,
    ) -> Result<Self, diesel::result::Error> {
        Ok(community::table
            .select(Community::as_select())
            .filter(community::dsl::id.eq(id))
            .first(pg_connection)
            .await?)
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

pub(crate) async fn create_community(
    state: GlobalServerContext,
    command: &CommunityCreateCommand,
) -> Result<Community, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let community = Community {
        id: CommunityId::new(),
        icon: command.icon.map(MaybeLoaded::NotLoaded),
        name: command.name.clone(),
    };
    diesel::insert_into(community::table)
        .values(community.clone())
        .execute(conn.as_mut())
        .await?;
    Ok(community)
}

pub(crate) async fn join_community(
    state: &GlobalServerContext,
    user: UserId,
    community: CommunityId,
) -> app::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    diesel::insert_into(community_user::table)
        .values(&CommunityUser { user, community })
        .execute(conn.as_mut())
        .await?;
    let event =
        ServerEvent::UserCommunity(UserCommunityEvent::Create(message_enum::UserCommunity {
            community,
            user,
        }));
    app::publish_event(state, &event).await?;
    Ok(())
}

pub(crate) async fn leave_community(
    state: &GlobalServerContext,
    user: UserId,
    community: CommunityId,
) -> app::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let deleted = diesel::delete(community_user::table)
        .filter(
            community_user::community
                .eq(community)
                .and(community_user::user.eq(user)),
        )
        .execute(conn.as_mut())
        .await?;
    if deleted > 0 {
        let event = ServerEvent::UserCommunity(UserCommunityEvent::Delete { community, user });
        app::publish_event(state, &event).await?;
    }
    Ok(())
}

pub(crate) async fn read_community_channels(
    state: &GlobalServerContext,
    community: CommunityId,
) -> app::error::Result<Vec<app::channel::Channel>> {
    let mut conn = state.connection_pool.get().await?;
    let channels = channel::table
        .select(app::channel::Channel::as_select())
        .filter(
            channel::community
                .eq(community)
                .and(channel::parent_category.is_null()),
        )
        .order_by(channel::sort_index.asc())
        .load(conn.as_mut())
        .await?;
    Ok(channels)
}
