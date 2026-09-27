//! An agent: plays the users the coordinator assigns it. It talks to the coordinator through a
//! pair of channels, which are a WebSocket for an agent on another machine (`connect`) and plain
//! in-process channels for the coordinator's own (`crate::coordinator`).

use crate::clock::{Clock, estimate_offset};
use crate::engine;
use aspen_bench_protocol::coordination::{AgentMessage, CoordinatorMessage};
use futures_util::{SinkExt, StreamExt};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message as WsMessage;

/// How long an agent waits for its last snapshots once its users have stopped.
const FINAL_SNAPSHOTS: Duration = Duration::from_secs(15);

/// Ping rounds for measuring the clock offset.
const CLOCK_ROUNDS: usize = 16;

/// Serves assignments until the coordinator goes away.
pub async fn serve(
    name: String,
    out: mpsc::UnboundedSender<AgentMessage>,
    mut incoming: mpsc::UnboundedReceiver<CoordinatorMessage>,
) -> Result<(), String> {
    let send = |message: AgentMessage| {
        out.send(message)
            .map_err(|_| "the coordinator went away".to_string())
    };
    send(AgentMessage::Hello {
        name,
        version: env!("CARGO_PKG_VERSION").to_string(),
    })?;
    let mut rounds = Vec::new();
    for _ in 0..CLOCK_ROUNDS {
        let sent = Clock::local_ns();
        send(AgentMessage::ClockPing { agent_ns: sent })?;
        match incoming.recv().await {
            Some(CoordinatorMessage::ClockPong {
                agent_ns,
                coordinator_ns,
            }) if agent_ns == sent => {
                rounds.push((sent, coordinator_ns, Clock::local_ns()));
            }
            Some(_) => {
                return Err("the coordinator answered a clock ping with something else".into());
            }
            None => return Err("the coordinator went away".into()),
        }
    }
    let (offset, error) = estimate_offset(&rounds).ok_or("no clock rounds")?;
    tracing::info!(
        offset_ms = offset as f64 / 1e6,
        error_ms = error as f64 / 1e6,
        "clock synchronised"
    );
    let clock = Clock::new(offset);
    send(AgentMessage::Ready)?;
    while let Some(message) = incoming.recv().await {
        match message {
            CoordinatorMessage::Assign(assignment) => {
                let (stop_tx, stop) = watch::channel(false);
                let (mut snapshots, handle) = match engine::run(*assignment, clock, stop) {
                    Ok(running) => running,
                    Err(error) => {
                        send(AgentMessage::Failed { error })?;
                        continue;
                    }
                };
                let mut handle = handle;
                let mut coordinator_gone = false;
                loop {
                    tokio::select! {
                        Some(snapshot) = snapshots.recv() => send(AgentMessage::Snapshot(snapshot))?,
                        message = incoming.recv(), if !coordinator_gone => match message {
                            Some(CoordinatorMessage::Stop) => { let _ = stop_tx.send(true); }
                            None => {
                                coordinator_gone = true;
                                let _ = stop_tx.send(true);
                            }
                            Some(_) => {}
                        },
                        summary = &mut handle => {
                            // The last snapshots come once every task that records has ended;
                            // one that has not by now is abandoned rather than waited for.
                            let deadline = tokio::time::Instant::now() + FINAL_SNAPSHOTS;
                            while let Ok(Some(snapshot)) = tokio::time::timeout_at(deadline, snapshots.recv()).await {
                                send(AgentMessage::Snapshot(snapshot))?;
                            }
                            let summary = summary.map_err(|e| e.to_string())?;
                            send(AgentMessage::Finished { summary })?;
                            break;
                        }
                    }
                }
            }
            CoordinatorMessage::Stop => {}
            CoordinatorMessage::ClockPong { .. } => {}
        }
    }
    Ok(())
}

/// Connects to a coordinator on another machine and serves it.
pub async fn connect(coordinator: &str, name: String) -> Result<(), String> {
    let url = format!("{}/agent", coordinator.trim_end_matches('/'));
    let (socket, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .map_err(|e| format!("could not reach the coordinator at {url}: {e}"))?;
    let (mut sink, mut source) = socket.split();
    let (out, mut out_rx) = mpsc::unbounded_channel::<AgentMessage>();
    let (incoming_tx, incoming) = mpsc::unbounded_channel::<CoordinatorMessage>();
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            let text = serde_json::to_string(&message).expect("agent messages serialize");
            if sink.send(WsMessage::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    let reader = tokio::spawn(async move {
        while let Some(Ok(frame)) = source.next().await {
            if let WsMessage::Text(text) = frame
                && let Ok(message) = serde_json::from_str::<CoordinatorMessage>(&text)
                && incoming_tx.send(message).is_err()
            {
                break;
            }
        }
    });
    let result = serve(name, out, incoming).await;
    reader.abort();
    let _ = tokio::time::timeout(Duration::from_secs(5), writer).await;
    result
}
