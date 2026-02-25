use crate::api::GlobalServerContext;
use crate::api::message_enum::command::UserCreateCommand;
use crate::app;
use crate::app::login::hash_password;
use crate::app::{IconId, Loadable, UserId};
use crate::database::schema::{self, user};
use diesel::result::Error;
use diesel::{ExpressionMethods, Queryable, Selectable};
use diesel::{QueryResult, prelude::*};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct User {
    pub id: UserId,
    pub name: String,
    pub icon: Option<IconId>,
    pub password_hash: String,
}

impl Loadable for User {
    type Id = UserId;

    async fn load_from_db(
        pg_connection: &mut AsyncPgConnection,
        id: Self::Id,
    ) -> Result<Self, Error> {
        let user = schema::user::table
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
            icon: command.icon,
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
