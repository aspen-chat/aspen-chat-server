//! Voice calls: joining a channel's call, reading who is in it, the voice server registry, and
//! failure reports. The media flows through the voice servers; see `app::voice` for how a call
//! is assigned to one and how their reports become events.

use crate::admin::AdminUser;
use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Created, Json, NoContent, Path};
use crate::message_enum::{VoiceParticipant, VoiceSession};
use crate::{API_PREFIX, TAG_VOICE};
use aspen_app as app;
use aspen_app::context::GlobalServerContext;
use aspen_app::deployment::DeploymentPermission;
use aspen_app::{ChannelId, UserId, VoiceServerId};
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A moderator's change to someone else's state in a call.
#[derive(Debug, Clone, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceParticipantModerationRequest {
    /// Server mute: their microphone is no longer forwarded to anyone, whatever they ask, until
    /// a moderator unmutes them. Unmuting lifts only the server mute: someone who also muted
    /// themself stays muted.
    pub muted: bool,
}

/// A voice server as operators see it.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceServer {
    pub id: VoiceServerId,
    pub name: String,
    /// The base URL clients open for signalling and measure latency against.
    pub url: String,
    /// The most participants the server carries at once.
    pub capacity: i32,
    /// Whether an operator lets it take calls. Changing it lifts any suspension.
    pub enabled: bool,
    /// Until when it takes no new calls because enough distinct people failed to reach it;
    /// absent when it is not suspended. It takes calls again on its own after that.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub suspended_until: Option<DateTime<Utc>>,
    /// Participants the server carried at its last report.
    pub participants: i32,
    /// When the server last reported its load; absent until it has reported once.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub last_report_at: Option<DateTime<Utc>>,
}

impl From<app::voice::VoiceServer> for VoiceServer {
    fn from(row: app::voice::VoiceServer) -> Self {
        VoiceServer {
            id: row.id,
            name: row.name,
            url: row.url,
            capacity: row.capacity,
            enabled: row.enabled,
            suspended_until: row.suspended_until.filter(|until| *until > Utc::now()),
            participants: row.reported_participants,
            last_report_at: row.last_report_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceServerCreateRequest {
    pub name: String,
    pub url: String,
    pub capacity: i32,
}

/// A merge patch: an absent field is unchanged. No field is nullable.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceServerUpdateRequest {
    #[serde(default)]
    #[schema(nullable = false)]
    pub name: Option<String>,
    #[serde(default)]
    #[schema(nullable = false)]
    pub url: Option<String>,
    #[serde(default)]
    #[schema(nullable = false)]
    pub capacity: Option<i32>,
    #[serde(default)]
    #[schema(nullable = false)]
    pub enabled: Option<bool>,
}

/// A voice server a client may try, as named in a join offer.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceServerCandidate {
    pub id: VoiceServerId,
    pub name: String,
    pub url: String,
}

/// What a client needs to join a channel's call. The client measures its latency to each
/// candidate, tries them nearest first, and reports a server that fails to start the session.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceJoinOffer {
    pub channel_id: ChannelId,
    /// The call already in progress on the channel, if any; then `candidates` holds only its
    /// server, since a call stays where it started.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub session: Option<VoiceSession>,
    /// At most ten servers, in no particular order.
    pub candidates: Vec<VoiceServerCandidate>,
    /// Presented to the voice server; good for every candidate until `expiresAt`.
    pub token: String,
    pub expires_at: DateTime<Utc>,
    /// Whether the caller may send their microphone in this call (Speak). Without it they
    /// join to listen; the voice server refuses a microphone producer.
    pub speak: bool,
    /// Whether they may share a screen or game (Share screen), which the voice server also
    /// enforces.
    pub share_screen: bool,
    /// Whether they may offer files to the others in the call (Transfer files), which the voice
    /// server also enforces.
    pub transfer_files: bool,
    /// Whether they may turn on a camera (Use camera), which the voice server also enforces.
    pub use_camera: bool,
}

