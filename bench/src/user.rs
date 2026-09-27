//! A virtual user: signs in, starts up the way the reference client does, keeps an event stream
//! open, and acts as its behaviour says.
//!
//! Actions follow an open model: each kind arrives as a Poisson process at its rate, and an
//! action starts when it is due whether or not earlier ones have been answered, as independent
//! people's actions do. Latency is measured from when the action was due, so a server that
//! answers slowly cannot also slow the load down and hide it (coordinated omission). How late
//! the generator itself started each action is recorded as `lag`.
//!
//! Every message a user sends carries a marker, `⟦bench:<kind>:<sender>:<ns>⟧`, with the
//! coordinator-clock time it was sent; every user who receives it on their event stream records
//! the difference as its delivery time. Messages sent before the receiver's stream was ready
//! arrive as the stream's replay of the past minute, and are counted as `replayed` instead.

use crate::clock::Clock;
use crate::profile::{Behaviour, Profile};
use crate::stats::Recorder;
use aspen_bench_protocol::Manifest;
use futures_util::{SinkExt, StreamExt};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde_json::{Value, json};
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use uuid::Uuid;

const API_PREFIX: &str = "/api/v1";
/// How often the reference client asks for presence, and sends activity while in use.
const PRESENCE_INTERVAL: Duration = Duration::from_secs(30);
const ACTIVITY_INTERVAL: Duration = Duration::from_secs(60);
/// The most user ids one presence request names.
const PRESENCE_BATCH: usize = 100;
/// Messages remembered per user for reactions, edits, and deletions.
const RECENT: usize = 50;
const MARKER_OPEN: &str = "⟦bench:";
const MARKER_CLOSE: char = '⟧';
const EMOJI: [&str; 4] = [
    "%F0%9F%91%8D",
    "%E2%9D%A4%EF%B8%8F",
    "%F0%9F%98%82",
    "%F0%9F%8E%89",
];

/// What every user of an agent shares.
pub struct World {
    pub profile: Profile,
    pub manifest: Manifest,
    pub api: String,
    pub events_url: String,
    /// The API server's address, looked up once, for event streams opened from a chosen
    /// local address.
    pub events_addr: tokio::sync::OnceCell<std::net::SocketAddr>,
    pub clock: Clock,
    pub recorder: Recorder,
    /// Each user's communities, by index into `manifest.communities`.
    pub memberships: Vec<Vec<usize>>,
    /// Multiplies every action rate; spikes raise it for a while.
    pub rate_factor: AtomicU64,
    /// Reconnect storms: the share of users who drop their stream.
    pub storms: broadcast::Sender<f64>,
    /// Set when the users are to stop.
    pub stop: watch::Receiver<bool>,
}

impl World {
    pub fn rate_factor(&self) -> f64 {
        f64::from_bits(self.rate_factor.load(Ordering::Relaxed))
    }

    pub fn set_rate_factor(&self, factor: f64) {
        self.rate_factor.store(factor.to_bits(), Ordering::Relaxed);
    }
}

pub fn memberships(manifest: &Manifest) -> Vec<Vec<usize>> {
    let mut by_user = vec![Vec::new(); manifest.users.len()];
    for (index, community) in manifest.communities.iter().enumerate() {
        for member in &community.members {
            by_user[*member as usize].push(index);
        }
    }
    by_user
}

/// The actions a behaviour takes, with their rates per second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Message,
    Dm,
    React,
    Edit,
    Delete,
    History,
    Reconnect,
    Attachment,
    Call,
}

