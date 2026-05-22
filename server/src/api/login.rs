use crate::api::GlobalServerContext;
use crate::app::login::{
    ChangePassword, ChangePasswordResponse, Login, LoginResponse, Logout, LogoutResponse,
    OtherServerAuth, OtherServerAuthResponse, TokenRefresh, TokenRefreshResponse,
};
use crate::app::user::UserPg;
use crate::app::{self, UserId};
use axum::Json;
use axum::extract::{FromRequestParts, State};
use axum::http::StatusCode;
use axum::http::request::Parts;
use futures_util::TryFutureExt;
use hyper::header::AUTHORIZATION;
use rust_i18n::t;
use std::borrow::Cow;
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

#[utoipa::path(post, path = "/logout", security(("loginKey" = [])), responses((status = OK, body=LogoutResponse)))]
pub async fn logout(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
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

#[utoipa::path(post, path = "/change_password", security(("loginKey" = [])), responses((status = OK, body=ChangePasswordResponse)))]
pub async fn change_password(
    State(state): State<GlobalServerContext>,
    SessionUser {
        user: _,
        session_token,
    }: SessionUser,
    Json(change_password): Json<ChangePassword>,
) -> (StatusCode, Json<ChangePasswordResponse>) {
    let conn = state.connection_pool.get().map_err(Into::into);
    let resp = match conn
        .and_then(|conn| app::login::try_change_password(conn, &change_password, session_token))
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
        ChangePasswordResponse::Ok => StatusCode::OK,
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
    let resp = match app::login::try_other_server_auth(&state, &other_server_auth).await {
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
    state: &GlobalServerContext,
    session_token: String,
) -> Result<Option<UserId>, app::Error> {
    app::user::authenticated_user(state, &session_token).await
}

#[derive(Clone)]
pub struct SessionUser {
    pub user: UserPg,
    pub session_token: String,
}
impl FromRequestParts<GlobalServerContext> for SessionUser {
    type Rejection = (StatusCode, Cow<'static, str>);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &GlobalServerContext,
    ) -> Result<Self, Self::Rejection> {
        let invalid_auth = || (StatusCode::UNAUTHORIZED, t!("invalidAuthToken"));
        let try_again_later = || (StatusCode::INTERNAL_SERVER_ERROR, t!("tryAgainLater"));
        let Some(auth) = parts.headers.get(AUTHORIZATION) else {
            return Err(invalid_auth());
        };
        let auth = match auth.to_str() {
            Ok(s) => s,
            Err(_) => return Err(invalid_auth()),
        };
        let token = auth
            .strip_prefix("Token ")
            .or_else(|| auth.strip_prefix("TOKEN "))
            .or_else(|| auth.strip_prefix("token "));
        let Some(token) = token else {
            return Err(invalid_auth());
        };
        let user = match app::user::user_for_token(state, token).await {
            Ok(value) => value,
            Err(e) => {
                error!("error during authentication: {e}");
                return Err(try_again_later());
            }
        };
        app::user::mark_user_online(state, &user);
        Ok(SessionUser {
            user,
            session_token: token.to_string(),
        })
    }
}
