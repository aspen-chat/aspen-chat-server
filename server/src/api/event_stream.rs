//! WebSocket event stream at `/api/v1/events`.
//!
//! Browsers cannot attach headers to a WebSocket upgrade, so authentication happens in-band:
//! the client's first frame must be an [`ClientMessage::Identify`] carrying a session token,
//! sent within [`IDENTIFY_TIMEOUT`]. The server answers with [`ServerMessage::Ready`] and then
//! streams [`ServerMessage::Event`] frames, each tagged with its JetStream sequence number.
//! A connection reads only the subjects its user is entitled to (`app::events`): their own
//! and every community they belong to. When their memberships change, the connection reopens
//! its consumer with the new filter from the last sequence it delivered, so nothing is missed
//! and nothing from a community they left arrives after the leave.
//!
//! A client that reconnects passes the last sequence it processed as `resumeAfter`. When that
//! position is still inside the stream's retention window the server replays exactly what was
//! missed and reports `resumed: true`; otherwise it falls back to the `MAX_EVENT_AGE` replay
//! window and reports `resumed: false`, which tells the client its cached state has a gap it
//! must repair from REST.
//!
//! Every frame in both directions is JSON. The full protocol is described by
//! `event_schema.json` (root type [`EventStreamProtocol`]).

use crate::api::message_enum::server_event::ServerEvent;
use crate::api::{GlobalServerContext, MAX_EVENT_AGE};
use crate::app;
use crate::app::ASPEN_NATS_STREAM_NAME;
use crate::app::events::{EVENT_ID_HEADER, community_filter, memberships, user_subject};
use crate::app::user::UserPg;
use crate::app::{CommunityId, UserId};
use async_nats::ConnectOptions;
use async_nats::jetstream::consumer::pull::{Ordered, OrderedConfig};
use async_nats::jetstream::consumer::{DeliverPolicy, ReplayPolicy};
use async_nats::jetstream::stream::Stream;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::Response;
use bytes::Bytes;
use rust_i18n::t;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::borrow::Cow;
use std::error::Error;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tracing::{debug, error, warn};

/// How long a freshly upgraded socket may stay silent before it is closed for not identifying.
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);
/// Cadence of server-initiated pings. Browsers answer pings automatically, so this both keeps
/// idle NAT and proxy mappings alive and detects peers that vanished without a close frame.
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// Consecutive unanswered pings after which the peer is presumed gone.
const MAX_MISSED_PONGS: u32 = 2;

/// Frames the client may send.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ClientMessage {
    /// Must be the first frame. Authenticates the connection and optionally asks to resume.
    #[serde(rename_all = "camelCase")]
    Identify {
        /// Session token from `POST /auth/login` or `POST /auth/token-refresh`.
        session_token: String,
        /// `sequence` of the last event this client processed on a previous connection. Omit on
        /// a first connection.
        #[serde(default)]
        resume_after: Option<u64>,
    },
    /// The user is using the app: send it while they interact with it (pointer, keyboard,
    /// touch, the window gaining focus), at most once every `ACTIVITY_INTERVAL` seconds, and not
    /// while they don't. A connected user shows as away once `[presence] away_after_seconds` pass
    /// without one from any of their devices. Frames closer together than that are ignored.
    Activity,
}

/// The least time between two `activity` frames that count; clients send them at most this
/// often.
pub const ACTIVITY_INTERVAL: Duration = Duration::from_secs(60);
/// Frames arriving a little early still count, so a client's timer jitter never costs one.
const ACTIVITY_GRACE: Duration = Duration::from_secs(5);

/// Why the server is closing the connection. Mirrors the WebSocket close code sent alongside.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum EventStreamErrorCode {
    /// The first frame was not a well-formed `identify` message. Close code 4400.
    BadRequest,
    /// The session token is missing, unknown, or expired. Close code 4401.
    Unauthorized,
    /// The server requires a second factor the account has not added yet; the session may only
    /// add one. Close code 4403.
    TwoFactorEnrollmentRequired,
    /// No `identify` frame arrived within the allowed time. Close code 4408.
    IdentifyTimeout,
    /// The server failed while setting up or serving the stream. Close code 1011.
    Internal,
}

