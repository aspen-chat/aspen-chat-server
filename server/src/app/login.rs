use argon2::{
    PasswordHash, PasswordVerifier as _,
    password_hash::{Salt, SaltString},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use diesel::{BoolExpressionMethods, ExpressionMethods as _, QueryDsl, SelectableHelper};
use diesel_async::{
    AsyncConnection, AsyncPgConnection, RunQueryDsl, scoped_futures::ScopedFutureExt,
};
use futures_util::StreamExt;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use tracing::error;

use crate::api::login::authenticated_user;
use crate::{CHACHA_RNG, app, app::UserId, database::schema};
use crate::{api::GlobalServerContext, app::user::UserPg};

const REFRESH_TOKEN_LIFETIME: Duration = Duration::weeks(52);
const SESSION_TOKEN_LIFETIME: Duration = Duration::hours(3);
const OTHER_SERVER_AUTH_LIFETIME: Duration = Duration::minutes(10);

#[derive(Serialize, utoipa::ToSchema)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LoginResponse {
    Ok {
        user_id: UserId,
        refresh_token: String,
        session_token: String,
        session_token_expires: DateTime<Utc>,
    },
    InvalidCredentials,
    ServerError,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Login {
    pub username: String,
    pub password: String,
}

pub async fn hash_password(password: String) -> app::Result<String> {
    // Prevent CPU blocking work from hoarding tokio workers
    tokio::task::spawn_blocking(|| {
        let argon2 = argon2::Argon2::default();
        CHACHA_RNG.with(|rng| {
            let bytes = rng.borrow_mut().random::<[u8; Salt::RECOMMENDED_LENGTH]>();
            SaltString::encode_b64(&bytes).and_then(|salt| {
                PasswordHash::generate(argon2, password, &salt).map(|h| h.to_string())
            })
        })
    })
    .await?
    .map_err(Into::into)
}

pub async fn check_password(password: String, entry_password_hash: String) -> app::Result<bool> {
    // Prevent CPU blocking work from hoarding tokio workers
    tokio::task::spawn_blocking(move || {
        let argon2 = argon2::Argon2::default();
        let entry_hash = match PasswordHash::try_from(entry_password_hash.as_str()) {
            Ok(v) => v,
            Err(e) => {
                error!("user entry password hash malformed in database {e}");
                return false;
            }
        };
        argon2
            .verify_password(password.as_bytes(), &entry_hash)
            .is_ok()
    })
    .await
    .map_err(Into::into)
}

fn make_token() -> String {
    BASE64_STANDARD.encode(CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 32]>()))
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum TokenRefreshResponse {
    Ok { new_session_token: String },
    InvalidToken,
    ServerError,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenRefresh {
    refresh_token: String,
}

pub async fn try_login(
    state: &GlobalServerContext,
    login: Login,
) -> Result<LoginResponse, app::Error> {
    use schema::user::dsl::*;

    let mut conn = state.connection_pool.get().await?;
    let conn = conn.as_mut();
    let Login { username, password } = &login;
    let user_entry: Result<UserPg, _> = user
        .select(UserPg::as_select())
        .filter(name.eq(username))
        .filter(deleted_at.is_null())
        .first(conn)
        .await;
    match user_entry {
        Ok(u) => {
            if check_password(password.to_string(), u.password_hash).await? {
                use crate::database::schema::{refresh_token, session};
                let session_token = make_token();
                let refresh_token = make_token();
                let now = Utc::now();
                let session_token_expires = now + SESSION_TOKEN_LIFETIME;
                diesel::insert_into(refresh_token::table)
                    .values((
                        refresh_token::dsl::token.eq(&refresh_token),
                        refresh_token::dsl::user.eq(u.id),
                        refresh_token::dsl::expires.eq((now + REFRESH_TOKEN_LIFETIME).naive_utc()),
                    ))
                    .execute(conn)
                    .await?;

                diesel::insert_into(session::table)
                    .values((
                        session::dsl::token.eq(&session_token),
                        session::dsl::refresh_token.eq(&refresh_token),
                        session::dsl::expires.eq(session_token_expires.naive_utc()),
                    ))
                    .execute(conn)
                    .await?;
                Ok(LoginResponse::Ok {
                    user_id: u.id,
                    refresh_token,
                    session_token,
                    session_token_expires,
                })
            } else {
                Ok(LoginResponse::InvalidCredentials)
            }
        }
        Err(e) => {
            if let diesel::result::Error::NotFound = e {
                Ok(LoginResponse::InvalidCredentials)
            } else {
                Err(e.into())
            }
        }
    }
}

pub async fn try_token_refresh(
    mut conn: impl AsMut<AsyncPgConnection>,
    t: &TokenRefresh,
) -> Result<TokenRefreshResponse, app::Error> {
    use schema::{refresh_token, session};
    let conn = conn.as_mut();
    let expires: Option<NaiveDateTime> = refresh_token::table
        .select(refresh_token::expires)
        .filter(refresh_token::dsl::token.eq(&t.refresh_token))
        .limit(1)
        .load_stream(conn)
        .await?
        .next()
        .await
        .transpose()?;
    match expires {
        Some(expires) => {
            let expires = expires.and_utc();
            if expires < Utc::now() {
                // Token expired
                return Ok(TokenRefreshResponse::InvalidToken);
            }
        }
        None => {
            return Ok(TokenRefreshResponse::InvalidToken);
        }
    }
    // If we got here then the token is valid. Issue a refresh.
    let new_token = make_token();
    diesel::insert_into(session::table)
        .values((
            session::dsl::token.eq(&new_token),
            session::dsl::expires.eq(Utc::now().naive_utc() + SESSION_TOKEN_LIFETIME),
            session::dsl::refresh_token.eq(&t.refresh_token),
        ))
        .execute(conn)
        .await?;

    Ok(TokenRefreshResponse::Ok {
        new_session_token: new_token,
    })
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LogoutResponse {
    Ok,
    InvalidToken,
    ServerError,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Logout {
    refresh_token: String,
}

pub async fn try_logout(
    mut conn: impl AsMut<AsyncPgConnection>,
    t: &Logout,
) -> Result<LogoutResponse, app::Error> {
    use schema::{refresh_token, session};
    let conn = conn.as_mut();
    diesel::delete(session::table)
        .filter(session::dsl::refresh_token.eq(&t.refresh_token))
        .execute(conn)
        .await?;
    // TODO: Kill any event streams associated with this refresh token
    // TODO stretch goal: If this server is ever sharded then tell the other shards to kill their event
    // streams too
    let rows_deleted = diesel::delete(refresh_token::table)
        .filter(refresh_token::dsl::token.eq(&t.refresh_token))
        .execute(conn)
        .await?;
    if rows_deleted > 0 {
        Ok(LogoutResponse::Ok)
    } else {
        Ok(LogoutResponse::InvalidToken)
    }
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ChangePasswordResponse {
    Ok,
    OldPasswordIncorrect,
    NewPasswordDoesntMeetRequirements { cause: PasswordRequirement },
    ServerError,
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PasswordRequirement {
    Length,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangePassword {
    user_id: UserId,
    old_password: String,
    new_password: String,
}

const PASSWORD_MIN_LENGTH: usize = 8;

pub async fn try_change_password(
    mut conn: impl AsMut<AsyncPgConnection>,
    c: &ChangePassword,
    current_session_token: String,
) -> Result<ChangePasswordResponse, app::Error> {
    let conn = conn.as_mut();
    let entry_password_hash: String = schema::user::table
        .select(schema::user::password_hash)
        .filter(
            schema::user::id
                .eq(&c.user_id.0)
                .and(schema::user::deleted_at.is_null()),
        )
        .first(conn)
        .await?;
    if check_password(c.old_password.to_string(), entry_password_hash).await? {
        if c.new_password.len() < PASSWORD_MIN_LENGTH {
            return Ok(ChangePasswordResponse::NewPasswordDoesntMeetRequirements {
                cause: PasswordRequirement::Length,
            });
        }
        let new_password_hash = hash_password(c.new_password.to_string()).await?;
        conn.transaction(|conn| {
            async move {
                diesel::update(
                    schema::user::table.filter(
                        schema::user::id
                            .eq(&c.user_id)
                            .and(schema::user::deleted_at.is_null()),
                    ),
                )
                .set(schema::user::password_hash.eq(new_password_hash))
                .execute(conn)
                .await?;
                diesel::sql_query(
                    "
                    UPDATE session
                    SET expires = now()
                    FROM refresh_token
                    WHERE refresh_token.token = session.refresh_token
                        AND session.token != $1
                        AND refresh_token.user = $2
                        AND session.expires > now();
                ",
                )
                .bind::<diesel::sql_types::Text, _>(&current_session_token)
                .bind::<diesel::sql_types::Uuid, _>(&c.user_id)
                .execute(conn)
                .await?;
                diesel::sql_query(
                    "
                    UPDATE refresh_token
                    SET expires = now()
                    FROM session 
                    WHERE refresh_token.token = session.refresh_token
                        AND session.token != $1
                        AND refresh_token.user = $2
                        AND refresh_token.expires > now();
                ",
                )
                .bind::<diesel::sql_types::Text, _>(&current_session_token)
                .bind::<diesel::sql_types::Uuid, _>(&c.user_id)
                .execute(conn)
                .await?;
                Result::<(), app::Error>::Ok(())
            }
            .scope_boxed()
        })
        .await?;

        Ok(ChangePasswordResponse::Ok)
    } else {
        Ok(ChangePasswordResponse::OldPasswordIncorrect)
    }
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OtherServerAuth {
    session_token: String,
    other_server_domain: String,
}

#[derive(Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum OtherServerAuthResponse {
    Ok { other_server_auth_token: String },
    InvalidToken,
    Error,
}

pub async fn try_other_server_auth(
    state: &GlobalServerContext,
    o: &OtherServerAuth,
) -> Result<OtherServerAuthResponse, app::Error> {
    use schema::other_server_auth_token;

    let Some(user) = authenticated_user(state, o.session_token.clone()).await? else {
        return Ok(OtherServerAuthResponse::InvalidToken);
    };
    let other_server_auth_token = make_token();
    let expires = (Utc::now() + OTHER_SERVER_AUTH_LIFETIME).naive_utc();
    diesel::insert_into(other_server_auth_token::table)
        .values((
            other_server_auth_token::dsl::token.eq(other_server_auth_token.as_str()),
            other_server_auth_token::dsl::user.eq(user.0),
            other_server_auth_token::dsl::expires.eq(expires),
            other_server_auth_token::dsl::domain.eq(&o.other_server_domain),
        ))
        .execute(&mut state.connection_pool.get().await?)
        .await?;
    Ok(OtherServerAuthResponse::Ok {
        other_server_auth_token,
    })
}
