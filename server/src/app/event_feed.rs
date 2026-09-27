//! The server's one reading of the event stream, shared by every event stream connection it
//! holds.
//!
//! A single ordered JetStream consumer per API server reads every event subject, and one
//! dispatcher task hands each event, in order, to a fixed set of routing shards. Every connection
//! belongs to one shard, which delivers to it the events it is entitled to: those on its user's
//! subject, or on the subject of a community its user belongs to (`events::subject_owner`).
//! NATS therefore does work in proportion to events times API servers, however many people are
//! connected, and each event is parsed once per server. Handing an event to a connection's queue
//! wakes the task writing its socket, which costs a little for each reader; the shards split that
//! across cores, so an event in a community of thousands does not hold every later event back.
//!
//! The dispatcher also keeps what the stream retains, the last `MAX_EVENT_AGE` of events,
//! indexed by owner. Catch-up is served from it: a reconnect's `resumeAfter`, and the window a
//! first connection replays so that nothing published while it loaded its state from REST is
//! lost. A connection is registered inside the dispatcher's loop, which queues what it missed
//! and then adds it to its shard through the same ordered channel as the events, ahead of the
//! next one, so it sees every event exactly once with no handoff to reconcile. A reconnect storm costs NATS nothing. The
//! retained events are loaded from the stream when the server starts, and registrations wait
//! until they are.
//!
//! What a connection reads is its user and their communities, read from the database when it
//! connects. Membership changes arrive as `userCommunity` events on the user's own subject; each
//! shard applies them to its connections of that user as it routes them, in stream order, so a
//! join brings the community's events from that point and a leave ends them. A catch-up replay
//! applies the ones it passes over the same way.
//!
//! A connection's queue is bounded (`event_queue_size`). One that fills is dropped, and its
//! client resumes from the last event it processed. If the feed itself misses events (the
//! stream's sequence jumps, or restarts after NATS lost it), every local connection is dropped,
//! and each learns from `resumed: false` that it must rebuild its state.

use crate::app::events::{SubjectOwner, memberships, subject_owner};
use crate::app::{self, ASPEN_NATS_STREAM_NAME, CommunityId, GlobalServerContext, UserId};
use async_nats::jetstream;
use async_nats::jetstream::consumer::pull::{Ordered, OrderedConfig};
use async_nats::jetstream::consumer::{DeliverPolicy, ReplayPolicy};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;
use tracing::{error, info, warn};

/// How long the stream keeps an event, and so how far back a client may resume and how much a
/// first connection replays.
pub const MAX_EVENT_AGE: Duration = Duration::from_secs(60);

/// How long a starting server waits for the retained events before serving connections with
/// what it has.
const LOAD_DEADLINE: Duration = Duration::from_secs(10);

/// How often retained events past `MAX_EVENT_AGE` are let go of when nothing new arrives.
const EVICT_INTERVAL: Duration = Duration::from_secs(1);

/// How many registrations may wait for the dispatcher at once.
const REGISTRATION_QUEUE: usize = 1024;

/// How many commands may wait for one routing shard before the dispatcher waits for it.
const SHARD_QUEUE: usize = 4096;

/// One event as the stream delivered it.
pub struct FeedEvent {
    /// The stream sequence, which clients hand back as `resumeAfter`.
    pub sequence: u64,
    /// The `Aspen-Event-Id` header, the same on every copy of the event.
    pub event_id: Option<String>,
    /// The `ServerEvent`, as published.
    pub payload: Box<RawValue>,
    owner: SubjectOwner,
    /// On a user's own subject, the membership of theirs the event makes or ends: the community
    /// and whether it was joined.
    membership: Option<(CommunityId, bool)>,
    /// When it was published, on this process's clock.
    published: Instant,
}

/// What a connection receives, in order.
pub enum Delivery {
    /// What it missed before it registered. Always first, and only once.
    CatchUp(Vec<Arc<FeedEvent>>),
    /// One event as it happens.
    Live(Arc<FeedEvent>),
}

