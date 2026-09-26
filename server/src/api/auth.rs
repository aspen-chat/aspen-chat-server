//! Session lifecycle: login, logout, session refresh, and the bearer-token extractor every
//! authenticated handler uses.
//!
//! Aspen issues two opaque tokens at login. The long-lived refresh token is exchanged for
//! short-lived session tokens through `POST /auth/token-refresh`; the session token is sent on
//! every other request as `Authorization: Bearer <session token>`.

use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::{Json, NoContent};
use crate::api::{GlobalServerContext, TAG_AUTH};
use crate::app;
use crate::app::UserId;
use crate::app::login::{LoginOutcome, TokenRefreshOutcome};
use crate::app::user::UserPg;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use chrono::{DateTime, Utc};
use hyper::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use tracing::error;
use utoipa::ToSchema;

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoginResponse {
    pub user_id: UserId,
    /// Long-lived token. Store it securely and exchange it for session tokens.
    pub refresh_token: String,
    /// Short-lived token sent as `Authorization: Bearer <sessionToken>`.
    pub session_token: String,
    pub session_token_expires: DateTime<Utc>,
}

#[utoipa::path(
    post,
    path = "/auth/login",
    tag = TAG_AUTH,
    responses(
        (status = OK, body = LoginResponse),
        (status = UNAUTHORIZED, description = "`invalidCredentials`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn login(
    State(state): State<GlobalServerContext>,
    Json(request): Json<LoginRequest>,
) -> ApiResult<Json<LoginResponse>> {
    match app::login::try_login(&state, &request.username, &request.password).await? {
        LoginOutcome::Ok(session) => Ok(Json(LoginResponse {
            user_id: session.user_id,
            refresh_token: session.refresh_token,
            session_token: session.session_token,
            session_token_expires: session.session_token_expires,
        })),
        LoginOutcome::InvalidCredentials => Err(ApiError::new(ProblemCode::InvalidCredentials)),
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LogoutRequest {
    pub refresh_token: String,
}

/// Revokes a refresh token and every session token issued from it.
///
/// Revocation is idempotent in the sense of RFC 7009: a token that is unknown or already revoked
/// still yields `204`, because the end state the client asked for already holds.
#[utoipa::path(
    post,
    path = "/auth/logout",
    tag = TAG_AUTH,
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn logout(
    State(state): State<GlobalServerContext>,
    _: SessionUser,
    Json(request): Json<LogoutRequest>,
) -> ApiResult<NoContent> {
    let conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    app::login::try_logout(conn, &request.refresh_token).await?;
    Ok(NoContent)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenRefreshRequest {
    pub refresh_token: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TokenRefreshResponse {
    pub session_token: String,
    pub session_token_expires: DateTime<Utc>,
}

#[utoipa::path(
    post,
    path = "/auth/token-refresh",
    tag = TAG_AUTH,
    responses(
        (status = OK, body = TokenRefreshResponse),
        (status = UNAUTHORIZED, description = "`invalidToken`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn token_refresh(
    State(state): State<GlobalServerContext>,
    Json(request): Json<TokenRefreshRequest>,
) -> ApiResult<Json<TokenRefreshResponse>> {
    let conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    match app::login::try_token_refresh(conn, &request.refresh_token).await? {
        TokenRefreshOutcome::Ok {
            session_token,
            session_token_expires,
        } => Ok(Json(TokenRefreshResponse {
            session_token,
            session_token_expires,
        })),
        TokenRefreshOutcome::InvalidToken => Err(ApiError::new(ProblemCode::InvalidToken)),
    }
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OtherServerTokenRequest {
    pub other_server_domain: String,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OtherServerTokenResponse {
    pub other_server_auth_token: String,
}

/// Placeholder for federation. Mints a short-lived token the calling user can present to
/// another Aspen server. Federation design is not finalised; do not build on this yet.
#[utoipa::path(
    post,
    path = "/auth/other-server-token",
    tag = TAG_AUTH,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = OtherServerTokenResponse),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn other_server_token(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(request): Json<OtherServerTokenRequest>,
) -> ApiResult<Json<OtherServerTokenResponse>> {
    let other_server_auth_token =
        app::login::try_other_server_auth(&state, user.id, &request.other_server_domain).await?;
    Ok(Json(OtherServerTokenResponse {
        other_server_auth_token,
    }))
}

/// The authenticated caller. Extracting it requires a valid `Authorization: Bearer <session
/// token>` header; anything else is rejected with a `401` Problem.
#[derive(Clone)]
pub struct SessionUser {
    pub user: UserPg,
    pub session_token: String,
}

impl FromRequestParts<GlobalServerContext> for SessionUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &GlobalServerContext,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(bearer_token)
            .ok_or_else(|| ApiError::new(ProblemCode::Unauthorized))?;
        let user = match app::user::user_for_token(state, token).await {
            Ok(Some(user)) => user,
            Ok(None) => return Err(ApiError::new(ProblemCode::Unauthorized)),
            Err(e) => {
                error!("error during authentication: {e}");
                return Err(ApiError::new(ProblemCode::Internal));
            }
        };
        app::user::mark_user_online(state, &user);
        Ok(SessionUser {
            user,
            session_token: token.to_string(),
        })
    }
}

/// Extracts the credential from an `Authorization` header value using the `Bearer` scheme
/// (RFC 6750). The scheme name is case-insensitive; the token is returned verbatim.
fn bearer_token(header: &str) -> Option<&str> {
    let (scheme, token) = header.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    (!token.is_empty()).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::bearer_token;

    #[test]
    fn bearer_scheme_is_case_insensitive() {
        assert_eq!(bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(bearer_token("bearer abc"), Some("abc"));
        assert_eq!(bearer_token("BEARER abc"), Some("abc"));
    }

    #[test]
    fn other_schemes_are_rejected() {
        assert_eq!(bearer_token("Token abc"), None);
        assert_eq!(bearer_token("Basic abc"), None);
        assert_eq!(bearer_token("abc"), None);
        assert_eq!(bearer_token("Bearer "), None);
    }
}