impl EventStreamErrorCode {
    fn close_code(self) -> u16 {
        match self {
            EventStreamErrorCode::BadRequest => 4400,
            EventStreamErrorCode::Unauthorized => 4401,
            EventStreamErrorCode::TwoFactorEnrollmentRequired => 4403,
            EventStreamErrorCode::IdentifyTimeout => 4408,
            EventStreamErrorCode::Internal => 1011,
        }
    }

    fn detail(self) -> Cow<'static, str> {
        match self {
            EventStreamErrorCode::BadRequest => t!("eventStreamIdentifyMalformed"),
            EventStreamErrorCode::Unauthorized => t!("invalidAuthToken"),
            EventStreamErrorCode::TwoFactorEnrollmentRequired => {
                t!("problemTwoFactorEnrollmentRequired")
            }
            EventStreamErrorCode::IdentifyTimeout => t!("eventStreamIdentifyTimeout"),
            EventStreamErrorCode::Internal => t!("eventStreamError"),
        }
    }
}

/// Frames the server sends.
#[derive(Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMessage {
    /// Acknowledges a successful `identify`. Events follow.
    #[serde(rename_all = "camelCase")]
    Ready {
        user_id: UserId,
        /// `true` when `resumeAfter` was honoured and every event since it will be replayed.
        /// `false` on a first connection, or when the requested position has already left the
        /// stream: the client must rebuild its cached state from REST.
        resumed: bool,
    },
    /// One server event. `sequence` increases monotonically within a connection and is what
    /// the client hands back as `resumeAfter`.
    #[serde(rename_all = "camelCase")]
    Event {
        sequence: u64,
        /// The same for every copy of an event: an event about a user goes to each community
        /// they share with the reader, and this is how the reader keeps one.
        #[serde(skip_serializing_if = "Option::is_none")]
        event_id: Option<String>,
        #[schemars(with = "ServerEvent")]
        event: Box<RawValue>,
    },
    /// Sent immediately before the server closes the connection because of a protocol or
    /// authentication failure. Never sent for an orderly shutdown.
    Error {
        code: EventStreamErrorCode,
        /// Localized explanation suitable for display.
        detail: Cow<'static, str>,
    },
}

/// Root of `event_schema.json`: both directions of the WebSocket protocol. Never instantiated;
/// it exists so one schema document describes everything a client needs.
#[derive(JsonSchema)]
#[allow(dead_code)]
pub struct EventStreamProtocol {
    pub client: ClientMessage,
    pub server: ServerMessage,
}

pub async fn event_stream(
    ws: WebSocketUpgrade,
    State(state): State<GlobalServerContext>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket_conn(socket, state))
}

struct Rejection(EventStreamErrorCode);

impl From<app::Error> for Rejection {
    fn from(e: app::Error) -> Self {
        error!("event stream setup failed: {e}");
        Rejection(EventStreamErrorCode::Internal)
    }
}

async fn handle_socket_conn(mut socket: WebSocket, state: GlobalServerContext) {
    let session = match identify(&mut socket, &state).await {
        Ok(session) => session,
        Err(Rejection(code)) => {
            reject(&mut socket, code).await;
            return;
        }
    };
    let subscription = match subscribe(&state, session.user.id, session.resume_after).await {
        Ok(subscription) => subscription,
        Err(Rejection(code)) => {
            reject(&mut socket, code).await;
            return;
        }
    };
    let ready = ServerMessage::Ready {
        user_id: session.user.id,
        resumed: subscription.resumed,
    };
    if let Err(e) = send_json(&mut socket, &ready).await {
        log_send_error(&e);
        return;
    }
    pump_events(socket, subscription, &state, session.user.id).await;
}

struct Identified {
    user: UserPg,
    resume_after: Option<u64>,
}

