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
//! Besides events, the stream carries what happens and is never kept (`ephemeral` frames: who
//! is typing, `app::typing`, and changes to the presence of those the client watches,
//! `app::presence_feed`), which has no sequence and is not replayed; the client says what its
//! user is typing, and whose presence it shows, on the same connection, and its closing ends
//! both.
//!
//! Errors are written in the language `?locale=` names on the upgrade URL, which the client
//! sets from its own language setting as it would `Accept-Language` (which a browser does not
//! let it set on a WebSocket), or else the one the upgrade's `Accept-Language` negotiates
//! (`app::locale`).
//!
//! Every frame in both directions is JSON. The full protocol is described by
//! `event_schema.json` (root type [`EventStreamProtocol`]).

use crate::extract::Query;
use crate::message_enum::server_event::ServerEvent;
use crate::rate_limit::{ClientIp, Connection};
use crate::t;
use aspen_app as app;
use aspen_app::context::GlobalServerContext;
use aspen_app::deployment_settings::DeploymentSettings;
use aspen_app::event_feed::{Delivery, FeedEvent, Refused, StreamEnd, StreamHold, Subscription};
use aspen_app::rate_limit::Decision;
use aspen_app::stream_admission::Identifying;
use aspen_app::two_factor::Caller;
use aspen_app::typing::{EphemeralEvent, Typist};
use aspen_app::user::UserPg;
use aspen_app::{ChannelId, UserId};
use axum::Extension;
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
use std::net::IpAddr;
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
    /// The user is typing in a channel: send it when they start, again every
    /// `TYPING_REFRESH_SECONDS` (3) while they go on, and not while they don't. Those who may
    /// view the channel are told by `ephemeral` frames, and show it for
    /// `TYPING_EXPIRY_SECONDS` (8) after the last. Taken only where the user may send messages;
    /// closer frames for one channel, and frames for more than a few channels at once, are
    /// ignored. Nothing answers it.
    #[serde(rename_all = "camelCase")]
    Typing { channel_id: ChannelId },
    /// The user stopped typing in a channel: they sent the message, emptied the box, or left
    /// it. A connection that closes says so on its own for every channel it was typing in.
    #[serde(rename_all = "camelCase")]
    StoppedTyping { channel_id: ChannelId },
    /// The channels the client has open where it shows who is typing, at most eight: it hears
    /// typing in these alone, so send it on every `ready` and whenever they change, an empty
    /// list when none is open. Each replaces the last. Nothing answers it.
    #[serde(rename_all = "camelCase")]
    Viewing { channel_ids: Vec<ChannelId> },
    /// The users whose presence the client shows, at most `MAX_WATCHED_PRESENCE` (500), the
    /// most wanted first: it is told by `ephemeral` `presence` frames of each as it is now, and
    /// then of each change, gathered for up to `PRESENCE_WINDOW_MILLIS`. Send it on every
    /// `ready` and whenever they change. Each replaces the last, and the server takes up at most
    /// one per window. Nothing else answers it.
    #[serde(rename_all = "camelCase")]
    WatchPresence { user_ids: Vec<UserId> },
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
    /// was open: signed out or revoked, which the `signInsEnded` event before this frame tells
    /// of, or expired. Close code 4401.
    Unauthorized,
    /// The server requires a second factor the account has not added yet; the session may only
    /// add one. Sent at `identify`, and to an open stream when the deployment starts requiring
    /// one. Close code 4403.
    TwoFactorEnrollmentRequired,
    /// The server requires a verified email address and the account's is not; the session may
    /// only verify, change, or resend it. Sent at `identify`, and to an open stream when the
    /// deployment starts requiring one. Close code 4428.
    EmailVerificationRequired,
    /// No `identify` frame arrived within the allowed time. Close code 4408.
    IdentifyTimeout,
    /// The account was banned from the deployment, which the `accountBanned` event before
    /// this frame tells of. Close code 4410.
    Banned,
    /// The account already holds as many event streams on this server as it may (`[limits]
    /// max_event_streams_per_user`). Close code 4429; the client keeps trying, and connects once
    /// one of its others closes.
    TooManyStreams,
    /// The client's network address already holds as many event streams on this server as it
    /// may before they identify (`[limits] max_event_streams_per_address`). Close code 4429.
    TooManyStreamsFromAddress,
    /// The server is taking on no more streams for now (`app::stream_admission`): so many
    /// connect at once that it serves those already connected first. Close code 1013. The client
    /// tries again no sooner than `retryAfterSeconds`, spreading its return over as long again.
    ServerBusy,
    /// The account opened streams faster than the event stream's limits per user allow
    /// (`[rate_limits]`). Close code 4429. The client tries again no sooner than
    /// `retryAfterSeconds`.
    RateLimited,
    /// The server failed while setting up or serving the stream. Close code 1011.
    Internal,
}

