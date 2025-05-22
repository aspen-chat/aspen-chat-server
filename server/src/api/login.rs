use anyhow::anyhow;
use argon2::{
    PasswordHash, PasswordVerifier as _,
    password_hash::{Salt, SaltString},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use diesel::{ExpressionMethods as _, QueryDsl};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use futures_util::StreamExt;
use rand::Rng;
use serde::{Deserialize, Serialize};
use tracing::error;
use uuid::Uuid;

use crate::{
    CHACHA_RNG,
    api::{UserId, authenticated_user},
    database::schema,
};

const REFRESH_TOKEN_LIFETIME: Duration = Duration::weeks(52);
const SESSION_TOKEN_LIFETIME: Duration = Duration::hours(3);
const OTHER_SERVER_AUTH_LIFETIME: Duration = Duration::minutes(10);

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LoginResponse {
    Ok {
        user_id: UserId,
        refresh_token: String,
        session_token: String,
        #[serde(with = "super::timestamp_serde")]
        session_token_expires: DateTime<Utc>,
    },
    InvalidCredentials,
    ServerError,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Login {
    pub username: String,
    pub password: String,
}

pub async fn try_login(
    mut conn: impl AsMut<AsyncPgConnection>,
    l: &Login,
) -> Result<LoginResponse, anyhow::Error> {
    use schema::user::dsl::*;
    let Login { username, password } = l;
    let conn = conn.as_mut();
    let user_entry: Result<(Uuid, String), _> = user
        .select((id, password_hash))
        .filter(name.eq(username))
        .first(conn)
        .await;
    match user_entry {
        Ok((user_id, entry_password_hash)) => {
            if check_password(password, &entry_password_hash) {
                use schema::{refresh_token, session};
                let session_token = make_token();
                let refresh_token = make_token();
                let now = chrono::Utc::now();
                let session_token_expires = now + SESSION_TOKEN_LIFETIME;
                diesel::insert_into(refresh_token::table)
                    .values((
                        refresh_token::dsl::token.eq(&refresh_token),
                        refresh_token::dsl::user.eq(user_id),
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
                    user_id: user_id.into(),
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

pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let argon2 = argon2::Argon2::default();
    CHACHA_RNG.with(|rng| {
        let bytes = rng.borrow_mut().random::<[u8; Salt::RECOMMENDED_LENGTH]>();
        SaltString::encode_b64(&bytes)
            .and_then(|salt| PasswordHash::generate(argon2, password, &salt).map(|h| h.to_string()))
    })
}

pub fn check_password(password: &str, entry_password_hash: &str) -> bool {
    let argon2 = argon2::Argon2::default();
    let entry_hash = match argon2::PasswordHash::try_from(entry_password_hash) {
        Ok(v) => v,
        Err(e) => {
            error!("user entry password hash malformed in database {e}");
            return false;
        }
    };
    argon2
        .verify_password(password.as_bytes(), &entry_hash)
        .is_ok()
}

fn make_token() -> String {
    BASE64_STANDARD.encode(CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 32]>()))
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum TokenRefreshResponse {
    Ok { new_session_token: String },
    InvalidToken,
    ServerError,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenRefresh {
    refresh_token: String,
}

pub async fn try_token_refresh(
    mut conn: impl AsMut<AsyncPgConnection>,
    t: &TokenRefresh,
) -> Result<TokenRefreshResponse, anyhow::Error> {
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

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum LogoutResponse {
    Ok,
    InvalidToken,
    ServerError,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Logout {
    refresh_token: String,
}

pub async fn try_logout(
    mut conn: impl AsMut<AsyncPgConnection>,
    t: &Logout,
) -> Result<LogoutResponse, anyhow::Error> {
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

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum ChangePasswordResponse {
    Ok,
    OldPasswordIncorrect,
    NewPasswordDoesntMeetRequirements { cause: PasswordRequirement },
    ServerError,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PasswordRequirement {
    Length,
}

#[derive(Deserialize)]
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
) -> Result<ChangePasswordResponse, anyhow::Error> {
    let conn = conn.as_mut();
    let entry_password_hash: String = schema::user::table
        .select(schema::user::password_hash)
        .filter(schema::user::id.eq(&c.user_id.0))
        .first(conn)
        .await?;
    if check_password(&c.old_password, &entry_password_hash) {
        if c.new_password.len() < PASSWORD_MIN_LENGTH {
            return Ok(ChangePasswordResponse::NewPasswordDoesntMeetRequirements {
                cause: PasswordRequirement::Length,
            });
        }
        let new_password_hash =
            hash_password(&c.new_password).map_err(|e| anyhow!("hashing password failed {e}"))?;
        diesel::update(schema::user::table.filter(schema::user::id.eq(&c.user_id.0)))
            .set(schema::user::password_hash.eq(new_password_hash))
            .execute(conn)
            .await?;
        Ok(ChangePasswordResponse::Ok)
    } else {
        Ok(ChangePasswordResponse::OldPasswordIncorrect)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OtherServerAuth {
    session_token: String,
    other_server_domain: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub enum OtherServerAuthResponse {
    Ok { other_server_auth_token: String },
    InvalidToken,
    Error,
}

pub async fn try_other_server_auth(
    mut conn: impl AsMut<AsyncPgConnection>,
    o: &OtherServerAuth,
) -> Result<OtherServerAuthResponse, anyhow::Error> {
    use schema::other_server_auth_token;

    let Some(user) = authenticated_user(&mut conn, o.session_token.clone()).await? else {
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
        .execute(conn.as_mut())
        .await?;
    Ok(OtherServerAuthResponse::Ok {
        other_server_auth_token,
    })
}
