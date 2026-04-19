use crate::api::message_enum::command::{UserCreateCommand, UserUpdateCommand};
use crate::api::message_enum::server_event::UserEvent;
use crate::api::user::UserOnlineStatus;
use crate::api::{GlobalServerContext, message_enum};
use crate::app;
use crate::app::icon::Icon;
use crate::app::login::hash_password;
use crate::app::{IconId, Loadable, MaybeLoaded, UserId, publish_event};
use crate::database::schema::{self, refresh_token, session, user};
use chrono::Utc;
use diesel::prelude::*;
use diesel::{BoolExpressionMethods, ExpressionMethods, Queryable, Selectable};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use fred::prelude::KeysInterface;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserPg {
    pub id: UserId,
    pub name: String,
    pub icon: Option<MaybeLoaded<Icon>>,
    pub password_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub struct User {
    pub user_pg: UserPg,
    pub online_status: UserOnlineStatus,
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

    async fn load_from_db(state: &GlobalServerContext, id: Self::Id) -> crate::app::Result<Self> {
        let user_pg = user::table
            .select(UserPg::as_select())
            .filter(user::dsl::id.eq(id).and(user::dsl::deleted_at.is_null()))
            .first(&mut state.connection_pool.get().await?)
            .await?;
        Ok(User {
            user_pg,
            online_status: user_online_status(state, id).await?,
        })
    }

    fn id(&self) -> &Self::Id {
        &self.user_pg.id
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
        .values(UserPg {
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

pub async fn read_user(state: &GlobalServerContext, id: UserId) -> crate::app::Result<User> {
    let user = User::load_from_db(state, id).await?;
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
            let Some(user_pg) = diesel::update(user::table)
                .set(UserChangeset {
                    name: command.name.clone(),
                    icon: command.icon,
                    password_hash: None,
                })
                .filter(user::id.eq(command.id).and(user::deleted_at.is_null()))
                .returning(UserPg::as_select())
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
            Ok(User {
                user_pg,
                online_status: user_online_status(&state, command.id).await?,
            })
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

pub async fn authenticated_user(
    state: &GlobalServerContext,
    session_token: &str,
) -> crate::app::Result<Option<UserId>> {
    use crate::database::schema::{refresh_token, session, user};
    use chrono::Utc;
    use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use futures_util::StreamExt;

    let now = Utc::now().naive_utc();
    let maybe_user_id = user::table
        .inner_join(refresh_token::table.inner_join(session::table))
        .select(user::dsl::id)
        .filter(
            session::token
                .eq(session_token)
                .and(session::expires.ge(now))
                .and(refresh_token::expires.ge(now))
                .and(user::deleted_at.is_null()),
        )
        .limit(1)
        .load_stream::<uuid::Uuid>(&mut state.connection_pool.get().await?)
        .await?
        .next()
        .await
        .transpose()?
        .map(UserId::from);
    Ok(maybe_user_id)
}

pub async fn user_for_token(
    state: &GlobalServerContext,
    token: &str,
) -> crate::app::Result<UserPg> {
    let mut conn = state.connection_pool.get().await?;
    let now = Utc::now().naive_utc();
    let user = schema::user::table
        .select(UserPg::as_select())
        .inner_join(refresh_token::table.inner_join(session::table))
        .filter(
            session::dsl::token
                .eq(&token)
                .and(session::dsl::expires.ge(now))
                .and(refresh_token::dsl::expires.ge(now))
                .and(schema::user::deleted_at.is_null()),
        )
        .first(conn.as_mut())
        .await?;
    Ok(user)
}

pub async fn user_online_status(
    state: &GlobalServerContext,
    user_id: UserId,
) -> crate::app::Result<UserOnlineStatus> {
    let online_value: Option<i64> = state.valkey.get(user_online_key_valkey(user_id)).await?;
    Ok(match online_value {
        Some(1) => UserOnlineStatus::Online,
        _ => UserOnlineStatus::Offline,
    })
}

pub async fn users_online_status(
    state: &GlobalServerContext,
    user_ids: Vec<UserId>,
) -> crate::app::Result<Vec<(UserId, UserOnlineStatus)>> {
    let online_values: Vec<Option<i64>> = state
        .valkey
        .mget(
            user_ids
                .iter()
                .map(|user_id| user_online_key_valkey(*user_id))
                .collect::<Vec<_>>(),
        )
        .await?;
    Ok(user_ids
        .into_iter()
        .zip(
            online_values
                .into_iter()
                .map(|online_value| match online_value {
                    Some(1) => UserOnlineStatus::Online,
                    _ => UserOnlineStatus::Offline,
                }),
        )
        .collect())
}

pub fn mark_user_online(state: &GlobalServerContext, user: &UserPg) {
    let valkey = state.valkey.clone();
    let nats = state.nats_context.clone();
    let user_id = user.id;
    tokio::spawn(async move {
        use fred::prelude::KeysInterface;
        let key = user_online_key_valkey(user_id);
        // SET NX: only succeeds if the key doesn't exist (user was offline)
        let became_online: bool = match valkey
            .set::<fred::types::Value, _, i64>(
                &key,
                1,
                Some(fred::types::Expiration::EX(60)),
                Some(fred::types::SetOptions::NX),
                false,
            )
            .await
        {
            Ok(v) => !v.is_null(),
            Err(e) => {
                tracing::warn!(error = %e, "failed to update user online status in Valkey");
                return;
            }
        };
        if became_online {
            // User just came online — publish event
            if let Err(e) = crate::app::user_status::publish_online(&nats, user_id).await {
                tracing::warn!(error = %e, "failed to publish user online event");
            }
        } else {
            // Already online — just refresh the TTL
            if let Err(e) = valkey
                .set::<(), _, _>(
                    &key,
                    "1",
                    Some(fred::types::Expiration::EX(60)),
                    None,
                    false,
                )
                .await
            {
                tracing::warn!(error = %e, "failed to refresh user online TTL in Valkey");
            }
        }
    });
}

fn user_online_key_valkey(user_id: UserId) -> String {
    format!("user:{}:online", user_id)
}
