use crate::auth::SessionUser;
use crate::error::{ApiError, ApiResult, Problem, ProblemCode};
use crate::extract::{Created, Json, NoContent, Path, Query};
use crate::include::{IncludeSet, Included, Sideloaded, SideloadedList};
use crate::link_preview::LinkPreview;
use crate::message_enum::request::{MessageCreateRequest, MessageUpdateRequest};
use crate::message_enum::{Message, UserCommunity};
use crate::poll::{OwnWriteIn, PollVote};

/// The polls on a page of messages, with the caller's own votes and write-ins on them.
type PollSideload = (
    Vec<crate::message_enum::Poll>,
    Vec<PollVote>,
    Vec<OwnWriteIn>,
);
use crate::t;
use crate::{API_PREFIX, TAG_MESSAGES};
use aspen_app as app;
use aspen_app::channel::{MAX_MESSAGES_QUERIED, MessageWindow};
use aspen_app::context::GlobalServerContext;
use aspen_app::{AttachmentId, ChannelId, CommunityId, HeldMessageId, MessageId, PollId, UserId};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::{IntoParams, ToSchema};

pub fn message_to_api(
    msg: app::message::Message,
    attachments: Vec<AttachmentId>,
    link_previews: Vec<LinkPreview>,
) -> Message {
    app::message::record(&msg, attachments, link_previews)
}

/// Relationships a message read can sideload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum MessageInclude {
    /// The users who wrote the messages, as `included.users`. Authors whose accounts have since
    /// been deleted are omitted.
    Authors,
    /// The users the messages tag by name (`mentions.users`), as `included.users`, so that a
    /// reader with no other source of names, a phone's notification code say, can show a tag
    /// as a name.
    Mentions,
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
    /// The channels the messages were posted in, as `included.channels`: what a search's
    /// results need to say where each was said, threads among them.
    Channels,
    /// What the caller finds at the messages the messages link to, as `included.linkedMessages`,
    /// with those they may read as `included.messages`. Their authors and attachments come
    /// with the messages' own when those are asked for.
    Linked,
    /// The messages the messages that are warnings are about, deleted or not, as
    /// `included.warnedMessages`; only the people of a warning's DM read it. Their authors and
    /// attachments come with the messages' own when those are asked for.
    Warnings,
    /// The authors' memberships of the communities the messages were posted in, with the roles
    /// each holds there, as `included.userCommunities`, so a reader can draw each author's name
    /// in their roles' colour. Authors of DMs, and those who have left, have none.
    Memberships,
    /// What plugins say about the messages, and about the other messages the read names, as
    /// `included.messageAnnotations`.
    Annotations,
}

/// Body of a message read; a named alias for the same reason as `api::community::CommunityRead`.
pub type MessageRead = Sideloaded<Message>;
/// Body of a message list read; see [`MessageRead`].
pub type MessageList = SideloadedList<Message>;

