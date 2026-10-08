//! Reactions are modelled as a set keyed by (message, emoji, user), so adding one is an
//! idempotent `PUT` on `/messages/{message}/reactions/{emoji}/@me` and removing one is a
//! `DELETE` on the same URL. Only the calling user's own reaction can be addressed. An emoji
//! is stored in the fully qualified form Unicode lists for it, however it arrives, so `❤` and
//! `❤️` are one reaction; records and events name it in that form.
//!
//! Message reads sideload each message's reactions in brief (`include=reactions`): per emoji,
//! the count, whether the caller reacted, and the first few to react. Everyone who reacted with
//! an emoji is `GET /messages/{message}/reactions/{emoji}`, earliest first, a page at a time.
//! The emoji someone reacts with most, which their quick reactions offer, are
//! `GET /users/@me/frequent-emoji`.

use crate::TAG_REACTIONS;
use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Json, NoContent, Path, Query};
use crate::message_enum::{React, User};
use crate::user::{UserRef, not_your_account};
use aspen_app as app;
use aspen_app::context::GlobalServerContext;
use aspen_app::{CommunityId, MessageId, UserId};
use axum::extract::State;
use axum::http::StatusCode;
use diesel::result::DatabaseErrorKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// One emoji's reactions to a message, in brief.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReactionSummary {
    pub message_id: MessageId,
    pub emoji: String,
    /// How many people reacted with it.
    pub count: u32,
    /// Whether the caller is one of them.
    pub me: bool,
    /// The first of them to react, earliest first, at most `app::react::SUMMARY_USERS` (four).
    pub users: Vec<UserId>,
}

