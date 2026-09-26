use crate::api::auth::SessionUser;
use crate::api::community::{CommunityList, CommunityReadQuery};
use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::{Created, Json, NoContent, Path, Query};
use crate::api::message_enum::User;
use crate::api::message_enum::request::{UserCreateRequest, UserUpdateRequest};
use crate::api::{API_PREFIX, GlobalServerContext, TAG_USERS};
use crate::app::UserId;
use crate::app::login::ChangePasswordOutcome;
use crate::{api, app};
use axum::extract::State;
use diesel::result::DatabaseErrorKind;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use utoipa::openapi::schema::{ObjectBuilder, SchemaType, Type};
use utoipa::openapi::{RefOr, Schema};
use utoipa::{PartialSchema, ToSchema};

/// What a user says they are up to: a short line of text and, optionally, an emoji beside it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CustomStatus {
    pub text: String,
    /// A single emoji, or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, utoipa::ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UserOnlineStatus {
    Online,
    Offline,
    Away,
}

/// A user addressed in a URL: either a user id or the literal `@me` for the calling user.
#[derive(Debug, Clone, Copy)]
pub enum UserRef {
    Me,
    Id(UserId),
}

impl UserRef {
    pub fn resolve(self, session: &SessionUser) -> UserId {
        match self {
            UserRef::Me => session.user.id,
            UserRef::Id(id) => id,
        }
    }
}

impl<'de> Deserialize<'de> for UserRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        if raw == "@me" {
            return Ok(UserRef::Me);
        }
        raw.parse::<uuid::Uuid>()
            .map(|id| UserRef::Id(UserId(id)))
            .map_err(|_| serde::de::Error::custom("expected a user id or `@me`"))
    }
}

impl PartialSchema for UserRef {
    fn schema() -> RefOr<Schema> {
        ObjectBuilder::new()
            .schema_type(SchemaType::Type(Type::String))
            .description(Some("A user id (UUID), or `@me` for the calling user."))
            .into()
    }
}

impl ToSchema for UserRef {}

pub fn user_to_api(user: app::user::User) -> User {
    User {
        id: user.user_pg.id,
        name: user.user_pg.name,
        icon: user.user_pg.icon.map(|i| *i.id()),
        online_status: user.online_status,
        display_name: user.user_pg.display_name,
        pronouns: user.user_pg.pronouns,
        bio: user.user_pg.bio,
        status: user.user_pg.status_text.map(|text| CustomStatus {
            text,
            emoji: user.user_pg.status_emoji,
        }),
    }
}

/// Registers a new account. This is one of the few unauthenticated endpoints. Usernames must be
/// 1 to 32 characters with no leading or trailing whitespace; passwords must be at least 8
/// characters. Registration does not log the user in; call `POST /auth/login` next.
#[utoipa::path(
    post,
    path = "/users",
    tag = TAG_USERS,
    responses(
        (status = CREATED, body = User, headers(("Location" = String, description = "URL of the new user"))),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (username rules)", body = Problem),
        (status = CONFLICT, description = "`usernameTaken`", body = Problem),
        (status = UNPROCESSABLE_ENTITY, description = "`passwordRequirementsNotMet`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_user(
    State(state): State<GlobalServerContext>,
    Json(request): Json<UserCreateRequest>,
) -> ApiResult<Created<User>> {
    let new_user_id = app::user::create_user(state, &request)
        .await
        .map_err(|e| match e {
            app::Error::Diesel(diesel::result::Error::DatabaseError(
                DatabaseErrorKind::UniqueViolation,
                _,
            )) => ApiError::new(ProblemCode::UsernameTaken),
            other => other.into(),
        })?;
    Ok(Created::new(
        format!("{API_PREFIX}/users/{}", new_user_id.0),
        User {
            id: new_user_id,
            name: request.name,
            icon: request.icon,
            online_status: UserOnlineStatus::Offline,
            display_name: request.display_name,
            pronouns: request.pronouns,
            bio: request.bio,
            status: request.status,
        },
    ))
}

#[utoipa::path(
    get,
    path = "/users/{user}",
    tag = TAG_USERS,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = User),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_user(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path(user): Path<UserRef>,
) -> ApiResult<Json<User>> {
    let user = app::user::read_user(&state, user.resolve(&session)).await?;
    Ok(Json(user_to_api(user)))
}

