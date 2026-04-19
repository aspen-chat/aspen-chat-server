use crate::api::message_enum::command::{CommunityCreateCommand, CommunityUpdateCommand};
use crate::api::message_enum::server_event::{CommunityEvent, ServerEvent, UserCommunityEvent};
use crate::api::{ChannelType, GlobalServerContext, message_enum};
use crate::app;
use crate::app::channel::create_channel;
use crate::app::icon::Icon;
use crate::app::user::users_online_status;
use crate::app::{CommunityId, IconId, Loadable, MaybeLoaded, UserId, publish_event};
use crate::database::schema::channel;
use crate::database::schema::community;
use crate::database::schema::community_user;
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable,
    Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use rust_i18n::t;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = community)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Community {
    pub id: CommunityId,
    pub name: String,
    pub icon: Option<MaybeLoaded<Icon>>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
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

    async fn load_from_db(state: &GlobalServerContext, id: CommunityId) -> app::Result<Self> {
        community::table
            .select(Community::as_select())
            .filter(
                community::dsl::id
                    .eq(id)
                    .and(community::deleted_at.is_null()),
            )
            .first(&mut state.connection_pool.get().await?)
            .await
            .map_err(Into::into)
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

pub(crate) async fn create_community(
    state: GlobalServerContext,
    user: UserId,
    command: &CommunityCreateCommand,
) -> Result<Community, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let community = Community {
        id: CommunityId::new(),
        icon: command.icon.map(MaybeLoaded::NotLoaded),
        name: command.name.clone(),
        deleted_at: None,
    };
    diesel::insert_into(community::table)
        .values(community.clone())
        .execute(conn.as_mut())
        .await?;
    // Don't publish create community event, it's not needed.
    join_community(&state, user, community.id, Invitation::AccessGranted).await?;
    create_channel(
        &state,
        t!("firstTextChannelName").to_string(),
        0,
        ChannelType::Text,
        Some(community.id),
        None,
    )
    .await?;
    create_channel(
        &state,
        t!("firstVoiceChannelName").to_string(),
        0,
        ChannelType::Voice,
        Some(community.id),
        None,
    )
    .await?;
    Ok(community)
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = community)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CommunityChangeset {
    pub name: Option<String>,
    pub icon: Option<Option<IconId>>,
}

pub(crate) async fn update_community(
    state: &GlobalServerContext,
    command: CommunityUpdateCommand,
) -> app::error::Result<Community> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let Some(community) = diesel::update(community::table)
                .set(CommunityChangeset {
                    name: command.name.clone(),
                    icon: command.icon,
                })
                .filter(
                    community::id
                        .eq(command.id)
                        .and(community::deleted_at.is_null()),
                )
                .returning(Community::as_select())
                .load(conn.as_mut())
                .await?
                .into_iter()
                .next()
            else {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            };
            publish_event(
                state,
                &ServerEvent::Community(CommunityEvent::Update {
                    id: command.id,
                    name: command.name,
                    icon: command.icon,
                }),
            )
            .await?;
            Ok(community)
        }
        .scope_boxed()
    })
    .await
}

pub(crate) async fn read_community(
    state: &GlobalServerContext,
    id: CommunityId,
) -> app::error::Result<Community> {
    Community::load_from_db(state, id).await
}

pub(crate) async fn delete_community(
    state: &GlobalServerContext,
    id: CommunityId,
) -> app::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let deleted = diesel::update(community::table)
                .set(community::deleted_at.eq(diesel::dsl::now))
                .filter(community::id.eq(id).and(community::deleted_at.is_null()))
                .execute(conn.as_mut())
                .await?;
            if deleted == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            publish_event(
                state,
                &ServerEvent::Community(CommunityEvent::Delete { id }),
            )
            .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

pub enum Invitation {
    Code(String),
    // Bypass the need for an invite code. Currently only used for adding the first user to a community.
    AccessGranted,
}

pub(crate) async fn join_community(
    state: &GlobalServerContext,
    user: UserId,
    community: CommunityId,
    invitation: Invitation,
) -> app::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            if let Invitation::Code(invite_code) = invitation {
                let invite_community = app::invite::validate_invite(conn, &invite_code).await?;
                if invite_community != community {
                    return Err(app::Error::Validation(t!("inviteCodeCommunityMismatch")));
                }
            }
            diesel::insert_into(community_user::table)
                .values(&CommunityUser { user, community })
                .execute(conn)
                .await?;
            let event = ServerEvent::UserCommunity(UserCommunityEvent::Create(
                message_enum::UserCommunity { community, user },
            ));
            app::publish_event(state, &event).await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
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

pub(crate) async fn read_community_users(
    state: &GlobalServerContext,
    community: CommunityId,
) -> app::error::Result<Vec<app::user::User>> {
    use crate::database::schema::user;

    let mut conn = state.connection_pool.get().await?;
    let users = community_user::table
        .inner_join(user::table)
        .select(app::user::UserPg::as_select())
        .filter(
            community_user::community
                .eq(community)
                .and(user::deleted_at.is_null()),
        )
        .order_by(user::last_seen_at.desc())
        .limit(100)
        .load(conn.as_mut())
        .await?;
    let online_status = users_online_status(state, users.iter().map(|u| u.id).collect()).await?;
    Ok(users
        .into_iter()
        .zip(online_status)
        .map(|(u, (_id, o))| app::user::User {
            user_pg: u,
            online_status: o,
        })
        .collect())
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
                .and(channel::parent_category.is_null())
                .and(channel::deleted_at.is_null()),
        )
        .order_by(channel::sort_index.asc())
        .load(conn.as_mut())
        .await?;
    Ok(channels)
}