#[derive(Debug, Default, Deserialize, IntoParams)]
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
    // The other messages the read names come first, since their authors and attachments are
    // sideloaded with the read's own.
    let (echoes, linked, warned) = tokio::try_join!(
        async {
            if include.contains(MessageInclude::Echoes) {
                let ids: Vec<MessageId> = messages.iter().filter_map(|m| m.echo_of).collect();
                app::message::read_messages(state, caller, &ids)
                    .await
                    .map(|rows| Some(rows.into_iter().map(Message::from).collect::<Vec<_>>()))
            } else {
                Ok(None)
            }
        },
        async {
            if include.contains(MessageInclude::Linked) {
                let links: Vec<MessageId> = messages
                    .iter()
                    .flat_map(|m| m.linked_messages.iter().copied())
                    .collect();
                app::message_link::read_linked(state, caller, &links)
                    .await
                    .map(|(linked, rows)| {
                        Some((
                            linked,
                            rows.into_iter().map(Message::from).collect::<Vec<_>>(),
                        ))
                    })
            } else {
                Ok(None)
            }
        },
        async {
            if include.contains(MessageInclude::Warnings) {
                app::report::warned_messages(state, messages)
                    .await
                    .map(|rows| {
                        Some(
                            rows.into_iter()
                                .map(crate::report::ReviewedMessage::from)
                                .collect::<Vec<_>>(),
                        )
                    })
            } else {
                Ok(None)
            }
        },
    )?;
    let (linked_messages, linked_records) = match linked {
        Some((linked, records)) => (Some(linked), records),
        None => (None, Vec::new()),
    };
    let named: Vec<&Message> = messages
        .iter()
        .chain(echoes.iter().flatten())
        .chain(linked_records.iter())
        .chain(warned.iter().flatten().map(|w| &w.message))
        .collect();
    let (users, attachments, polls, threads, channels, reactions, memberships, annotations) = tokio::try_join!(
        async {
            let authors = include.contains(MessageInclude::Authors);
            let mentions = include.contains(MessageInclude::Mentions);
            if authors || mentions {
                let users: Vec<UserId> = named
                    .iter()
                    .flat_map(|m| {
                        let author = authors.then_some(m.author);
                        let tagged = mentions.then(|| m.mentions.users.iter().copied());
                        author.into_iter().chain(tagged.into_iter().flatten())
                    })
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();
                app::user::read_users(state, caller, &users).await.map(Some)
            } else {
                Ok(None)
            }
        },
        async {
            if include.contains(MessageInclude::Attachments) {
                let ids: Vec<AttachmentId> =
                    named.iter().flat_map(|m| &m.attachments).copied().collect();
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
            if include.contains(MessageInclude::Channels) {
                let ids: Vec<ChannelId> = messages
                    .iter()
                    .map(|m| m.channel_id)
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();
                app::channel::read_channels(state, &ids).await.map(Some)
            } else {
                Ok(None)
            }
        },
        async {
            if include.contains(MessageInclude::Reactions) {
                let ids: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
                app::react::read_summaries(state, caller, &ids)
                    .await
                    .map(|rows| {
                        Some(
                            rows.into_iter()
                                .map(crate::react::ReactionSummary::from)
                                .collect(),
                        )
                    })
            } else {
                Ok(None)
            }
        },
        async {
            if include.contains(MessageInclude::Memberships) {
                let written: Vec<(ChannelId, UserId)> = named
                    .iter()
                    .map(|m| (m.channel_id, m.author))
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();
                app::community::read_authors_memberships(state, caller, &written)
                    .await
                    .map(|rows| Some(rows.iter().map(UserCommunity::from).collect()))
            } else {
                Ok(None)
            }
        },
        async {
            if include.contains(MessageInclude::Annotations) {
                // Every message named here is one the caller may read.
                let ids: Vec<MessageId> = named
                    .iter()
                    .map(|m| m.id)
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();
                app::plugin::annotation::of_messages(state, &ids)
                    .await
                    .map(Some)
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
        users: users.map(|users| {
            users
                .into_iter()
                .map(crate::message_enum::User::from)
                .collect()
        }),
        attachments: attachments.map(|rows| {
            rows.into_iter()
                .map(|row| crate::attachment::attachment_to_api(state, row))
                .collect()
        }),
        polls,
        poll_votes,
        own_write_ins,
        // Both are channel records; a thread the messages started and were posted in is listed
        // once.
        channels: match (threads, channels) {
            (Some(mut threads), Some(channels)) => {
                let started: HashSet<ChannelId> = threads.iter().map(|t| t.id).collect();
                threads.extend(channels.into_iter().filter(|c| !started.contains(&c.id)));
                Some(threads)
            }
            (threads, channels) => threads.or(channels),
        },
        // Echoed replies and linked messages are both message records; one both echoed and
        // linked is listed once.
        messages: match (echoes, linked_messages.is_some()) {
            (None, false) => None,
            (echoes, _) => {
                let mut records = echoes.unwrap_or_default();
                let listed: HashSet<MessageId> = records.iter().map(|m| m.id).collect();
                records.extend(
                    linked_records
                        .iter()
                        .filter(|m| !listed.contains(&m.id))
                        .cloned(),
                );
                Some(records)
            }
        },
        linked_messages,
        warned_messages: warned,
        reactions,
        user_communities: memberships,
        message_annotations: annotations,
        ..Included::default()
    })
}

/// Something a found message may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum MessageHolding {
    /// Any attachment.
    Attachment,
    /// An attachment that is a picture.
    Image,
    /// A poll.
    Poll,
}

