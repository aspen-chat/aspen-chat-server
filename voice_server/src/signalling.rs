//! One client's WebSocket: frames in are dispatched to the rooms, frames out come from the
//! participant's outbox. The first frame must be `identify` with a valid join token, or
//! `resume` with the secret a participant was given, to take it back. Every
//! route and frame is limited (`limits`). The server pings every `PING_INTERVAL` and closes a
//! socket it has heard nothing from, pongs included, for `IDLE_TIMEOUT`, and one whose outbox
//! overflows (`outbox`), so a client that stops reading is let go.

use crate::limits::{Caller, HEALTH, Limits, PendingSocket, SIGNALLING};
use crate::outbox::{OUTBOX_BYTES, Outbox};
use crate::rooms::{RoomError, Rooms};
use axum::extract::{ConnectInfo, MatchedPath, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header::RETRY_AFTER};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use uuid::Uuid;
use voice_protocol::signal::{ClientMessage, ServerMessage};
use voice_protocol::token::{JoinClaims, TokenError, key_of, verify, verify_shared};
use yawc::close::CloseCode;
use yawc::frame::{Frame, OpCode};
use yawc::{DeflatePool, HttpWebSocket, IncomingUpgrade, Options};

/// The paths of this server's two routes: the router serves them and `limit_http` finds each
/// one's limits by them.
pub const HEALTH_PATH: &str = "/health";
pub const SIGNALLING_PATH: &str = "/ws";

/// How long a client has to identify before the socket is closed.
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the server pings a socket. Browsers answer on their own.
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// How long a socket may go without the server hearing anything, a pong included, before it is
/// closed: two pings missed.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a closing socket is given to write what is queued for it before it is dropped.
const CLOSE_GRACE: Duration = Duration::from_secs(5);
/// Frames shorter than this go uncompressed: deflate rarely shrinks a few dozen bytes, and a
/// socket that only ever sends small frames never takes a compressor.
const COMPRESS_FROM_BYTES: usize = 256;

#[derive(Clone)]
pub struct AppState {
    pub server: Uuid,
    /// The shared secret tokens of that form are checked under, when this server is given one.
    pub token_secret: Option<Arc<str>>,
    pub token_keys: Arc<crate::token_keys::TokenKeys>,
    pub rooms: Arc<Rooms>,
    pub limits: Arc<Limits>,
    pub used_tokens: Arc<UsedTokens>,
    /// The deflate state every socket borrows a message at a time (`socket_options`).
    pub deflate: DeflatePool,
}

/// How each signalling socket is set up. Frames are compressed (permessage-deflate), which a
/// browser negotiates on its own: an offer is about a kilobyte per section and repeats most of
/// it section to section, so it deflates to about a ninth, and a call whose slots change speaker
/// sends one to each listener every few seconds. Neither side keeps deflate state between
/// messages, and what a socket compresses with is borrowed from `pool` for one message, so an
/// idle socket costs a few kilobytes rather than the few hundred its own deflate state would;
/// each offer is compressed whole, which loses little against one compressed after the last.
pub fn socket_options(max_message_bytes: usize, pool: &DeflatePool) -> Options {
    Options::default()
        .with_low_latency_compression()
        .server_no_context_takeover()
        .client_no_context_takeover()
        .with_deflate_pool(pool.clone())
        .with_compression_threshold(COMPRESS_FROM_BYTES)
        .with_limits(max_message_bytes, max_message_bytes)
        .with_utf8()
}

/// The join tokens this server has accepted that have not yet expired, by nonce. A token
/// admits one connection: someone removed from a call cannot come back on the token they
/// joined with, and every join asks the API server, which decides again.
#[derive(Default)]
pub struct UsedTokens(std::sync::Mutex<HashMap<Uuid, i64>>);

impl UsedTokens {
    /// Records a token with `nonce`, expiring at `expires_at`, as used at `now`. Returns false
    /// when it already was.
    fn claim(&self, nonce: Uuid, expires_at: i64, now: i64) -> bool {
        let mut used = self.0.lock().expect("used tokens lock");
        used.retain(|_, expires| *expires > now);
        used.insert(nonce, expires_at).is_none()
    }
}

