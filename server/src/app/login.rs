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
use tracing::error;

use crate::api::error::PasswordRequirement;
use crate::{CHACHA_RNG, app, app::UserId, database::schema};
use crate::{api::GlobalServerContext, app::user::UserPg};

const REFRESH_TOKEN_LIFETIME: Duration = Duration::weeks(52);
const SESSION_TOKEN_LIFETIME: Duration = Duration::hours(3);
const OTHER_SERVER_AUTH_LIFETIME: Duration = Duration::minutes(10);
pub const PASSWORD_MIN_LENGTH: usize = 8;

/// Credentials issued by a successful login.
pub struct Session {
    pub user_id: UserId,
    pub refresh_token: String,
    pub session_token: String,
    pub session_token_expires: DateTime<Utc>,
}

pub enum LoginOutcome {
    Ok(Session),
    InvalidCredentials,
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

pub enum TokenRefreshOutcome {
    Ok {
        session_token: String,
        session_token_expires: DateTime<Utc>,
    },
    InvalidToken,
}

pub async fn try_login(
    state: &GlobalServerContext,
    username: &str,
    password: &str,
) -> Result<LoginOutcome, app::Error> {
    use schema::user::dsl::*;

    let mut conn = state.connection_pool.get().await?;
    let conn = conn.as_mut();
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
                Ok(LoginOutcome::Ok(Session {
                    user_id: u.id,
                    refresh_token,
                    session_token,
                    session_token_expires,
                }))
            } else {
                Ok(LoginOutcome::InvalidCredentials)
            }
        }
        Err(e) => {
            if let diesel::result::Error::NotFound = e {
                Ok(LoginOutcome::InvalidCredentials)
            } else {
                Err(e.into())
            }
        }
    }
}

pub async fn try_token_refresh(
    mut conn: impl AsMut<AsyncPgConnection>,
    refresh_token_value: &str,
) -> Result<TokenRefreshOutcome, app::Error> {
    use schema::{refresh_token, session};
    let conn = conn.as_mut();
    let expires: Option<NaiveDateTime> = refresh_token::table
        .select(refresh_token::expires)
        .filter(refresh_token::dsl::token.eq(refresh_token_value))
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
                return Ok(TokenRefreshOutcome::InvalidToken);
            }
        }
        None => {
            return Ok(TokenRefreshOutcome::InvalidToken);
        }
    }
    // If we got here then the token is valid. Issue a refresh.
    let new_token = make_token();
    let session_token_expires = Utc::now() + SESSION_TOKEN_LIFETIME;
    diesel::insert_into(session::table)
        .values((
            session::dsl::token.eq(&new_token),
            session::dsl::expires.eq(session_token_expires.naive_utc()),
            session::dsl::refresh_token.eq(refresh_token_value),
        ))
        .execute(conn)
        .await?;

    Ok(TokenRefreshOutcome::Ok {
        session_token: new_token,
        session_token_expires,
    })
}

/// Revokes a refresh token and all sessions issued from it. Returns whether a refresh token was
/// actually removed; callers treat an unknown token as already revoked.
pub async fn try_logout(
    mut conn: impl AsMut<AsyncPgConnection>,
    refresh_token_value: &str,
) -> Result<bool, app::Error> {
    use schema::{refresh_token, session};
    let conn = conn.as_mut();
    diesel::delete(session::table)
        .filter(session::dsl::refresh_token.eq(refresh_token_value))
        .execute(conn)
        .await?;
    // TODO: Kill any event streams associated with this refresh token
    // TODO stretch goal: If this server is ever sharded then tell the other shards to kill their event
    // streams too
    let rows_deleted = diesel::delete(refresh_token::table)
        .filter(refresh_token::dsl::token.eq(refresh_token_value))
        .execute(conn)
        .await?;
    Ok(rows_deleted > 0)
}

pub enum ChangePasswordOutcome {
    Ok,
    OldPasswordIncorrect,
    RequirementNotMet(PasswordRequirement),
}

/// Changes `user_id`'s password and expires every other session and refresh token belonging to
/// the user, so a stolen credential stops working the moment the owner rotates their password.
/// The session performing the change (`current_session_token`) stays valid.
pub async fn try_change_password(
    mut conn: impl AsMut<AsyncPgConnection>,
    user_id: UserId,
    old_password: &str,
    new_password: &str,
    current_session_token: &str,
) -> Result<ChangePasswordOutcome, app::Error> {
    let conn = conn.as_mut();
    let entry_password_hash: String = schema::user::table
        .select(schema::user::password_hash)
        .filter(
            schema::user::id
                .eq(&user_id.0)
                .and(schema::user::deleted_at.is_null()),
        )
        .first(conn)
        .await?;
    if !check_password(old_password.to_string(), entry_password_hash).await? {
        return Ok(ChangePasswordOutcome::OldPasswordIncorrect);
    }
    if new_password.len() < PASSWORD_MIN_LENGTH {
        return Ok(ChangePasswordOutcome::RequirementNotMet(
            PasswordRequirement::Length,
        ));
    }
    let new_password_hash = hash_password(new_password.to_string()).await?;
    conn.transaction(|conn| {
        async move {
            diesel::update(
                schema::user::table.filter(
                    schema::user::id
                        .eq(&user_id)
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
            .bind::<diesel::sql_types::Text, _>(current_session_token)
            .bind::<diesel::sql_types::Uuid, _>(&user_id)
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
            .bind::<diesel::sql_types::Text, _>(current_session_token)
            .bind::<diesel::sql_types::Uuid, _>(&user_id)
            .execute(conn)
            .await?;
            Result::<(), app::Error>::Ok(())
        }
        .scope_boxed()
    })
    .await?;

    Ok(ChangePasswordOutcome::Ok)
}

pub async fn try_other_server_auth(
    state: &GlobalServerContext,
    user: UserId,
    other_server_domain: &str,
) -> Result<String, app::Error> {
    use schema::other_server_auth_token;

    let other_server_auth_token = make_token();
    let expires = (Utc::now() + OTHER_SERVER_AUTH_LIFETIME).naive_utc();
    diesel::insert_into(other_server_auth_token::table)
        .values((
            other_server_auth_token::dsl::token.eq(other_server_auth_token.as_str()),
            other_server_auth_token::dsl::user.eq(user.0),
            other_server_auth_token::dsl::expires.eq(expires),
            other_server_auth_token::dsl::domain.eq(other_server_domain),
        ))
        .execute(&mut state.connection_pool.get().await?)
        .await?;
    Ok(other_server_auth_token)
}
