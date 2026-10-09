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
//! the difference as its delivery time, under the name `delivery_metric` gives its kind.
//! Messages sent before the receiver's stream was ready arrive as the stream's replay of the
//! past minute, and are counted as `replayed` instead.
//!
//! Besides what its behaviour names, a user does what the client does on its own: polls
//! presence, sends activity, and, when it has a channel open (`Behaviour::viewing_share`), names
//! it in a `viewing` frame, types before each message it writes there, and reports how far it
//! has read as messages arrive.

use crate::clock::Clock;
use crate::profile::{Behaviour, Profile};
use crate::stats::Recorder;
use aspen_bench_protocol::Manifest;
use aspen_bench_protocol::words;
use futures_util::{SinkExt, StreamExt};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
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
/// The connections each user keeps open between requests.
const IDLE_CONNECTIONS: usize = 2;
/// How long the client gathers read positions before reporting them (`READ_REPORT_MS`).
const READ_REPORT_DELAY: Duration = Duration::from_secs(1);
/// How often the client repeats `typing` while its user goes on (`TYPING_REFRESH_MS`).
const TYPING_REFRESH: Duration = Duration::from_secs(3);
/// How long a presence override a user chooses lasts.
const PRESENCE_OVERRIDE_SECONDS: u32 = 600;
const PRESENCE_OVERRIDES: [&str; 3] = ["away", "doNotDisturb", "invisible"];
/// Messages, threads, and polls remembered per user to act on.
const RECENT: usize = 50;
/// The client's page sizes (`client/packages/protocol/src/sync.ts`), so reads ask for what its
/// reads do.
const MESSAGE_PAGE: u32 = 50;
const DM_PAGE: u32 = 100;
const LIST_PAGE: u32 = 100;
const SEARCH_PAGE: u32 = 25;
const ACTIVITY_PAGE: u32 = 25;
const SAVED_PAGE: u32 = 50;
/// What the client sideloads with each read, comma separated as it sends them.
const COMMUNITY_INCLUDES: &str =
    "channels,categories,members,voice,readStates,mutes,collapses,roles,notifications,emoji";
const DM_INCLUDES: &str = "users,readStates,mutes,notifications,voice";
const WINDOW_INCLUDES: &str =
    "authors,memberships,attachments,polls,threads,echoes,reactions,linked,warnings,annotations";
const SEARCH_INCLUDES: &str = "authors,memberships,attachments,polls,channels,reactions";
/// `LISTED_INCLUDES`: the activity feed's and the saved list's.
const LISTED_INCLUDES: &str = "authors,memberships,attachments,polls,channels,reactions,readStates";
/// How long a poll a user opens stays open.
const POLL_SECONDS: u32 = 3600;
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
    /// Every community text channel, where a thread may start.
    pub text_channels: HashSet<Uuid>,
    /// The pictures users post, JPEG, by width and height, made once.
    pub images: Mutex<HashMap<(u32, u32), bytes::Bytes>>,
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

pub fn text_channels(manifest: &Manifest) -> HashSet<Uuid> {
    manifest
        .communities
        .iter()
        .flat_map(|c| c.text_channels.iter().copied())
        .collect()
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
    Mention,
    ThreadReply,
    ThreadStart,
    Poll,
    Vote,
    Search,
    Activity,
    Save,
    SavedRead,
    Image,
    Presence,
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
        (Action::Mention, behaviour.mentions_per_hour),
        (Action::ThreadReply, behaviour.thread_replies_per_hour),
        (Action::ThreadStart, behaviour.threads_per_hour),
        (Action::Poll, behaviour.polls_per_hour),
        (Action::Vote, behaviour.poll_votes_per_hour),
        (Action::Search, behaviour.searches_per_hour),
        (Action::Activity, behaviour.activity_reads_per_hour),
        (Action::Save, behaviour.saves_per_hour),
        (Action::SavedRead, behaviour.saved_reads_per_hour),
        (Action::Image, behaviour.images_per_hour),
        (Action::Presence, behaviour.presence_changes_per_hour),
    ]
    .into_iter()
    .filter(|(_, per_hour)| *per_hour > 0.0)
    .map(|(action, per_hour)| (action, per_hour / 3600.0))
    .collect()
}