/// The client address of a request, behind any trusted proxies.
fn client_ip(state: &AppState, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    let forwarded = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok());
    state.limits.client(peer.ip(), forwarded)
}

fn too_many(wait: Duration) -> Response {
    let seconds = u64::try_from(wait.as_millis().div_ceil(1000).max(1)).unwrap_or(u64::MAX);
    let mut response = StatusCode::TOO_MANY_REQUESTS.into_response();
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from(seconds));
    response
}

/// The name a route's limits are kept under (`limits`), by its path.
fn limited_as(path: &str) -> Option<&'static str> {
    match path {
        HEALTH_PATH => Some(HEALTH),
        SIGNALLING_PATH => Some(SIGNALLING),
        _ => None,
    }
}

/// Middleware on both routes: counts the request against its route's limits. A route under it
/// that `limited_as` does not name is refused rather than served unlimited.
pub async fn limit_http(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    matched: MatchedPath,
    request: Request,
    next: Next,
) -> Response {
    let Some(route) = limited_as(matched.as_str()) else {
        error!(
            path = matched.as_str(),
            "a route has no rate limits and is refused; name it in `limited_as`"
        );
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let ip = client_ip(&state, peer, request.headers());
    match state.limits.check_http(route, ip) {
        Ok(()) => next.run(request).await,
        Err(wait) => {
            crate::metrics::http_refused(route);
            too_many(wait)
        }
    }
}

pub async fn upgrade(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    ws: IncomingUpgrade,
) -> Response {
    let ip = client_ip(&state, peer, &headers);
    let Some(pending) = state.limits.pending_socket(ip) else {
        crate::metrics::http_refused(SIGNALLING);
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let options = socket_options(state.limits.max_message_bytes, &state.deflate);
    let Ok((response, upgraded)) = ws.upgrade(options) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    tokio::spawn(async move {
        match upgraded.await {
            Ok(socket) => handle(socket, state, ip, pending).await,
            Err(e) => info!(error = %e, "a signalling socket's upgrade did not complete"),
        }
    });
    response.into_response()
}

fn now() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_secs(),
    )
    .unwrap_or(i64::MAX)
}

/// Closes the socket: once every sender of `outbox` is gone the writer finishes what is queued
/// and closes, given `CLOSE_GRACE` to, after which it is dropped with whatever it holds.
async fn close(outbox: Outbox, mut writer: tokio::task::JoinHandle<()>) {
    drop(outbox);
    if tokio::time::timeout(CLOSE_GRACE, &mut writer)
        .await
        .is_err()
    {
        writer.abort();
    }
}