impl EventStreamErrorCode {
    fn close_code(self) -> u16 {
        match self {
            EventStreamErrorCode::BadRequest => 4400,
            EventStreamErrorCode::Unauthorized => 4401,
            EventStreamErrorCode::TwoFactorEnrollmentRequired => 4403,
            EventStreamErrorCode::EmailVerificationRequired => 4428,
            EventStreamErrorCode::IdentifyTimeout => 4408,
            EventStreamErrorCode::Banned => 4410,
            EventStreamErrorCode::TooManyStreams
            | EventStreamErrorCode::TooManyStreamsFromAddress
            | EventStreamErrorCode::RateLimited => 4429,
            EventStreamErrorCode::ServerBusy => 1013,
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
            EventStreamErrorCode::EmailVerificationRequired => {
                t!("problemEmailVerificationRequired")
            }
            EventStreamErrorCode::IdentifyTimeout => t!("eventStreamIdentifyTimeout"),
            EventStreamErrorCode::Banned => t!("eventStreamBanned"),
            EventStreamErrorCode::TooManyStreams => t!("eventStreamTooMany"),
            EventStreamErrorCode::TooManyStreamsFromAddress => t!("eventStreamTooManyFromAddress"),
            EventStreamErrorCode::ServerBusy => t!("eventStreamBusy"),
            EventStreamErrorCode::RateLimited => t!("eventStreamRateLimited"),
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
    /// Something that happens and is never kept: no sequence, never replayed on resuming, and
    /// lost with the connection. A client that loses its connection forgets what these told it.
    Ephemeral {
        #[schemars(with = "EphemeralEvent")]
        event: &'a RawValue,
    },
    /// Sent immediately before the server closes the connection because of a protocol or
    /// authentication failure. Never sent for an orderly shutdown.
    #[serde(rename_all = "camelCase")]
    Error {
        code: EventStreamErrorCode,
        /// Localized explanation suitable for display.
        detail: Cow<'a, str>,
        /// For `serverBusy` and `rateLimited`, the least number of seconds to wait before
        /// connecting again.
        #[serde(skip_serializing_if = "Option::is_none")]
        retry_after_seconds: Option<u64>,
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
    client: Option<Extension<ClientIp>>,
    connection: Option<Extension<Connection>>,
    Query(query): Query<EventStreamQuery>,
) -> Response {
    let ip = client.and_then(|Extension(ClientIp(ip))| ip);
    let connection = connection.map(|Extension(connection)| connection);
    // Counted from the upgrade until the stream identifies, so sockets that never do are bounded
    // too. A request with no address (none reaches here without one) is counted by user alone.
    let address = ip.map(|ip| {
        let key = state.rate_limiter.addresses().key(ip);
        state.event_feed.caps.hold_address(key)
    });
    let locale = query
        .locale
        .as_deref()
        .map_or_else(app::locale::current, app::locale::negotiate);
    // Client frames are small (`identify`, `activity`, `typing`), and events are written a few at a
    // time, so the buffers are a small fraction of tungstenite's defaults, which are sized for
    // bulk transfer and would otherwise dominate each connection's memory.
    ws.read_buffer_size(READ_BUFFER_BYTES)
        .write_buffer_size(WRITE_BUFFER_BYTES)
        .max_write_buffer_size(MAX_WRITE_BUFFER_BYTES)
        .max_message_size(MAX_CLIENT_MESSAGE_BYTES)
        .max_frame_size(MAX_CLIENT_MESSAGE_BYTES)
        .on_upgrade(move |socket| {
            app::locale::scope(locale, async move {
                let mut socket = socket;
                let address = match address {
                    Some(None) => {
                        count_connect("rejected");
                        reject(&mut socket, EventStreamErrorCode::TooManyStreamsFromAddress).await;
                        return;
                    }
                    held => held.flatten(),
                };
                let unidentified = Unidentified {
                    ip,
                    address,
                    connection,
                };
                handle_socket_conn(socket, state, unidentified).await;
            })
        })
}

/// Where a stream comes from, and what it holds there until it identifies.
struct Unidentified {
    ip: Option<IpAddr>,
    /// Its place under `[limits] max_event_streams_per_address`.
    address: Option<StreamHold>,
    /// The connection it was upgraded from, which counts toward its address's share until then.
    connection: Option<Connection>,
}

impl Unidentified {
    /// The stream identified as `user`, and holds a place under their cap: it no longer counts
    /// toward its address's.
    fn identified(self, user: UserId) {
        let Unidentified {
            address,
            connection,
            ..
        } = self;
        drop(address);
        if let Some(connection) = connection {
            connection.0.signed_in(user);
        }
    }
}

struct Rejection {
    code: EventStreamErrorCode,
    retry_after: Option<Duration>,
}

impl From<EventStreamErrorCode> for Rejection {
    fn from(code: EventStreamErrorCode) -> Self {
        Rejection {
            code,
            retry_after: None,
        }
    }
}

impl From<app::Error> for Rejection {
    fn from(e: app::Error) -> Self {
        error!("event stream setup failed: {e}");
        EventStreamErrorCode::Internal.into()
    }
}

impl From<StreamEnd> for EventStreamErrorCode {
    fn from(end: StreamEnd) -> Self {
        match end {
            StreamEnd::Banned => EventStreamErrorCode::Banned,
            StreamEnd::SignedOut => EventStreamErrorCode::Unauthorized,
        }
    }
}

impl From<Refused> for Rejection {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::Ended(end) => EventStreamErrorCode::from(end).into(),
            Refused::Failed(e) => e.into(),
        }
    }
}

/// Counts an identified stream as open for as long as it lives.
struct OpenStream;

impl OpenStream {
    fn new() -> Self {
        metrics::gauge!(aspen_metrics::api::EVENT_STREAMS).increment(1.0);
        aspen_app::fleet::note_event_stream(1);
        OpenStream
    }
}

impl Drop for OpenStream {
    fn drop(&mut self) {
        metrics::gauge!(aspen_metrics::api::EVENT_STREAMS).decrement(1.0);
        aspen_app::fleet::note_event_stream(-1);
    }
}

fn count_connect(outcome: &'static str) {
    metrics::counter!(aspen_metrics::api::EVENT_STREAM_CONNECTS, "outcome" => outcome).increment(1);
}

async fn handle_socket_conn(
    mut socket: WebSocket,
    state: GlobalServerContext,
    unidentified: Unidentified,
) {
    // Taken before `identify` checks the settings, so a change after that check is seen.
    let settings = state.settings.subscribe();
    let (session, identifying) = match identify(&mut socket, &state, unidentified.ip).await {
        Ok(identified) => identified,
        Err(rejection) => {
            count_connect(if rejection.code == EventStreamErrorCode::ServerBusy {
                "busy"
            } else {
                "rejected"
            });
            reject(&mut socket, rejection).await;
            return;
        }
    };
    let Some(_held) = state.event_feed.caps.hold_user(session.user.id) else {
        count_connect("rejected");
        reject(&mut socket, EventStreamErrorCode::TooManyStreams).await;
        return;
    };
    unidentified.identified(session.user.id);
    let subscription = match app::event_feed::subscribe(
        &state,
        session.user.id,
        session.sign_in,
        session.resume_after,
    )
    .await
    {
        Ok(subscription) => subscription,
        Err(refused) => {
            reject(&mut socket, Rejection::from(refused)).await;
            return;
        }
    };
    drop(identifying);
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
    let Identified {
        user,
        caller,
        expires,
        ..
    } = session;
    pump_events(
        socket,
        subscription,
        &state,
        &user,
        &caller,
        expires,
        settings,
    )
    .await;
}

struct Identified {
    user: UserPg,
    /// The session, as it stood when the stream was identified.
    caller: Caller,
    /// The sign-in the session belongs to.
    sign_in: app::event_feed::SignIn,
    /// When the sign-in expires, closing the stream; `None` for a bot's, which does not.
    expires: Option<tokio::time::Instant>,
    resume_after: Option<u64>,
}

/// Waits for the `identify` frame and authenticates it. Control frames that arrive first are
/// tolerated; any other data frame, a malformed body, or silence past the deadline rejects the
/// connection.
///
/// Answers the stream's place among those identifying (`app::stream_admission`), which its
/// caller holds until the stream is registered with the feed, the last of setting it up that
/// reads the database.
async fn identify(
    socket: &mut WebSocket,
    state: &GlobalServerContext,
    ip: Option<IpAddr>,
) -> Result<(Identified, Identifying), Rejection> {
    let deadline = tokio::time::Instant::now() + IDENTIFY_TIMEOUT;
    let text = loop {
        match tokio::time::timeout_at(deadline, socket.recv()).await {
            Err(_) => return Err(EventStreamErrorCode::IdentifyTimeout.into()),
            // The peer went away before identifying; there is nobody to reject.
            Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(Message::Close(_)))) => {
                return Err(EventStreamErrorCode::BadRequest.into());
            }
            Ok(Some(Ok(Message::Text(text)))) => break text,
            Ok(Some(Ok(Message::Binary(_)))) => {
                return Err(EventStreamErrorCode::BadRequest.into());
            }
            Ok(Some(Ok(Message::Ping(_) | Message::Pong(_)))) => continue,
        }
    };
    let ClientMessage::Identify {
        session_token,
        resume_after,
    } = serde_json::from_str(text.as_str()).map_err(|e| {
        debug!("malformed identify frame: {e}");
        Rejection::from(EventStreamErrorCode::BadRequest)
    })?
    else {
        debug!("the first frame was not identify");
        return Err(EventStreamErrorCode::BadRequest.into());
    };
    let busy = Rejection {
        code: EventStreamErrorCode::ServerBusy,
        retry_after: Some(app::stream_admission::BUSY_RETRY_AFTER),
    };
    let identifying = state.stream_admission.admit().await.ok_or(busy)?;
    let (user, caller) = app::user::user_for_token(state, &session_token)
        .await?
        .ok_or(Rejection::from(EventStreamErrorCode::Unauthorized))?;
    if caller.enrollment_required(&state.settings()) {
        return Err(EventStreamErrorCode::TwoFactorEnrollmentRequired.into());
    }
    if caller.verification_required(&state.settings()) {
        return Err(EventStreamErrorCode::EmailVerificationRequired.into());
    }
    match crate::rate_limit::limit_event_stream(state, ip, user.id).await {
        Decision::Allowed => {}
        Decision::Limited { retry_after } => {
            return Err(Rejection {
                code: EventStreamErrorCode::RateLimited,
                retry_after: Some(retry_after),
            });
        }
        Decision::Unavailable => {
            return Err(Rejection {
                code: EventStreamErrorCode::ServerBusy,
                retry_after: Some(app::stream_admission::BUSY_RETRY_AFTER),
            });
        }
    }
    let times = app::user::sign_in_times(state, &caller).await?;
    let expires = times.expires.map(|at| {
        let left = (at - chrono::Utc::now()).to_std().unwrap_or_default();
        tokio::time::Instant::now() + left
    });
    app::user_status::mark_user_online(state, &user);
    let identified = Identified {
        user,
        sign_in: app::event_feed::SignIn {
            id: caller.sign_in(),
            began: times.began,
        },
        caller,
        expires,
        resume_after,
    };
    Ok((identified, identifying))
}

