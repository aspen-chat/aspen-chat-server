use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Created, Json, NoContent, Path, Query};
use crate::include::{IncludeSet, Included, SideloadedList};
use crate::message_enum::Pin;
use crate::message_enum::request::{ChannelCreateRequest, ChannelUpdateRequest};
use crate::{API_PREFIX, TAG_CHANNELS, message_enum};
use aspen_app as app;
use aspen_app::ChannelId;
use aspen_app::context::GlobalServerContext;
use axum::extract::State;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// A community channel's wire record; a DM's carries its recipients too (`app::channel::record`).
pub fn channel_to_api(c: app::channel::Channel) -> message_enum::Channel {
    app::channel::record(&c, Vec::new())
}

/// Creates a channel. A channel may belong to a community and optionally to a category within it;
/// both references are set through the request body because either may be absent. `overrides`
/// sets who may use it from the start, which takes what setting each override takes.
#[utoipa::path(
    post,
    path = "/channels",
    tag = TAG_CHANNELS,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = message_enum::Channel, headers(("Location" = String, description = "URL of the new channel"))),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing, or an override names a role not ranked below the caller's or a permission the caller lacks", body = Problem),
        (status = NOT_FOUND, description = "No such community, or the caller is not a member, or an override names a role the community lacks", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_channel(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(request): Json<ChannelCreateRequest>,
) -> ApiResult<Created<message_enum::Channel>> {
    let c = app::channel::create_channel(&state, user.id, request).await?;
    Ok(Created::new(
        format!("{API_PREFIX}/channels/{}", c.id.0),
        channel_to_api(c),
    ))
}

#[utoipa::path(
    get,
    path = "/channels/{channel}",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = message_enum::Channel),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_channel(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<Json<message_enum::Channel>> {
    Ok(Json(
        app::channel::read_channel(&state, user.id, channel).await?,
    ))
}

/// Who is online in a channel.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChannelPresence {
    /// How many people who may view the channel are online, not away: the members of its
    /// community who may view it, or a DM's recipients. The caller is among them when online.
    pub online: u32,
}

/// How many people who may view a channel are online. Each server works a channel's count out
/// at most every ten seconds, so it may be that old.
#[utoipa::path(
    get,
    path = "/channels/{channel}/presence",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ChannelPresence),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such channel, or the caller may not view it", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_channel_presence(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<Json<ChannelPresence>> {
    Ok(Json(ChannelPresence {
        online: app::channel_presence::online_in_channel(&state, user.id, channel).await?,
    }))
}

/// Pinned messages in the channel, in pin order.
#[utoipa::path(
    get,
    path = "/channels/{channel}/pins",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<Pin>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_channel_pins(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<Json<Vec<Pin>>> {
    let pins = app::channel::read_channel_pins(&state, user.id, channel).await?;
    Ok(Json(
        pins.into_iter()
            .map(|p| Pin {
                message_id: p.message_id,
                timestamp: p.timestamp,
                sort_index: p.sort_index,
            })
            .collect(),
    ))
}

#[utoipa::path(
    patch,
    path = "/channels/{channel}",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = message_enum::Channel),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, description = "No such channel, or the caller may not view it", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_channel(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Json(request): Json<ChannelUpdateRequest>,
) -> ApiResult<Json<message_enum::Channel>> {
    let c = app::channel::update_channel(&state, user.id, channel, request).await?;
    Ok(Json(channel_to_api(c)))
}

#[utoipa::path(
    delete,
    path = "/channels/{channel}",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, description = "No such channel, or the caller may not view it", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_channel(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<NoContent> {
    app::channel::delete_channel(&state, user.id, channel).await?;
    Ok(NoContent)
}

/// One of a channel's top senders of the past hour.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ManaSender {
    pub user: aspen_app::UserId,
    /// The mana they spent there in the past hour, in whole mana.
    pub mana: u64,
}

/// Relationships a top senders read can sideload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ManaSenderInclude {
    /// The senders, as `included.users`: those raiding a channel are mostly unknown to its
    /// readers.
    Users,
}

/// Body of a top senders read; a named alias for the same reason as
/// `api::community::CommunityRead`.
pub type ManaSenderList = SideloadedList<ManaSender>;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ManaSenderQuery {
    /// Related records to return alongside the senders, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<ManaSenderInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<ManaSenderInclude>,
}

/// Who spent most of the channel's mana in the past hour, the most first, at most 100, each
/// with the mana they spent: posting, with Bypass channel mana, or waiting and then posted;
/// never what was refused or dropped. A thread counts its own. Read afresh at most every five
/// seconds. Empty where the channel has no mana (a one-to-one DM, a voice or plugin channel),
/// or while the deployment cannot count it. `include=users` sideloads the senders.
#[utoipa::path(
    get,
    path = "/channels/{channel}/mana/top-senders",
    tag = TAG_CHANNELS,
    params(("channel" = ChannelId, Path), ManaSenderQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = ManaSenderList),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such channel, or the caller may not read it", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_mana_senders(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
    Query(query): Query<ManaSenderQuery>,
) -> ApiResult<Json<ManaSenderList>> {
    let senders = app::mana::read_top_senders(&state, user.id, channel).await?;
    let users = if query.include.contains(ManaSenderInclude::Users) {
        let ids: Vec<aspen_app::UserId> = senders.iter().map(|(sender, _)| *sender).collect();
        Some(
            app::user::read_users(&state, user.id, &ids)
                .await?
                .into_iter()
                .map(crate::message_enum::User::from)
                .collect(),
        )
    } else {
        None
    };
    Ok(Json(ManaSenderList::new(
        senders
            .into_iter()
            .map(|(user, mana)| ManaSender { user, mana })
            .collect(),
        Included {
            users,
            ..Included::default()
        },
    )))
}