async fn handle(socket: HttpWebSocket, state: AppState, ip: IpAddr, pending: PendingSocket) {
    let (mut sink, mut stream) = socket.split();
    let (outbox, mut inbox) = Outbox::new(OUTBOX_BYTES);
    // Frames for the client are written by their own task so room work never waits on a
    // slow socket. It pings between them.
    let writer = tokio::spawn(async move {
        let mut ping =
            tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
        loop {
            let frame = tokio::select! {
                text = inbox.recv() => match text {
                    Some(text) => Frame::text(text),
                    None => break,
                },
                _ = ping.tick() => Frame::ping(Vec::new()),
            };
            if sink.send(frame).await.is_err() {
                break;
            }
        }
        // Closing the sink ends the stream alone, so the close frame goes first.
        let _ = sink.send(Frame::close(CloseCode::Normal, "")).await;
        let _ = sink.close().await;
    });

    let first = match tokio::time::timeout(IDENTIFY_TIMEOUT, next_frame(&mut stream)).await {
        Ok(Some(Received::Frame(frame))) => {
            let kind = frame.kind();
            match frame {
                ClientMessage::Identify { token } => Some((kind, First::Identify(token))),
                ClientMessage::Resume { secret } => Some((kind, First::Resume(secret))),
                _ => None,
            }
        }
        _ => None,
    };
    let Some((first_kind, first)) = first else {
        outbox.send(&ServerMessage::Error {
            detail: "the first frame must be identify or resume".to_string(),
            fatal: true,
            retry_after_seconds: None,
            refused: None,
        });
        close(outbox, writer).await;
        return;
    };
    let fatal = |detail: String, retry_after_seconds: Option<u64>| ServerMessage::Error {
        detail,
        fatal: true,
        retry_after_seconds,
        refused: None,
    };
    let seat = match first {
        First::Identify(token) => {
            let at = now();
            let claims = match check_token(&state, &token, at).await.and_then(|claims| {
                if state.used_tokens.claim(claims.nonce, claims.expires_at, at) {
                    Ok(claims)
                } else {
                    Err(TokenError::Used)
                }
            }) {
                Ok(claims) => claims,
                Err(e) => {
                    outbox.send(&fatal(e.to_string(), None));
                    close(outbox, writer).await;
                    return;
                }
            };
            let caller = Caller {
                ip: Some(ip),
                user: Some(claims.user),
                channel: Some(claims.channel),
            };
            if let Err(wait) = state.limits.check_frame(first_kind, &caller) {
                let seconds = wait.as_secs().max(1);
                outbox.send(&fatal(
                    format!("too many {first_kind} frames; try again in {seconds}s"),
                    Some(seconds),
                ));
                close(outbox, writer).await;
                return;
            }
            // Identified: the socket no longer counts against its address's unidentified ones.
            drop(pending);
            match state
                .rooms
                .join(
                    claims.channel,
                    claims.user,
                    outbox.clone(),
                    (&claims).into(),
                )
                .await
            {
                Ok(seat) => seat,
                Err(e) => {
                    warn!(error = e.to_string(), "join failed");
                    outbox.send(&fatal(e.to_string(), None));
                    close(outbox, writer).await;
                    return;
                }
            }
        }
        First::Resume(secret) => {
            // Limited by address before the secret is looked up, so secrets cannot be guessed
            // at any speed.
            let caller = Caller {
                ip: Some(ip),
                user: None,
                channel: None,
            };
            if let Err(wait) = state.limits.check_frame(first_kind, &caller) {
                let seconds = wait.as_secs().max(1);
                outbox.send(&fatal(
                    format!("too many {first_kind} frames; try again in {seconds}s"),
                    Some(seconds),
                ));
                close(outbox, writer).await;
                return;
            }
            drop(pending);
            match state.rooms.resume(&secret, outbox.clone()) {
                Ok(seat) => seat,
                Err(e) => {
                    outbox.send(&fatal(e.to_string(), None));
                    close(outbox, writer).await;
                    return;
                }
            }
        }
    };
    let (user, channel) = (seat.user, seat.channel);
    let caller = Caller {
        ip: Some(ip),
        user: Some(user),
        channel: Some(channel),
    };

    let mut left = false;
    loop {
        let frame = tokio::select! {
            frame = next_frame(&mut stream) => frame,
            () = outbox.hung_up() => None,
        };
        let frame = match frame {
            Some(Received::Frame(frame)) => frame,
            Some(Received::Unknown { kind, detail }) => {
                // Counted against `any` alone, so a flood of them is refused like any other.
                crate::metrics::frame(UNKNOWN_FRAME);
                let error = match state.limits.check_frame(UNKNOWN_FRAME, &caller) {
                    Ok(()) => {
                        warn!(
                            user = user.to_string(),
                            error = detail,
                            "a client frame this server does not know was refused"
                        );
                        ServerMessage::Error {
                            detail: format!("not a frame this server knows: {detail}"),
                            fatal: false,
                            retry_after_seconds: None,
                            refused: kind,
                        }
                    }
                    Err(wait) => {
                        crate::metrics::frame_refused(UNKNOWN_FRAME);
                        ServerMessage::Error {
                            detail: format!(
                                "too many frames; try again in {}s",
                                wait.as_secs().max(1)
                            ),
                            fatal: false,
                            retry_after_seconds: Some(wait.as_secs().max(1)),
                            refused: kind,
                        }
                    }
                };
                outbox.send(&error);
                continue;
            }
            None => break,
        };
        if matches!(frame, ClientMessage::Leave) {
            left = true;
            break;
        }
        let kind = frame.kind();
        crate::metrics::frame(kind);
        // Muting, deafening, and holding always go through; only lifting them is limited.
        let quietens = matches!(frame, ClientMessage::SetState { muted, deafened, held }
            if state.rooms.quietens(seat, muted, deafened, held));
        let limited = if quietens {
            Ok(())
        } else {
            state.limits.check_frame(kind, &caller)
        };
        if let Err(wait) = limited {
            crate::metrics::frame_refused(kind);
            outbox.send(&ServerMessage::Error {
                detail: format!(
                    "too many {kind} frames; try again in {}s",
                    wait.as_secs().max(1)
                ),
                fatal: false,
                retry_after_seconds: Some(wait.as_secs().max(1)),
                refused: Some(kind.to_string()),
            });
            continue;
        }
        let result = match frame {
            ClientMessage::Identify { .. } | ClientMessage::Resume { .. } => {
                Err(RoomError::BadParameters("already identified".to_string()))
            }
            ClientMessage::Answer {
                connection,
                revision,
                sdp,
            } => state.rooms.answer(seat, connection, revision, sdp),
            ClientMessage::Send { source, .. } if !state.rooms.grants(seat).may_send(source) => {
                Err(RoomError::NotPermitted(source))
            }
            ClientMessage::Send { source, external } => state.rooms.send(seat, source, external),
            ClientMessage::StopSending { source } => state.rooms.stop_sending(seat, source),
            ClientMessage::Watch { video } => state.rooms.watch(seat, video),
            ClientMessage::Silence { voices, shares } => state.rooms.silence(seat, voices, shares),
            ClientMessage::SetReceiving { mid, receiving } => {
                state.rooms.set_receiving(seat, mid, receiving)
            }
            ClientMessage::RestartIce { connection } => state.rooms.restart_ice(seat, connection),
            ClientMessage::SetState {
                muted,
                deafened,
                held,
            } => state.rooms.set_state(seat, muted, deafened, held).await,
            ClientMessage::OfferFile { .. } if !state.rooms.grants(seat).transfer_files => {
                Err(RoomError::TransferNotPermitted)
            }
            ClientMessage::OfferFile {
                offer,
                name,
                size,
                allow_direct,
                valid_for_seconds,
            } => {
                state
                    .rooms
                    .offer_file(seat, offer, name, size, allow_direct, valid_for_seconds)
                    .await
            }
            ClientMessage::WithdrawFile { offer } => state.rooms.withdraw_file(seat, offer),
            ClientMessage::AcceptFile { offer, mode } => {
                state.rooms.accept_file(seat, offer, mode).await
            }
            ClientMessage::TransferSignal {
                offer,
                peer,
                signal,
            } => state.rooms.transfer_signal(seat, offer, peer, signal),
            ClientMessage::EndTransfer {
                offer,
                peer,
                reason,
            } => state.rooms.end_transfer(seat, offer, peer, reason).await,
            // Taken before the limits, so a leave always goes through.
            ClientMessage::Leave => Ok(()),
        };
        if let Err(e) = result {
            outbox.send(&ServerMessage::Error {
                detail: e.to_string(),
                fatal: false,
                retry_after_seconds: None,
                refused: Some(kind.to_string()),
            });
        }
    }
    info!(user = user.to_string(), left, "socket closed");
    state.rooms.socket_closed(seat, left).await;
    close(outbox, writer).await;
}