fn rates(behaviour: &Behaviour) -> Vec<(Action, f64)> {
    [
        (Action::Message, behaviour.messages_per_hour),
        (Action::Dm, behaviour.dm_messages_per_hour),
        (Action::React, behaviour.reactions_per_hour),
        (Action::Edit, behaviour.edits_per_hour),
        (Action::Delete, behaviour.deletes_per_hour),
        (Action::History, behaviour.history_reads_per_hour),
        (Action::Reconnect, behaviour.reconnects_per_hour),
        (Action::Attachment, behaviour.attachments_per_hour),
        (
            Action::Call,
            behaviour.voice.as_ref().map_or(0.0, |v| v.calls_per_hour),
        ),
    ]
    .into_iter()
    .filter(|(_, per_hour)| *per_hour > 0.0)
    .map(|(action, per_hour)| (action, per_hour / 3600.0))
    .collect()
}

/// The marker a message carries.
pub fn marker(kind: char, sender: u32, sent_ns: i64) -> String {
    format!("{MARKER_OPEN}{kind}:{sender}:{sent_ns}{MARKER_CLOSE}")
}

/// Reads a marker: `(kind, sender, sent_ns)`.
pub fn read_marker(text: &str) -> Option<(char, u32, i64)> {
    let start = text.find(MARKER_OPEN)? + MARKER_OPEN.len();
    let end = start + text[start..].find(MARKER_CLOSE)?;
    let mut parts = text[start..end].split(':');
    let kind = parts.next()?.chars().next()?;
    let sender = parts.next()?.parse().ok()?;
    let sent = parts.next()?.parse().ok()?;
    Some((kind, sender, sent))
}

/// What a user's tasks share.
struct State {
    token: Mutex<String>,
    /// The last event sequence seen, to resume from.
    last_sequence: AtomicU64,
    /// Coordinator-clock nanoseconds when the current event stream became ready.
    /// A message sent before then reaches this user as replay, not as a live delivery.
    ready_ns: AtomicI64,
    /// Recent messages by others (for reactions) and by this user (to edit or delete).
    seen: Mutex<VecDeque<Uuid>>,
    own: Mutex<VecDeque<Uuid>>,
    /// DM channels by the other person's index.
    dms: Mutex<HashMap<u32, Uuid>>,
    /// Whether the user is in a call; one at a time.
    in_call: std::sync::atomic::AtomicBool,
}

pub struct User {
    world: Arc<World>,
    index: u32,
    behaviour: Behaviour,
    http: reqwest::Client,
    state: Arc<State>,
    rng: Mutex<StdRng>,
}

/// The local address user `index` connects from: the profile's source addresses in turn.
fn source_address(world: &World, index: u32) -> Option<std::net::IpAddr> {
    let addresses = &world.profile.target.source_addresses;
    (!addresses.is_empty()).then(|| addresses[index as usize % addresses.len()])
}

/// Why a request failed, for the counters.
struct Failed;

impl User {
    pub fn new(world: Arc<World>, index: u32, behaviour: Behaviour) -> Self {
        let seed = world.profile.load.seed ^ (u64::from(index) << 20);
        let source = source_address(&world, index);
        Self {
            world,
            index,
            behaviour,
            // One client each: a real user's connections are their own.
            http: reqwest::Client::builder()
                .local_address(source)
                .pool_idle_timeout(Duration::from_secs(90))
                // A request unanswered this long has failed, and must not hold the run open.
                .timeout(Duration::from_secs(30))
                .build()
                .expect("an HTTP client builds"),
            state: Arc::new(State {
                token: Mutex::new(String::new()),
                last_sequence: AtomicU64::new(0),
                ready_ns: AtomicI64::new(i64::MAX),
                seen: Mutex::new(VecDeque::new()),
                own: Mutex::new(VecDeque::new()),
                dms: Mutex::new(HashMap::new()),
                in_call: std::sync::atomic::AtomicBool::new(false),
            }),
            rng: Mutex::new(StdRng::seed_from_u64(seed)),
        }
    }

    fn random(&self) -> f64 {
        self.rng.lock().expect("rng lock").random::<f64>()
    }

    fn pick<T: Copy>(&self, items: &[T]) -> Option<T> {
        if items.is_empty() {
            return None;
        }
        let i = self
            .rng
            .lock()
            .expect("rng lock")
            .random_range(0..items.len());
        Some(items[i])
    }

