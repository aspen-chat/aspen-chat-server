use crate::api::{GlobalServerContext, MAX_EVENT_AGE};
use crate::app;
use crate::app::ASPEN_NATS_STREAM_NAME;
use async_nats::ConnectOptions;
use async_nats::jetstream::consumer::pull::{Ordered, OrderedConfig};
use async_nats::jetstream::consumer::{DeliverPolicy, ReplayPolicy};
use axum::extract::ws::{Message, WebSocket};
use axum::extract::{State, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::Response;
use log::error;
use rust_i18n::t;
use std::error::Error;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

pub async fn event_stream(
    ws: WebSocketUpgrade,
    State(state): State<GlobalServerContext>,
) -> Response {
    prepare_event_stream(ws, state).await.unwrap_or_else(|e| {
        error!("failed to setup NATS event stream {e}");
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(t!("eventStreamError").into())
            .expect("infallible")
    })
}

async fn prepare_event_stream(
    ws: WebSocketUpgrade,
    state: GlobalServerContext,
) -> Result<Response, app::Error> {
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
    .await?;
    let context = async_nats::jetstream::new(client);
    let stream = context.get_stream(ASPEN_NATS_STREAM_NAME).await?;
    let consumer = stream
        .create_consumer(OrderedConfig {
            replay_policy: ReplayPolicy::Instant,
            deliver_policy: DeliverPolicy::ByStartTime {
                start_time: time::OffsetDateTime::now_utc() - MAX_EVENT_AGE,
            },
            max_batch: state.config.event_queue_size as i64,
            max_bytes: 1024 * 1024,
            max_expires: Duration::from_secs(5),
            ..Default::default()
        })
        .await?;
    let messages = consumer.messages().await?;
    Ok(ws.on_upgrade(|ws| handle_socket_conn(ws, messages, force_shutdown_rx)))
}

async fn handle_socket_conn(
    mut socket: WebSocket,
    mut nats_consumer: Ordered,
    mut force_shutdown_rx: mpsc::Receiver<()>,
) {
    loop {
        tokio::select! {
            msg = nats_consumer.next() => {
                let Some(msg) = msg else {
                    break;
                };
                match msg {
                    Ok(msg) => {
                        let async_nats::jetstream::Message {
                            message: async_nats::message::Message { payload, .. },
                            ..
                        } = msg;
                        match payload.try_into() {
                            Ok(v) => {
                                let ws_result = socket.send(Message::Text(v)).await;
                                if let Err(e) = ws_result {
                                    let Some(e) = e
                                        .source()
                                        .and_then(|e| e.downcast_ref::<tungstenite::Error>())
                                    else {
                                        error!("unexpected error type on websocket send {e}");
                                        return;
                                    };
                                    match e {
                                        tungstenite::Error::ConnectionClosed
                                        | tungstenite::Error::AlreadyClosed => {
                                            return;
                                        }
                                        other => {
                                            error!("websocket error on event stream {other}");
                                            return;
                                        }
                                    }
                                }
                            }
                            Err(e) => {
                                error!("NATS stream message contained invalid UTF-8 {e}");
                            }
                        }
                    }
                    Err(e) => {
                        error!("NATS OrderedError {e}");
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
