use axum::extract::State;
use axum::Json;
use axum::http::StatusCode;
use tracing::error;
use diesel_async::AsyncPgConnection;
use futures_util::TryFutureExt;
use crate::api::{GlobalServerContext, UserId};
use crate::app;
use crate::app::login::{ChangePassword, ChangePasswordResponse, Login, LoginResponse, Logout, LogoutResponse, OtherServerAuth, OtherServerAuthResponse, TokenRefresh, TokenRefreshResponse};

pub async fn login(
    State(state): State<GlobalServerContext>,
    Json(login): Json<Login>,
) -> (StatusCode, Json<LoginResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn.and_then(|conn| app::login::try_login(conn, &login)).await {
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

pub async fn logout(
    State(state): State<GlobalServerContext>,
    Json(logout): Json<Logout>,
) -> (StatusCode, Json<LogoutResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn.and_then(|conn| app::login::try_logout(conn, &logout)).await {
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
        ChangePasswordResponse::NewPasswordDoesntMeetRequirements { .. } => {
            StatusCode::BAD_REQUEST
        }
        ChangePasswordResponse::ServerError => StatusCode::INTERNAL_SERVER_ERROR,
    };
    (status_code, resp.into())
}

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
) -> Result<Option<UserId>, anyhow::Error> {
    use chrono::Utc;
    use diesel::{BoolExpressionMethods, ExpressionMethods, QueryDsl};
    use diesel_async::RunQueryDsl;
    use futures_util::StreamExt;
    use crate::database::schema::{refresh_token, session};

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