    fn token(&self) -> String {
        self.state.token.lock().expect("token lock").clone()
    }

    /// Sends a request, recording it under `route` from `due` to its answer.
    async fn request(
        &self,
        route: &str,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
        due: Instant,
    ) -> Result<Value, Failed> {
        let mut builder = self
            .http
            .request(method, format!("{}{API_PREFIX}{path}", self.world.api));
        let token = self.token();
        if !token.is_empty() {
            builder = builder.bearer_auth(token);
        }
        if let Some(body) = body {
            builder = builder.json(&body);
        }
        let recorder = &self.world.recorder;
        recorder.count("requests", 1);
        let outcome = builder.send().await;
        recorder.latency(format!("http:{route}"), due.elapsed());
        match outcome {
            Ok(response) => {
                let status = response.status();
                recorder.count(format!("status:{route}:{}", status.as_u16()), 1);
                if status.is_success() {
                    let bytes = response.bytes().await.map_err(|_| {
                        recorder.count("errors", 1);
                        Failed
                    })?;
                    Ok(if bytes.is_empty() {
                        Value::Null
                    } else {
                        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
                    })
                } else {
                    if status.as_u16() == 429 {
                        recorder.count("rate_limited", 1);
                    }
                    recorder.count("errors", 1);
                    Err(Failed)
                }
            }
            Err(_) => {
                recorder.count(format!("status:{route}:transport"), 1);
                recorder.count("errors", 1);
                Err(Failed)
            }
        }
    }