impl From<MessageHolding> for app::search::Holding {
    fn from(holding: MessageHolding) -> Self {
        match holding {
            MessageHolding::Attachment => Self::Attachment,
            MessageHolding::Image => Self::Image,
            MessageHolding::Poll => Self::Poll,
        }
    }
}

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MessageSearchQuery {
    /// Words the messages contain, in the syntax of web search engines: every word must
    /// appear, `"a phrase"` must appear as written, `or` accepts either side, and `-word`
    /// leaves out messages with it. Words are compared ignoring case, and whole, in any
    /// language. At most 200 characters.
    #[serde(rename = "filter[text]")]
    #[param(rename = "filter[text]")]
    pub text: Option<String>,
    /// Only messages in this community.
    #[serde(rename = "filter[community]")]
    #[param(rename = "filter[community]")]
    pub community: Option<CommunityId>,
    /// Only messages in this channel or DM, or its threads.
    #[serde(rename = "filter[channel]")]
    #[param(rename = "filter[channel]")]
    pub channel: Option<ChannelId>,
    /// Only messages written by this user.
    #[serde(rename = "filter[author]")]
    #[param(rename = "filter[author]")]
    pub author: Option<UserId>,
    /// Only messages tagging this user by name.
    #[serde(rename = "filter[mentions]")]
    #[param(rename = "filter[mentions]")]
    pub mentions: Option<UserId>,
    /// Only messages holding each of these, comma separated.
    #[serde(rename = "filter[has]", default)]
    #[param(rename = "filter[has]", value_type = Option<Vec<MessageHolding>>, style = Form, explode = false)]
    pub has: IncludeSet<MessageHolding>,
    /// Only messages older than this one: the last of the previous page.
    pub before: Option<MessageId>,
    /// How many to return, at most 50; 25 when absent.
    pub limit: Option<u32>,
    /// Related records to return alongside the messages, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<MessageInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<MessageInclude>,
}

/// How many messages a search returns when it does not say.
const DEFAULT_SEARCH_RESULTS: u32 = 25;

