//! One client's WebSocket: frames in are dispatched to the rooms, frames out come from the
//! participant's outbox. The first frame must be `identify` with a valid join token.

use crate::rooms::{RoomError, Rooms};
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;
use voice_protocol::signal::{ClientMessage, ServerMessage};
use voice_protocol::token::{JoinClaims, verify};

/// How long a client has to identify before the socket is closed.
const IDENTIFY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct AppState {
    pub server: Uuid,
    pub token_secret: Arc<str>,
    pub rooms: Arc<Rooms>,
}

pub async fn upgrade(State(state): State<AppState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| handle(socket, state))
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

async fn handle(socket: WebSocket, state: AppState) {
    let (mut sink, mut stream) = socket.split();
    let (outbox, mut inbox) = mpsc::unbounded_channel::<ServerMessage>();
    // Frames for the client are written by their own task so room work never waits on a
    // slow socket.
    let writer = tokio::spawn(async move {
        while let Some(message) = inbox.recv().await {
            let text = serde_json::to_string(&message).expect("frames serialize");
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    let claims = match tokio::time::timeout(IDENTIFY_TIMEOUT, next_frame(&mut stream)).await {
        Ok(Some(ClientMessage::Identify { token })) => {
            match verify(&token, state.token_secret.as_bytes(), state.server, now()) {
                Ok(claims) => claims,
                Err(e) => {
                    let _ = outbox.send(ServerMessage::Error {
                        detail: e.to_string(),
                        fatal: true,
                    });
                    drop(outbox);
                    let _ = writer.await;
                    return;
                }
            }
        }
        _ => {
            let _ = outbox.send(ServerMessage::Error {
                detail: "the first frame must be identify".to_string(),
                fatal: true,
            });
            drop(outbox);
            let _ = writer.await;
            return;
        }
    };
    let JoinClaims { user, channel, .. } = claims;
    if let Err(e) = state.rooms.join(channel, user, outbox.clone()).await {
        warn!(error = e.to_string(), "join failed");
        let _ = outbox.send(ServerMessage::Error {
            detail: e.to_string(),
            fatal: true,
        });
        drop(outbox);
        let _ = writer.await;
        return;
    }

    while let Some(frame) = next_frame(&mut stream).await {
        let result = match frame {
            ClientMessage::Identify { .. } => {
                Err(RoomError::BadParameters("already identified".to_string()))
            }
            ClientMessage::SetCapabilities { rtp_capabilities } => {
                state
                    .rooms
                    .set_capabilities(channel, user, rtp_capabilities)
                    .await
            }
            ClientMessage::CreateTransport { direction } => {
                state.rooms.create_transport(channel, user, direction).await
            }
            ClientMessage::ConnectTransport {
                transport_id,
                dtls_parameters,
            } => {
                state
                    .rooms
                    .connect_transport(channel, user, &transport_id, dtls_parameters)
                    .await
            }
            ClientMessage::Produce {
                transport_id,
                kind,
                source,
                rtp_parameters,
            } => {
                state
                    .rooms
                    .produce(channel, user, &transport_id, kind, source, rtp_parameters)
                    .await
            }
            ClientMessage::CloseProducer { producer_id } => {
                state
                    .rooms
                    .close_producer(channel, user, &producer_id)
                    .await
            }
            ClientMessage::ResumeConsumer { consumer_id } => {
                state
                    .rooms
                    .resume_consumer(channel, user, &consumer_id)
                    .await
            }
            ClientMessage::SetState { muted, deafened } => {
                state.rooms.set_state(channel, user, muted, deafened).await
            }
            ClientMessage::Leave => break,
        };
        if let Err(e) = result {
            let _ = outbox.send(ServerMessage::Error {
                detail: e.to_string(),
                fatal: false,
            });
        }
    }
    info!(user = user.to_string(), "socket closed");
    state.rooms.leave(channel, user, None).await;
    drop(outbox);
    let _ = writer.await;
}

/// The next client frame, or `None` once the socket is closed or sends something unreadable.
async fn next_frame(
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Option<ClientMessage> {
    loop {
        match stream.next().await? {
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