/// Most frames written to the socket before it is flushed, so a burst goes out in few writes
/// without one connection holding the task for long.
const FLUSH_EVERY: usize = 64;

/// `event`'s frame, serialized by the first connection to write it and shared by the rest
/// (`FeedEvent::frame`): an `event` frame, or an `ephemeral` one for what is never kept.
fn event_frame(event: &FeedEvent) -> Result<Message, axum::Error> {
    let bytes = event.frame(|event| {
        let frame = if event.is_ephemeral() {
            ServerMessage::Ephemeral {
                event: &event.payload,
            }
        } else {
            ServerMessage::Event {
                sequence: event.sequence,
                event_id: event.event_id.as_deref(),
                event: &event.payload,
            }
        };
        serde_json::to_string(&frame).map_err(axum::Error::new)
    })?;
    let text = axum::extract::ws::Utf8Bytes::try_from(bytes).map_err(axum::Error::new)?;
    Ok(Message::Text(text))
}

/// What writing a delivery did: how many frames, and whether one of them ends the connection.
struct Fed {
    written: usize,
    ends: Option<StreamEnd>,
    /// When one of the events told the user their email account changed, whether the last of
    /// them says it now holds an address it has not verified.
    email_unverified: Option<bool>,
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
                email_unverified: events
                    .iter()
                    .rev()
                    .find_map(|event| event.email_unverified()),
            })
        }
        Delivery::Live(event) => {
            socket.feed(event_frame(&event)?).await?;
            Ok(Fed {
                written: 1,
                ends: event.ends(),
                email_unverified: event.email_unverified(),
            })
        }
        Delivery::Ephemeral(event) => {
            socket.feed(event_frame(&event)?).await?;
            Ok(Fed {
                written: 1,
                ends: None,
                email_unverified: None,
            })
        }
    }
}

