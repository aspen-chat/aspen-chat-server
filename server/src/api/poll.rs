//! Polls are opened with `POST /channels/{channel}/polls`, which also posts the message of kind
//! `poll` that shows them. Votes are a set keyed by (poll, option, user), so casting one is an
//! idempotent `PUT` on `/polls/{poll}/votes/{option}/@me` and withdrawing one a `DELETE` on the
//! same URL; only the calling user's own votes can be addressed. The poll's tally travels in
//! its record and is republished with every change, so there is no endpoint for it.

use crate::api::auth::SessionUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Created, Json, NoContent, Path, Query};
use crate::api::include::{IncludeSet, Included, Sideloaded};
use crate::api::message_enum::Poll;
use crate::api::message_enum::request::PollCreateRequest;
use crate::api::{API_PREFIX, GlobalServerContext, TAG_POLLS};
use crate::app;
use crate::app::{ChannelId, PollId, UserId};
use axum::extract::State;
use axum::http::StatusCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// The tally for one option of a poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PollOptionResult {
    pub count: u32,
    /// Who voted for the option, oldest vote first. Absent on an anonymous poll.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voters: Option<Vec<UserId>>,
}

/// One choice on a poll: its text and, optionally, an emoji shown beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PollOption {
    pub label: String,
    /// A single emoji, or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

/// One of the calling user's votes: the option they chose on a poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PollVote {
    pub poll: PollId,
    pub option: u32,
}

/// Relationships a poll read can sideload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum PollInclude {
    /// The calling user's own votes on the poll, as `included.pollVotes`.
    Votes,
}

/// Body of a poll read; a named alias for the same reason as `api::community::CommunityRead`.
pub type PollRead = Sideloaded<Poll>;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct PollReadQuery {
    /// Related records to return alongside the poll, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<PollInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<PollInclude>,
}

/// Opens a poll. The message showing it is delivered on the event stream.
#[utoipa::path(
    post,
    path = "/channels/{channel}/polls",
    tag = TAG_POLLS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = Poll, headers(("Location" = String, description = "URL of the new poll"))),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (question, options, or duration out of bounds)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_poll(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Json(request): Json<PollCreateRequest>,
) -> ApiResult<Created<Poll>> {
    let (poll, _) = app::poll::create_poll(&state, user.id, channel, request).await?;
    let location = format!("{API_PREFIX}/polls/{}", poll.id.0);
    Ok(Created::new(location, poll))
}

/// Reads a poll with its current tally. `include` sideloads the caller's own votes.
#[utoipa::path(
    get,
    path = "/polls/{poll}",
    tag = TAG_POLLS,
    params(("poll" = PollId, Path), PollReadQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = PollRead),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_poll(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(poll): Path<PollId>,
    Query(query): Query<PollReadQuery>,
) -> ApiResult<Json<PollRead>> {
    let record = app::poll::read_poll(&state, poll).await?;
    let poll_votes = if query.include.contains(PollInclude::Votes) {
        Some(app::poll::read_votes(&state, user.id, &[poll]).await?)
    } else {
        None
    };
    Ok(Json(PollRead::new(
        record,
        Included {
            poll_votes,
            ..Included::default()
        },
    )))
}

/// Casts a vote. On a single-choice poll this replaces any earlier vote by the caller.
#[utoipa::path(
    put,
    path = "/polls/{poll}/votes/{option}/@me",
    tag = TAG_POLLS,
    params(
        ("poll" = PollId, Path),
        ("option" = u32, Path, description = "Index into the poll's `options`"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Vote cast; the poll with its updated tally", body = Poll),
        (status = OK, description = "Vote already cast", body = Poll),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (no such option)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`pollClosed`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn add_vote(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((poll, option)): Path<(PollId, u32)>,
) -> ApiResult<(StatusCode, Json<Poll>)> {
    let (created, record) = app::poll::add_vote(&state, user.id, poll, option).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(record)))
}

/// Withdraws a vote. Succeeds whether or not the caller had voted for the option.
#[utoipa::path(
    delete,
    path = "/polls/{poll}/votes/{option}/@me",
    tag = TAG_POLLS,
    params(
        ("poll" = PollId, Path),
        ("option" = u32, Path, description = "Index into the poll's `options`"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`pollClosed`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_vote(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((poll, option)): Path<(PollId, u32)>,
) -> ApiResult<NoContent> {
    app::poll::remove_vote(&state, user.id, poll, option).await?;
    Ok(NoContent)
}