/// The call on a channel, if any, and who is in it.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceChannelState {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(nullable = false)]
    pub session: Option<VoiceSession>,
    pub participants: Vec<VoiceParticipant>,
}

/// What a failure report led to.
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceServerFailureOutcome {
    /// Distinct users whose reports about the server count within the failure window.
    pub failures: u32,
    /// Whether the server is now out of join offers: disabled, or suspended for failures.
    pub disabled: bool,
    /// Whether this report counted: only one from one of this deployment's people offered the
    /// server in a recent join offer, who has not joined a call on it lately, does.
    pub counted: bool,
}

/// Asks to join the channel's call. Returns a short-lived token and the servers to try; the
/// client connects to one of them with the token.
#[utoipa::path(
    post,
    path = "/channels/{channel}/voice/join",
    tag = TAG_VOICE,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = VoiceJoinOffer),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (not a voice channel, DM, or group DM, or no voice server is available)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing; `blocked`: a block stands between the two people of this one-to-one DM", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn join_voice(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<Json<VoiceJoinOffer>> {
    let offer = app::voice::join_offer(&state, user.id, channel).await?;
    Ok(Json(VoiceJoinOffer {
        channel_id: channel,
        session: offer.session,
        candidates: offer
            .candidates
            .into_iter()
            .map(|server| VoiceServerCandidate {
                id: server.id,
                name: server.name,
                url: server.url,
            })
            .collect(),
        token: offer.token,
        expires_at: offer.expires_at,
        speak: offer.speak,
        share_screen: offer.share_screen,
        transfer_files: offer.transfer_files,
        use_camera: offer.camera,
    }))
}

/// Reads the call on a channel and who is in it.
/// Declines the call in a DM or group DM that is ringing the caller: it stops ringing them
/// on every device, which their `voiceRing` delete event tells. A call that is not ringing
/// them, or no call at all, is left as it is, and still answers `204`.
#[utoipa::path(
    delete,
    path = "/channels/{channel}/voice/rings/@me",
    tag = TAG_VOICE,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such channel, or not one the caller is in", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn decline_call(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<NoContent> {
    app::voice::decline_ring(&state, user.id, channel).await?;
    Ok(NoContent)
}

#[utoipa::path(
    get,
    path = "/channels/{channel}/voice",
    tag = TAG_VOICE,
    params(("channel" = ChannelId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = VoiceChannelState),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_channel_voice(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(channel): Path<ChannelId>,
) -> ApiResult<Json<VoiceChannelState>> {
    let (session, participants) = app::voice::read_channel_voice(&state, user.id, channel).await?;
    Ok(Json(VoiceChannelState {
        session,
        participants,
    }))
}

#[utoipa::path(
    get,
    path = "/voice-servers",
    tag = TAG_VOICE,
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<VoiceServer>),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "Without Manage voice servers", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_voice_servers(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
) -> ApiResult<Json<Vec<VoiceServer>>> {
    access.require(DeploymentPermission::ManageVoiceServers)?;
    let servers = app::voice::list_servers(&state).await?;
    Ok(Json(servers.into_iter().map(VoiceServer::from).collect()))
}

