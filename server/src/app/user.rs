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

#[derive(Debug, Clone, Queryable, Selectable)]
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

    fn load_from_db(pg_connection: &AsyncPgConnection, id: Self::Id) -> Result<Self, Error> {
        todo!()
    }

    fn id(&self) -> Self::Id {
        self.id
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
        .values((
            user::columns::id.eq(new_user_id.0),
            user::columns::password_hash.eq(password_hash),
            user::columns::name.eq(&command.name),
        ))
        .execute(&mut conn)
        .await?;
    Ok(new_user_id)
}

pub async fn read_user(state: GlobalServerContext, id: UserId) -> Result<User, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    todo!()
}