    /// Plays the user until told to stop. `connect_at` is when to sign in.
    pub async fn run(self, connect_at: Instant) {
        let this = Arc::new(self);
        tokio::time::sleep_until(connect_at).await;
        let mut stop = this.world.stop.clone();
        if *stop.borrow() {
            return;
        }
        let started = Instant::now();
        let mut stream = None;
        for attempt in 0..3u32 {
            match this.clone().connect(false).await {
                Ok(connected) => {
                    stream = Some(connected);
                    break;
                }
                Err(Failed) => {
                    this.world.recorder.count("connect_failures", 1);
                    tokio::time::sleep(Duration::from_secs(1 << attempt)).await;
                }
            }
        }
        let Some(mut stream) = stream else {
            return;
        };
        this.world.recorder.latency("connect", started.elapsed());
        this.world
            .recorder
            .connected()
            .fetch_add(1, Ordering::Relaxed);

        let mut schedule: Vec<(Action, f64, Instant)> = rates(&this.behaviour)
            .into_iter()
            .map(|(action, rate)| (action, rate, Instant::now()))
            .collect();
        let factor = this.world.rate_factor();
        for entry in &mut schedule {
            entry.2 = Instant::now() + this.exponential(entry.1 * factor);
        }
        let mut presence = tokio::time::interval(PRESENCE_INTERVAL);
        let mut storms = this.world.storms.subscribe();
        loop {
            let next = schedule
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, _, due))| *due)
                .map(|(i, e)| (i, e.2));
            let sleep_until = next
                .map(|(_, due)| due)
                .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
            tokio::select! {
                _ = tokio::time::sleep_until(sleep_until), if next.is_some() => {
                    let (i, due) = next.expect("guarded");
                    let action = schedule[i].0;
                    let now = Instant::now();
                    this.world.recorder.latency("lag", now.saturating_duration_since(due));
                    schedule[i].2 = due + this.exponential(schedule[i].1 * this.world.rate_factor());
                    if action == Action::Reconnect {
                        stream = this.clone().reconnect(stream).await;
                        this.reschedule(&mut schedule);
                    } else {
                        let user = Arc::clone(&this);
                        tokio::spawn(async move { user.act(action, due).await });
                    }
                }
                _ = presence.tick() => {
                    let user = Arc::clone(&this);
                    tokio::spawn(async move { user.poll_presence().await });
                }
                storm = storms.recv() => {
                    if let Ok(fraction) = storm
                        && this.random() < fraction
                    {
                        stream = this.clone().reconnect(stream).await;
                        this.reschedule(&mut schedule);
                    }
                }
                ended = &mut stream.ended => {
                    // The server closed the stream: reconnect as the client would.
                    let _ = ended;
                    if *stop.borrow() {
                        break;
                    }
                    this.world.recorder.count("disconnects", 1);
                    this.world.recorder.connected().fetch_sub(1, Ordering::Relaxed);
                    let lost = Instant::now();
                    match this.clone().resume_after_loss(&mut stop).await {
                        Some(connected) => {
                            stream = connected;
                            this.world.recorder.latency("recovery", lost.elapsed());
                            this.world.recorder.connected().fetch_add(1, Ordering::Relaxed);
                            this.reschedule(&mut schedule);
                        }
                        None => return,
                    }
                }
                _ = stop.changed() => break,
            }
        }
        stream.close();
        this.world
            .recorder
            .connected()
            .fetch_sub(1, Ordering::Relaxed);
    }

    /// Draws every action's next time afresh from now. After the user was away reconnecting,
    /// actions that fell due meanwhile are dropped rather than fired late: a person offline
    /// does not do them, and firing them late would read as the generator falling behind.
    fn reschedule(&self, schedule: &mut [(Action, f64, Instant)]) {
        let factor = self.world.rate_factor();
        let now = Instant::now();
        for entry in schedule.iter_mut() {
            if entry.2 < now {
                entry.2 = now + self.exponential(entry.1 * factor);
            }
        }
    }

    fn exponential(&self, rate_per_second: f64) -> Duration {
        if rate_per_second <= 0.0 {
            return Duration::from_secs(365 * 24 * 3600);
        }
        let u: f64 = self.random().max(f64::MIN_POSITIVE);
        Duration::from_secs_f64(-u.ln() / rate_per_second)
    }

    /// Signs in (unless `resume`), starts up, and opens the event stream.
    async fn connect(self: Arc<Self>, resume: bool) -> Result<Stream, Failed> {
        if !resume {
            let name = &self.world.manifest.users[self.index as usize].name;
            let answer = self
                .request(
                    "POST /auth/login",
                    reqwest::Method::POST,
                    "/auth/login",
                    Some(json!({ "username": name, "password": self.world.manifest.password })),
                    Instant::now(),
                )
                .await?;
            let token = answer["sessionToken"].as_str().ok_or(Failed)?.to_string();
            *self.state.token.lock().expect("token lock") = token;
            let now = Instant::now();
            let (me, communities, dms, preferences) = tokio::join!(
                self.request(
                    "GET /users/{user}",
                    reqwest::Method::GET,
                    "/users/@me",
                    None,
                    now
                ),
                self.request(
                    "GET /users/{user}/communities",
                    reqwest::Method::GET,
                    "/users/@me/communities?include=channels,categories,members,voice",
                    None,
                    now
                ),
                self.request(
                    "GET /users/@me/dms",
                    reqwest::Method::GET,
                    "/users/@me/dms?include=users",
                    None,
                    now
                ),
                self.request(
                    "GET /users/{user}/preferences",
                    reqwest::Method::GET,
                    "/users/@me/preferences",
                    None,
                    now
                ),
            );
            me?;
            communities?;
            dms?;
            preferences?;
            self.world.recorder.latency("bootstrap", now.elapsed());
        }
        Stream::open(self, resume).await
    }

    /// After the server drops the stream: tries again with backoff, as the reference client
    /// does, until it is back or the run ends. `None` when the run ended first.
    async fn resume_after_loss(
        self: Arc<Self>,
        stop: &mut watch::Receiver<bool>,
    ) -> Option<Stream> {
        let mut delay = Duration::from_millis(500);
        loop {
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = stop.changed() => return None,
            }
            match self.clone().connect(true).await {
                Ok(stream) => return Some(stream),
                Err(Failed) => {
                    self.world.recorder.count("connect_failures", 1);
                    delay = (delay * 2).min(Duration::from_secs(10));
                }
            }
        }
    }

    async fn reconnect(self: Arc<Self>, stream: Stream) -> Stream {
        stream.close();
        loop {
            match self.clone().connect(true).await {
                Ok(stream) => {
                    self.world.recorder.count("reconnects", 1);
                    return stream;
                }
                Err(Failed) => {
                    self.world.recorder.count("connect_failures", 1);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    async fn poll_presence(&self) {
        let Some(community) = self.world.memberships[self.index as usize].first() else {
            return;
        };
        let members = &self.world.manifest.communities[*community].members;
        let ids: Vec<String> = members
            .iter()
            .take(PRESENCE_BATCH)
            .map(|m| self.world.manifest.users[*m as usize].id.to_string())
            .collect();
        let _ = self
            .request(
                "GET /users/statuses",
                reqwest::Method::GET,
                &format!("/users/statuses?ids={}", ids.join(",")),
                None,
                Instant::now(),
            )
            .await;
    }

    /// A text channel of one of the user's communities, the busy first one `hot` of the time.
    fn channel(&self) -> Option<Uuid> {
        let community = self.pick(&self.world.memberships[self.index as usize])?;
        let channels = &self.world.manifest.communities[community].text_channels;
        if self.random() < self.behaviour.hot_channel_share {
            channels.first().copied()
        } else {
            self.pick(channels)
        }
    }

    async fn post_message(&self, channel: Uuid, kind: char, attachments: Vec<Uuid>, due: Instant) {
        let sent = self.world.clock.now_ns();
        let content = format!("benchmark message {}", marker(kind, self.index, sent));
        if let Ok(message) = self
            .request(
                "POST /channels/{channel}/messages",
                reqwest::Method::POST,
                &format!("/channels/{channel}/messages"),
                Some(json!({ "content": content, "attachments": attachments })),
                due,
            )
            .await
        {
            self.world.recorder.count("messages_sent", 1);
            if let Some(id) = message["id"].as_str().and_then(|s| s.parse().ok()) {
                let mut own = self.state.own.lock().expect("own lock");
                own.push_back(id);
                if own.len() > RECENT {
                    own.pop_front();
                }
            }
        }
    }

    async fn act(&self, action: Action, due: Instant) {
        match action {
            Action::Message => {
                if let Some(channel) = self.channel() {
                    self.post_message(channel, 'c', Vec::new(), due).await;
                }
            }
            Action::Dm => self.send_dm(due).await,
            Action::React => {
                let target = {
                    let seen = self.state.seen.lock().expect("seen lock");
                    let items: Vec<Uuid> = seen.iter().copied().collect();
                    drop(seen);
                    self.pick(&items)
                };
                if let (Some(message), Some(emoji)) = (target, self.pick(&EMOJI)) {
                    let _ = self
                        .request(
                            "PUT /messages/{message}/reactions/{emoji}/@me",
                            reqwest::Method::PUT,
                            &format!("/messages/{message}/reactions/{emoji}/@me"),
                            None,
                            due,
                        )
                        .await;
                }
            }
            Action::Edit => {
                let target = self.state.own.lock().expect("own lock").back().copied();
                if let Some(message) = target {
                    let _ = self
                        .request(
                            "PATCH /messages/{message}",
                            reqwest::Method::PATCH,
                            &format!("/messages/{message}"),
                            Some(json!({ "content": "benchmark message, edited" })),
                            due,
                        )
                        .await;
                }
            }
            Action::Delete => {
                let target = self.state.own.lock().expect("own lock").pop_front();
                if let Some(message) = target {
                    let _ = self
                        .request(
                            "DELETE /messages/{message}",
                            reqwest::Method::DELETE,
                            &format!("/messages/{message}"),
                            None,
                            due,
                        )
                        .await;
                }
            }
            Action::History => {
                if let Some(channel) = self.channel() {
                    let _ = self
                        .request(
                            "GET /channels/{channel}/messages",
                            reqwest::Method::GET,
                            &format!(
                                "/channels/{channel}/messages?limit=50&include=authors,attachments,polls,threads,echoes"
                            ),
                            None,
                            due,
                        )
                        .await;
                }
            }
            Action::Attachment => self.send_attachment(due).await,
            Action::Call => self.call(due).await,
            Action::Reconnect => {}
        }
    }

    /// A DM to someone the user shares a community with, opening it the first time.
    async fn send_dm(&self, due: Instant) {
        let Some(community) = self.pick(&self.world.memberships[self.index as usize]) else {
            return;
        };
        let Some(peer) = self.pick(&self.world.manifest.communities[community].members) else {
            return;
        };
        if peer == self.index {
            return;
        }
        let known = self.state.dms.lock().expect("dms lock").get(&peer).copied();
        let channel = match known {
            Some(channel) => channel,
            None => {
                let peer_id = self.world.manifest.users[peer as usize].id;
                let Ok(opened) = self
                    .request(
                        "POST /users/@me/dms",
                        reqwest::Method::POST,
                        "/users/@me/dms",
                        Some(json!({ "recipients": [peer_id] })),
                        due,
                    )
                    .await
                else {
                    return;
                };
                let Some(channel) = opened["data"]["id"]
                    .as_str()
                    .or(opened["id"].as_str())
                    .and_then(|s| s.parse().ok())
                else {
                    return;
                };
                self.state
                    .dms
                    .lock()
                    .expect("dms lock")
                    .insert(peer, channel);
                channel
            }
        };
        self.post_message(channel, 'd', Vec::new(), due).await;
    }

    /// Joins a call in one of the first `channels_used` voice channels of one of the user's
    /// communities, stays for the behaviour's call length, and leaves (`crate::voice`).
    async fn call(&self, due: Instant) {
        let Some(voice) = self.behaviour.voice.clone() else {
            return;
        };
        if self.state.in_call.swap(true, Ordering::Relaxed) {
            return;
        }
        let channel = self
            .pick(&self.world.memberships[self.index as usize])
            .and_then(|community| {
                let channels = &self.world.manifest.communities[community].voice_channels;
                let used = &channels[..channels.len().min(voice.channels_used.max(1) as usize)];
                self.pick(used)
            });
        if let Some(channel) = channel
            && let Ok(offer) = self
                .request(
                    "POST /channels/{channel}/voice/join",
                    reqwest::Method::POST,
                    &format!("/channels/{channel}/voice/join"),
                    None,
                    due,
                )
                .await
        {
            let plan = crate::voice::CallPlan {
                seconds: voice.call_minutes * 60.0,
                screen: self.random() < voice.screen_share,
                audio_bitrate: voice.audio_bitrate,
                screen_bitrate: voice.screen_bitrate,
            };
            if let Err(error) = crate::voice::run_call(
                self.http.clone(),
                offer,
                plan,
                self.world.recorder.clone(),
                self.world.stop.clone(),
                due,
            )
            .await
            {
                tracing::debug!(error, user = self.index, "call failed");
                self.world.recorder.count("voice_failures", 1);
                self.world.recorder.count("errors", 1);
            }
        }
        self.state.in_call.store(false, Ordering::Relaxed);
    }

    /// Uploads a file of the behaviour's size to object storage and posts it.
    async fn send_attachment(&self, due: Instant) {
        let Some(channel) = self.channel() else {
            return;
        };
        let Ok(handle) = self
            .request(
                "POST /attachments",
                reqwest::Method::POST,
                "/attachments",
                Some(
                    json!({ "fileName": "benchmark.bin", "mimeType": "application/octet-stream" }),
                ),
                due,
            )
            .await
        else {
            return;
        };
        let (Some(id), Some(url)) = (
            handle["id"].as_str().and_then(|s| s.parse::<Uuid>().ok()),
            handle["uploadUrl"].as_str(),
        ) else {
            return;
        };
        let body = vec![0x5a_u8; usize::try_from(self.behaviour.attachment_bytes).unwrap_or(0)];
        let upload_started = Instant::now();
        let uploaded = self
            .http
            .put(url)
            .header("content-type", "application/octet-stream")
            .body(body)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        self.world
            .recorder
            .latency("upload", upload_started.elapsed());
        if !uploaded {
            self.world.recorder.count("errors", 1);
            self.world.recorder.count("upload_failures", 1);
            return;
        }
        if self
            .request(
                "POST /attachments/{attachment}/confirm",
                reqwest::Method::POST,
                &format!("/attachments/{id}/confirm"),
                None,
                due,
            )
            .await
            .is_ok()
        {
            self.post_message(channel, 'c', vec![id], due).await;
        }
    }
}

/// An open event stream: a task reading it and writing activity, which ends when the server
/// closes it or `close` is called.
struct Stream {
    ended: tokio::sync::oneshot::Receiver<()>,
    close: Option<tokio::sync::oneshot::Sender<()>>,
}

/// Opens the event stream socket, from the user's source address when the profile names some.
async fn connect_events(
    user: &User,
) -> Result<
    (
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        tokio_tungstenite::tungstenite::handshake::client::Response,
    ),
    Box<dyn std::error::Error + Send + Sync>,
> {
    let url = user.world.events_url.as_str();
    let Some(source) = source_address(&user.world, user.index) else {
        return Ok(tokio_tungstenite::connect_async(url).await?);
    };
    let server = *user
        .world
        .events_addr
        .get_or_try_init(|| async {
            let parsed = reqwest::Url::parse(url)?;
            let host = parsed.host_str().ok_or("the API URL has no host")?;
            let port = parsed
                .port_or_known_default()
                .ok_or("the API URL has no port")?;
            let address: Result<std::net::SocketAddr, Box<dyn std::error::Error + Send + Sync>> =
                tokio::net::lookup_host((host, port))
                    .await?
                    .find(|a| a.is_ipv4() == source.is_ipv4())
                    .ok_or_else(|| "the API host has no address of the source's family".into());
            address
        })
        .await?;
    let socket = if source.is_ipv4() {
        tokio::net::TcpSocket::new_v4()?
    } else {
        tokio::net::TcpSocket::new_v6()?
    };
    socket.bind(std::net::SocketAddr::new(source, 0))?;
    let stream = socket.connect(server).await?;
    Ok(tokio_tungstenite::client_async_tls(url, stream).await?)
}

impl Stream {
    async fn open(user: Arc<User>, resume: bool) -> Result<Stream, Failed> {
        let opened = Instant::now();
        let (mut socket, _) = connect_events(&user).await.map_err(|_| {
            user.world.recorder.count("ws_failures", 1);
            Failed
        })?;
        let mut identify = json!({ "type": "identify", "sessionToken": user.token() });
        let last = user.state.last_sequence.load(Ordering::Relaxed);
        if resume && last > 0 {
            identify["resumeAfter"] = json!(last);
        }
        socket
            .send(WsMessage::Text(identify.to_string().into()))
            .await
            .map_err(|_| Failed)?;
        // Wait for `ready`.
        loop {
            match tokio::time::timeout(Duration::from_secs(15), socket.next()).await {
                Ok(Some(Ok(WsMessage::Text(text)))) => {
                    if text.contains("\"type\":\"ready\"") {
                        break;
                    }
                    if text.contains("\"type\":\"error\"") {
                        user.world.recorder.count("ws_rejected", 1);
                        return Err(Failed);
                    }
                }
                Ok(Some(Ok(_))) => {}
                _ => {
                    user.world.recorder.count("ws_failures", 1);
                    return Err(Failed);
                }
            }
        }
        user.world.recorder.latency("ready", opened.elapsed());
        user.state
            .ready_ns
            .store(user.world.clock.now_ns(), Ordering::Relaxed);
        let (ended_tx, ended) = tokio::sync::oneshot::channel();
        let (close, mut closing) = tokio::sync::oneshot::channel::<()>();
        let (mut sink, mut source) = socket.split();
        let (outbox, mut outbox_rx) = mpsc::unbounded_channel::<WsMessage>();
        tokio::spawn(async move {
            let mut activity = tokio::time::interval(ACTIVITY_INTERVAL);
            loop {
                tokio::select! {
                    frame = source.next() => match frame {
                        Some(Ok(WsMessage::Text(text))) => user.on_frame(&text),
                        Some(Ok(WsMessage::Ping(payload))) => {
                            let _ = outbox.send(WsMessage::Pong(payload));
                        }
                        Some(Ok(_)) => {}
                        _ => break,
                    },
                    Some(message) = outbox_rx.recv() => {
                        if sink.send(message).await.is_err() {
                            break;
                        }
                    }
                    _ = activity.tick() => {
                        let _ = outbox.send(WsMessage::Text("{\"type\":\"activity\"}".into()));
                    }
                    _ = &mut closing => {
                        let _ = sink.send(WsMessage::Close(None)).await;
                        return;
                    }
                }
            }
            let _ = ended_tx.send(());
        });
        Ok(Stream {
            ended,
            close: Some(close),
        })
    }

    fn close(mut self) {
        if let Some(close) = self.close.take() {
            let _ = close.send(());
        }
    }
}

impl User {
    /// One frame from the event stream.
    fn on_frame(&self, text: &str) {
        if !text.starts_with("{\"type\":\"event\"") {
            return;
        }
        let recorder = &self.world.recorder;
        recorder.count("events", 1);
        // The sequence comes right after the type; it is cheap to read without parsing.
        if let Some(sequence) = text
            .find("\"sequence\":")
            .map(|at| &text[at + 11..])
            .and_then(|rest| rest.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|digits| digits.parse::<u64>().ok())
        {
            self.state
                .last_sequence
                .fetch_max(sequence, Ordering::Relaxed);
        }
        let Some((kind, sender, sent_ns)) = read_marker(text) else {
            return;
        };
        if !text.contains("\"serverEvent\":\"message\"") || !text.contains("\"type\":\"create\"") {
            return;
        }
        if self.state.ready_ns.load(Ordering::Relaxed) > sent_ns {
            recorder.count("replayed", 1);
            return;
        }
        if sender != self.index {
            let delay = self.world.clock.now_ns() - sent_ns;
            let micros = u64::try_from((delay / 1000).max(1)).unwrap_or(u64::MAX);
            recorder.latency_micros(
                if kind == 'd' {
                    "delivery:dm"
                } else {
                    "delivery"
                },
                micros,
            );
            if let Ok(frame) = serde_json::from_str::<Value>(text)
                && let Some(id) = frame["event"]["id"].as_str().and_then(|s| s.parse().ok())
            {
                let mut seen = self.state.seen.lock().expect("seen lock");
                seen.push_back(id);
                if seen.len() > RECENT {
                    seen.pop_front();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_round_trip_inside_json() {
        let text = format!(
            "{{\"type\":\"event\",\"sequence\":9,\"event\":{{\"serverEvent\":\"message\",\"type\":\"create\",\"content\":\"hi {}\"}}}}",
            marker('c', 42, 1_700_000_000_123_456_789)
        );
        assert_eq!(
            read_marker(&text),
            Some(('c', 42, 1_700_000_000_123_456_789))
        );
        assert_eq!(read_marker("no marker here"), None);
        assert_eq!(read_marker("⟦bench:c:x:1⟧"), None);
    }

    #[test]
    fn only_active_actions_are_scheduled() {
        let behaviour = Behaviour {
            share: 1.0,
            messages_per_hour: 36.0,
            reactions_per_hour: 0.0,
            ..Behaviour::default()
        };
        assert_eq!(rates(&behaviour), vec![(Action::Message, 0.01)]);
    }
}