/// Waits for the `identify` frame and authenticates it. Control frames that arrive first are
/// tolerated; any other data frame, a malformed body, or silence past the deadline rejects the
/// connection.
async fn identify(
    socket: &mut WebSocket,
    state: &GlobalServerContext,
) -> Result<Identified, Rejection> {
    let deadline = tokio::time::Instant::now() + IDENTIFY_TIMEOUT;
    let text = loop {
        match tokio::time::timeout_at(deadline, socket.recv()).await {
            Err(_) => return Err(Rejection(EventStreamErrorCode::IdentifyTimeout)),
            // The peer went away before identifying; there is nobody to reject.
            Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => {
                return Err(Rejection(EventStreamErrorCode::BadRequest));
            }
            Ok(Some(Ok(Message::Text(text)))) => break text,
            Ok(Some(Ok(Message::Binary(_)))) => {
                return Err(Rejection(EventStreamErrorCode::BadRequest));
            }
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
        }
    };
    let ClientMessage::Identify {
        session_token,
        resume_after,
    } = serde_json::from_str(text.as_str()).map_err(|e| {
        debug!("malformed identify frame: {e}");
        Rejection(EventStreamErrorCode::BadRequest)
    })?
    else {
        debug!("the first frame was not identify");
        return Err(Rejection(EventStreamErrorCode::BadRequest));
    };
    let (user, caller) = app::user::user_for_token(state, &session_token)
        .await?
        .ok_or(Rejection(EventStreamErrorCode::Unauthorized))?;
    if caller.enrollment_required(&state.config.auth) {
        return Err(Rejection(EventStreamErrorCode::TwoFactorEnrollmentRequired));
    }
    app::user::mark_user_online(state, &user);
    Ok(Identified { user, resume_after })
}

struct Subscription {
    messages: Ordered,
    resumed: bool,
    /// The stream the consumer reads, kept so the consumer can be reopened with new filters.
    stream: Stream,
    /// Fires when NATS flags this connection as a slow consumer; the socket is then dropped
    /// rather than allowed to fall arbitrarily far behind.
    force_shutdown_rx: mpsc::Receiver<()>,
}

/// The subjects a user's connection reads: their own, and every community they belong to.
async fn filters_for(state: &GlobalServerContext, user: UserId) -> Result<Vec<String>, Rejection> {
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
    let mut filters = vec![user_subject(user)];
    filters.extend(
        memberships(conn.as_mut(), user)
            .await?
            .into_iter()
            .map(community_filter),
    );
    Ok(filters)
}

async fn open_messages(
    stream: &Stream,
    state: &GlobalServerContext,
    deliver_policy: DeliverPolicy,
    filters: Vec<String>,
) -> Result<Ordered, app::Error> {
    let consumer = stream
        .create_consumer(OrderedConfig {
            replay_policy: ReplayPolicy::Instant,
            deliver_policy,
            filter_subjects: filters,
            max_batch: state.config.event_queue_size as i64,
            max_bytes: 1024 * 1024,
            max_expires: Duration::from_secs(5),
            ..Default::default()
        })
        .await?;
    Ok(consumer.messages().await?)
}

async fn subscribe(
    state: &GlobalServerContext,
    user: UserId,
    resume_after: Option<u64>,
) -> Result<Subscription, Rejection> {
    let (force_shutdown_tx, force_shutdown_rx) = mpsc::channel(1);
    let client = async_nats::connect_with_options(
        &state.config.nats_url,
        ConnectOptions::new()
            .token(state.config.nats_auth_token.clone())
            .subscription_capacity(state.config.event_queue_size)
            .event_callback(move |event| {
                let force_shutdown_tx = force_shutdown_tx.clone();
                async move {
                    if let async_nats::Event::SlowConsumer(_) = event {
                        // Intentionally ignore errors, if the remote is already shutdown
                        // then our objective is already accomplished.
                        std::mem::drop(force_shutdown_tx.send(()));
                    }
                }
            }),
    )
    .await
    .map_err(app::Error::from)?;
    let context = async_nats::jetstream::new(client);
    let mut stream = context
        .get_stream(ASPEN_NATS_STREAM_NAME)
        .await
        .map_err(app::Error::from)?;
    let (deliver_policy, resumed) = match resume_after {
        Some(last_seen) => {
            let info = stream.info().await.map_err(app::Error::from)?;
            let first = info.state.first_sequence;
            let last = info.state.last_sequence;
            // Resumable only if the next event is still retained and the client's position
            // is not ahead of the stream (which happens when the in-memory stream was
            // recreated and sequence numbers restarted).
            if last_seen + 1 >= first && last_seen <= last {
                (
                    DeliverPolicy::ByStartSequence {
                        start_sequence: last_seen + 1,
                    },
                    true,
                )
            } else {
                (replay_window_policy(), false)
            }
        }
        None => (replay_window_policy(), false),
    };
    let filters = filters_for(state, user).await?;
    let messages = open_messages(&stream, state, deliver_policy, filters).await?;
    Ok(Subscription {
        messages,
        resumed,
        stream,
        force_shutdown_rx,
    })
}