/// Lists the communities the user belongs to, by name. Only the calling user's own list is
/// available. `include` sideloads the channels, categories, and members of every listed
/// community, which is how a client bootstraps its state after login in one request.
#[utoipa::path(
    get,
    path = "/users/{user}/communities",
    tag = TAG_USERS,
    params(("user" = inline(UserRef), Path), CommunityReadQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = CommunityList),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_user_communities(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path(user): Path<UserRef>,
    Query(query): Query<CommunityReadQuery>,
) -> ApiResult<Json<CommunityList>> {
    let user_id = user.resolve(&session);
    if user_id != session.user.id {
        return Err(ApiError::new(ProblemCode::Forbidden));
    }
    let communities = app::user::read_user_communities(state.clone(), user_id).await?;
    let ids: Vec<_> = communities.iter().map(|c| c.id).collect();
    let included =
        api::community::sideload_communities(&state, user_id, &ids, &query.include).await?;
    Ok(Json(CommunityList::new(
        communities
            .into_iter()
            .map(api::community::community_to_api)
            .collect(),
        included,
    )))
}

#[utoipa::path(
    patch,
    path = "/users/{user}",
    tag = TAG_USERS,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = User),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_user(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path(user): Path<UserRef>,
    Json(request): Json<UserUpdateRequest>,
) -> ApiResult<Json<User>> {
    let user_id = user.resolve(&session);
    let updated = app::user::update_user(state, session.user.id, user_id, request)
        .await
        .map_err(not_your_account)?;
    Ok(Json(user_to_api(updated)))
}

#[utoipa::path(
    delete,
    path = "/users/{user}",
    tag = TAG_USERS,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_user(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path(user): Path<UserRef>,
) -> ApiResult<NoContent> {
    let user_id = user.resolve(&session);
    app::user::delete_user(state, session.user.id, user_id)
        .await
        .map_err(not_your_account)?;
    Ok(NoContent)
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    pub old_password: String,
    pub new_password: String,
}

/// Replaces the user's password. Every other session and refresh token belonging to the user is
/// revoked; the session making this call remains valid.
#[utoipa::path(
    put,
    path = "/users/{user}/password",
    tag = TAG_USERS,
    params(("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` or `oldPasswordIncorrect`", body = Problem),
        (status = UNPROCESSABLE_ENTITY, description = "`passwordRequirementsNotMet`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn change_password(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path(user): Path<UserRef>,
    Json(request): Json<ChangePasswordRequest>,
) -> ApiResult<NoContent> {
    let user_id = user.resolve(&session);
    if user_id != session.user.id {
        return Err(not_your_account(app::Error::Unauthorized));
    }
    let conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    match app::login::try_change_password(
        conn,
        user_id,
        &request.old_password,
        &request.new_password,
        &session.session_token,
    )
    .await?
    {
        ChangePasswordOutcome::Ok => Ok(NoContent),
        ChangePasswordOutcome::OldPasswordIncorrect => {
            Err(ApiError::new(ProblemCode::OldPasswordIncorrect))
        }
        ChangePasswordOutcome::RequirementNotMet(requirement) => {
            Err(ApiError::password_requirement(requirement).with_detail(t!(
                "passwordTooShort",
                min = app::login::PASSWORD_MIN_LENGTH
            )))
        }
    }
}

fn not_your_account(e: app::Error) -> ApiError {
    match e {
        app::Error::Unauthorized => {
            ApiError::new(ProblemCode::Forbidden).with_detail(t!("notYourAccount"))
        }
        other => other.into(),
    }
}