/// The claims of `token` if it admits its holder here at `at`: a signed token checked against
/// the API servers' key it names, or, on a server given `token_secret`, one of the shared-secret
/// form under that secret.
async fn check_token(state: &AppState, token: &str, at: i64) -> Result<JoinClaims, TokenError> {
    match key_of(token) {
        Some(key) => {
            let public = state
                .token_keys
                .public_key(key)
                .await
                .ok_or(TokenError::UnknownKey)?;
            verify(token, &public, state.server, at)
        }
        None => match &state.token_secret {
            Some(secret) => verify_shared(token, secret.as_bytes(), state.server, at),
            None => Err(TokenError::Malformed),
        },
    }
}

/// The frame a socket opens with.
enum First {
    Identify(String),
    Resume(String),
}

/// What a socket sent.
#[derive(Debug, PartialEq)]
enum Received {
    Frame(ClientMessage),
    /// A JSON object that is no frame this build knows: a type, or a value in a known type,
    /// that a newer client sends. `kind` is its `type`, when it has one.
    Unknown {
        kind: Option<String>,
        detail: String,
    },
}

/// The label unknown frames are counted under in the metrics; they have no limits of their
/// own, only `any`'s.
const UNKNOWN_FRAME: &str = "unknown";

/// Reads a text frame; `None` when it is not a JSON object at all.
fn read_frame(text: &str) -> Option<Received> {
    match serde_json::from_str::<ClientMessage>(text) {
        Ok(frame) => Some(Received::Frame(frame)),
        Err(e) => {
            let object: serde_json::Map<String, serde_json::Value> =
                serde_json::from_str(text).ok()?;
            Some(Received::Unknown {
                kind: object
                    .get("type")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                detail: e.to_string(),
            })
        }
    }
}

