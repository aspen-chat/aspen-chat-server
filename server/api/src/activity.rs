//! The caller's activity feed (`app::activity`): every message that tells them of itself, by
//! the rule their notifications follow, newest first.

use crate::TAG_MESSAGES;
use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Json, Query};
use crate::include::IncludeSet;
use crate::message::{MessageInclude, MessageList, sideload_messages};
use crate::message_enum::Message;
use crate::t;
use aspen_app::context::GlobalServerContext;
use aspen_app::{self as app, CommunityId, MessageId};
use axum::extract::State;
use serde::Deserialize;
use utoipa::IntoParams;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ActivityQuery {
    /// Only messages of these communities, comma separated; every community's when absent.
    #[serde(rename = "filter[community]")]
    #[param(rename = "filter[community]", value_type = Option<Vec<CommunityId>>, style = Form, explode = false)]
    pub communities: Option<String>,
    /// Whether DMs' messages are read; they are when absent.
    #[serde(rename = "filter[dms]")]
    #[param(rename = "filter[dms]")]
    pub dms: Option<bool>,
    /// Only messages the caller has not read, when true.
    #[serde(rename = "filter[unread]")]
    #[param(rename = "filter[unread]")]
    pub unread: Option<bool>,
    /// Only messages older than this one: the last of the previous page.
    pub before: Option<MessageId>,
    /// How many to return, at most `app::activity::MAX_PAGE` (50); 25 when absent.
    pub limit: Option<u32>,
    /// Related records to return alongside the messages, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<MessageInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<MessageInclude>,
}

/// How many messages a page of the feed holds when it does not say.
const DEFAULT_ACTIVITY_PAGE: u32 = 25;

/// The messages that tell the caller of themselves, newest first: in a DM, community, or
/// channel, every message or only those tagging them as their notification level there says,
/// and every reply in a thread they follow; never in a channel they muted, by them or anyone
/// they blocked, or from before they joined. A community the caller does not belong to is
/// passed over.
#[utoipa::path(
    get,
    path = "/users/@me/activity",
    tag = TAG_MESSAGES,
    params(ActivityQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = MessageList),
        (status = BAD_REQUEST, description = "`validation`: a malformed community id", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn read_activity(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Query(query): Query<ActivityQuery>,
) -> ApiResult<Json<MessageList>> {
    let communities = match &query.communities {
        None => None,
        Some(listed) => Some(
            listed
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|raw| {
                    uuid::Uuid::parse_str(raw)
                        .map(CommunityId)
                        .map_err(|_| app::Error::Validation(t!("invalidCommunityId")))
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    let messages: Vec<Message> = app::activity::read_activity(
        &state,
        user.id,
        app::activity::ActivityQuery {
            communities,
            dms: query.dms.unwrap_or(true),
            unread: query.unread.unwrap_or(false),
            before: query.before,
            limit: query.limit.unwrap_or(DEFAULT_ACTIVITY_PAGE),
        },
    )
    .await?
    .into_iter()
    .map(Message::from)
    .collect();
    let included = sideload_messages(&state, user.id, &messages, &query.include).await?;
    Ok(Json(MessageList::new(messages, included)))
}
