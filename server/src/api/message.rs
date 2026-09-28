use crate::api::auth::SessionUser;
use crate::api::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::api::extract::{Created, Json, NoContent, Path, Query};
use crate::api::include::{IncludeSet, Included, Sideloaded, SideloadedList};
use crate::api::link_preview::LinkPreview;
use crate::api::message_enum::Message;
use crate::api::message_enum::request::{MessageCreateRequest, MessageUpdateRequest};
use crate::api::poll::{OwnWriteIn, PollVote};

/// The polls on a page of messages, with the caller's own votes and write-ins on them.
type PollSideload = (
    Vec<crate::api::message_enum::Poll>,
    Vec<PollVote>,
    Vec<OwnWriteIn>,
);
use crate::api::{API_PREFIX, GlobalServerContext, TAG_MESSAGES};
use crate::app::channel::{MAX_MESSAGES_QUERIED, MessageWindow};
use crate::app::{AttachmentId, ChannelId, MessageId, PollId, UserId};
use crate::{api, app};
use axum::extract::State;
use axum::http::StatusCode;
use rust_i18n::t;
use serde::Deserialize;
use std::collections::HashSet;
use utoipa::{IntoParams, ToSchema};

pub fn message_to_api(
    msg: app::message::Message,
    attachments: Vec<AttachmentId>,
    link_previews: Vec<LinkPreview>,
) -> Message {
    app::message::record(&msg, attachments, link_previews)
}

fn with_relations_to_api(m: app::message::MessageWithRelations) -> Message {
    message_to_api(m.message, m.attachments, m.link_previews)
}

/// Relationships a message read can sideload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum MessageInclude {
    /// The users who wrote the messages, as `included.users`. Authors whose accounts have since
    /// been deleted are omitted.
    Authors,
    /// The messages' attachments, as `included.attachments`.
    Attachments,
    /// The polls the messages show or announce, as `included.polls`, with the caller's own
    /// votes on them as `included.pollVotes`.
    Polls,
    /// The threads the messages started, as `included.channels`, for their reply summaries.
    Threads,
    /// The thread replies the messages that are echoes show, as `included.messages`.
    Echoes,
    /// The messages' reactions in brief, one summary per message and emoji, as
    /// `included.reactions`.
    Reactions,
}