/// Whether `settings` require a verified email address and `user`'s, as it is now, is not. A
/// failure to read it is logged and keeps the stream open; the next request decides.
async fn owes_verified_email(
    state: &GlobalServerContext,
    user: aspen_app::UserId,
    settings: &DeploymentSettings,
) -> bool {
    if !settings.email_verification_required {
        return false;
    }
    match app::email::unverified(state, user).await {
        Ok(unverified) => unverified,
        Err(e) => {
            warn!(error = %e, "could not read whether an email address is verified");
            false
        }
    }
}

/// Delivers the event feed to the socket until either side ends it, for `caller`'s session of
/// `user_row`. `settings` is this server's copy of the deployment's settings: when the
/// deployment starts requiring a second factor the session's account lacked when it identified,
/// or a verified email address, the stream closes, and the client's next `identify` decides
/// afresh, so one that added a factor meanwhile reconnects. Whether the address is verified is
/// read again when the settings change, and taken from each `emailAccountChanged` of the user,
/// since they may change their address while the stream is open. The stream closes as
/// unauthorized at `expires`, when the sign-in it belongs to runs out.
async fn pump_events(
    mut socket: WebSocket,
    mut subscription: Subscription,
    state: &GlobalServerContext,
    user_row: &UserPg,
    caller: &Caller,
    expires: Option<tokio::time::Instant>,
    mut settings: watch::Receiver<Arc<DeploymentSettings>>,
) {
    let user = user_row.id;
    let bot = user_row.bot;
    let mut ping_interval =
        tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
    let mut unanswered_pings: u32 = 0;
    let mut last_activity: Option<tokio::time::Instant> = None;
    // Dropped with the connection, which says the user stopped wherever they were typing.
    let typist = Typist::spawn(state.clone(), user);
    // Dropped with the connection, which ends its watch.
    let mut presence = state.presence_feed.watch(user);
    loop {
        tokio::select! {
            delivery = subscription.deliveries.recv() => {
                // The feed let go of this connection: it fell a whole queue behind, or the feed
                // missed events. Either way the client resumes from what it last processed.
                let Some(delivery) = delivery else {
                    debug!("the event feed dropped a connection");
                    return;
                };
                let Fed { mut written, mut ends, mut email_unverified } = match feed_delivery(&mut socket, delivery).await {
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
                            email_unverified = more.email_unverified.or(email_unverified);
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
                    reject(&mut socket, EventStreamErrorCode::from(end)).await;
                    return;
                }
                let required = settings.borrow().email_verification_required;
                if required && email_unverified == Some(true) {
                    reject(&mut socket, EventStreamErrorCode::EmailVerificationRequired).await;
                    return;
                }
            },
            Some(statuses) = presence.next() => {
                if let Err(e) = send_presence(&mut socket, statuses).await {
                    log_send_error(&e);
                    return;
                }
            },
            () = async {
                match expires {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                reject(&mut socket, EventStreamErrorCode::Unauthorized).await;
                return;
            },
            changed = settings.changed() => {
                // The server is stopping when its copy of the settings is gone.
                if changed.is_err() {
                    return;
                }
                let settings = settings.borrow_and_update().clone();
                if caller.enrollment_required(&settings) {
                    reject(&mut socket, EventStreamErrorCode::TwoFactorEnrollmentRequired).await;
                    return;
                }
                if owes_verified_email(state, user, &settings).await {
                    reject(&mut socket, EventStreamErrorCode::EmailVerificationRequired).await;
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
            // `activity`, typing, and watching frames, but the WebSocket protocol's control frames (Close, Ping, Pong)
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
                    Some(Ok(Message::Text(text))) => match serde_json::from_str(text.as_str()) {
                        Ok(ClientMessage::Activity) => {
                            let now = tokio::time::Instant::now();
                            let due = last_activity.is_none_or(|last| {
                                now.duration_since(last) + ACTIVITY_GRACE >= ACTIVITY_INTERVAL
                            });
                            if due {
                                last_activity = Some(now);
                                app::user_status::mark_active(state, user);
                            }
                        }
                        Ok(ClientMessage::Typing { channel_id }) => typist.typing(channel_id),
                        Ok(ClientMessage::StoppedTyping { channel_id }) => {
                            typist.stopped(channel_id);
                        }
                        Ok(ClientMessage::Viewing { channel_ids }) => {
                            subscription.viewing(channel_ids);
                        }
                        Ok(ClientMessage::WatchPresence { user_ids }) => presence.set(user_ids),
                        // Anything else (a second `identify`, an unknown frame) is dropped
                        // rather than tearing the connection down.
                        Ok(ClientMessage::Identify { .. }) | Err(_) => {}
                    },
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

/// Writes a `presence` frame telling of `statuses`.
async fn send_presence(
    socket: &mut WebSocket,
    statuses: Vec<crate::user::UserStatusRecord>,
) -> Result<(), axum::Error> {
    let event = serde_json::value::to_raw_value(&EphemeralEvent::Presence { statuses })
        .map_err(axum::Error::new)?;
    send_json(socket, &ServerMessage::Ephemeral { event: &event }).await
}

async fn send_json(socket: &mut WebSocket, message: &ServerMessage<'_>) -> Result<(), axum::Error> {
    let text = serde_json::to_string(message).map_err(axum::Error::new)?;
    socket.send(Message::Text(text.into())).await
}

/// Tells the peer why it is being dropped, then closes with the matching close code. Both
/// sends are best-effort; the peer may already be gone.
async fn reject(socket: &mut WebSocket, rejection: impl Into<Rejection>) {
    let Rejection { code, retry_after } = rejection.into();
    let _ = send_json(
        socket,
        &ServerMessage::Error {
            code,
            detail: code.detail(),
            // Rounded up, so a client waiting this long finds the limit allows it.
            retry_after_seconds: retry_after.map(|wait| wait.as_millis().div_ceil(1000) as u64),
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
    fn typing_parses() {
        let channel = "0199b6a1-0000-7000-8000-000000000001";
        let frame: ClientMessage =
            serde_json::from_str(&format!(r#"{{"type":"typing","channelId":"{channel}"}}"#))
                .unwrap();
        assert!(
            matches!(frame, ClientMessage::Typing { channel_id } if channel_id.0.to_string() == channel)
        );
        let frame: ClientMessage = serde_json::from_str(&format!(
            r#"{{"type":"stoppedTyping","channelId":"{channel}"}}"#
        ))
        .unwrap();
        assert!(matches!(frame, ClientMessage::StoppedTyping { .. }));
    }

    #[test]
    fn ephemeral_frames_carry_no_sequence() {
        let payload = r#"{"type":"typing","channelId":"c","userId":"u","typing":true}"#;
        let event = RawValue::from_string(payload.into()).unwrap();
        let frame = ServerMessage::Ephemeral { event: &event };
        assert_eq!(
            serde_json::to_string(&frame).unwrap(),
            format!(r#"{{"type":"ephemeral","event":{payload}}}"#)
        );
    }

    #[test]
    fn unknown_message_types_are_rejected() {
        assert!(serde_json::from_str::<ClientMessage>(r#"{"type":"hello"}"#).is_err());
        assert!(serde_json::from_str::<ClientMessage>(r#"{"sessionToken":"abc"}"#).is_err());
    }
}
