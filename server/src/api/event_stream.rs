//! WebSocket event stream at `/api/v1/events`.
//!
//! Browsers cannot attach headers to a WebSocket upgrade, so authentication happens in-band:
//! the client's first frame must be an [`ClientMessage::Identify`] carrying a session token,
//! sent within [`IDENTIFY_TIMEOUT`]. The server answers with [`ServerMessage::Ready`] and then
//! streams [`ServerMessage::Event`] frames, each tagged with its JetStream sequence number.
//! The events come from the server's shared feed (`app::event_feed`), which routes to each
//! connection only what its user is entitled to (`app::events`): their own subject and every
//! community they belong to, following their memberships as they change.
//!
//! A client that reconnects passes the last sequence it processed as `resumeAfter`. When that
//! position is still inside the stream's retention window the server replays exactly what was
//! missed and reports `resumed: true`; otherwise it replays the whole `MAX_EVENT_AGE` window
//! and reports `resumed: false`, which tells the client its cached state has a gap it must
//! repair from REST.
//!
//! Errors are written in the language `?locale=` names on the upgrade URL, which the client
//! sets from its own language setting as it would `Accept-Language` (which a browser does not
//! let it set on a WebSocket), or else the one the upgrade's `Accept-Language` negotiates
//! (`app::locale`).
//!
//! Every frame in both directions is JSON. The full protocol is described by
//! `event_schema.json` (root type [`EventStreamProtocol`]).

use crate::api::extract::Query;
use crate::api::message_enum::server_event::ServerEvent;
use crate::app;
use crate::app::UserId;
use crate::app::context::GlobalServerContext;
use crate::app::deployment_settings::DeploymentSettings;
use crate::app::event_feed::{Delivery, FeedEvent, StreamEnd, Subscription};
use crate::app::two_factor::Caller;
use crate::app::user::UserPg;
use crate::t;
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::response::Response;
use bytes::Bytes;
use futures_util::SinkExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use std::borrow::Cow;
use std::error::Error;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{debug, error, warn};

/// How long a freshly upgraded socket may stay silent before it is closed for not identifying.
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);
/// Cadence of server-initiated pings. Browsers answer pings automatically, so this both keeps
/// idle NAT and proxy mappings alive and detects peers that vanished without a close frame.
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// Consecutive unanswered pings after which the peer is presumed gone.
const MAX_MISSED_PONGS: u32 = 2;

/// The socket's read buffer.
const READ_BUFFER_BYTES: usize = 4 * 1024;
/// Frames are written through once this much is buffered, and on every flush.
const WRITE_BUFFER_BYTES: usize = 16 * 1024;
/// The most a socket may buffer for writing before the write fails; a client that cannot
/// keep up is instead dropped by the feed when its queue fills.
const MAX_WRITE_BUFFER_BYTES: usize = 1024 * 1024;
/// The largest frame a client may send.
const MAX_CLIENT_MESSAGE_BYTES: usize = 64 * 1024;

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
    /// The session token is missing, unknown, or expired, or its sign-in ended while the stream
    /// was open, which the `signInsEnded` event before this frame tells of. Close code 4401.
    Unauthorized,
    /// The server requires a second factor the account has not added yet; the session may only
    /// add one. Sent at `identify`, and to an open stream when the deployment starts requiring
    /// one. Close code 4403.
    TwoFactorEnrollmentRequired,
    /// No `identify` frame arrived within the allowed time. Close code 4408.
    IdentifyTimeout,
    /// The account was banned from the deployment, which the `accountBanned` event before
    /// this frame tells of. Close code 4410.
    Banned,
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
            EventStreamErrorCode::Banned => 4410,
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
            EventStreamErrorCode::Banned => t!("eventStreamBanned"),
            EventStreamErrorCode::Internal => t!("eventStreamError"),
        }
    }
}

/// Frames the server sends.
#[derive(Serialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerMessage<'a> {
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
        event_id: Option<&'a str>,
        #[schemars(with = "ServerEvent")]
        event: &'a RawValue,
    },
    /// Sent immediately before the server closes the connection because of a protocol or
    /// authentication failure. Never sent for an orderly shutdown.
    Error {
        code: EventStreamErrorCode,
        /// Localized explanation suitable for display.
        detail: Cow<'a, str>,
    },
}

/// Root of `event_schema.json`: both directions of the WebSocket protocol. Never instantiated;
/// it exists so one schema document describes everything a client needs.
#[derive(JsonSchema)]
#[allow(dead_code)]
pub struct EventStreamProtocol {
    pub client: ClientMessage,
    pub server: ServerMessage<'static>,
}