/// Body of a message read; a named alias for the same reason as `api::community::CommunityRead`.
pub type MessageRead = Sideloaded<Message>;
/// Body of a message list read; see [`MessageRead`].
pub type MessageList = SideloadedList<Message>;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[serde(deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct MessageReadQuery {
    /// Related records to return alongside the message, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<MessageInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<MessageInclude>,
}

/// Loads the relationships named in `include` for every message in `messages`, one batched
/// read per relationship, run concurrently.
async fn sideload_messages(
    state: &GlobalServerContext,
    caller: UserId,
    messages: &[Message],
    include: &IncludeSet<MessageInclude>,
) -> ApiResult<Included> {
    let poll_ids: Vec<PollId> = messages
        .iter()
        .filter_map(|m| m.poll)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let (users, attachments, polls, threads, echoes, reactions) = tokio::try_join!(
        async {
            if include.contains(MessageInclude::Authors) {
                let authors: Vec<UserId> = messages
                    .iter()
                    .map(|m| m.author)
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();
                app::user::read_users(state, &authors).await.map(Some)
            } else {
                Ok(None)
            }
        },
        async {
            if include.contains(MessageInclude::Attachments) {
                let ids: Vec<AttachmentId> = messages
                    .iter()
                    .flat_map(|m| &m.attachments)
                    .copied()
                    .collect();
                app::attachment::read_attachments(state, &ids)
                    .await
                    .map(Some)
            } else {
                Ok(None)
            }
        },
        async {
            if include.contains(MessageInclude::Polls) {
                let (polls, votes, write_ins) = tokio::try_join!(
                    app::poll::read_polls(state, &poll_ids),
                    app::poll::read_votes(state, caller, &poll_ids),
                    app::poll::read_own_write_ins(state, caller, &poll_ids),
                )?;
                Ok::<Option<PollSideload>, app::Error>(Some((polls, votes, write_ins)))
            } else {
                Ok(None)
            }
        },
        async {
            if include.contains(MessageInclude::Threads) {
                let ids: Vec<ChannelId> = messages.iter().filter_map(|m| m.thread).collect();
                app::thread::read_threads(state, &ids).await.map(Some)
            } else {
                Ok(None)
            }
        },
        async {
            if include.contains(MessageInclude::Echoes) {
                let ids: Vec<MessageId> = messages.iter().filter_map(|m| m.echo_of).collect();
                app::message::read_messages(state, caller, &ids)
                    .await
                    .map(|rows| Some(rows.into_iter().map(with_relations_to_api).collect()))
            } else {
                Ok(None)
            }
        },
        async {
            if include.contains(MessageInclude::Reactions) {
                let ids: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
                app::react::read_summaries(state, caller, &ids)
                    .await
                    .map(|rows| Some(rows.into_iter().map(api::react::summary_to_api).collect()))
            } else {
                Ok(None)
            }
        },
    )?;
    let (polls, poll_votes, own_write_ins) = match polls {
        Some((polls, votes, write_ins)) => (Some(polls), Some(votes), Some(write_ins)),
        None => (None, None, None),
    };
    Ok(Included {
        users: users.map(|users| users.into_iter().map(api::user::user_to_api).collect()),
        attachments: attachments.map(|rows| {
            rows.into_iter()
                .map(|row| api::attachment::attachment_to_api(state, row))
                .collect()
        }),
        polls,
        poll_votes,
        own_write_ins,
        channels: threads,
        messages: echoes,
        reactions,
        ..Included::default()
    })
}

#[utoipa::path(
    post,
    path = "/channels/{channel}/messages",
    tag = TAG_MESSAGES,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = Message, headers(("Location" = String, description = "URL of the new message"))),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (an attachment is not ready, or `echoToParent` outside a thread)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, description = "No such channel, or a DM the caller is not in", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Json(request): Json<MessageCreateRequest>,
) -> ApiResult<Created<Message>> {
    let msg = app::message::create_message(
        &state,
        user.id,
        channel,
        request.content,
        request.attachments.clone(),
        request.echo_to_parent.unwrap_or(false),
    )
    .await?;
    let location = format!("{API_PREFIX}/messages/{}", msg.id.0);
    // Freshly-created messages always ship with an empty preview list; the async fetcher's
    // `Update` event will populate the final set shortly.
    Ok(Created::new(
        location,
        message_to_api(msg, request.attachments, Vec::new()),
    ))
}

const DEFAULT_MESSAGE_LIMIT: u32 = 50;

/// Selects a window of a channel's history. At most one of `before`, `after`, and `around` may be
/// given; with none of them the newest messages are returned.
#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[into_params(parameter_in = Query)]
pub struct MessageListQuery {
    /// Return messages older than this message id (exclusive).
    pub before: Option<MessageId>,
    /// Return messages newer than this message id (exclusive).
    pub after: Option<MessageId>,
    /// Return this message and up to `limit` messages on each side of it.
    pub around: Option<MessageId>,
    /// Maximum number of messages to return (per side, for `around`). Defaults to 50.
    #[param(minimum = 1, maximum = 200)]
    pub limit: Option<u32>,
    /// Related records to return alongside the messages, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<MessageInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<MessageInclude>,
}

impl MessageListQuery {
    fn into_window(self) -> ApiResult<MessageWindow> {
        let limit = self.limit.unwrap_or(DEFAULT_MESSAGE_LIMIT);
        if limit == 0 || limit > MAX_MESSAGES_QUERIED {
            return Err(ApiError::new(ProblemCode::Validation)
                .with_detail(t!("messageLimitOutOfRange", max = MAX_MESSAGES_QUERIED)));
        }
        match (self.before, self.after, self.around) {
            (None, None, None) => Ok(MessageWindow::Latest { limit }),
            (Some(anchor), None, None) => Ok(MessageWindow::Before { anchor, limit }),
            (None, Some(anchor), None) => Ok(MessageWindow::After { anchor, limit }),
            (None, None, Some(anchor)) => Ok(MessageWindow::Around {
                anchor,
                radius: limit,
            }),
            _ => Err(ApiError::new(ProblemCode::Validation)
                .with_detail(t!("messageWindowMultipleAnchors"))),
        }
    }
}