/// What the client sent next, or `None` once the socket is closed or fails, sends something that
/// is not a JSON object, or goes `IDLE_TIMEOUT` without sending anything. yawc answers pings
/// itself, and reassembles fragmented messages before they come here.
async fn next_frame(
    stream: &mut futures_util::stream::SplitStream<HttpWebSocket>,
) -> Option<Received> {
    loop {
        let Ok(next) = tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await else {
            info!("a signalling socket went silent");
            return None;
        };
        let frame = next?;
        match frame.opcode() {
            OpCode::Text => {
                let received = read_frame(frame.as_str());
                if received.is_none() {
                    warn!("a client sent text that is not a JSON object");
                }
                return received;
            }
            OpCode::Binary | OpCode::Close => return None,
            OpCode::Ping | OpCode::Pong | OpCode::Continuation => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_route_has_limits_and_no_other_path_does() {
        assert_eq!(limited_as(HEALTH_PATH), Some(HEALTH));
        assert_eq!(limited_as(SIGNALLING_PATH), Some(SIGNALLING));
        assert_eq!(limited_as("/ws/v2"), None);
    }

    #[test]
    fn a_frame_this_build_does_not_know_is_told_from_one_that_is_no_frame() {
        assert_eq!(
            read_frame(r#"{"type":"leave"}"#),
            Some(Received::Frame(ClientMessage::Leave))
        );
        // A type added later, and a value added later to a type known now.
        for (text, kind) in [
            (r#"{"type":"raiseHand","raised":true}"#, Some("raiseHand")),
            (r#"{"type":"send","source":"whiteboard"}"#, Some("send")),
            (r#"{"kind":"leave"}"#, None),
            (r#"{"type":7}"#, None),
        ] {
            match read_frame(text) {
                Some(Received::Unknown { kind: found, .. }) => assert_eq!(found.as_deref(), kind),
                other => panic!("{text} read as {other:?}"),
            }
        }
        for text in ["", "leave", r#""leave""#, r#"[{"type":"leave"}]"#, "{"] {
            assert_eq!(read_frame(text), None, "{text}");
        }
        assert!(!ClientMessage::KINDS.contains(&UNKNOWN_FRAME));
    }

    #[test]
    fn a_token_admits_one_connection_until_it_expires() {
        let used = UsedTokens::default();
        let (nonce, other) = (Uuid::now_v7(), Uuid::now_v7());
        assert!(used.claim(nonce, 160, 100));
        assert!(!used.claim(nonce, 160, 120));
        assert!(used.claim(other, 160, 120));
        // Once expired it is forgotten; verifying refuses it by then anyway.
        assert!(used.claim(nonce, 260, 200));
    }
}