fn replay_window_policy() -> DeliverPolicy {
    DeliverPolicy::ByStartTime {
        start_time: time::OffsetDateTime::now_utc() - MAX_EVENT_AGE,
    }
}

/// A membership of this user coming or going, read off a delivered event without parsing the
/// whole of it: the community and whether it was joined.
fn membership_change(payload: &str, user: UserId) -> Option<(CommunityId, bool)> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Glance {
        server_event: String,
        #[serde(rename = "type")]
        kind: Option<String>,
        user: Option<UserId>,
        community: Option<CommunityId>,
    }
    let glance: Glance = serde_json::from_str(payload).ok()?;
    if glance.server_event != "userCommunity" || glance.user != Some(user) {
        return None;
    }
    match (glance.kind.as_deref(), glance.community) {
        (Some("create"), Some(community)) => Some((community, true)),
        (Some("delete"), Some(community)) => Some((community, false)),
        _ => None,
    }
}

async fn pump_events(
    mut socket: WebSocket,
    subscription: Subscription,
    state: &GlobalServerContext,
    user: UserId,
) {
    let Subscription {
        mut messages,
        mut force_shutdown_rx,
        stream,
        ..
    } = subscription;
    let mut ping_interval =
        tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
    let mut unanswered_pings: u32 = 0;
    let mut last_activity: Option<tokio::time::Instant> = None;
    loop {
        tokio::select! {
            msg = messages.next() => {
                let Some(msg) = msg else {
                    break;
                };
                let msg = match msg {
                    Ok(msg) => msg,
                    Err(e) => {
                        error!("NATS OrderedError {e}");
                        return;
                    }
                };
                let sequence = match msg.info() {
                    Ok(info) => info.stream_sequence,
                    Err(e) => {
                        error!("NATS message without metadata {e}");
                        return;
                    }
                };
                let payload = match String::from_utf8(msg.message.payload.to_vec())
                    .map_err(|e| e.to_string())
                    .and_then(|s| RawValue::from_string(s).map_err(|e| e.to_string()))
                {
                    Ok(v) => v,
                    Err(e) => {
                        error!("NATS stream message was not JSON text {e}");
                        continue;
                    }
                };
                let event_id = msg
                    .message
                    .headers
                    .as_ref()
                    .and_then(|headers| headers.get(EVENT_ID_HEADER))
                    .map(|value| value.to_string());
                // Only the copy on the user's own subject counts: the community's copy of the
                // same membership event arrives after the reopen and must not cause another.
                let reopen = if msg.message.subject.as_str() == user_subject(user) {
                    membership_change(payload.get(), user)
                } else {
                    None
                };
                let frame = ServerMessage::Event { sequence, event_id, event: payload };
                if let Err(e) = send_json(&mut socket, &frame).await {
                    log_send_error(&e);
                    return;
                }
                if let Some((community, joined)) = reopen {
                    // The user joined or left a community: read the new set of subjects from
                    // right after this event, so nothing is missed and nothing more arrives
                    // from a community they left. The event was published before its
                    // transaction committed, so the change it announces is applied on top of
                    // whatever the database shows.
                    let mut filters = match filters_for(state, user).await {
                        Ok(filters) => filters,
                        Err(_) => return,
                    };
                    let filter = community_filter(community);
                    filters.retain(|f| *f != filter);
                    if joined {
                        filters.push(filter);
                    }
                    let policy = DeliverPolicy::ByStartSequence { start_sequence: sequence + 1 };
                    match open_messages(&stream, state, policy, filters).await {
                        Ok(reopened) => messages = reopened,
                        Err(e) => {
                            error!("could not reopen the event consumer after a membership change: {e}");
                            return;
                        }
                    }
                }
            },
            _ = ping_interval.tick() => {
                if unanswered_pings >= MAX_MISSED_PONGS {
                    warn!("event stream peer stopped answering pings, closing");
                    return;
                }
                unanswered_pings += 1;
                // The connection is the user's presence: the key is refreshed as long as it is up.
                app::user::mark_user_online_id(state, user);
                if let Err(e) = socket.send(Message::Ping(Bytes::new())).await {
                    log_send_error(&e);
                    return;
                }
            },
            // Drive the read side of the socket too. After `identify` the client sends only
            // `activity` frames, but the WebSocket protocol's control frames (Close, Ping, Pong)
            // arrive on this same channel. Tungstenite only reacts to them while the stream is
            // being polled, so without this arm a client-initiated close frame would sit unread
            // indefinitely and pongs would never be counted.
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => {
                        // Explicitly send our half of the close handshake; `send` is what
                        // actually flushes the queued acknowledgement. Best-effort, the peer
                        // may already have aborted TCP.
                        let _ = socket.send(Message::Close(None)).await;
                        return;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        unanswered_pings = 0;
                    }
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(ClientMessage::Activity) = serde_json::from_str(text.as_str()) {
                            let now = tokio::time::Instant::now();
                            let due = last_activity.is_none_or(|last| {
                                now.duration_since(last) + ACTIVITY_GRACE >= ACTIVITY_INTERVAL
                            });
                            if due {
                                last_activity = Some(now);
                                app::user_status::mark_active(state, user);
                            }
                        }
                        // Anything else (a second `identify`, an unknown frame) is dropped
                        // rather than tearing the connection down.
                    }
                    Some(Ok(_)) => {
                        // Pings are auto-pong'd by tungstenite at the protocol layer; binary
                        // frames are not part of this stream's contract.
                    }
                    Some(Err(e)) => {
                        error!("websocket recv error on event stream {e}");
                        return;
                    }
                }
            },
            _ = force_shutdown_rx.recv() => {
                error!("forcefully disconnecting user due to slow events download");
                break;
            }
        }
    }
}

