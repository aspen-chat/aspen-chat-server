//! Messages scheduled for later (`app::message::scheduled`). One is scheduled with
//! `POST /channels/{channel}/scheduled-messages` and is then its author's alone, addressed
//! under `/users/@me/scheduled-messages`: they list them all, give one another time, post one
//! now, or delete it. Each is a held message (`api::message::HeldMessage`) with a time, and is
//! posted as one is; changes reach the author's apps as `scheduledMessageChanged`.

use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Created, Json, NoContent, Path, Query};
use crate::include::{IncludeSet, Included, Sideloaded};
use crate::{API_PREFIX, TAG_MESSAGES};
use aspen_app as app;
use aspen_app::context::GlobalServerContext;
pub use aspen_app::message::scheduled::{ScheduledMessage, ScheduledStatus};
use aspen_app::{AttachmentId, ChannelId, HeldMessageId, MessageId};
use axum::extract::State;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

/// A message to post later, as `POST /channels/{channel}/messages` would post it now.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledMessageCreateRequest {
    #[serde(default)]
    pub content: String,
    /// Uploads of the caller's, confirmed, which stay theirs alone until it is posted.
    #[serde(default)]
    pub attachments: Vec<AttachmentId>,
    /// A message of the same channel to post it as a reply to.
    pub reply_to: Option<MessageId>,
    /// With `replyTo`, whether it tags the author of that message; true when absent.
    pub tag_replied: Option<bool>,
    /// When to post it: from one minute to two years from now.
    pub send_at: DateTime<Utc>,
}

/// A change to a scheduled message; a field that is absent is unchanged.
#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledMessageUpdateRequest {
    /// When to post it instead: from one minute to two years from now. A failed one is tried
    /// again then; one waiting for its channel's mana leaves the queue until then.
    pub send_at: Option<DateTime<Utc>>,
}

/// Relationships a scheduled message read can sideload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledMessageInclude {
    /// Its files' records, as `included.attachments`.
    Attachments,
}

/// Body of a scheduled message read; a named alias for the same reason as
/// `api::community::CommunityRead`.
pub type ScheduledMessageRead = Sideloaded<ScheduledMessage>;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ScheduledMessageReadQuery {
    /// Related records to return alongside the scheduled message, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<ScheduledMessageInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<ScheduledMessageInclude>,
}