#[utoipa::path(
    post,
    path = "/voice-servers",
    tag = TAG_VOICE,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = VoiceServer, headers(("Location" = String, description = "URL of the new server"))),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "Without Manage voice servers", body = Problem),
        (status = CONFLICT, description = "A server with that name exists", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_voice_server(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Json(request): Json<VoiceServerCreateRequest>,
) -> ApiResult<Created<VoiceServer>> {
    access.require(DeploymentPermission::ManageVoiceServers)?;
    let server =
        app::voice::create_server(&state, request.name, request.url, request.capacity).await?;
    Ok(Created::new(
        format!("{API_PREFIX}/voice-servers/{}", server.id.0),
        VoiceServer::from(server),
    ))
}

#[utoipa::path(
    patch,
    path = "/voice-servers/{server}",
    tag = TAG_VOICE,
    params(("server" = VoiceServerId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = VoiceServer),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = FORBIDDEN, description = "Without Manage voice servers", body = Problem),
        (status = CONFLICT, description = "A server with that name exists", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_voice_server(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(server): Path<VoiceServerId>,
    Json(request): Json<VoiceServerUpdateRequest>,
) -> ApiResult<Json<VoiceServer>> {
    access.require(DeploymentPermission::ManageVoiceServers)?;
    let updated = app::voice::update_server(
        &state,
        server,
        app::voice::VoiceServerChangeset {
            name: request.name,
            url: request.url,
            capacity: request.capacity,
            enabled: request.enabled,
        },
    )
    .await?;
    Ok(Json(VoiceServer::from(updated)))
}

/// Removes a server. Any call on it ends.
#[utoipa::path(
    delete,
    path = "/voice-servers/{server}",
    tag = TAG_VOICE,
    params(("server" = VoiceServerId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "Without Manage voice servers", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_voice_server(
    State(state): State<GlobalServerContext>,
    AdminUser(_session, access): AdminUser,
    Path(server): Path<VoiceServerId>,
) -> ApiResult<NoContent> {
    access.require(DeploymentPermission::ManageVoiceServers)?;
    app::voice::delete_server(&state, server).await?;
    Ok(NoContent)
}

/// Reports that the server failed to start the caller's session. A report counts only from one
/// of this deployment's people (not a bot, nor a foreign user) whom a recent join offer named
/// the server to and who has not joined a call on it within the failure window; each counts
/// once per window, and at the configured number of distinct users the server is suspended for
/// the window, unless no other server would be left taking calls. A report that does not count
/// is answered the same way, with `counted` false, and changes nothing.
#[utoipa::path(
    post,
    path = "/voice-servers/{server}/failures",
    tag = TAG_VOICE,
    params(("server" = VoiceServerId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = VoiceServerFailureOutcome),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such server", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn report_voice_server_failure(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(server): Path<VoiceServerId>,
) -> ApiResult<Json<VoiceServerFailureOutcome>> {
    let outcome = app::voice::report_failure(&state, &user, server).await?;
    Ok(Json(VoiceServerFailureOutcome {
        failures: outcome.failures,
        disabled: outcome.disabled,
        counted: outcome.counted,
    }))
}

#[utoipa::path(
    patch,
    path = "/channels/{channel}/voice/participants/{user}",
    tag = TAG_VOICE,
    params(("channel" = ChannelId, Path), ("user" = UserId, Path)),
    request_body = VoiceParticipantModerationRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = ACCEPTED, description = "The voice server has been told; the participant's `update` event follows once it applies", body = VoiceParticipant),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: lacks Manage calls, or the participant is the owner or ranks at or above the caller", body = Problem),
        (status = NOT_FOUND, description = "No call on the channel, or the user is not in it", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn moderate_voice_participant(
    State(state): State<GlobalServerContext>,
    SessionUser { user: caller, .. }: SessionUser,
    Path((channel, user)): Path<(ChannelId, UserId)>,
    Json(request): Json<VoiceParticipantModerationRequest>,
) -> ApiResult<(StatusCode, Json<VoiceParticipant>)> {
    let participant =
        app::voice::mute_participant(&state, caller.id, channel, user, request.muted).await?;
    Ok((StatusCode::ACCEPTED, Json(participant)))
}

#[utoipa::path(
    delete,
    path = "/channels/{channel}/voice/participants/{user}",
    tag = TAG_VOICE,
    params(("channel" = ChannelId, Path), ("user" = UserId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = ACCEPTED, description = "The voice server has been told to disconnect them; their participant `delete` event follows"),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: lacks Manage calls, or the participant is the owner or ranks at or above the caller", body = Problem),
        (status = NOT_FOUND, description = "No call on the channel, or the user is not in it", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn kick_voice_participant(
    State(state): State<GlobalServerContext>,
    SessionUser { user: caller, .. }: SessionUser,
    Path((channel, user)): Path<(ChannelId, UserId)>,
) -> ApiResult<StatusCode> {
    app::voice::kick_participant(&state, caller.id, channel, user).await?;
    Ok(StatusCode::ACCEPTED)
}