/// A registered connection's end of the feed. Dropping it unregisters the connection. The
/// deliveries end when the server drops the connection for falling behind or for a gap.
pub struct Subscription {
    /// Whether `resume_after` was honoured; when not, the catch-up is the whole retained window.
    pub resumed: bool,
    pub deliveries: mpsc::Receiver<Delivery>,
    _registration: Registration,
}

struct Registration {
    id: u64,
    unregister: mpsc::UnboundedSender<u64>,
}

impl Drop for Registration {
    fn drop(&mut self) {
        // The dispatcher is gone only when the server is shutting down.
        let _ = self.unregister.send(self.id);
    }
}

/// The handle connections register through; cheap to clone.
#[derive(Clone)]
pub struct EventFeed {
    registrations: mpsc::Sender<Register>,
    unregister: mpsc::UnboundedSender<u64>,
    next_id: Arc<AtomicU64>,
    queue_size: usize,
}

struct Register {
    id: u64,
    user: UserId,
    communities: Vec<CommunityId>,
    resume_after: Option<u64>,
    deliveries: mpsc::Sender<Delivery>,
    resumed: oneshot::Sender<bool>,
}

impl EventFeed {
    /// Starts the dispatcher on `context`'s event stream, routing through `shards` tasks.
    /// `queue_size` bounds each connection's queue.
    pub fn start(context: jetstream::Context, queue_size: usize, shards: usize) -> Self {
        let (registrations, registrations_rx) = mpsc::channel(REGISTRATION_QUEUE);
        let (unregister, unregister_rx) = mpsc::unbounded_channel();
        let shards = (0..shards.max(1))
            .map(|_| {
                let (commands, commands_rx) = mpsc::channel(SHARD_QUEUE);
                tokio::spawn(route_shard(commands_rx));
                commands
            })
            .collect();
        tokio::spawn(dispatch(context, shards, registrations_rx, unregister_rx));
        Self {
            registrations,
            unregister,
            next_id: Arc::new(AtomicU64::new(0)),
            queue_size: queue_size.max(1),
        }
    }
}

/// Registers a connection of `user`'s, resuming after `resume_after` when that is still
/// retained.
pub async fn subscribe(
    state: &GlobalServerContext,
    user: UserId,
    resume_after: Option<u64>,
) -> app::Result<Subscription> {
    let communities = {
        let mut conn = state.connection_pool.get().await?;
        memberships(conn.as_mut(), user).await?
    };
    let feed = &state.event_feed;
    let id = feed.next_id.fetch_add(1, Ordering::Relaxed);
    let (deliveries_tx, deliveries) = mpsc::channel(feed.queue_size);
    let (resumed_tx, resumed_rx) = oneshot::channel();
    feed.registrations
        .send(Register {
            id,
            user,
            communities,
            resume_after,
            deliveries: deliveries_tx,
            resumed: resumed_tx,
        })
        .await
        .map_err(|_| app::Error::EventFeedStopped)?;
    let resumed = resumed_rx.await.map_err(|_| app::Error::EventFeedStopped)?;
    Ok(Subscription {
        resumed,
        deliveries,
        _registration: Registration {
            id,
            unregister: feed.unregister.clone(),
        },
    })
}