/// The event stream's query parameters.
#[derive(Debug, Deserialize)]
pub struct EventStreamQuery {
    /// The languages to write errors in, as `Accept-Language` names them.
    locale: Option<String>,
}

pub async fn event_stream(
    ws: WebSocketUpgrade,
    State(state): State<GlobalServerContext>,
    Query(query): Query<EventStreamQuery>,
) -> Response {
    let locale = query
        .locale
        .as_deref()
        .map_or_else(app::locale::current, app::locale::negotiate);
    // Client frames are small (`identify`, `activity`), and events are written a few at a
    // time, so the buffers are a small fraction of tungstenite's defaults, which are sized for
    // bulk transfer and would otherwise dominate each connection's memory.
    ws.read_buffer_size(READ_BUFFER_BYTES)
        .write_buffer_size(WRITE_BUFFER_BYTES)
        .max_write_buffer_size(MAX_WRITE_BUFFER_BYTES)
        .max_message_size(MAX_CLIENT_MESSAGE_BYTES)
        .max_frame_size(MAX_CLIENT_MESSAGE_BYTES)
        .on_upgrade(move |socket| app::locale::scope(locale, handle_socket_conn(socket, state)))
}

struct Rejection(EventStreamErrorCode);

impl From<app::Error> for Rejection {
    fn from(e: app::Error) -> Self {
        error!("event stream setup failed: {e}");
        Rejection(EventStreamErrorCode::Internal)
    }
}

/// Counts an identified stream as open for as long as it lives.
struct OpenStream;

impl OpenStream {
    fn new() -> Self {
        metrics::gauge!(aspen_metrics::api::EVENT_STREAMS).increment(1.0);
        crate::app::fleet::note_event_stream(1);
        OpenStream
    }
}

impl Drop for OpenStream {
    fn drop(&mut self) {
        metrics::gauge!(aspen_metrics::api::EVENT_STREAMS).decrement(1.0);
        crate::app::fleet::note_event_stream(-1);
    }
}

fn count_connect(outcome: &'static str) {
    metrics::counter!(aspen_metrics::api::EVENT_STREAM_CONNECTS, "outcome" => outcome).increment(1);
}

async fn handle_socket_conn(mut socket: WebSocket, state: GlobalServerContext) {
    // Taken before `identify` checks the settings, so a change after that check is seen.
    let settings = state.settings.subscribe();
    let session = match identify(&mut socket, &state).await {
        Ok(session) => session,
        Err(Rejection(code)) => {
            count_connect("rejected");
            reject(&mut socket, code).await;
            return;
        }
    };
    let subscription = match app::event_feed::subscribe(
        &state,
        session.user.id,
        session.sign_in,
        session.resume_after,
    )
    .await
    {
        Ok(subscription) => subscription,
        Err(e) => {
            let Rejection(code) = e.into();
            reject(&mut socket, code).await;
            return;
        }
    };
    count_connect(if subscription.resumed {
        "resumed"
    } else {
        "replayed"
    });
    let _open = OpenStream::new();
    let ready = ServerMessage::Ready {
        user_id: session.user.id,
        resumed: subscription.resumed,
    };
    if let Err(e) = send_json(&mut socket, &ready).await {
        log_send_error(&e);
        return;
    }
    let Identified { user, caller, .. } = session;
    pump_events(socket, subscription, &state, &user, &caller, settings).await;
}

struct Identified {
    user: UserPg,
    /// The session, as it stood when the stream was identified.
    caller: Caller,
    /// The sign-in the session belongs to (`app::login::sign_in_id`).
    sign_in: String,
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
    if caller.enrollment_required(&state.settings()) {
        return Err(Rejection(EventStreamErrorCode::TwoFactorEnrollmentRequired));
    }
    app::user_status::mark_user_online(state, &user);
    Ok(Identified {
        user,
        sign_in: app::login::sign_in_id(&caller.refresh_token),
        caller,
        resume_after,
    })
}

/// Most frames written to the socket before it is flushed, so a burst goes out in few writes
/// without one connection holding the task for long.
const FLUSH_EVERY: usize = 64;

fn event_frame(event: &FeedEvent) -> Result<Message, axum::Error> {
    let frame = ServerMessage::Event {
        sequence: event.sequence,
        event_id: event.event_id.as_deref(),
        event: &event.payload,
    };
    let text = serde_json::to_string(&frame).map_err(axum::Error::new)?;
    Ok(Message::Text(text.into()))
}

/// What writing a delivery did: how many frames, and whether one of them ends the connection.
struct Fed {
    written: usize,
    ends: Option<StreamEnd>,
}

