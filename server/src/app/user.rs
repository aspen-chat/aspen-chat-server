use crate::api::{message_enum, GlobalServerContext};
use crate::api::message_enum::command::{UserCreateCommand, UserUpdateCommand};
use crate::app;
use crate::app::icon::Icon;
use crate::app::login::hash_password;
use crate::app::{publish_event, IconId, Loadable, MaybeLoaded, UserId};
use crate::database::schema::user;
use diesel::prelude::*;
use diesel::{ExpressionMethods, Queryable, Selectable};
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use diesel_async::scoped_futures::ScopedFutureExt;
use crate::api::message_enum::server_event::UserEvent;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: UserId,
    pub name: String,
    pub icon: Option<MaybeLoaded<Icon>>,
    pub password_hash: String,
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
            .filter(user::dsl::id.eq(id))
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
    diesel::insert_into(user::table)
        .values(User {
            id: new_user_id,
            name: command.name.clone(),
            icon: command.icon.map(MaybeLoaded::NotLoaded),
            password_hash,
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

pub(crate) async fn update_user(
    state: GlobalServerContext,
    command: UserUpdateCommand,
) -> Result<User, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| async move {
        let user = diesel::update(user::table)
            .set(UserChangeset {
                name: command.name.clone(),
                icon: command.icon,
                password_hash: None,
            })
            .filter(user::id.eq(command.id))
            .returning(User::as_select())
            .load(conn.as_mut())
            .await?;
        if user.len() == 0 {
            return Err(app::Error::Diesel(diesel::result::Error::NotFound));
        }
        publish_event(&state, &message_enum::server_event::ServerEvent::User(UserEvent::Update{
            id: command.id,
            name: command.name,
            icon: command.icon,
        })).await?;
        Ok(user.into_iter().next().unwrap())
    }.scope_boxed()).await
}
