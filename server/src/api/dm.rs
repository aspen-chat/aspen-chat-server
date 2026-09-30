use crate::api::auth::SessionUser;
use crate::api::error::{ApiResult, Problem};
use crate::api::extract::{Json, NoContent, Path, Query};
use crate::api::include::{IncludeSet, Included, SideloadedList};
use crate::api::message_enum::Channel;
use crate::api::{API_PREFIX, GlobalServerContext, TAG_DMS};
use crate::app::{self, ChannelId, UserId};
use axum::extract::State;
use axum::http::{StatusCode, header};
use serde::Deserialize;
use std::collections::HashSet;
use utoipa::{IntoParams, ToSchema};

/// Opens a DM: with one other person their one-to-one DM, with more a new group DM.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct DmCreateRequest {
    /// The people to talk to, besides the caller: one for a DM, up to nine for a group DM. Each
    /// must share a community with the caller.
    pub recipients: Vec<UserId>,
}

/// Relationships a DM list read can sideload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum DmInclude {
    /// The DMs' recipients, as `included.users`.
    Users,
    /// How far the caller has read each DM, as `included.readStates`.
    ReadStates,
    /// The caller's mutes of the DMs, as `included.channelMutes`.
    Mutes,
    /// The caller's notification settings for the DMs, as `included.notificationSettings`.
    Notifications,
    /// The calls under way in the DMs, as `included.voiceSessions`, with who is in each, as
    /// `included.voiceParticipants`, and who each is ringing, as `included.voiceRings`.
    Voice,
}

/// Body of a DM list read; a named alias for the same reason as `api::community::CommunityRead`.
pub type DmList = SideloadedList<Channel>;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DmListQuery {
    /// Related records to return alongside the DMs, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<DmInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<DmInclude>,
}

/// Opens a DM with the people named. With one other person this is their one-to-one DM: made
/// on first use (`201`) and returned as it is afterwards (`200`). With more it is always a new
/// group DM, of at most ten people including the caller.
#[utoipa::path(
    post,
    path = "/users/@me/dms",
    tag = TAG_DMS,
    security(("bearerAuth" = [])),
    request_body = DmCreateRequest,
    responses(
        (status = CREATED, description = "A new DM or group DM", body = Channel, headers(("Location" = String, description = "URL of the DM"))),
        (status = OK, description = "The existing one-to-one DM", body = Channel, headers(("Location" = String, description = "URL of the DM"))),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (no recipient, too many, or one who shares no community with the caller)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`blocked`: a block stands between the two people of this one-to-one DM; `federationRefused`: everyone in it belongs to other deployments", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn open_dm(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(request): Json<DmCreateRequest>,
) -> ApiResult<(StatusCode, [(header::HeaderName, String); 1], Json<Channel>)> {
    let (dm, recipients, created) = app::dm::open_dm(&state, user.id, request.recipients).await?;
    let status = if created {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((
        status,
        [(
            header::LOCATION,
            format!("{API_PREFIX}/channels/{}", dm.id.0),
        )],
        Json(app::channel::record(&dm, recipients)),
    ))
}

/// The caller's DMs and group DMs, the most recently active first. `include=users` sideloads
/// their recipients, `include=readStates` how far the caller has read each, and
/// `include=mutes` which the caller has muted.
#[utoipa::path(
    get,
    path = "/users/@me/dms",
    tag = TAG_DMS,
    params(DmListQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = DmList),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_dms(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Query(query): Query<DmListQuery>,
) -> ApiResult<Json<DmList>> {
    let dms = app::dm::list_dms(&state, user.id).await?;
    let users = if query.include.contains(DmInclude::Users) {
        let ids: Vec<UserId> = dms
            .iter()
            .flat_map(|(_, recipients)| recipients.iter().copied())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        Some(
            app::user::read_users(&state, &ids)
                .await?
                .into_iter()
                .map(crate::api::message_enum::User::from)
                .collect(),
        )
    } else {
        None
    };
    let read_states = if query.include.contains(DmInclude::ReadStates) {
        let ids: Vec<ChannelId> = dms.iter().map(|(dm, _)| dm.id).collect();
        Some(
            app::read_state::read_channels_read_states(&state, user.id, &ids)
                .await?
                .into_iter()
                .map(crate::api::read_state::ReadState::from)
                .collect(),
        )
    } else {
        None
    };
    let channel_mutes = if query.include.contains(DmInclude::Mutes) {
        let ids: Vec<ChannelId> = dms.iter().map(|(dm, _)| dm.id).collect();
        Some(
            app::channel_mute::read_mutes(&state, user.id, &ids, &[])
                .await?
                .into_iter()
                .map(crate::api::channel_mute::ChannelMute::from)
                .collect(),
        )
    } else {
        None
    };
    let notification_settings = if query.include.contains(DmInclude::Notifications) {
        let ids: Vec<ChannelId> = dms.iter().map(|(dm, _)| dm.id).collect();
        Some(
            app::notification_setting::read_settings(&state, user.id, &ids, &[])
                .await?
                .into_iter()
                .map(crate::api::notification_setting::NotificationSetting::from)
                .collect(),
        )
    } else {
        None
    };
    let (voice_sessions, voice_participants, voice_rings) =
        if query.include.contains(DmInclude::Voice) {
            let ids: Vec<ChannelId> = dms.iter().map(|(dm, _)| dm.id).collect();
            let (sessions, participants) = app::voice::read_channels_voice(&state, &ids).await?;
            let rings = app::voice::read_channels_rings(&state, &ids).await?;
            (Some(sessions), Some(participants), Some(rings))
        } else {
            (None, None, None)
        };
    let records = dms
        .into_iter()
        .map(|(dm, recipients)| app::channel::record(&dm, recipients))
        .collect();
    Ok(Json(DmList::new(
        records,
        Included {
            users,
            read_states,
            channel_mutes,
            notification_settings,
            voice_sessions,
            voice_participants,
            voice_rings,
            ..Included::default()
        },
    )))
}

/// Adds someone to a group DM the caller is in. They must share a community with the caller,
/// and the group holds at most ten people.
#[utoipa::path(
    put,
    path = "/channels/{channel}/recipients/{user}",
    tag = TAG_DMS,
    params(("channel" = ChannelId, Path), ("user" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Added", body = Channel),
        (status = OK, description = "Already a recipient", body = Channel),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (not a group DM, full, or someone who shares no community with the caller)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
        (status = NOT_FOUND, description = "No such channel, or a DM the caller is not in", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn add_recipient(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path((channel, recipient)): Path<(ChannelId, UserId)>,
) -> ApiResult<(StatusCode, Json<Channel>)> {
    let added = app::dm::add_recipient(&state, user.id, channel, recipient).await?;
    let record = app::channel::read_channel(&state, user.id, channel).await?;
    let status = if added {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((status, Json(record)))
}

/// Takes the caller out of a group DM.
#[utoipa::path(
    delete,
    path = "/channels/{channel}/recipients/@me",
    tag = TAG_DMS,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (not a group DM)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such channel, or a DM the caller is not in", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn leave_dm(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<NoContent> {
    app::dm::leave(&state, user.id, channel).await?;
    Ok(NoContent)
}
