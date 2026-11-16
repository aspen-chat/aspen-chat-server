//! A call between browsers through a media thread, with plain HTTP standing in for the voice
//! server's signalling, for what in-process clients cannot check: that browsers answer the
//! server's offers, send on the sections offered to them, and take an audio slot changing
//! speaker. `browser.html` is the page; `browser_check.cjs` drives it with Playwright.
//!
//! cargo run -p voice_media --example browser -- <address that is not loopback> [audio slots]

use axum::extract::{Path, State};
use axum::response::Html;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::Notify;
use uuid::Uuid;
use voice_media::{CallId, Command, Event, Media, MediaAddress, MediaConfig, ParticipantId};
use voice_protocol::signal::{ConnectionKind, MediaSource};

struct Inbox {
    user: Uuid,
    events: VecDeque<Value>,
    notify: Arc<Notify>,
}

struct App {
    media: Media,
    call: CallId,
    next: AtomicU64,
    inboxes: Mutex<HashMap<u64, Inbox>>,
}

fn event_json(event: &Event, users: &HashMap<u64, Uuid>) -> Value {
    match event {
        Event::Offer {
            connection,
            revision,
            sdp,
            sections,
            ..
        } => {
            json!({"type": "offer", "connection": connection, "revision": revision, "sdp": sdp, "sections": sections})
        }
        Event::Slots { assignments, .. } => json!({"type": "slots", "assignments": assignments}),
        Event::Speaking {
            participant,
            speaking,
        } => json!({"type": "speaking", "user": users.get(&participant.0), "speaking": speaking}),
        other => json!({"type": "other", "detail": format!("{other:?}")}),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let ip = args
        .next()
        .expect("the address media is sent to, not loopback (Firefox does not pair with it)")
        .parse()?;
    let audio_slots = args.next().map_or(Ok(16), |s| s.parse())?;
    let app = Arc::new(App {
        media: Media::start(MediaConfig {
            addresses: vec![MediaAddress {
                ip,
                announced: None,
            }],
            first_port: 0,
            threads: 1,
            audio_slots,
            max_watched_video: 25,
        })?,
        call: CallId(Uuid::now_v7()),
        next: AtomicU64::new(1),
        inboxes: Mutex::new(HashMap::new()),
    });
    let router = Router::new()
        .route("/", get(|| async { Html(include_str!("browser.html")) }))
        .route("/join", post(join))
        .route("/events/{id}", get(events))
        .route("/answer/{id}", post(answer))
        .route("/send/{id}", post(send))
        .route("/pause/{id}", post(pause))
        .with_state(app);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8710").await?;
    eprintln!("page on http://127.0.0.1:8710, media on {ip}");
    axum::serve(listener, router).await?;
    Ok(())
}

async fn join(State(app): State<Arc<App>>) -> Json<Value> {
    let id = app.next.fetch_add(1, Ordering::Relaxed);
    let user = Uuid::now_v7();
    let notify = Arc::new(Notify::new());
    app.inboxes.lock().unwrap().insert(
        id,
        Inbox {
            user,
            events: VecDeque::new(),
            notify: Arc::clone(&notify),
        },
    );
    let (events, mut received) = tokio::sync::mpsc::unbounded_channel();
    let inbox_app = Arc::clone(&app);
    tokio::spawn(async move {
        while let Some(event) = received.recv().await {
            let mut inboxes = inbox_app.inboxes.lock().unwrap();
            let users: HashMap<u64, Uuid> = inboxes.iter().map(|(k, v)| (*k, v.user)).collect();
            if let Some(inbox) = inboxes.get_mut(&id) {
                inbox.events.push_back(event_json(&event, &users));
                inbox.notify.notify_one();
            }
        }
    });
    app.media.send(
        0,
        Command::Join {
            call: app.call,
            participant: ParticipantId(id),
            user,
            events,
        },
    );
    Json(json!({"id": id, "user": user}))
}

/// The events waiting for a participant, or those that come within twenty seconds.
async fn events(State(app): State<Arc<App>>, Path(id): Path<u64>) -> Json<Vec<Value>> {
    let notify = {
        let mut inboxes = app.inboxes.lock().unwrap();
        let Some(inbox) = inboxes.get_mut(&id) else {
            return Json(Vec::new());
        };
        if !inbox.events.is_empty() {
            return Json(inbox.events.drain(..).collect());
        }
        Arc::clone(&inbox.notify)
    };
    let _ = tokio::time::timeout(Duration::from_secs(20), notify.notified()).await;
    let mut inboxes = app.inboxes.lock().unwrap();
    Json(
        inboxes
            .get_mut(&id)
            .map(|inbox| inbox.events.drain(..).collect())
            .unwrap_or_default(),
    )
}

async fn answer(State(app): State<Arc<App>>, Path(id): Path<u64>, Json(body): Json<Value>) {
    app.media.send(
        0,
        Command::Answer {
            participant: ParticipantId(id),
            connection: ConnectionKind::Call,
            revision: body["revision"].as_u64().unwrap_or_default(),
            sdp: body["sdp"].as_str().unwrap_or_default().to_owned(),
        },
    );
}

fn source(body: &Value) -> MediaSource {
    serde_json::from_value(body["source"].clone()).unwrap_or(MediaSource::Microphone)
}

async fn send(State(app): State<Arc<App>>, Path(id): Path<u64>, Json(body): Json<Value>) {
    app.media.send(
        0,
        Command::Send {
            participant: ParticipantId(id),
            source: source(&body),
            external: false,
        },
    );
}

async fn pause(State(app): State<Arc<App>>, Path(id): Path<u64>, Json(body): Json<Value>) {
    app.media.send(
        0,
        Command::SetSourcePaused {
            participant: ParticipantId(id),
            source: source(&body),
            paused: body["paused"].as_bool().unwrap_or_default(),
        },
    );
}