/// Messages are returned newest first for `before` and the default window, oldest first for
/// `after`, and in ascending id order for `around`. `include` sideloads the messages' authors,
/// attachments, polls, the threads they started, and the replies their echoes show.
#[utoipa::path(
    get,
    path = "/channels/{channel}/messages",
    tag = TAG_MESSAGES,
    params(("channel" = ChannelId, Path), MessageListQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = MessageList),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_channel_messages(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Query(query): Query<MessageListQuery>,
) -> ApiResult<Json<MessageList>> {
    let include = query.include.clone();
    let window = query.into_window()?;
    let messages: Vec<Message> =
        app::channel::read_channel_messages(&state, user.id, channel, window)
            .await?
            .into_iter()
            .map(with_relations_to_api)
            .collect();
    let included = sideload_messages(&state, user.id, &messages, &include).await?;
    Ok(Json(MessageList::new(messages, included)))
}

/// Reads a message. `include` sideloads its author, attachments, and poll.
#[utoipa::path(
    get,
    path = "/messages/{message}",
    tag = TAG_MESSAGES,
    params(("message" = MessageId, Path), MessageReadQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = MessageRead),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
    Query(query): Query<MessageReadQuery>,
) -> ApiResult<Json<MessageRead>> {
    let m = with_relations_to_api(app::message::read_message(&state, user.id, message).await?);
    let included =
        sideload_messages(&state, user.id, std::slice::from_ref(&m), &query.include).await?;
    Ok(Json(MessageRead::new(m, included)))
}

#[utoipa::path(
    patch,
    path = "/messages/{message}",
    tag = TAG_MESSAGES,
    params(("message" = MessageId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Message),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
    Json(request): Json<MessageUpdateRequest>,
) -> ApiResult<Json<Message>> {
    let m = app::message::update_message(&state, user.id, message, request).await?;
    Ok(Json(with_relations_to_api(m)))
}

#[utoipa::path(
    delete,
    path = "/messages/{message}",
    tag = TAG_MESSAGES,
    params(("message" = MessageId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
) -> ApiResult<NoContent> {
    app::message::delete_message(&state, user.id, message).await?;
    Ok(NoContent)
}

/// Opens the thread a message started: made the first time (`201`), returned as it is after
/// that (`200`). A message in a thread, and an echo, cannot start one.
#[utoipa::path(
    put,
    path = "/messages/{message}/thread",
    tag = TAG_MESSAGES,
    params(("message" = MessageId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "The thread, just made", body = crate::api::message_enum::Channel),
        (status = OK, description = "The thread the message already started", body = crate::api::message_enum::Channel),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (the message is in a thread, is an echo, or is in a voice channel)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, description = "No such message, or one in a DM the caller is not in", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn open_thread(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
) -> ApiResult<(StatusCode, Json<crate::api::message_enum::Channel>)> {
    let (thread, created) = app::thread::open_thread(&state, user.id, message).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(app::channel::record(&thread, Vec::new()))))
}

/// Pins a message in its channel, after every pin already there: `201` when it was not pinned,
/// `200` when it was. In a community this takes Pin messages; in a DM any recipient may.
#[utoipa::path(
    put,
    path = "/messages/{message}/pin",
    tag = TAG_MESSAGES,
    params(("message" = MessageId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Pinned", body = crate::api::message_enum::Pin),
        (status = OK, description = "Already pinned", body = crate::api::message_enum::Pin),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn pin_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
) -> ApiResult<(StatusCode, Json<crate::api::message_enum::Pin>)> {
    let (pin, created) = app::message::set_pinned(&state, user.id, message, true).await?;
    let pin = pin.ok_or(app::Error::Diesel(diesel::result::Error::NotFound))?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(crate::api::message_enum::Pin {
            message_id: pin.message_id,
            timestamp: pin.timestamp,
            sort_index: pin.sort_index,
        }),
    ))
}

/// Unpins a message, on the same terms as pinning it. Unpinning one that is not pinned still
/// yields `204`.
#[utoipa::path(
    delete,
    path = "/messages/{message}/pin",
    tag = TAG_MESSAGES,
    params(("message" = MessageId, Path)),
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
pub async fn unpin_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
) -> ApiResult<NoContent> {
    app::message::set_pinned(&state, user.id, message, false).await?;
    Ok(NoContent)
}