/// Searches the messages the caller may read, newest first: the channels they may view in the
/// communities they belong to, the DMs they are in, and the threads of both, leaving out
/// messages by anyone they blocked. `filter[community]` or `filter[channel]` narrows where;
/// `filter[text]`, `filter[author]`, `filter[mentions]`, and `filter[has]` narrow what, and at
/// least one of them must be given. A community the caller does not belong to, or a channel they
/// may not read, is answered `404`.
#[utoipa::path(
    get,
    path = "/messages",
    tag = TAG_MESSAGES,
    params(MessageSearchQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = MessageList),
        (status = BAD_REQUEST, description = "`validation`: nothing to search for, text too long, or both a community and a channel", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn search_messages(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Query(query): Query<MessageSearchQuery>,
) -> ApiResult<Json<MessageList>> {
    let scope = match (query.community, query.channel) {
        (None, None) => app::search::SearchScope::Everywhere,
        (Some(community), None) => app::search::SearchScope::Community(community),
        (None, Some(channel)) => app::search::SearchScope::Channel(channel),
        (Some(_), Some(_)) => {
            return Err(
                ApiError::new(ProblemCode::Validation).with_detail(t!("searchCommunityAndChannel"))
            );
        }
    };
    let holding = [
        MessageHolding::Attachment,
        MessageHolding::Image,
        MessageHolding::Poll,
    ]
    .into_iter()
    .filter(|h| query.has.contains(*h))
    .map(app::search::Holding::from)
    .collect();
    let messages: Vec<Message> = app::search::search_messages(
        &state,
        user.id,
        app::search::MessageSearch {
            text: query.text,
            scope,
            author: query.author,
            mentions: query.mentions,
            holding,
            before: query.before,
            limit: query.limit.unwrap_or(DEFAULT_SEARCH_RESULTS),
        },
    )
    .await?
    .into_iter()
    .map(Message::from)
    .collect();
    let included = sideload_messages(&state, user.id, &messages, &query.include).await?;
    Ok(Json(MessageList::new(messages, included)))
}

/// A message held while a preview of one of its attachments is being made, as its author's
/// apps show it waiting (`app::message::held`). `heldMessagePosted` names the message it
/// becomes; `heldMessageFailed` says why it was dropped instead.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct HeldMessage {
    pub id: HeldMessageId,
    pub channel_id: ChannelId,
    pub content: String,
    pub attachments: Vec<AttachmentId>,
    pub echo_to_parent: bool,
    pub held_at: DateTime<Utc>,
}

impl From<app::message::held::HeldMessage> for HeldMessage {
    fn from(held: app::message::held::HeldMessage) -> Self {
        HeldMessage {
            id: held.id,
            channel_id: held.channel,
            content: held.content,
            attachments: held.attachments,
            echo_to_parent: held.echo_to_parent,
            held_at: held.held_at,
        }
    }
}