/// Writes one delivery's frames without flushing.
async fn feed_delivery(socket: &mut WebSocket, delivery: Delivery) -> Result<Fed, axum::Error> {
    match delivery {
        Delivery::CatchUp(events) => {
            for event in &events {
                socket.feed(event_frame(event)?).await?;
            }
            Ok(Fed {
                written: events.len(),
                ends: events.iter().find_map(|event| event.ends()),
            })
        }
        Delivery::Live(event) => {
            socket.feed(event_frame(&event)?).await?;
            Ok(Fed {
                written: 1,
                ends: event.ends(),
            })
        }
    }
}

/// Delivers the event feed to the socket until either side ends it, for `caller`'s session of
/// `user_row`. `settings` is this server's copy of the deployment's settings: when the deployment starts requiring a second
/// factor the session's account lacked when it identified, the stream closes, and the client's
/// next `identify` decides afresh, so one that added a factor meanwhile reconnects.
async fn pump_events(
    mut socket: WebSocket,
    mut subscription: Subscription,
    state: &GlobalServerContext,
    user_row: &UserPg,
    caller: &Caller,
    mut settings: watch::Receiver<Arc<DeploymentSettings>>,
) {
    let user = user_row.id;
    let bot = user_row.bot;
    let mut ping_interval =
        tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
    let mut unanswered_pings: u32 = 0;
    let mut last_activity: Option<tokio::time::Instant> = None;
    loop {
        tokio::select! {
            delivery = subscription.deliveries.recv() => {
                // The feed let go of this connection: it fell a whole queue behind, or the feed
                // missed events. Either way the client resumes from what it last processed.
                let Some(delivery) = delivery else {
                    debug!("the event feed dropped a connection");
                    return;
                };
                let Fed { mut written, mut ends } = match feed_delivery(&mut socket, delivery).await {
                    Ok(fed) => fed,
                    Err(e) => {
                        log_send_error(&e);
                        return;
                    }
                };
                while written < FLUSH_EVERY && ends.is_none() {
                    let Ok(delivery) = subscription.deliveries.try_recv() else {
                        break;
                    };
                    match feed_delivery(&mut socket, delivery).await {
                        Ok(more) => {
                            written += more.written;
                            ends = more.ends;
                        }
                        Err(e) => {
                            log_send_error(&e);
                            return;
                        }
                    }
                }
                if let Err(e) = socket.flush().await {
                    log_send_error(&e);
                    return;
                }
                metrics::counter!(aspen_metrics::api::EVENTS_DELIVERED).increment(written as u64);
                if let Some(end) = ends {
                    let code = match end {
                        StreamEnd::Banned => EventStreamErrorCode::Banned,
                        StreamEnd::SignedOut => EventStreamErrorCode::Unauthorized,
                    };
                    reject(&mut socket, code).await;
                    return;
                }
            },
            changed = settings.changed() => {
                // The server is stopping when its copy of the settings is gone.
                if changed.is_err() {
                    return;
                }
                let owes_factor = caller.enrollment_required(&settings.borrow_and_update());
                if owes_factor {
                    reject(&mut socket, EventStreamErrorCode::TwoFactorEnrollmentRequired).await;
                    return;
                }
            },
            _ = ping_interval.tick() => {
                if unanswered_pings >= MAX_MISSED_PONGS {
                    warn!("event stream peer stopped answering pings, closing");
                    return;
                }
                unanswered_pings += 1;
                // The connection is the user's presence: the key is refreshed as long as it is up.
                app::user_status::mark_user_online_id(state, user, bot);
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
        }
    }
}

async fn send_json(socket: &mut WebSocket, message: &ServerMessage<'_>) -> Result<(), axum::Error> {
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
    use super::{ClientMessage, ServerMessage};
    use serde_json::value::RawValue;

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
    fn event_frames_carry_the_event_verbatim() {
        let payload = r#"{"serverEvent":"message","type":"delete","id":"x"}"#;
        let event = RawValue::from_string(payload.into()).unwrap();
        let frame = ServerMessage::Event {
            sequence: 7,
            event_id: Some("e"),
            event: &event,
        };
        assert_eq!(
            serde_json::to_string(&frame).unwrap(),
            format!(r#"{{"type":"event","sequence":7,"eventId":"e","event":{payload}}}"#)
        );
        let frame = ServerMessage::Event {
            sequence: 8,
            event_id: None,
            event: &event,
        };
        assert!(!serde_json::to_string(&frame).unwrap().contains("eventId"));
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