/// A membership of `user`'s made or ended by an event on their subject, read without parsing
/// the rest of it.
fn membership_change(payload: &str, user: UserId) -> Option<(CommunityId, bool)> {
    if !payload.contains("\"userCommunity\"") {
        return None;
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Glance {
        server_event: String,
        #[serde(rename = "type")]
        kind: Option<String>,
        user: Option<UserId>,
        community: Option<CommunityId>,
    }
    let glance: Glance = serde_json::from_str(payload).ok()?;
    if glance.server_event != "userCommunity" || glance.user != Some(user) {
        return None;
    }
    match (glance.kind.as_deref(), glance.community) {
        (Some("create"), Some(community)) => Some((community, true)),
        (Some("delete"), Some(community)) => Some((community, false)),
        _ => None,
    }
}

/// The events the stream retains, by owner.
#[derive(Default)]
struct Retained {
    by_owner: HashMap<SubjectOwner, VecDeque<Arc<FeedEvent>>>,
    /// Every retained event's owner and sequence, oldest first, for letting them go in order.
    order: VecDeque<(Instant, SubjectOwner, u64)>,
    /// The last sequence seen, 0 before any.
    last_sequence: u64,
    bytes: usize,
}

impl Retained {
    /// The sequence of the oldest event retained, or the next to come when none is.
    fn first_sequence(&self) -> u64 {
        self.order
            .front()
            .map_or(self.last_sequence + 1, |(_, _, sequence)| *sequence)
    }

    fn push(&mut self, event: Arc<FeedEvent>) {
        self.last_sequence = event.sequence;
        self.bytes += event.payload.get().len();
        self.order
            .push_back((event.published, event.owner, event.sequence));
        self.by_owner
            .entry(event.owner)
            .or_default()
            .push_back(event);
    }

    fn evict(&mut self, now: Instant) {
        while let Some(&(published, owner, _)) = self.order.front() {
            if now.duration_since(published) < MAX_EVENT_AGE {
                break;
            }
            self.order.pop_front();
            if let Some(events) = self.by_owner.get_mut(&owner) {
                if let Some(event) = events.pop_front() {
                    self.bytes -= event.payload.get().len();
                }
                if events.is_empty() {
                    self.by_owner.remove(&owner);
                }
            }
        }
    }

    /// Forgets everything, as after a gap: nothing retained can be resumed across it.
    fn clear(&mut self) {
        let last_sequence = self.last_sequence;
        *self = Self {
            last_sequence,
            ..Self::default()
        };
    }

    /// `owner`'s retained events after `after`, oldest first.
    fn since(&self, owner: SubjectOwner, after: u64) -> impl Iterator<Item = &Arc<FeedEvent>> {
        let events = self.by_owner.get(&owner);
        let start = events.map_or(0, |events| events.partition_point(|e| e.sequence <= after));
        events
            .into_iter()
            .flat_map(move |events| events.range(start..))
    }

    /// Where a connection's catch-up starts and whether that honours `resume_after`: a position
    /// the stream still holds the next event after, and not ahead of it (which happens when the
    /// in-memory stream was recreated and its sequence restarted).
    fn start_after(&self, resume_after: Option<u64>) -> (u64, bool) {
        let first = self.first_sequence();
        match resume_after {
            Some(after) if after + 1 >= first && after <= self.last_sequence => (after, true),
            _ => (first - 1, false),
        }
    }

    /// What a connection of `user`'s reading `communities` missed after `after`, and the
    /// communities it reads once it has caught up: memberships made or ended in between are
    /// applied in order as they are passed.
    fn catch_up(
        &self,
        user: UserId,
        communities: Vec<CommunityId>,
        after: u64,
    ) -> (Vec<Arc<FeedEvent>>, HashSet<CommunityId>) {
        let own = SubjectOwner::User(user);
        let mut reading: HashSet<CommunityId> = communities.into_iter().collect();
        let mut owners = reading.clone();
        owners.extend(
            self.since(own, after)
                .filter_map(|e| e.membership)
                .filter(|(_, joined)| *joined)
                .map(|(community, _)| community),
        );
        let mut events: Vec<Arc<FeedEvent>> = self.since(own, after).cloned().collect();
        for community in owners {
            events.extend(
                self.since(SubjectOwner::Community(community), after)
                    .cloned(),
            );
        }
        events.sort_unstable_by_key(|e| e.sequence);
        events.retain(|e| match e.owner {
            SubjectOwner::User(_) => {
                apply_membership(&mut reading, e.membership);
                true
            }
            SubjectOwner::Community(community) => reading.contains(&community),
        });
        (events, reading)
    }
}

fn apply_membership(reading: &mut HashSet<CommunityId>, change: Option<(CommunityId, bool)>) {
    match change {
        Some((community, true)) => {
            reading.insert(community);
        }
        Some((community, false)) => {
            reading.remove(&community);
        }
        None => {}
    }
}

struct Connection {
    user: UserId,
    communities: HashSet<CommunityId>,
    deliveries: mpsc::Sender<Delivery>,
}

/// Who reads what, on this server.
#[derive(Default)]
struct Routes {
    connections: HashMap<u64, Connection>,
    by_user: HashMap<UserId, Vec<u64>>,
    by_community: HashMap<CommunityId, Vec<u64>>,
}

impl Routes {
    fn add(&mut self, id: u64, connection: Connection) {
        self.by_user.entry(connection.user).or_default().push(id);
        for community in &connection.communities {
            self.by_community.entry(*community).or_default().push(id);
        }
        self.connections.insert(id, connection);
    }

    fn remove(&mut self, id: u64) {
        let Some(connection) = self.connections.remove(&id) else {
            return;
        };
        remove_from(&mut self.by_user, connection.user, id);
        for community in connection.communities {
            remove_from(&mut self.by_community, community, id);
        }
    }

    fn join(&mut self, id: u64, community: CommunityId) {
        if let Some(connection) = self.connections.get_mut(&id)
            && connection.communities.insert(community)
        {
            self.by_community.entry(community).or_default().push(id);
        }
    }

    fn leave(&mut self, id: u64, community: CommunityId) {
        if let Some(connection) = self.connections.get_mut(&id)
            && connection.communities.remove(&community)
        {
            remove_from(&mut self.by_community, community, id);
        }
    }

    /// Sends `event` to every connection that reads it and applies any membership change it
    /// carries. Returns the connections that must be dropped: those whose queue is full.
    fn route(&mut self, event: &Arc<FeedEvent>) -> Vec<u64> {
        let readers = match event.owner {
            SubjectOwner::User(user) => self.by_user.get(&user),
            SubjectOwner::Community(community) => self.by_community.get(&community),
        };
        let Some(readers) = readers else {
            return Vec::new();
        };
        let readers = readers.clone();
        let mut slow = Vec::new();
        for id in &readers {
            let Some(connection) = self.connections.get(id) else {
                continue;
            };
            match connection
                .deliveries
                .try_send(Delivery::Live(event.clone()))
            {
                Ok(()) => {}
                Err(mpsc::error::TrySendError::Full(_)) => slow.push(*id),
                // The connection is ending; its unregistration is on the way.
                Err(mpsc::error::TrySendError::Closed(_)) => {}
            }
        }
        if let Some((community, joined)) = event.membership {
            for id in readers {
                if joined {
                    self.join(id, community);
                } else {
                    self.leave(id, community);
                }
            }
        }
        slow
    }

    fn drop_all(&mut self, reason: &'static str) {
        let dropped = self.connections.len();
        if dropped > 0 {
            metrics::counter!(aspen_metrics::api::EVENT_STREAMS_DROPPED, "reason" => reason)
                .increment(dropped as u64);
        }
        *self = Self::default();
    }
}

/// What the dispatcher tells a routing shard, in stream order.
enum ShardCommand {
    Route(Arc<FeedEvent>),
    Add(u64, Connection),
    Remove(u64),
    DropAll(&'static str),
}

/// Sends a shard a command. A shard ends only with the server, so a failed send means it is
/// shutting down.
async fn tell(shard: &mpsc::Sender<ShardCommand>, command: ShardCommand) {
    let _ = shard.send(command).await;
}

/// One routing shard: the connections whose id falls to it, and the events they read.
async fn route_shard(mut commands: mpsc::Receiver<ShardCommand>) {
    let mut routes = Routes::default();
    while let Some(command) = commands.recv().await {
        match command {
            ShardCommand::Route(event) => {
                let started = Instant::now();
                for id in routes.route(&event) {
                    routes.remove(id);
                    metrics::counter!(aspen_metrics::api::EVENT_STREAMS_DROPPED, "reason" => "slow")
                        .increment(1);
                }
                metrics::histogram!(aspen_metrics::api::EVENT_ROUTE_DURATION)
                    .record(started.elapsed().as_secs_f64());
            }
            ShardCommand::Add(id, connection) => routes.add(id, connection),
            ShardCommand::Remove(id) => routes.remove(id),
            ShardCommand::DropAll(reason) => routes.drop_all(reason),
        }
    }
}

fn remove_from<K: std::hash::Hash + Eq>(map: &mut HashMap<K, Vec<u64>>, key: K, id: u64) {
    if let Some(ids) = map.get_mut(&key) {
        if let Some(i) = ids.iter().position(|x| *x == id) {
            ids.swap_remove(i);
        }
        if ids.is_empty() {
            map.remove(&key);
        }
    }
}

/// Opens the server's consumer, from `policy`. Returns it with how many events it has to
/// deliver before it reaches the end of the stream.
async fn open(
    context: &jetstream::Context,
    policy: DeliverPolicy,
) -> Result<(Ordered, u64), app::Error> {
    let stream = context.get_stream(ASPEN_NATS_STREAM_NAME).await?;
    let consumer = stream
        .create_consumer(OrderedConfig {
            deliver_policy: policy,
            replay_policy: ReplayPolicy::Instant,
            max_batch: 1000,
            max_bytes: 8 * 1024 * 1024,
            max_expires: Duration::from_secs(5),
            ..Default::default()
        })
        .await?;
    let pending = consumer.cached_info().num_pending;
    Ok((consumer.messages().await?, pending))
}

/// `open`, retried until NATS answers.
async fn reopen(context: &jetstream::Context, policy: DeliverPolicy) -> (Ordered, u64) {
    let mut wait = Duration::from_millis(250);
    loop {
        match open(context, policy).await {
            Ok(opened) => return opened,
            Err(e) => {
                error!("could not open the event feed's consumer, retrying: {e}");
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(5));
            }
        }
    }
}

/// Reads a delivered message into an event; `None` for one that is not an event.
fn read(message: &jetstream::Message) -> Option<(FeedEvent, u64)> {
    let info = match message.info() {
        Ok(info) => info,
        Err(e) => {
            error!("event stream message without metadata: {e}");
            return None;
        }
    };
    let Some(owner) = subject_owner(message.subject.as_str()) else {
        warn!(subject = %message.subject, "event on a subject with no owner");
        return None;
    };
    let payload = match std::str::from_utf8(&message.payload)
        .map_err(|e| e.to_string())
        .and_then(|s| RawValue::from_string(s.to_owned()).map_err(|e| e.to_string()))
    {
        Ok(payload) => payload,
        Err(e) => {
            error!("event stream message was not JSON text: {e}");
            return None;
        }
    };
    let membership = match owner {
        SubjectOwner::User(user) => membership_change(payload.get(), user),
        SubjectOwner::Community(_) => None,
    };
    let age = (time::OffsetDateTime::now_utc() - info.published).clamp(
        time::Duration::ZERO,
        time::Duration::try_from(MAX_EVENT_AGE).unwrap_or_default(),
    );
    let published = Instant::now()
        .checked_sub(age.try_into().unwrap_or_default())
        .unwrap_or_else(Instant::now);
    let event_id = message
        .headers
        .as_ref()
        .and_then(|headers| headers.get(app::events::EVENT_ID_HEADER))
        .map(|value| value.to_string());
    Some((
        FeedEvent {
            sequence: info.stream_sequence,
            event_id,
            payload,
            owner,
            membership,
            published,
        },
        info.pending,
    ))
}

async fn dispatch(
    context: jetstream::Context,
    shards: Vec<mpsc::Sender<ShardCommand>>,
    mut registrations: mpsc::Receiver<Register>,
    mut unregister: mpsc::UnboundedReceiver<u64>,
) {
    let window_start = || DeliverPolicy::ByStartTime {
        start_time: time::OffsetDateTime::now_utc() - MAX_EVENT_AGE,
    };
    let (mut messages, pending) = reopen(&context, window_start()).await;
    let mut retained = Retained::default();
    let shard = |id: u64| &shards[(id % shards.len() as u64) as usize];
    // Connections wait until the retained window is loaded, or the deadline passes.
    let mut loaded = pending == 0;
    let load_deadline = Instant::now() + LOAD_DEADLINE;
    if loaded {
        if let Ok(mut stream) = context.get_stream(ASPEN_NATS_STREAM_NAME).await
            && let Ok(stream_info) = stream.info().await
        {
            retained.last_sequence = stream_info.state.last_sequence;
        }
        info!("event feed ready");
    }
    let mut evict = tokio::time::interval(EVICT_INTERVAL);
    loop {
        tokio::select! {
            message = messages.next() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    failure => {
                        match failure {
                            Some(Err(e)) => error!("event feed consumer failed, reopening: {e}"),
                            _ => error!("event feed consumer ended, reopening"),
                        }
                        let policy = DeliverPolicy::ByStartSequence {
                            start_sequence: retained.last_sequence + 1,
                        };
                        (messages, _) = reopen(&context, policy).await;
                        continue;
                    }
                };
                let Some((event, pending)) = read(&message) else {
                    continue;
                };
                let last = retained.last_sequence;
                if last != 0 && event.sequence != last + 1 {
                    warn!(
                        last,
                        next = event.sequence,
                        "the event feed missed events; every connection must resume"
                    );
                    for to in &shards {
                        tell(to, ShardCommand::DropAll("gap")).await;
                    }
                    retained.clear();
                }
                let event = Arc::new(event);
                for to in &shards {
                    tell(to, ShardCommand::Route(event.clone())).await;
                }
                retained.push(event);
                if !loaded && pending == 0 {
                    loaded = true;
                    info!(retained = retained.order.len(), "event feed ready");
                }
            }
            Some(registration) = registrations.recv(), if loaded || Instant::now() >= load_deadline => {
                if !loaded {
                    warn!("event feed serving connections before its retained window loaded");
                    loaded = true;
                }
                let (after, resumed) = retained.start_after(registration.resume_after);
                let (missed, communities) =
                    retained.catch_up(registration.user, registration.communities, after);
                if !missed.is_empty()
                    && registration.deliveries.try_send(Delivery::CatchUp(missed)).is_err()
                {
                    continue;
                }
                if registration.resumed.send(resumed).is_err() {
                    continue;
                }
                let connection = Connection {
                    user: registration.user,
                    communities,
                    deliveries: registration.deliveries,
                };
                tell(shard(registration.id), ShardCommand::Add(registration.id, connection)).await;
            }
            Some(id) = unregister.recv() => tell(shard(id), ShardCommand::Remove(id)).await,
            _ = evict.tick() => {
                retained.evict(Instant::now());
                metrics::gauge!(aspen_metrics::api::EVENT_FEED_RETAINED)
                    .set(retained.order.len() as f64);
                metrics::gauge!(aspen_metrics::api::EVENT_FEED_RETAINED_BYTES)
                    .set(retained.bytes as f64);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(
        sequence: u64,
        owner: SubjectOwner,
        membership: Option<(CommunityId, bool)>,
    ) -> Arc<FeedEvent> {
        Arc::new(FeedEvent {
            sequence,
            event_id: None,
            payload: RawValue::from_string(format!("{{\"n\":{sequence}}}")).unwrap(),
            owner,
            membership,
            published: Instant::now(),
        })
    }

    fn sequences(events: &[Arc<FeedEvent>]) -> Vec<u64> {
        events.iter().map(|e| e.sequence).collect()
    }

    #[test]
    fn membership_changes_are_read_from_the_users_own_events() {
        let user = UserId::new();
        let community = CommunityId::new();
        let join = format!(
            r#"{{"serverEvent":"userCommunity","type":"create","user":"{}","community":"{}"}}"#,
            user.0, community.0
        );
        assert_eq!(membership_change(&join, user), Some((community, true)));
        let leave = join.replace("create", "delete");
        assert_eq!(membership_change(&leave, user), Some((community, false)));
        assert_eq!(membership_change(&join, UserId::new()), None);
        assert_eq!(
            membership_change(r#"{"serverEvent":"message","type":"create"}"#, user),
            None
        );
    }

    #[test]
    fn catch_up_follows_memberships_made_and_ended_along_the_way() {
        let user = UserId::new();
        let (kept, joined, left, other) = (
            CommunityId::new(),
            CommunityId::new(),
            CommunityId::new(),
            CommunityId::new(),
        );
        let own = SubjectOwner::User(user);
        let mut retained = Retained::default();
        for e in [
            event(1, SubjectOwner::Community(joined), None),
            event(2, SubjectOwner::Community(kept), None),
            event(3, own, Some((joined, true))),
            event(4, SubjectOwner::Community(joined), None),
            event(5, SubjectOwner::Community(left), None),
            event(6, own, Some((left, false))),
            event(7, SubjectOwner::Community(left), None),
            event(8, SubjectOwner::Community(other), None),
            event(9, SubjectOwner::User(UserId::new()), None),
        ] {
            retained.push(e);
        }
        // The database, read at connect, still showed `left` and did not yet show `joined`.
        let (missed, reading) = retained.catch_up(user, vec![kept, left], 0);
        assert_eq!(sequences(&missed), vec![2, 3, 4, 5, 6]);
        assert_eq!(reading, HashSet::from([kept, joined]));
        let (missed, _) = retained.catch_up(user, vec![kept, left], 4);
        assert_eq!(sequences(&missed), vec![5, 6]);
    }

    #[test]
    fn resuming_needs_the_next_event_to_be_retained() {
        let owner = SubjectOwner::User(UserId::new());
        let mut retained = Retained::default();
        for sequence in 10..=20 {
            retained.push(event(sequence, owner, None));
        }
        assert_eq!(retained.start_after(Some(9)), (9, true));
        assert_eq!(retained.start_after(Some(20)), (20, true));
        assert_eq!(retained.start_after(Some(8)), (9, false));
        assert_eq!(retained.start_after(Some(21)), (9, false));
        assert_eq!(retained.start_after(None), (9, false));
        retained.evict(Instant::now() + MAX_EVENT_AGE);
        assert_eq!(retained.first_sequence(), 21);
        assert_eq!(retained.bytes, 0);
        assert_eq!(retained.start_after(Some(20)), (20, true));
        assert_eq!(retained.start_after(Some(19)), (20, false));
    }

    #[test]
    fn routing_drops_full_queues_and_follows_memberships() {
        let user = UserId::new();
        let community = CommunityId::new();
        let mut routes = Routes::default();
        let (tx, mut rx) = mpsc::channel(1);
        routes.add(
            1,
            Connection {
                user,
                communities: HashSet::new(),
                deliveries: tx,
            },
        );
        let in_community = event(2, SubjectOwner::Community(community), None);
        assert!(routes.route(&in_community).is_empty());
        assert!(rx.try_recv().is_err());
        assert!(
            routes
                .route(&event(1, SubjectOwner::User(user), Some((community, true))))
                .is_empty()
        );
        assert!(matches!(rx.try_recv(), Ok(Delivery::Live(_))));
        assert!(routes.route(&in_community).is_empty());
        assert_eq!(routes.route(&in_community), vec![1]);
        routes.remove(1);
        assert!(routes.by_user.is_empty() && routes.by_community.is_empty());
    }
}