async fn send_json(socket: &mut WebSocket, message: &ServerMessage) -> Result<(), axum::Error> {
    let text = serde_json::to_string(message).map_err(axum::Error::new)?;
    socket.send(Message::Text(text.into())).await
}

/// Tells the peer why it is being dropped, then closes with the matching close code. Both
/// sends are best-effort; the peer may already be gone.
async fn reject(socket: &mut WebSocket, code: EventStreamErrorCode) {
    let _ = send_json(
        socket,
        &ServerMessage::Error {
            code,
            detail: code.detail(),
        },
    )
    .await;
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: code.close_code(),
            reason: serde_json::to_string(&code)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string()
                .into(),
        })))
        .await;
}

/// A closed connection is the normal end of a stream and is not worth an error log.
fn log_send_error(e: &axum::Error) {
    match e
        .source()
        .and_then(|e| e.downcast_ref::<tungstenite::Error>())
    {
        Some(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {}
        Some(other) => error!("websocket error on event stream {other}"),
        None => error!("unexpected error type on websocket send {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::ClientMessage;

    #[test]
    fn identify_parses_with_and_without_resume() {
        let first: ClientMessage =
            serde_json::from_str(r#"{"type":"identify","sessionToken":"abc"}"#).unwrap();
        let ClientMessage::Identify {
            session_token,
            resume_after,
        } = first
        else {
            panic!("expected identify");
        };
        assert_eq!(session_token, "abc");
        assert_eq!(resume_after, None);

        let again: ClientMessage =
            serde_json::from_str(r#"{"type":"identify","sessionToken":"abc","resumeAfter":41}"#)
                .unwrap();
        let ClientMessage::Identify { resume_after, .. } = again else {
            panic!("expected identify");
        };
        assert_eq!(resume_after, Some(41));
    }

    #[test]
    fn activity_parses() {
        let frame: ClientMessage = serde_json::from_str(r#"{"type":"activity"}"#).unwrap();
        assert!(matches!(frame, ClientMessage::Activity));
    }

    #[test]
    fn unknown_message_types_are_rejected() {
        assert!(serde_json::from_str::<ClientMessage>(r#"{"type":"hello"}"#).is_err());
        assert!(serde_json::from_str::<ClientMessage>(r#"{"sessionToken":"abc"}"#).is_err());
    }
}