#[utoipa::path(
    post,
    path = "/channels/{channel}/messages",
    tag = TAG_MESSAGES,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = Message, headers(("Location" = String, description = "URL of the new message"))),
        (status = ACCEPTED, description = "Held, when `mayHold` was given, while a preview of one of its attachments is being made; it is posted later", body = HeldMessage),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (the text is over `app::message::MAX_CONTENT_CHARS` characters, an attachment is not ready, or `echoToParent` outside a thread)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing; `blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
        (status = NOT_FOUND, description = "No such channel, or a DM the caller is not in", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Json(request): Json<MessageCreateRequest>,
) -> ApiResult<Response> {
    let posted = app::message::held::post(
        &state,
        user.id,
        channel,
        request.content,
        request.attachments.clone(),
        request.echo_to_parent.unwrap_or(false),
        request.may_hold.unwrap_or(false),
    )
    .await?;
    let msg = match posted {
        app::message::held::Posted::Sent(msg) => *msg,
        app::message::held::Posted::Held(held) => {
            return Ok((StatusCode::ACCEPTED, Json(HeldMessage::from(held))).into_response());
        }
    };
    let location = format!("{API_PREFIX}/messages/{}", msg.id.0);
    // Freshly-created messages always ship with an empty preview list; the async fetcher's
    // `Update` event will populate the final set shortly.
    Ok(Created::new(
        location,
        message_to_api(msg, request.attachments, Vec::new()),
    )
    .into_response())
}

/// The caller's messages held for their attachments' previews, oldest first, which their apps
/// show waiting until each is posted.
#[utoipa::path(
    get,
    path = "/users/@me/held-messages",
    tag = TAG_MESSAGES,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<HeldMessage>),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_held_messages(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
) -> ApiResult<Json<Vec<HeldMessage>>> {
    let held = app::message::held::read_held(&state, user.id).await?;
    Ok(Json(held.into_iter().map(HeldMessage::from).collect()))
}

const DEFAULT_MESSAGE_LIMIT: u32 = 50;

/// Selects a window of a channel's history. At most one of `before`, `after`, and `around` may be
/// given; with none of them the newest messages are returned.
#[derive(Debug, Deserialize, IntoParams)]
#[serde(rename_all = "camelCase")]
#[into_params(parameter_in = Query)]
pub struct MessageListQuery {
    /// Return messages older than this message id (exclusive).
    pub before: Option<MessageId>,
    /// Return messages newer than this message id (exclusive).
    pub after: Option<MessageId>,
    /// Return this message and up to `limit` messages on each side of it, at most 100 a side
    /// whatever `limit` says.
    pub around: Option<MessageId>,
    /// Maximum number of messages to return (per side, for `around`, where more than 100 reads
    /// as 100). Defaults to 50.
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
            .map(Message::from)
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
    let m = Message::from(app::message::read_message(&state, user.id, message).await?);
    let included =
        sideload_messages(&state, user.id, std::slice::from_ref(&m), &query.include).await?;
    Ok(Json(MessageRead::new(m, included)))
}

/// Edits the caller's own message. New text or a new attachment takes Send messages (Send
/// messages in threads in a thread), and a new attachment Attach files besides; clearing the
/// text or removing attachments takes neither.
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
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing; `blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
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
    Ok(Json(Message::from(m)))
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
        (status = CREATED, description = "The thread, just made", body = crate::message_enum::Channel),
        (status = OK, description = "The thread the message already started", body = crate::message_enum::Channel),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (the message is in a thread, is an echo, or is in a voice channel)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing; `blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
        (status = NOT_FOUND, description = "No such message, or one in a DM the caller is not in", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn open_thread(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
) -> ApiResult<(StatusCode, Json<crate::message_enum::Channel>)> {
    let (thread, created) = app::thread::open_thread(&state, user.id, message).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(app::channel::record(&thread, Vec::new()))))
}

/// Shows a thread reply that was posted without an echo in the thread's parent channel, as
/// posting it with `echoToParent` would have: the echo is made now (`201`), or the reply's live
/// echo is returned as it is (`200`). Only the reply's author may, and it takes Send messages in
/// the parent channel. Once its echo is deleted, a reply may be echoed again.
#[utoipa::path(
    put,
    path = "/messages/{message}/echo",
    tag = TAG_MESSAGES,
    params(("message" = MessageId, Path, description = "The thread reply")),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "The echo, just made", body = Message),
        (status = OK, description = "The echo the reply already has", body = Message),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (the message is not in a thread, or is a poll or one of its results)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: the caller did not write the reply, or may not send messages in the parent channel; `blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
        (status = NOT_FOUND, description = "No such message, or one in a DM the caller is not in", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn echo_reply(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    let (echo, created) = app::thread::echo_reply(&state, user.id, message).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(app::message::record(&echo, Vec::new(), Vec::new())),
    ))
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
        (status = CREATED, description = "Pinned", body = crate::message_enum::Pin),
        (status = OK, description = "Already pinned", body = crate::message_enum::Pin),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing; `blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn pin_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(message): Path<MessageId>,
) -> ApiResult<(StatusCode, Json<crate::message_enum::Pin>)> {
    let (pin, created) = app::message::set_pinned(&state, user.id, message, true).await?;
    let pin = pin.ok_or(app::Error::Diesel(diesel::result::Error::NotFound))?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        Json(crate::message_enum::Pin {
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
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing; `blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
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

/// Takes one attachment off a message: its author may, and anyone with Manage messages. The
/// message's update names the attachments left.
#[utoipa::path(
    delete,
    path = "/messages/{message}/attachments/{attachment}",
    tag = TAG_MESSAGES,
    params(("message" = MessageId, Path), ("attachment" = AttachmentId, Path)),
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
pub async fn remove_attachment(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((message, attachment)): Path<(MessageId, AttachmentId)>,
) -> ApiResult<NoContent> {
    app::message::remove_attachment(&state, user.id, message, attachment).await?;
    Ok(NoContent)
}
