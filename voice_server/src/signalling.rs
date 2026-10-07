//! One client's WebSocket: frames in are dispatched to the rooms, frames out come from the
//! participant's outbox. The first frame must be `identify` with a valid join token. Every
//! route and frame is limited (`limits`). The server pings every `PING_INTERVAL` and closes a
//! socket it has heard nothing from, pongs included, for `IDLE_TIMEOUT`, and one whose outbox
//! overflows (`outbox`), so a client that stops reading is let go.

use crate::limits::{Caller, HEALTH, Limits, PendingSocket, SIGNALLING};
use crate::outbox::{OUTBOX_BYTES, Outbox};
use crate::rooms::{RoomError, Rooms};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, MatchedPath, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header::RETRY_AFTER};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use futures_util::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{info, warn};
use uuid::Uuid;
use voice_protocol::signal::{ClientMessage, ServerMessage};
use voice_protocol::token::{TokenError, verify};

/// How long a client has to identify before the socket is closed.
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the server pings a socket. Browsers answer on their own.
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// How long a socket may go without the server hearing anything, a pong included, before it is
/// closed: two pings missed.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long a closing socket is given to write what is queued for it before it is dropped.
const CLOSE_GRACE: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct AppState {
    pub server: Uuid,
    pub token_secret: Arc<str>,
    pub rooms: Arc<Rooms>,
    pub limits: Arc<Limits>,
    pub used_tokens: Arc<UsedTokens>,
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

/// Middleware on both routes: counts the request against its route's limits.
pub async fn limit_http(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    matched: MatchedPath,
    request: Request,
    next: Next,
) -> Response {
    let route = match matched.as_str() {
        "/health" => HEALTH,
        "/ws" => SIGNALLING,
        _ => return next.run(request).await,
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
    ws: WebSocketUpgrade,
) -> Response {
    let ip = client_ip(&state, peer, &headers);
    let Some(pending) = state.limits.pending_socket(ip) else {
        crate::metrics::http_refused(SIGNALLING);
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let max = state.limits.max_message_bytes;
    ws.max_message_size(max)
        .max_frame_size(max)
        .on_upgrade(move |socket| handle(socket, state, ip, pending))
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

async fn handle(socket: WebSocket, state: AppState, ip: IpAddr, pending: PendingSocket) {
    let (mut sink, mut stream) = socket.split();
    let (outbox, mut inbox) = Outbox::new(OUTBOX_BYTES);
    // Frames for the client are written by their own task so room work never waits on a
    // slow socket. It pings between them.
    let writer = tokio::spawn(async move {
        let mut ping =
            tokio::time::interval_at(tokio::time::Instant::now() + PING_INTERVAL, PING_INTERVAL);
        loop {
            let message = tokio::select! {
                text = inbox.recv() => match text {
                    Some(text) => Message::Text(text),
                    None => break,
                },
                _ = ping.tick() => Message::Ping(Default::default()),
            };
            if sink.send(message).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    let claims = match tokio::time::timeout(IDENTIFY_TIMEOUT, next_frame(&mut stream)).await {
        Ok(Some(ClientMessage::Identify { token })) => {
            let at = now();
            match verify(&token, state.token_secret.as_bytes(), state.server, at).and_then(
                |claims| {
                    if state.used_tokens.claim(claims.nonce, claims.expires_at, at) {
                        Ok(claims)
                    } else {
                        Err(TokenError::Used)
                    }
                },
            ) {
                Ok(claims) => claims,
                Err(e) => {
                    outbox.send(&ServerMessage::Error {
                        detail: e.to_string(),
                        fatal: true,
                        retry_after_seconds: None,
                        refused: None,
                    });
                    close(outbox, writer).await;
                    return;
                }
            }
        }
        _ => {
            outbox.send(&ServerMessage::Error {
                detail: "the first frame must be identify".to_string(),
                fatal: true,
                retry_after_seconds: None,
                refused: None,
            });
            close(outbox, writer).await;
            return;
        }
    };
    // Identified: the socket no longer counts against its address's unidentified ones.
    drop(pending);
    let (user, channel) = (claims.user, claims.channel);
    let caller = Caller {
        ip: Some(ip),
        user: Some(user),
        channel: Some(channel),
    };
    if let Err(wait) = state.limits.check_frame("identify", &caller) {
        outbox.send(&ServerMessage::Error {
            detail: format!(
                "too many identify frames; try again in {}s",
                wait.as_secs().max(1)
            ),
            fatal: true,
            retry_after_seconds: Some(wait.as_secs().max(1)),
            refused: None,
        });
        close(outbox, writer).await;
        return;
    }
    let seat = match state
        .rooms
        .join(
            channel,
            user,
            outbox.clone(),
            claims.grants(),
            claims.sign_in.clone(),
        )
        .await
    {
        Ok(seat) => seat,
        Err(e) => {
            warn!(error = e.to_string(), "join failed");
            outbox.send(&ServerMessage::Error {
                detail: e.to_string(),
                fatal: true,
                retry_after_seconds: None,
                refused: None,
            });
            close(outbox, writer).await;
            return;
        }
    };

    loop {
        let frame = tokio::select! {
            frame = next_frame(&mut stream) => frame,
            () = outbox.hung_up() => None,
        };
        let Some(frame) = frame else {
            break;
        };
        let kind = frame.kind();
        crate::metrics::frame(kind);
        // Muting and deafening always go through; only lifting them is limited.
        let quietens = matches!(frame, ClientMessage::SetState { muted, deafened }
            if state.rooms.quietens(seat, muted, deafened));
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
            ClientMessage::Identify { .. } => {
                Err(RoomError::BadParameters("already identified".to_string()))
            }
            ClientMessage::SetCapabilities { rtp_capabilities } => {
                state.rooms.set_capabilities(seat, rtp_capabilities).await
            }
            ClientMessage::CreateTransport { direction } => {
                state.rooms.create_transport(seat, direction).await
            }
            ClientMessage::ConnectTransport {
                transport_id,
                dtls_parameters,
            } => {
                state
                    .rooms
                    .connect_transport(seat, &transport_id, dtls_parameters)
                    .await
            }
            ClientMessage::Produce { source, .. } | ClientMessage::ProduceRtp { source }
                if !state.rooms.grants(seat).may_produce(source) =>
            {
                Err(RoomError::NotPermitted(source))
            }
            ClientMessage::Produce {
                transport_id,
                kind,
                source,
                rtp_parameters,
            } => {
                state
                    .rooms
                    .produce(seat, &transport_id, kind, source, rtp_parameters)
                    .await
            }
            ClientMessage::ProduceRtp { source } => state.rooms.produce_rtp(seat, source).await,
            ClientMessage::ConsumeRtp => state.rooms.consume_rtp(seat).await,
            ClientMessage::CloseProducer { producer_id } => {
                state.rooms.close_producer(seat, &producer_id).await
            }
            ClientMessage::ResumeConsumer { consumer_id } => {
                state.rooms.resume_consumer(seat, &consumer_id).await
            }
            ClientMessage::SetState { muted, deafened } => {
                state.rooms.set_state(seat, muted, deafened).await
            }
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
            ClientMessage::Leave => break,
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
    info!(user = user.to_string(), "socket closed");
    state.rooms.leave_seat(seat).await;
    close(outbox, writer).await;
}

/// The next client frame, or `None` once the socket is closed, sends something unreadable, or
/// goes `IDLE_TIMEOUT` without sending anything.
async fn next_frame(
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Option<ClientMessage> {
    loop {
        let Ok(next) = tokio::time::timeout(IDLE_TIMEOUT, stream.next()).await else {
            info!("a signalling socket went silent");
            return None;
        };
        match next? {
            Ok(Message::Text(text)) => match serde_json::from_str::<ClientMessage>(&text) {
                Ok(frame) => return Some(frame),
                Err(e) => {
                    warn!(error = e.to_string(), "unreadable client frame");
                    return None;
                }
            },
            Ok(Message::Binary(_)) => return None,
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(Message::Ping(_) | Message::Pong(_)) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