/// Kinds of marked message, each with its own delivery time.
const CHANNEL: char = 'c';
const DM: char = 'd';
const THREAD: char = 't';
const IMAGE: char = 'i';
const EVERYONE: char = 'e';

/// The delivery measurement a kind of message counts toward.
fn delivery_metric(kind: char) -> &'static str {
    match kind {
        DM => "delivery:dm",
        THREAD => "delivery:thread",
        IMAGE => "delivery:image",
        EVERYONE => "delivery:everyone",
        _ => "delivery",
    }
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
    /// Recent messages by others, with their channels (to react to, save, or start a thread
    /// from), and by this user (to edit or delete).
    seen: Mutex<VecDeque<(Uuid, Uuid)>>,
    own: Mutex<VecDeque<Uuid>>,
    /// DM channels by the other person's index.
    dms: Mutex<HashMap<u32, Uuid>>,
    /// Threads and open polls (with their answer counts) heard of or made, beside those seeded.
    threads: Mutex<VecDeque<Uuid>>,
    polls: Mutex<VecDeque<(Uuid, u32)>>,
    /// Whether the user is in a call; one at a time.
    in_call: AtomicBool,
    /// The channel the user has open, for a user who has one (`Behaviour::viewing_share`).
    viewing: Mutex<Option<Uuid>>,
    /// Frames for the open event stream to send.
    outbox: Mutex<Option<mpsc::UnboundedSender<WsMessage>>>,
    /// The latest message read and not yet reported, and whether a report is due.
    unreported: Mutex<Option<(Uuid, Uuid)>>,
    report_due: AtomicBool,
    /// Whether the user has chosen a presence override.
    presence_override: AtomicBool,
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
        let user = Self {
            world,
            index,
            behaviour,
            // One client each: a real user's connections are their own.
            http: reqwest::Client::builder()
                .local_address(source)
                .pool_idle_timeout(Duration::from_secs(90))
                // Over plain HTTP each request at once takes a connection of its own, and the
                // startup reads would leave ten per user open: sockets the generator and the
                // listener both hold for every user. Over HTTPS they share one HTTP/2
                // connection, as a browser's do.
                .pool_max_idle_per_host(IDLE_CONNECTIONS)
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
                threads: Mutex::new(VecDeque::new()),
                polls: Mutex::new(VecDeque::new()),
                in_call: AtomicBool::new(false),
                viewing: Mutex::new(None),
                outbox: Mutex::new(None),
                unreported: Mutex::new(None),
                report_due: AtomicBool::new(false),
                presence_override: AtomicBool::new(false),
            }),
            rng: Mutex::new(StdRng::seed_from_u64(seed)),
        };
        if user.random() < user.behaviour.viewing_share {
            // The busy first channel of their first community, until they open another.
            let first = user.world.memberships[index as usize]
                .first()
                .and_then(|c| user.world.manifest.communities[*c].text_channels.first())
                .copied();
            *user.state.viewing.lock().expect("viewing lock") = first;
        }
        user
    }

    fn viewing(&self) -> Option<Uuid> {
        *self.state.viewing.lock().expect("viewing lock")
    }

    /// Sends a frame on the open event stream, if there is one.
    fn send_frame(&self, frame: Value) {
        if let Some(outbox) = self.state.outbox.lock().expect("outbox lock").as_ref() {
            let _ = outbox.send(WsMessage::Text(frame.to_string().into()));
        }
    }

    /// Names the open channel to the server, as the client does on every `ready` and whenever
    /// it changes, so the user hears who is typing there.
    fn send_viewing(&self) {
        if let Some(channel) = self.viewing() {
            self.send_frame(json!({ "type": "viewing", "channelIds": [channel] }));
        }
    }

    /// Opens a channel: it becomes the one the user views, for a user who views one.
    fn open_channel(&self, channel: Uuid) {
        let mut viewing = self.state.viewing.lock().expect("viewing lock");
        if viewing.is_some() && *viewing != Some(channel) {
            *viewing = Some(channel);
            drop(viewing);
            self.send_viewing();
        }
    }

    /// Notes that the user has read up to `message` in `channel`, reporting it a moment later
    /// with whatever else is read meanwhile, as the client gathers its reports.
    fn read(self: &Arc<Self>, channel: Uuid, message: Uuid) {
        *self.state.unreported.lock().expect("unreported lock") = Some((channel, message));
        if self.state.report_due.swap(true, Ordering::Relaxed) {
            return;
        }
        let user = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(READ_REPORT_DELAY).await;
            user.state.report_due.store(false, Ordering::Relaxed);
            let read = user
                .state
                .unreported
                .lock()
                .expect("unreported lock")
                .take();
            if let Some((channel, message)) = read {
                let _ = user
                    .request(
                        "PUT /channels/{channel}/read-states/@me",
                        reqwest::Method::PUT,
                        &format!("/channels/{channel}/read-states/@me"),
                        Some(json!({ "lastRead": message })),
                        Instant::now(),
                    )
                    .await;
            }
        });
    }

    /// Types in `channel` for the behaviour's typing time from `due`, saying so as the client
    /// does, and returns when the message is to be sent. A user with the app in the background
    /// (no channel open) sends no typing frames.
    async fn type_in(&self, channel: Uuid, due: Instant) -> Instant {
        let seconds = self.behaviour.typing_seconds;
        if seconds <= 0.0 {
            return due;
        }
        let send_at = due + Duration::from_secs_f64(seconds);
        let shown = self.viewing().is_some();
        let mut at = due;
        while at < send_at {
            tokio::time::sleep_until(at).await;
            if shown {
                self.send_frame(json!({ "type": "typing", "channelId": channel }));
                self.world.recorder.count("typing_sent", 1);
            }
            at += TYPING_REFRESH;
        }
        tokio::time::sleep_until(send_at).await;
        if shown {
            self.send_frame(json!({ "type": "stoppedTyping", "channelId": channel }));
        }
        send_at
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
            self.sign_in().await?;
            let now = Instant::now();
            // What the client reads at startup (`AspenSync#bootstrap`), at once.
            let get = |route: &'static str, path: String| {
                let user = Arc::clone(&self);
                async move {
                    user.request(route, reqwest::Method::GET, &path, None, now)
                        .await
                }
            };
            let reads = futures_util::future::join_all([
                get("GET /users/{user}", "/users/@me".into()),
                get(
                    "GET /users/{user}/communities",
                    format!("/users/@me/communities?include={COMMUNITY_INCLUDES}"),
                ),
                get(
                    "GET /users/@me/dms",
                    format!("/users/@me/dms?include={DM_INCLUDES}&limit={DM_PAGE}"),
                ),
                get("GET /users/@me/admin", "/users/@me/admin".into()),
                get(
                    "GET /users/@me/blocks",
                    "/users/@me/blocks?include=users".into(),
                ),
                get("GET /plugins", "/plugins".into()),
                get(
                    "GET /users/@me/held-messages",
                    format!("/users/@me/held-messages?limit={LIST_PAGE}"),
                ),
                get(
                    "GET /users/@me/saved-messages",
                    "/users/@me/saved-messages".into(),
                ),
                get(
                    "GET /users/@me/thread-follows",
                    "/users/@me/thread-follows".into(),
                ),
                get(
                    "GET /users/@me/presence-override",
                    "/users/@me/presence-override".into(),
                ),
            ])
            .await;
            for read in reads {
                read?;
            }
            // Read once the rest are in, as the client's preferences load.
            self.request(
                "GET /users/{user}/preferences",
                reqwest::Method::GET,
                "/users/@me/preferences",
                None,
                now,
            )
            .await?;
            self.world.recorder.latency("bootstrap", now.elapsed());
        }
        Stream::open(self, resume).await
    }

    async fn sign_in(&self) -> Result<(), Failed> {
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
        Ok(())
    }

    /// For an `announcement` event: signs in as this user, the owner of the community at
    /// `community`, and tags `@everyone` in its first channel.
    pub async fn announce(self: Arc<Self>, community: usize) {
        let Some(channel) = self.world.manifest.communities[community]
            .text_channels
            .first()
            .copied()
        else {
            return;
        };
        if self.sign_in().await.is_err() {
            return;
        }
        self.world.recorder.count("announcements", 1);
        self.post(
            "POST /channels/{channel}/messages",
            &format!("/channels/{channel}/messages"),
            EVERYONE,
            "@everyone benchmark announcement",
            Vec::new(),
            Instant::now(),
        )
        .await;
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
        if let Some(channel) = self.viewing() {
            // The open channel's header shows how many of its people are online.
            let _ = self
                .request(
                    "GET /channels/{channel}/presence",
                    reqwest::Method::GET,
                    &format!("/channels/{channel}/presence"),
                    None,
                    Instant::now(),
                )
                .await;
        }
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

    /// Posts a marked message, `content` before the marker, to `path` (a channel's messages,
    /// or a message's thread), recorded under `route`. Returns the posted (or held) message.
    async fn post(
        &self,
        route: &str,
        path: &str,
        kind: char,
        content: &str,
        attachments: Vec<Uuid>,
        due: Instant,
    ) -> Option<Value> {
        let sent = self.world.clock.now_ns();
        let content = format!("{content} {}", marker(kind, self.index, sent));
        let message = self
            .request(
                route,
                reqwest::Method::POST,
                path,
                Some(json!({ "content": content, "attachments": attachments, "mayHold": true })),
                due,
            )
            .await
            .ok()?;
        self.world.recorder.count("messages_sent", 1);
        if let Some(id) = message["id"].as_str().and_then(|s| s.parse().ok()) {
            remember(&self.state.own, id);
        }
        Some(message)
    }

    /// Writes a message where the user has it open, as a person does: the channel becomes the
    /// one they view, for a user who views one.
    async fn post_message(&self, channel: Uuid, kind: char, content: &str, due: Instant) {
        self.open_channel(channel);
        let due = self.type_in(channel, due).await;
        self.post(
            "POST /channels/{channel}/messages",
            &format!("/channels/{channel}/messages"),
            kind,
            content,
            Vec::new(),
            due,
        )
        .await;
    }

    /// A recent message by someone else, and its channel.
    fn seen_message(&self) -> Option<(Uuid, Uuid)> {
        let items: Vec<(Uuid, Uuid)> = self
            .state
            .seen
            .lock()
            .expect("seen lock")
            .iter()
            .copied()
            .collect();
        self.pick(&items)
    }

    async fn act(self: &Arc<Self>, action: Action, due: Instant) {
        match action {
            Action::Message => {
                if let Some(channel) = self.channel() {
                    self.post_message(channel, CHANNEL, "benchmark message", due)
                        .await;
                }
            }
            Action::Mention => self.mention(due).await,
            Action::Dm => self.send_dm(due).await,
            Action::React => {
                let target = self.seen_message();
                if let (Some((message, _)), Some(emoji)) = (target, self.pick(&EMOJI)) {
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
                    self.open_channel(channel);
                    if let Ok(page) = self
                        .request(
                            "GET /channels/{channel}/messages",
                            reqwest::Method::GET,
                            &format!(
                                "/channels/{channel}/messages?limit={MESSAGE_PAGE}&include={WINDOW_INCLUDES}"
                            ),
                            None,
                            due,
                        )
                        .await
                        && self.viewing() == Some(channel)
                        && let Some(latest) = ids(&page["data"]).max()
                    {
                        self.read(channel, latest);
                    }
                }
            }
            Action::ThreadReply => self.reply_in_thread(due).await,
            Action::ThreadStart => self.start_thread(due).await,
            Action::Poll => self.open_poll(due).await,
            Action::Vote => self.vote(due).await,
            Action::Search => self.search(due).await,
            Action::Activity => {
                let _ = self
                    .request(
                        "GET /users/@me/activity",
                        reqwest::Method::GET,
                        &format!(
                            "/users/@me/activity?filter[dms]=true&filter[unread]=false&limit={ACTIVITY_PAGE}&include={LISTED_INCLUDES}"
                        ),
                        None,
                        due,
                    )
                    .await;
            }
            Action::Save => {
                if let Some((message, _)) = self.seen_message() {
                    let _ = self
                        .request(
                            "PUT /users/@me/saved-messages/{message}",
                            reqwest::Method::PUT,
                            &format!("/users/@me/saved-messages/{message}"),
                            None,
                            due,
                        )
                        .await;
                }
            }
            Action::SavedRead => {
                let _ = self
                    .request(
                        "GET /users/@me/saved-messages/messages",
                        reqwest::Method::GET,
                        &format!(
                            "/users/@me/saved-messages/messages?limit={SAVED_PAGE}&include={LISTED_INCLUDES}"
                        ),
                        None,
                        due,
                    )
                    .await;
            }
            Action::Attachment => self.send_attachment(due).await,
            Action::Image => self.send_image(due).await,
            Action::Presence => self.change_presence(due).await,
            Action::Call => self.call(due).await,
            Action::Reconnect => {}
        }
    }

    /// A message tagging a member of the channel's community by name.
    async fn mention(&self, due: Instant) {
        let Some(community) = self.pick(&self.world.memberships[self.index as usize]) else {
            return;
        };
        let community = &self.world.manifest.communities[community];
        let channel = if self.random() < self.behaviour.hot_channel_share {
            community.text_channels.first().copied()
        } else {
            self.pick(&community.text_channels)
        };
        let (Some(channel), Some(peer)) = (channel, self.pick(&community.members)) else {
            return;
        };
        let peer = self.world.manifest.users[peer as usize].id;
        self.world.recorder.count("mentions_sent", 1);
        self.post_message(
            channel,
            CHANNEL,
            &format!("<@{peer}> benchmark mention"),
            due,
        )
        .await;
    }

    /// A reply in a thread of one of the user's communities: one seeded, or one heard of.
    async fn reply_in_thread(&self, due: Instant) {
        let heard: Vec<Uuid> = self
            .state
            .threads
            .lock()
            .expect("threads lock")
            .iter()
            .copied()
            .collect();
        let thread = if !heard.is_empty() && self.random() < 0.5 {
            self.pick(&heard)
        } else {
            self.pick(&self.world.memberships[self.index as usize])
                .and_then(|c| self.pick(&self.world.manifest.communities[c].threads))
                .or_else(|| self.pick(&heard))
        };
        if let Some(thread) = thread {
            self.post_message(thread, THREAD, "benchmark reply", due)
                .await;
        }
    }

    /// Starts a thread from a message someone else posted in a community channel, with its
    /// first reply.
    async fn start_thread(&self, due: Instant) {
        let candidates: Vec<Uuid> = self
            .state
            .seen
            .lock()
            .expect("seen lock")
            .iter()
            .filter(|(_, channel)| self.world.text_channels.contains(channel))
            .map(|(message, _)| *message)
            .collect();
        let Some(starter) = self.pick(&candidates) else {
            return;
        };
        if let Some(reply) = self
            .post(
                "POST /messages/{message}/thread/messages",
                &format!("/messages/{starter}/thread/messages"),
                THREAD,
                "benchmark thread",
                Vec::new(),
                due,
            )
            .await
            && let Some(thread) = reply["channelId"].as_str().and_then(|s| s.parse().ok())
        {
            remember(&self.state.threads, thread);
        }
    }

    async fn open_poll(&self, due: Instant) {
        let Some(channel) = self.channel() else {
            return;
        };
        let options = 2 + (self.random() * 3.0) as usize;
        let question = format!("benchmark poll {}", self.words(4));
        let body = json!({
            "question": question,
            "options": (0..options).map(|_| json!({ "label": self.words(1) })).collect::<Vec<_>>(),
            "durationSeconds": POLL_SECONDS,
            "multipleChoice": false,
            "anonymous": false,
            "allowWriteIns": false,
        });
        if let Ok(poll) = self
            .request(
                "POST /channels/{channel}/polls",
                reqwest::Method::POST,
                &format!("/channels/{channel}/polls"),
                Some(body),
                due,
            )
            .await
            && let Some(id) = poll["id"].as_str().and_then(|s| s.parse().ok())
        {
            self.world.recorder.count("polls_opened", 1);
            remember(&self.state.polls, (id, u32::try_from(options).unwrap_or(2)));
        }
    }

    /// Votes in an open poll of one of the user's communities: one seeded, or one heard of.
    async fn vote(&self, due: Instant) {
        let heard: Vec<(Uuid, u32)> = self
            .state
            .polls
            .lock()
            .expect("polls lock")
            .iter()
            .copied()
            .collect();
        let poll = if !heard.is_empty() && self.random() < 0.5 {
            self.pick(&heard)
        } else {
            self.pick(&self.world.memberships[self.index as usize])
                .and_then(|c| {
                    let open = &self.world.manifest.communities[c].open_polls;
                    self.pick(open).map(|p| (p.id, p.options))
                })
                .or_else(|| self.pick(&heard))
        };
        let Some((poll, options)) = poll else {
            return;
        };
        let option = (self.random() * f64::from(options.max(1))) as u32;
        let _ = self
            .request(
                "PUT /polls/{poll}/votes/{option}/@me",
                reqwest::Method::PUT,
                &format!("/polls/{poll}/votes/{option}/@me"),
                None,
                due,
            )
            .await;
    }

    /// Searches for a word the seeded history and messages are written in, within one of the
    /// user's communities half the time, everywhere they may read otherwise.
    async fn search(&self, due: Instant) {
        let text = self.words(1 + usize::from(self.random() < 0.3));
        let mut path = format!(
            "/messages?filter[text]={}&limit={SEARCH_PAGE}&include={SEARCH_INCLUDES}",
            percent_encode(&text)
        );
        if self.random() < 0.5
            && let Some(community) = self.pick(&self.world.memberships[self.index as usize])
        {
            path.push_str(&format!(
                "&filter[community]={}",
                self.world.manifest.communities[community].id
            ));
        }
        let _ = self
            .request("GET /messages", reqwest::Method::GET, &path, None, due)
            .await;
    }

    /// Chooses a presence override, or takes the one chosen away.
    async fn change_presence(&self, due: Instant) {
        let chosen = self.state.presence_override.load(Ordering::Relaxed);
        let outcome = if chosen && self.random() < 0.5 {
            self.request(
                "DELETE /users/@me/presence-override",
                reqwest::Method::DELETE,
                "/users/@me/presence-override",
                None,
                due,
            )
            .await
            .map(|_| false)
        } else {
            let presence = self.pick(&PRESENCE_OVERRIDES).unwrap_or("away");
            self.request(
                "PUT /users/@me/presence-override",
                reqwest::Method::PUT,
                "/users/@me/presence-override",
                Some(json!({
                    "presenceOverride": presence,
                    "durationSeconds": PRESENCE_OVERRIDE_SECONDS,
                })),
                due,
            )
            .await
            .map(|_| true)
        };
        if let Ok(chosen) = outcome {
            self.state
                .presence_override
                .store(chosen, Ordering::Relaxed);
        }
    }

    /// `count` words drawn as seeded history's are.
    fn words(&self, count: usize) -> String {
        (0..count)
            .map(|_| words::word(self.random()))
            .collect::<Vec<_>>()
            .join(" ")
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
        self.post_message(channel, DM, "benchmark message", due)
            .await;
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
        let size = usize::try_from(self.behaviour.attachment_bytes).unwrap_or(0);
        let body = bytes::Bytes::from(vec![0x5a_u8; size]);
        if let Some(id) = self
            .upload("benchmark.bin", "application/octet-stream", body, due)
            .await
        {
            self.post(
                "POST /channels/{channel}/messages",
                &format!("/channels/{channel}/messages"),
                CHANNEL,
                "benchmark attachment",
                vec![id],
                due,
            )
            .await;
        }
    }

    /// Uploads a picture and posts it. The server holds the message until it has made the
    /// picture's preview, so its delivery (`delivery:image`) includes making the preview.
    async fn send_image(&self, due: Instant) {
        let Some(channel) = self.channel() else {
            return;
        };
        let size = (self.behaviour.image_width, self.behaviour.image_height);
        let picture = {
            let mut images = self.world.images.lock().expect("images lock");
            images
                .entry(size)
                .or_insert_with(|| crate::picture::jpeg(size.0, size.1))
                .clone()
        };
        if let Some(id) = self
            .upload("benchmark.jpg", "image/jpeg", picture, due)
            .await
        {
            self.post(
                "POST /channels/{channel}/messages",
                &format!("/channels/{channel}/messages"),
                IMAGE,
                "benchmark picture",
                vec![id],
                due,
            )
            .await;
        }
    }

    /// Uploads a file to object storage as the client does: asks for an upload URL, puts the
    /// file there, and confirms it. Returns the attachment.
    async fn upload(
        &self,
        file_name: &str,
        mime_type: &str,
        body: bytes::Bytes,
        due: Instant,
    ) -> Option<Uuid> {
        let handle = self
            .request(
                "POST /attachments",
                reqwest::Method::POST,
                "/attachments",
                Some(json!({
                    "fileName": file_name,
                    "mimeType": mime_type,
                    "byteSize": body.len(),
                })),
                due,
            )
            .await
            .ok()?;
        let id = handle["id"].as_str().and_then(|s| s.parse::<Uuid>().ok())?;
        let url = handle["uploadUrl"].as_str()?;
        let upload_started = Instant::now();
        let uploaded = self
            .http
            .put(url)
            .header("content-type", mime_type)
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
            return None;
        }
        self.request(
            "POST /attachments/{attachment}/confirm",
            reqwest::Method::POST,
            &format!("/attachments/{id}/confirm"),
            None,
            due,
        )
        .await
        .ok()?;
        Some(id)
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
    // As `connect_async` does, and browsers do for WebSockets.
    stream.set_nodelay(true)?;
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
        *user.state.outbox.lock().expect("outbox lock") = Some(outbox.clone());
        user.send_viewing();
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
    fn on_frame(self: &Arc<Self>, text: &str) {
        let recorder = &self.world.recorder;
        if text.starts_with("{\"type\":\"ephemeral\"") {
            // Who is typing in the channel the user has open.
            recorder.count("ephemeral", 1);
            return;
        }
        if !text.starts_with("{\"type\":\"event\"") {
            return;
        }
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
        // Only these are parsed, which keeps a generator playing many users light.
        let message = text.contains("\"serverEvent\":\"message\"");
        let created = text.contains("\"type\":\"create\"");
        let poll_created = created && text.contains("\"serverEvent\":\"poll\"");
        let thread_started =
            message && text.contains("\"type\":\"update\"") && text.contains("\"thread\":\"");
        if !(message && created || poll_created || thread_started) {
            return;
        }
        let Ok(frame) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let event = &frame["event"];
        let uuid = |field: &str| event[field].as_str().and_then(|s| s.parse::<Uuid>().ok());
        if poll_created {
            if let (Some(poll), Some(options)) = (uuid("id"), event["options"].as_array()) {
                remember(
                    &self.state.polls,
                    (poll, u32::try_from(options.len()).unwrap_or(0)),
                );
            }
            return;
        }
        if thread_started {
            if let Some(thread) = uuid("thread") {
                remember(&self.state.threads, thread);
            }
            return;
        }
        let (Some(id), Some(channel)) = (uuid("id"), uuid("channelId")) else {
            return;
        };
        if uuid("author") != Some(self.world.manifest.users[self.index as usize].id) {
            remember(&self.state.seen, (id, channel));
            if self.viewing() == Some(channel) {
                self.read(channel, id);
            }
        }
        let Some((kind, sender, sent_ns)) = read_marker(text) else {
            return;
        };
        if self.state.ready_ns.load(Ordering::Relaxed) > sent_ns {
            recorder.count("replayed", 1);
            return;
        }
        if sender != self.index {
            let delay = self.world.clock.now_ns() - sent_ns;
            let micros = u64::try_from((delay / 1000).max(1)).unwrap_or(u64::MAX);
            recorder.latency_micros(delivery_metric(kind), micros);
        }
    }
}

/// Adds `item` to a list of recent things, forgetting the oldest past `RECENT`.
fn remember<T>(list: &Mutex<VecDeque<T>>, item: T) {
    let mut list = list.lock().expect("recent lock");
    list.push_back(item);
    if list.len() > RECENT {
        list.pop_front();
    }
}

/// The ids of a list of records.
fn ids(records: &Value) -> impl Iterator<Item = Uuid> + '_ {
    records
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|record| record["id"].as_str().and_then(|s| s.parse().ok()))
}

/// `text` as a query string value.
fn percent_encode(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
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