/// Schedules a message to be posted in the channel (or thread, or DM) at `sendAt`. It is
/// checked as posting it now would be, and again when it is posted, with the caller's
/// permissions as they are then. Until then only the caller is told of it.
#[utoipa::path(
    post,
    path = "/channels/{channel}/scheduled-messages",
    tag = TAG_MESSAGES,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = ScheduledMessage, headers(("Location" = String, description = "URL of the scheduled message"))),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (`sendAt` is under a minute or over two years from now, the caller already has as many scheduled as one person may, the text is over `app::message::MAX_CONTENT_CHARS` characters, an attachment is not ready, or `replyTo` names a message deleted or not in this channel)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission posting this needs is missing, or the caller is a bot; `blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
        (status = NOT_FOUND, description = "No such channel, or a DM the caller is not in", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn schedule_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Json(request): Json<ScheduledMessageCreateRequest>,
) -> ApiResult<Created<ScheduledMessage>> {
    let scheduled = app::message::scheduled::schedule(
        &state,
        user.id,
        channel,
        request.content,
        request.attachments,
        app::message::reply::Reply::requested(request.reply_to, request.tag_replied),
        request.send_at,
    )
    .await?;
    let location = format!(
        "{API_PREFIX}/users/@me/scheduled-messages/{}",
        scheduled.id.0
    );
    Ok(Created::new(location, scheduled))
}

/// Every message the caller has scheduled, the soonest due first, those that failed and those
/// waiting for a busy channel's mana among them. One person has at most a hundred, so this is
/// all of them.
#[utoipa::path(
    get,
    path = "/users/@me/scheduled-messages",
    tag = TAG_MESSAGES,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<ScheduledMessage>),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_scheduled_messages(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
) -> ApiResult<Json<Vec<ScheduledMessage>>> {
    Ok(Json(
        app::message::scheduled::read_scheduled(&state, user.id).await?,
    ))
}

/// Reads one of the caller's scheduled messages. `include` sideloads its files' records, which
/// an app putting it back in the message box to be changed needs.
#[utoipa::path(
    get,
    path = "/users/@me/scheduled-messages/{scheduled}",
    tag = TAG_MESSAGES,
    params(("scheduled" = HeldMessageId, Path), ScheduledMessageReadQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ScheduledMessageRead),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such scheduled message of the caller's: never one, posted already, or deleted", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_scheduled_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(scheduled): Path<HeldMessageId>,
    Query(query): Query<ScheduledMessageReadQuery>,
) -> ApiResult<Json<ScheduledMessageRead>> {
    let scheduled = app::message::scheduled::read(&state, user.id, scheduled).await?;
    let attachments = if query.include.contains(ScheduledMessageInclude::Attachments) {
        Some(
            app::attachment::read_attachments(&state, &scheduled.attachments)
                .await?
                .into_iter()
                .map(|row| crate::attachment::attachment_to_api(&state, row))
                .collect(),
        )
    } else {
        None
    };
    Ok(Json(ScheduledMessageRead::new(
        scheduled,
        Included {
            attachments,
            ..Included::default()
        },
    )))
}

/// Gives one of the caller's scheduled messages another time. One that failed is tried again
/// then.
#[utoipa::path(
    patch,
    path = "/users/@me/scheduled-messages/{scheduled}",
    tag = TAG_MESSAGES,
    params(("scheduled" = HeldMessageId, Path)),
    request_body = ScheduledMessageUpdateRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ScheduledMessage),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (`sendAt` is under a minute or over two years from now)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such scheduled message of the caller's: never one, posted already, or deleted", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_scheduled_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(scheduled): Path<HeldMessageId>,
    Json(request): Json<ScheduledMessageUpdateRequest>,
) -> ApiResult<Json<ScheduledMessage>> {
    let scheduled = match request.send_at {
        Some(send_at) => {
            app::message::scheduled::reschedule(&state, user.id, scheduled, send_at).await?
        }
        None => app::message::scheduled::read(&state, user.id, scheduled).await?,
    };
    Ok(Json(scheduled))
}

/// Posts one of the caller's scheduled messages now rather than at its time, one that failed
/// among them. It is posted as it would have been then, moments from now: the answer is the
/// scheduled message, due now, and the stream tells what becomes of it. One already due and
/// waiting for its channel's mana is left as it is.
#[utoipa::path(
    post,
    path = "/users/@me/scheduled-messages/{scheduled}/send",
    tag = TAG_MESSAGES,
    params(("scheduled" = HeldMessageId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ScheduledMessage),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such scheduled message of the caller's: never one, posted already, or deleted", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn send_scheduled_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(scheduled): Path<HeldMessageId>,
) -> ApiResult<Json<ScheduledMessage>> {
    Ok(Json(
        app::message::scheduled::send_now(&state, user.id, scheduled).await?,
    ))
}

/// Deletes one of the caller's scheduled messages before it is posted, whatever it waits for.
/// Its files stay the caller's to send another way for a day. Their apps learn of it by
/// `heldMessageCancelled`.
#[utoipa::path(
    delete,
    path = "/users/@me/scheduled-messages/{scheduled}",
    tag = TAG_MESSAGES,
    params(("scheduled" = HeldMessageId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such scheduled message of the caller's: never one, posted already, or deleted", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_scheduled_message(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(scheduled): Path<HeldMessageId>,
) -> ApiResult<NoContent> {
    app::message::scheduled::cancel(&state, user.id, scheduled).await?;
    Ok(NoContent)
}
