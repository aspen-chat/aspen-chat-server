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
use crate::api::{API_PREFIX, TAG_POLLS};
use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::{ChannelId, PollId, UserId};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
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
/// An answer a voter added to a poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PollWriteIn {
    pub label: String,
    /// Who wrote it in. Absent on an anonymous poll, where it would say how they voted, and
    /// once their account is gone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written_by: Option<UserId>,
}

/// One of the calling user's own write-ins: the poll and the answer's index. Sent only to the
/// writer, since an anonymous poll's record does not say who wrote what.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct OwnWriteIn {
    pub poll: PollId,
    pub option: u32,
}

/// What adding a written-in answer did: the answer the caller's vote went to, a new one or the
/// poll's matching answer, and the poll with its updated tally.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WriteInResult {
    pub option: u32,
    pub poll: Poll,
}

/// A voter's own answer to add to a poll.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WriteInRequest {
    pub label: String,
}

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
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing; `blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = UNPROCESSABLE_ENTITY, description = "`pluginRefused`: a plugin refused the question or answers, or would have changed them", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
        (status = SERVICE_UNAVAILABLE, description = "`pluginUnavailable`: a plugin that must decide what is posted here could not", body = Problem),
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
    let record = app::poll::read_poll(&state, user.id, poll).await?;
    let (poll_votes, own_write_ins) = if query.include.contains(PollInclude::Votes) {
        let ids = [poll];
        let (votes, write_ins) = tokio::try_join!(
            app::poll::read_votes(&state, user.id, &ids),
            app::poll::read_own_write_ins(&state, user.id, &ids),
        )?;
        (Some(votes), Some(write_ins))
    } else {
        (None, None)
    };
    Ok(Json(PollRead::new(
        record,
        Included {
            poll_votes,
            own_write_ins,
            ..Included::default()
        },
    )))
}

/// Closes a poll before its deadline, as the deadline would: the final tally is published and
/// the poll-closed message posted. The poll's creator may, and so may a holder of Manage
/// messages in its channel. A poll already closed is answered as it stands.
#[utoipa::path(
    post,
    path = "/polls/{poll}/close",
    tag = TAG_POLLS,
    params(("poll" = PollId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Poll),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: neither the creator nor a holder of Manage messages", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn close_poll(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(poll): Path<PollId>,
) -> ApiResult<Json<Poll>> {
    Ok(Json(app::poll::close_poll(&state, user.id, poll).await?))
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
        (status = FORBIDDEN, description = "`blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
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

/// Withdraws a vote. Succeeds whether or not the caller had voted for the option, in a channel
/// they may view, as voting does.
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
        (status = FORBIDDEN, description = "`blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
        (status = NOT_FOUND, description = "No such poll, or one in a channel the caller may not view", body = Problem),
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

/// Adds the caller's own answer to a poll that allows write-ins, and votes for it for them,
/// which takes posting in its channel. An answer the poll already has (ignoring case and spacing) is voted for instead of added, and
/// does not use up the caller's one write-in.
#[utoipa::path(
    post,
    path = "/polls/{poll}/write-ins",
    tag = TAG_POLLS,
    params(("poll" = PollId, Path)),
    request_body = WriteInRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Added, with the caller's vote", body = WriteInResult, headers(("Location" = String, description = "URL of the new answer"))),
        (status = OK, description = "The poll already had this answer; the caller's vote went to it", body = WriteInResult),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (write-ins not allowed, answer length, or the poll holds all it may)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden` without Send messages (or Send in threads); `blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = CONFLICT, description = "`pollClosed`, or `conflict`: the caller already has a write-in on this poll", body = Problem),
        (status = UNPROCESSABLE_ENTITY, description = "`pluginRefused`: a plugin refused the answer, or would have changed it", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
        (status = SERVICE_UNAVAILABLE, description = "`pluginUnavailable`: a plugin that must decide what is posted here could not", body = Problem),
    )
)]
pub async fn add_write_in(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(poll): Path<PollId>,
    Json(request): Json<WriteInRequest>,
) -> ApiResult<Response> {
    let (outcome, record) = app::poll::write_in(&state, user.id, poll, &request.label).await?;
    Ok(match outcome {
        app::poll::WriteInOutcome::Added(option) => Created::new(
            format!("{API_PREFIX}/polls/{}/write-ins/{option}", poll.0),
            WriteInResult {
                option,
                poll: record,
            },
        )
        .into_response(),
        app::poll::WriteInOutcome::Existing(option) => (
            StatusCode::OK,
            Json(WriteInResult {
                option,
                poll: record,
            }),
        )
            .into_response(),
    })
}

/// Removes a written-in answer and every vote for it. Its index stays empty (`null` in
/// `writeIns`), so no other answer's index changes. Its writer and the poll's creator may, and
/// so may anyone with Manage messages.
#[utoipa::path(
    delete,
    path = "/polls/{poll}/write-ins/{option}",
    tag = TAG_POLLS,
    params(
        ("poll" = PollId, Path),
        ("option" = u32, Path, description = "The answer's index: `options.len()` plus its place in `writeIns`"),
    ),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT, description = "Removed"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, description = "No such standing write-in", body = Problem),
        (status = CONFLICT, description = "`pollClosed`", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn remove_write_in(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((poll, option)): Path<(PollId, u32)>,
) -> ApiResult<NoContent> {
    app::poll::remove_write_in(&state, user.id, poll, option).await?;
    Ok(NoContent)
}
