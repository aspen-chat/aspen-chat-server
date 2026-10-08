//! Bots, made and managed by the people who own them (`app::bot`). A bot's profile is changed
//! through `PATCH /users/{user}`, by the bot itself or by its owner.

use crate::auth::SessionUser;
use crate::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::extract::{Created, Json, NoContent, Path};
use crate::message_enum::{User, UserCommunity};
use crate::{API_PREFIX, TAG_USERS};
use aspen_app::context::GlobalServerContext;
use aspen_app::permissions::{Permission, from_names};
use aspen_app::{self as app, CommunityId, UserId};
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use diesel::result::DatabaseErrorKind;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A new bot.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BotCreateRequest {
    /// Its username, unique among everyone's, people and bots alike.
    pub name: String,
    /// What to call it, in place of its username; absent or `null` for none.
    #[serde(default)]
    pub display_name: Option<String>,
}

/// A bot just made, and the token it signs in with, which is not shown again.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BotCreated {
    pub bot: User,
    pub token: String,
}

/// A bot's new token; the one before it stopped working when this was issued.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BotToken {
    /// Sent as `Authorization: Bearer <token>` on every request, and as the `sessionToken` of
    /// the event stream's `identify`.
    pub token: String,
}

/// Who a bot is offered to.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BotOwnerRequest {
    /// A person, not a bot, who owns fewer bots than the server allows.
    pub owner: UserId,
}

fn username_taken(e: app::Error) -> ApiError {
    match e {
        app::Error::Diesel(diesel::result::Error::DatabaseError(
            DatabaseErrorKind::UniqueViolation,
            _,
        )) => ApiError::new(ProblemCode::UsernameTaken),
        other => other.into(),
    }
}

/// The bots the caller owns, the oldest first.
#[utoipa::path(
    get,
    path = "/users/@me/bots",
    tag = TAG_USERS,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<User>),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_bots(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
) -> ApiResult<Json<Vec<User>>> {
    let bots = app::bot::list_owned(&state, user.id).await?;
    Ok(Json(bots.into_iter().map(User::from).collect()))
}

/// Makes a bot the caller owns, and answers with the token it signs in with, once.
#[utoipa::path(
    post,
    path = "/users/@me/bots",
    tag = TAG_USERS,
    request_body = BotCreateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = BotCreated, headers(("Location" = String, description = "URL of the bot"))),
        (status = BAD_REQUEST, description = "`validation`: a bad name, or the caller owns as many bots as allowed", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: the server allows no new bots, or the caller is a bot", body = Problem),
        (status = CONFLICT, description = "`usernameTaken`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_bot(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(request): Json<BotCreateRequest>,
) -> ApiResult<Created<BotCreated>> {
    let (bot, token) = app::bot::create(&state, user.id, request.name, request.display_name)
        .await
        .map_err(username_taken)?;
    let bot = User::from(bot);
    Ok(Created::new(
        format!("{API_PREFIX}/users/{}", bot.id.0),
        BotCreated { bot, token },
    ))
}

/// Issues a new token for a bot the caller owns; the old one stops working at once, and what it
/// holds open closes. Takes a recently verified sign-in.
#[utoipa::path(
    post,
    path = "/bots/{bot}/token",
    tag = TAG_USERS,
    params(("bot" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = BotToken),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: not the caller's bot; `reauthenticationRequired`: verify again first", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn rotate_bot_token(
    State(state): State<GlobalServerContext>,
    SessionUser { caller, .. }: SessionUser,
    Path(bot): Path<UserId>,
) -> ApiResult<Json<BotToken>> {
    let token = app::bot::rotate_token(&state, &caller, bot).await?;
    Ok(Json(BotToken { token }))
}

/// An offer to hand a bot over, as its giver and its recipient see it.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BotTransfer {
    pub bot: User,
    /// Who offers it: its owner.
    pub from: User,
    /// Who it is offered to.
    pub to: User,
    pub created_at: DateTime<Utc>,
    /// When the offer lapses unaccepted.
    pub expires_at: DateTime<Utc>,
}

impl From<app::bot::Transfer> for BotTransfer {
    fn from(transfer: app::bot::Transfer) -> Self {
        Self {
            bot: transfer.bot.into(),
            from: transfer.from.into(),
            to: transfer.to.into(),
            created_at: transfer.created_at,
            expires_at: transfer.expires_at,
        }
    }
}