impl From<app::react::ReactionSummary> for ReactionSummary {
    fn from(summary: app::react::ReactionSummary) -> Self {
        ReactionSummary {
            message_id: summary.message,
            emoji: summary.emoji,
            count: summary.count,
            me: summary.me,
            users: summary.first,
        }
    }
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReactorsQuery {
    /// Continue after this person, the last of the previous page.
    pub after: Option<UserId>,
    /// How many to return, at most 100; 50 when absent.
    pub limit: Option<u32>,
}

/// Everyone who reacted to a message with an emoji, earliest first, a page at a time. A page
/// shorter than `limit` is the last.
#[utoipa::path(
    get,
    path = "/messages/{message}/reactions/{emoji}",
    tag = TAG_REACTIONS,
    params(
        ("message" = MessageId, Path),
        ("emoji" = String, Path, description = "A single Unicode emoji, percent-encoded"),
        ReactorsQuery,
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<User>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such message, or one in a DM the caller is not in", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_reactors(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((message, emoji)): Path<(MessageId, String)>,
    Query(query): Query<ReactorsQuery>,
) -> ApiResult<Json<Vec<User>>> {
    let ids = app::react::read_reactors(
        &state,
        user.id,
        message,
        &emoji,
        query.after,
        query.limit.unwrap_or(50),
    )
    .await?;
    // Read in one batch, then put back in the list's order.
    let mut users: std::collections::HashMap<UserId, User> =
        app::user::read_users(&state, user.id, &ids)
            .await?
            .into_iter()
            .map(|u| {
                let record = User::from(u);
                (record.id, record)
            })
            .collect();
    Ok(Json(ids.iter().filter_map(|id| users.remove(id)).collect()))
}

#[utoipa::path(
    put,
    path = "/messages/{message}/reactions/{emoji}/@me",
    tag = TAG_REACTIONS,
    params(
        ("message" = MessageId, Path),
        ("emoji" = String, Path, description = "A single Unicode emoji, percent-encoded"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Reaction added", body = React),
        (status = OK, description = "Reaction already present", body = React),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (not a single emoji, or a message already carrying 50 different reactions)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing; `blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn add_reaction(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((message, emoji)): Path<(MessageId, String)>,
) -> ApiResult<(StatusCode, Json<React>)> {
    // The reaction as stored, its emoji in canonical form (or a custom emoji's reference);
    // anything else is refused by `create_react` before the record is used.
    let record = React {
        message_id: message,
        emoji: app::react::stored_key(&emoji).unwrap_or_else(|| emoji.clone()),
        user_id: user.id,
    };
    match app::react::create_react(&state, user.id, message, emoji).await {
        Ok(_) => Ok((StatusCode::CREATED, Json(record))),
        Err(app::Error::Diesel(diesel::result::Error::DatabaseError(
            DatabaseErrorKind::UniqueViolation,
            _,
        ))) => Ok((StatusCode::OK, Json(record))),
        Err(e) => Err(e.into()),
    }
}

#[utoipa::path(
    delete,
    path = "/messages/{message}/reactions/{emoji}/@me",
    tag = TAG_REACTIONS,
    params(
        ("message" = MessageId, Path),
        ("emoji" = String, Path, description = "A single Unicode emoji, percent-encoded"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_reaction(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((message, emoji)): Path<(MessageId, String)>,
) -> ApiResult<NoContent> {
    app::react::delete_react(&state, user.id, message, emoji).await?;
    Ok(NoContent)
}

/// Takes someone's reaction off a message. Taking another person's takes Manage messages.
#[utoipa::path(
    delete,
    path = "/messages/{message}/reactions/{emoji}/{user}",
    tag = TAG_REACTIONS,
    params(
        ("message" = MessageId, Path),
        ("emoji" = String, Path, description = "A single Unicode emoji, percent-encoded"),
        ("user" = UserId, Path, description = "Whose reaction"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: lacks Manage messages", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_users_reaction(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((message, emoji, author)): Path<(MessageId, String, UserId)>,
) -> ApiResult<NoContent> {
    app::react::remove_others_react(&state, user.id, message, emoji, author).await?;
    Ok(NoContent)
}

/// One emoji the caller reacts with, and how often they have.
#[derive(Debug, Clone, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FrequentEmoji {
    /// A Unicode emoji in canonical form, or a custom emoji's reference (`<:id>`).
    pub emoji: String,
    /// How many of the caller's reactions use it from the past 90 days.
    pub recent_uses: u32,
    /// How many of the caller's latest 10,000 reactions use it.
    pub uses: u32,
}

impl From<app::react::FrequentEmoji> for FrequentEmoji {
    fn from(frequent: app::react::FrequentEmoji) -> Self {
        FrequentEmoji {
            emoji: frequent.emoji,
            recent_uses: frequent.recent,
            uses: frequent.total,
        }
    }
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct FrequentEmojiQuery {
    /// The community the emoji are for: its own custom emoji are among them. Without one (a
    /// DM), only Unicode emoji are.
    pub community: Option<CommunityId>,
    /// How many to return, at most 20; 5 when absent.
    pub limit: Option<u32>,
}

/// The emoji the caller reacts with most: first those used in the past 90 days, most used
/// first, then the rest by how often they were used. Counted from the caller's latest 10,000
/// reactions on messages now. Only the caller may read their own.
#[utoipa::path(
    get,
    path = "/users/{user}/frequent-emoji",
    tag = TAG_REACTIONS,
    params(("user" = inline(UserRef), Path), FrequentEmojiQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<FrequentEmoji>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "Only the user themself may read their most used emoji", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_frequent_emoji(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path(user): Path<UserRef>,
    Query(query): Query<FrequentEmojiQuery>,
) -> ApiResult<Json<Vec<FrequentEmoji>>> {
    let user_id = user.resolve(&session);
    let frequent = app::react::read_frequent(
        &state,
        session.user.id,
        user_id,
        query.community,
        query.limit.unwrap_or(5),
    )
    .await
    .map_err(not_your_account)?;
    Ok(Json(
        frequent.into_iter().map(FrequentEmoji::from).collect(),
    ))
}
