use crate::api::{GlobalServerContext, UserId};
use crate::app;
use crate::app::login::{
    ChangePassword, ChangePasswordResponse, Login, LoginResponse, Logout, LogoutResponse,
    OtherServerAuth, OtherServerAuthResponse, TokenRefresh, TokenRefreshResponse,
};
use crate::app::user::User;
use crate::database::schema::refresh_token;
use axum::Json;
use axum::extract::{FromRequest, FromRequestParts, Request, State};
use axum::http::StatusCode;
use axum::http::header::ToStrError;
use axum::http::request::Parts;
use chrono::Utc;
use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl, SelectableHelper};
use diesel_async::return_futures::GetResult;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use futures_util::TryFutureExt;
use hyper::header::AUTHORIZATION;
use std::task::{Context, Poll};
use tower::Service;
use tracing::error;

#[utoipa::path(post, path = "/login", responses((status = OK, body=LoginResponse)))]
pub async fn login(
    State(state): State<GlobalServerContext>,
    Json(login): Json<Login>,
) -> (StatusCode, Json<LoginResponse>) {
    let resp = match app::login::try_login(&state, login).await {
        Ok(resp) => resp,
        Err(e) => {
            error!("error during login {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                LoginResponse::ServerError.into(),
            );
        }
    };
    let status_code = match &resp {
        LoginResponse::Ok { .. } => StatusCode::OK,
        LoginResponse::InvalidCredentials => StatusCode::UNAUTHORIZED,
        LoginResponse::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status_code, resp.into())
}

#[utoipa::path(post, path = "/logout", responses((status = OK, body=LogoutResponse)))]
pub async fn logout(
    State(state): State<GlobalServerContext>,
    Json(logout): Json<Logout>,
) -> (StatusCode, Json<LogoutResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn
        .and_then(|conn| app::login::try_logout(conn, &logout))
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            error!("error during logout {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                LogoutResponse::ServerError.into(),
            );
        }
    };
    let status_code = match &resp {
        LogoutResponse::Ok => StatusCode::OK,
        LogoutResponse::InvalidToken => StatusCode::UNAUTHORIZED,
        LogoutResponse::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status_code, resp.into())
}

#[utoipa::path(post, path = "/token_refresh", responses((status = OK, body=TokenRefreshResponse)))]
pub async fn token_refresh(
    State(state): State<GlobalServerContext>,
    Json(token_refresh): Json<TokenRefresh>,
) -> (StatusCode, Json<TokenRefreshResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn
        .and_then(|conn| app::login::try_token_refresh(conn, &token_refresh))
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            error!("error during token refresh {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                TokenRefreshResponse::ServerError.into(),
            );
        }
    };
    let status_code = match &resp {
        TokenRefreshResponse::Ok { .. } => StatusCode::OK,
        TokenRefreshResponse::InvalidToken => StatusCode::UNAUTHORIZED,
        TokenRefreshResponse::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status_code, resp.into())
}

#[utoipa::path(post, path = "/change_password", responses((status = OK, body=ChangePasswordResponse)))]
pub async fn change_password(
    State(state): State<GlobalServerContext>,
    Json(change_password): Json<ChangePassword>,
) -> (StatusCode, Json<ChangePasswordResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn
        .and_then(|conn| app::login::try_change_password(conn, &change_password))
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            error!("error during change password {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ChangePasswordResponse::ServerError.into(),
            );
        }
    };
    let status_code = match &resp {
        ChangePasswordResponse::Ok { .. } => StatusCode::OK,
        ChangePasswordResponse::OldPasswordIncorrect => StatusCode::UNAUTHORIZED,
        ChangePasswordResponse::NewPasswordDoesntMeetRequirements { .. } => StatusCode::BAD_REQUEST,
        ChangePasswordResponse::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status_code, resp.into())
}

#[utoipa::path(post, path = "/other_server_login", responses((status = OK, body=OtherServerAuthResponse)))]
pub async fn other_server_login(
    State(state): State<GlobalServerContext>,
    Json(other_server_auth): Json<OtherServerAuth>,
) -> (StatusCode, Json<OtherServerAuthResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn
        .and_then(|conn| app::login::try_other_server_auth(conn, &other_server_auth))
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            error!("error during other_server_auth_token {e}");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                OtherServerAuthResponse::Error.into(),
            );
        }
    };
    let status_code = match &resp {
        OtherServerAuthResponse::Ok { .. } => StatusCode::OK,
        OtherServerAuthResponse::InvalidToken => StatusCode::BAD_REQUEST,
        OtherServerAuthResponse::Error => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status_code, resp.into())
}

pub async fn authenticated_user(
    mut conn: impl AsMut<AsyncPgConnection>,
    session_token: String,
) -> Result<Option<UserId>, app::Error> {
    use crate::database::schema::{refresh_token, session};
    use chrono::Utc;
    use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use futures_util::StreamExt;

    let now = Utc::now().naive_utc();
    let maybe_user_id = session::table
        .inner_join(refresh_token::table)
        .select(refresh_token::dsl::user)
        .filter(
            session::token
                .eq(session_token)
                .and(session::expires.ge(now))
                .and(refresh_token::expires.ge(now)),
        )
        .limit(1)
        .load_stream::<uuid::Uuid>(conn.as_mut())
        .await?
        .next()
        .await
        .transpose()?
        .map(UserId::from);
    Ok(maybe_user_id)
}

#[derive(Clone)]
pub struct SessionUser(pub User);
impl FromRequestParts<GlobalServerContext> for SessionUser {
    type Rejection = (StatusCode, &'static str);

    fn from_request_parts(
        parts: &mut Parts,
        state: &GlobalServerContext,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        use crate::database::schema::{self, refresh_token, session};
        async move {
            const INVALID_AUTH: (StatusCode, &'static str) =
                (StatusCode::UNAUTHORIZED, "invalid auth token");
            let Some(auth) = parts.headers.get(AUTHORIZATION) else {
                return Err(INVALID_AUTH);
            };
            let auth = match auth.to_str() {
                Ok(s) => s,
                Err(_) => return Err(INVALID_AUTH),
            };
            let token = auth
                .strip_prefix("Token ")
                .or_else(|| auth.strip_prefix("TOKEN "))
                .or_else(|| auth.strip_prefix("token "));
            let Some(token) = token else {
                return Err(INVALID_AUTH);
            };
            let mut conn = match state.connection_pool.get().await {
                Ok(conn) => conn,
                Err(e) => {
                    error!("error getting database connection from pool {e}");
                    return Err((StatusCode::INTERNAL_SERVER_ERROR, "Please try again later."));
                }
            };
            let now = Utc::now().naive_utc();
            let select_result = schema::user::table
                .select(User::as_select())
                .inner_join(refresh_token::table.inner_join(session::table))
                .filter(
                    session::dsl::token
                        .eq(&token)
                        .and(session::dsl::expires.ge(now))
                        .and(refresh_token::dsl::expires.ge(now)),
                )
                .first(conn.as_mut())
                .await;
            match select_result {
                Ok(user) => Ok(SessionUser(user)),
                Err(e) => {
                    if let diesel::result::Error::NotFound = e {
                        Err(INVALID_AUTH)
                    } else {
                        error!("error during authentication {e}");
                        Err((StatusCode::INTERNAL_SERVER_ERROR, "Please try again later."))
                    }
                }
            }
        }
    }
}