/// Offers a bot the caller owns to someone else, who owns it once they accept. Offering it again
/// replaces the offer. The recipient is told by the system account. Takes a recently verified
/// sign-in.
#[utoipa::path(
    put,
    path = "/bots/{bot}/transfer",
    tag = TAG_USERS,
    params(("bot" = UserId, Path)),
    request_body = BotOwnerRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Offered", body = BotTransfer),
        (status = OK, description = "An earlier offer replaced", body = BotTransfer),
        (status = BAD_REQUEST, description = "`validation`: the recipient is a bot, the caller, someone of another deployment, or owns as many bots as allowed", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: not the caller's bot; `reauthenticationRequired`: verify again first", body = Problem),
        (status = NOT_FOUND, description = "No such bot, or no such person", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn offer_bot_transfer(
    State(state): State<GlobalServerContext>,
    SessionUser { caller, .. }: SessionUser,
    Path(bot): Path<UserId>,
    Json(request): Json<BotOwnerRequest>,
) -> ApiResult<(StatusCode, Json<BotTransfer>)> {
    let (transfer, replaced) =
        app::bot::offer_transfer(&state, &caller, bot, request.owner).await?;
    let status = if replaced {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((status, Json(transfer.into())))
}

/// Withdraws the offer of a bot the caller made, or declines one made to them.
#[utoipa::path(
    delete,
    path = "/bots/{bot}/transfer",
    tag = TAG_USERS,
    params(("bot" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Withdrawn or declined"),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No offer of this bot made by or to the caller", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn end_bot_transfer(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(bot): Path<UserId>,
) -> ApiResult<NoContent> {
    app::bot::end_transfer(&state, user.id, bot).await?;
    Ok(NoContent)
}

/// Accepts a bot offered to the caller: they own it from now on, and it gets a new token, shown
/// only in this answer; the old one stops working, and what it holds open closes.
#[utoipa::path(
    post,
    path = "/bots/{bot}/transfer/acceptance",
    tag = TAG_USERS,
    params(("bot" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = BotCreated),
        (status = BAD_REQUEST, description = "`validation`: the caller owns as many bots as allowed", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No standing offer of this bot to the caller: none was made, or it expired, was withdrawn, or its giver no longer holds the bot", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn accept_bot_transfer(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(bot): Path<UserId>,
) -> ApiResult<Json<BotCreated>> {
    let (bot, token) = app::bot::accept_transfer(&state, user.id, bot).await?;
    Ok(Json(BotCreated {
        bot: bot.into(),
        token,
    }))
}

/// The offers of bots made to the caller or by them that still stand, the newest first.
#[utoipa::path(
    get,
    path = "/users/@me/bot-transfers",
    tag = TAG_USERS,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<BotTransfer>),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_bot_transfers(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
) -> ApiResult<Json<Vec<BotTransfer>>> {
    let transfers = app::bot::list_transfers(&state, user.id).await?;
    Ok(Json(transfers.into_iter().map(BotTransfer::from).collect()))
}

/// Deletes a bot: the caller's own, or, with Manage deployment settings, one whose owner is
/// gone.
#[utoipa::path(
    delete,
    path = "/bots/{bot}",
    tag = TAG_USERS,
    params(("bot" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Deleted"),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: not the caller's bot, or an ownerless one without Manage deployment settings", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_bot(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(bot): Path<UserId>,
) -> ApiResult<NoContent> {
    app::bot::delete(&state, user.id, bot).await?;
    Ok(NoContent)
}

/// A bot's settings, as its owner changes them.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BotUpdateRequest {
    /// Whether anyone allowed to add bots to a community may add it through its link, rather
    /// than its owner alone.
    pub public: Option<bool>,
}

/// Changes a bot the caller owns. Its profile is changed like anyone's, with
/// `PATCH /users/{user}`.
#[utoipa::path(
    patch,
    path = "/bots/{bot}",
    tag = TAG_USERS,
    params(("bot" = UserId, Path)),
    request_body = BotUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = User),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: not the caller's bot", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_bot(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(bot): Path<UserId>,
    Json(request): Json<BotUpdateRequest>,
) -> ApiResult<Json<User>> {
    let bot = match request.public {
        Some(public) => app::bot::set_public(&state, user.id, bot, public).await?,
        None => app::bot::read_owned(&state, user.id, bot).await?,
    };
    Ok(Json(User::from(bot)))
}

/// A bot joining a community through its link.
#[derive(Debug, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BotAddRequest {
    /// What the bot is given, on a role of its own; none when absent. Granting any takes Manage
    /// roles and Assign roles, and each must be held by the caller.
    #[serde(default)]
    pub permissions: Vec<Permission>,
}

/// Adds a bot to a community, as its link offers. The caller needs Add bots there, and the bot
/// must be public or theirs; people join with an invite instead (`PUT .../members/@me`).
#[utoipa::path(
    put,
    path = "/communities/{community}/members/{user}",
    tag = TAG_USERS,
    params(("community" = CommunityId, Path), ("user" = UserId, Path)),
    request_body = BotAddRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Added", body = UserCommunity),
        (status = OK, description = "Already a member", body = UserCommunity),
        (status = BAD_REQUEST, description = "`validation`: not a bot, or it belongs to as many communities as allowed", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: Add bots, or what granting the permissions takes, is missing; or a private bot not the caller's", body = Problem),
        (status = NOT_FOUND, description = "No such community, or no such bot", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn add_bot(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((community, bot)): Path<(CommunityId, UserId)>,
    Json(request): Json<BotAddRequest>,
) -> ApiResult<(StatusCode, Json<UserCommunity>)> {
    let (membership, created) = app::bot::add_to_community(
        &state,
        user.id,
        community,
        bot,
        from_names(&request.permissions),
    )
    .await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(membership)))
}
