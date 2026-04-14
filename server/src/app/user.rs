use crate::api::message_enum::command::{UserCreateCommand, UserUpdateCommand};
use crate::api::message_enum::server_event::UserEvent;
use crate::api::{GlobalServerContext, message_enum};
use crate::app;
use crate::app::icon::Icon;
use crate::app::login::hash_password;
use crate::app::{IconId, Loadable, MaybeLoaded, UserId, publish_event};
use crate::database::schema::user;
use diesel::prelude::*;
use diesel::{BoolExpressionMethods, ExpressionMethods, Queryable, Selectable};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: UserId,
    pub name: String,
    pub icon: Option<MaybeLoaded<Icon>>,
    pub password_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserChangeset {
    pub name: Option<String>,
    pub icon: Option<Option<IconId>>,
    pub password_hash: Option<String>,
}

impl Loadable for User {
    type Id = UserId;

    async fn load_from_db(
        pg_connection: &mut AsyncPgConnection,
        id: Self::Id,
    ) -> Result<Self, diesel::result::Error> {
        let user = user::table
            .select(User::as_select())
            .filter(user::dsl::id.eq(id).and(user::dsl::deleted_at.is_null()))
            .first(pg_connection)
            .await?;
        Ok(user)
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}
pub async fn create_user(
    state: GlobalServerContext,
    command: &UserCreateCommand,
) -> Result<UserId, app::Error> {
    let mut conn = match state.connection_pool.get().await {
        Ok(conn) => conn,
        Err(e) => {
            return Err(e.into());
        }
    };
    let password_hash_result = hash_password(&command.password);
    let password_hash = match password_hash_result {
        Ok(s) => s,
        Err(e) => {
            return Err(e.into());
        }
    };
    let new_user_id = UserId::new();
    let now = chrono::Utc::now();
    diesel::insert_into(user::table)
        .values(User {
            id: new_user_id,
            name: command.name.clone(),
            icon: command.icon.map(MaybeLoaded::NotLoaded),
            password_hash,
            created_at: now,
            last_seen_at: now,
            deleted_at: None,
        })
        .execute(conn.as_mut())
        .await?;
    Ok(new_user_id)
}

pub async fn read_user(state: GlobalServerContext, id: UserId) -> Result<User, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let user = User::load_from_db(conn.as_mut(), id).await?;
    Ok(user)
}

pub async fn read_user_communities(
    state: GlobalServerContext,
    user_id: UserId,
) -> Result<Vec<app::community::Community>, app::Error> {
    use crate::database::schema::{community, community_user};

    let mut conn = state.connection_pool.get().await?;
    let communities = community_user::table
        .inner_join(community::table)
        .select(app::community::Community::as_select())
        .filter(
            community_user::user
                .eq(user_id)
                .and(community::deleted_at.is_null()),
        )
        .order_by(community::name.asc())
        .load(conn.as_mut())
        .await?;
    Ok(communities)
}

pub(crate) async fn update_user(
    state: GlobalServerContext,
    command: UserUpdateCommand,
) -> Result<User, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let Some(user) = diesel::update(user::table)
                .set(UserChangeset {
                    name: command.name.clone(),
                    icon: command.icon,
                    password_hash: None,
                })
                .filter(user::id.eq(command.id).and(user::deleted_at.is_null()))
                .returning(User::as_select())
                .load(conn.as_mut())
                .await?
                .into_iter()
                .next()
            else {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            };
            publish_event(
                &state,
                &message_enum::server_event::ServerEvent::User(UserEvent::Update {
                    id: command.id,
                    name: command.name,
                    icon: command.icon,
                }),
            )
            .await?;
            Ok(user)
        }
        .scope_boxed()
    })
    .await
}

pub(crate) async fn delete_user(state: GlobalServerContext, id: UserId) -> Result<(), app::Error> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let deleted = diesel::update(user::table)
                .set(user::deleted_at.eq(diesel::dsl::now))
                .filter(user::id.eq(id).and(user::deleted_at.is_null()))
                .execute(conn.as_mut())
                .await?;
            if deleted == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            publish_event(
                &state,
                &message_enum::server_event::ServerEvent::User(UserEvent::Delete { id }),
            )
            .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}
