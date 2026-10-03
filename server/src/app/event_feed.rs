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
//! What happens in a community channel, and the channel's own events, reach only those who may
//! view it. Publishers name the channel that decides it in the `Aspen-Channel` header (a
//! thread's parent), and the dispatcher keeps a `CommunityModel` of every community its
//! connections read, applying each change in stream order and attaching the model as it then
//! stands to every event of the community it routes. A shard decides each reader's view from
//! that attached model and the roles the reader holds, which it follows from their own
//! `userCommunity` events as it follows their memberships. The decision for an event is
//! therefore the one the permissions of that moment in the stream give, whichever shard makes
//! it and however late. A model is loaded with the connection that first needs it, and dropped
//! with the last; a connection that joins a community no local connection reads is dropped so
//! that it resumes, loading the model as it registers again.
//!
//! A connection's queue is bounded (`event_queue_size`). One that fills is dropped, and its
//! client resumes from the last event it processed. If the feed itself misses events (the
//! stream's sequence jumps, or restarts after NATS lost it), every local connection is dropped,
//! and each learns from `resumed: false` that it must rebuild its state.

use crate::app::context::GlobalServerContext;
use crate::app::events::{
    CHANNEL_HEADER, CREATOR_HEADER, REQUIRES_HEADER, SubjectOwner, memberships, subject_owner,
};
use crate::app::permissions::{Permission, Permissions};
use crate::app::visibility::{CommunityModel, ModelChange, member_roles};
use crate::app::{self, ASPEN_NATS_STREAM_NAME, ChannelId, CommunityId, RoleId, UserId};
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
    /// On a user's own subject, the roles they now hold in a community besides everyone's.
    roles: Option<(CommunityId, Vec<RoleId>)>,
    /// On a user's own subject, whether they now moderate the deployment, and so view every
    /// channel of the communities they read.
    moderator: Option<bool>,
    /// The community channel whose View channel permission decides who receives the event.
    channel: Option<ChannelId>,
    /// A community permission the event needs besides membership (Manage invites for an
    /// invite's), and the member who receives it without that permission (the invite's creator).
    requires: Option<Permissions>,
    creator: Option<UserId>,
    /// On a community's subject, the change the event makes to who may view what.
    change: Option<ModelChange>,
    /// The community's model as it stood after this event, when the dispatcher held one.
    access: Option<Arc<CommunityModel>>,
    /// When it was published, on this process's clock.
    published: Instant,
    /// On a user's own subject, that they were banned from the deployment: their connections
    /// end once it is written to them.
    ends_streams: bool,
}

impl FeedEvent {
    /// Whether a connection that delivers this event closes after it: its user was banned from
    /// the deployment (`app::user_ban`).
    pub fn ends_streams(&self) -> bool {
        self.ends_streams
    }
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
    roles: HashMap<CommunityId, Vec<RoleId>>,
    moderator: bool,
    /// Models of the communities, as the database had them, for those the dispatcher lacks.
    models: HashMap<CommunityId, CommunityModel>,
    resume_after: Option<u64>,
    deliveries: mpsc::Sender<Delivery>,
    /// Whether `resume_after` was honoured; or the communities the connection reads once
    /// caught up whose models it must load before registering again.
    outcome: oneshot::Sender<Result<bool, Vec<CommunityId>>>,
}

/// How many times a connection loads models it turns out to need before giving up. Each retry
/// is for a community joined in the moments since the last.
const REGISTER_ATTEMPTS: usize = 3;

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
    let mut conn = state.connection_pool.get().await?;
    let communities = memberships(conn.as_mut(), user).await?;
    let roles = member_roles(conn.as_mut(), user, &communities).await?;
    let mut models = CommunityModel::load(conn.as_mut(), &communities).await?;
    let moderator = app::deployment::is_moderator(conn.as_mut(), user).await?;
    let feed = &state.event_feed;
    let id = feed.next_id.fetch_add(1, Ordering::Relaxed);
    let (deliveries_tx, deliveries) = mpsc::channel(feed.queue_size);
    for _ in 0..REGISTER_ATTEMPTS {
        let (outcome_tx, outcome_rx) = oneshot::channel();
        feed.registrations
            .send(Register {
                id,
                user,
                communities: communities.clone(),
                roles: roles.clone(),
                moderator,
                models: std::mem::take(&mut models),
                resume_after,
                deliveries: deliveries_tx.clone(),
                outcome: outcome_tx,
            })
            .await
            .map_err(|_| app::Error::EventFeedStopped)?;
        match outcome_rx.await.map_err(|_| app::Error::EventFeedStopped)? {
            Ok(resumed) => {
                return Ok(Subscription {
                    resumed,
                    deliveries,
                    _registration: Registration {
                        id,
                        unregister: feed.unregister.clone(),
                    },
                });
            }
            Err(missing) => models = CommunityModel::load(conn.as_mut(), &missing).await?,
        }
    }
    Err(app::Error::EventFeedStopped)
}

/// What an event on `user`'s own subject changes about their memberships, read without parsing
/// the rest of it: a membership made or ended (the community, and whether it was joined), and
/// the roles they now hold in it besides everyone's.
#[allow(clippy::type_complexity)]
fn own_membership(
    payload: &str,
    user: UserId,
) -> (
    Option<(CommunityId, bool)>,
    Option<(CommunityId, Vec<RoleId>)>,
) {
    if !payload.contains("\"userCommunity\"") {
        return (None, None);
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Glance {
        server_event: String,
        #[serde(rename = "type")]
        kind: Option<String>,
        user: Option<UserId>,
        community: Option<CommunityId>,
        roles: Option<Vec<RoleId>>,
    }
    let Ok(glance) = serde_json::from_str::<Glance>(payload) else {
        return (None, None);
    };
    if glance.server_event != "userCommunity" || glance.user != Some(user) {
        return (None, None);
    }
    let Some(community) = glance.community else {
        return (None, None);
    };
    let membership = match glance.kind.as_deref() {
        Some("create") => Some((community, true)),
        Some("delete") => Some((community, false)),
        _ => None,
    };
    (membership, glance.roles.map(|roles| (community, roles)))
}

/// Whether an event on a user's own subject says they now moderate the deployment, or no
/// longer do, read without parsing anything else.
fn moderation_change(payload: &str) -> Option<bool> {
    if !payload.contains(r#""serverEvent":"deploymentAccessChanged""#) {
        return None;
    }
    #[derive(Deserialize)]
    struct Glance {
        permissions: Vec<crate::app::deployment::DeploymentPermission>,
    }
    let glance: Glance = serde_json::from_str(payload).ok()?;
    Some(
        glance
            .permissions
            .contains(&crate::app::deployment::DeploymentPermission::ModerateCommunities),
    )
}

/// Whether `event`, of a community's, may reach `user` holding `roles` there: they hold the
/// permission it requires, if any, or made what it is about; and it names no channel, or they
/// may view the one it names, by `model`. A deployment moderator reads everything. Anything
/// restricted reaches no one without a model.
fn may_read(
    event: &FeedEvent,
    model: Option<&CommunityModel>,
    user: UserId,
    roles: Option<&Vec<RoleId>>,
    moderator: bool,
) -> bool {
    if moderator || (event.requires.is_none() && event.channel.is_none()) {
        return true;
    }
    let none = Vec::new();
    let roles = roles.unwrap_or(&none);
    let Some(model) = model else {
        return false;
    };
    if let Some(required) = event.requires
        && event.creator != Some(user)
        && !model.holds(user, roles, required)
    {
        return false;
    }
    event
        .channel
        .is_none_or(|channel| model.can_view(user, roles, channel))
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

    /// What a connection of `user`'s reading `communities` and holding `roles` missed after
    /// `after`, and the communities it reads and roles it holds once it has caught up.
    /// The database was read at some moment in the retained window, possibly before changes
    /// published just ahead of it were committed, so every membership and role change of the
    /// user's retained up to `after` is applied first (each sets a value, so applying one the
    /// database already shows changes nothing), and those after it as they are passed. Channel
    /// events are kept by the model attached to each, or `models`' for one routed before the
    /// dispatcher held its community's.
    #[allow(clippy::type_complexity)]
    fn catch_up(
        &self,
        user: UserId,
        communities: Vec<CommunityId>,
        mut roles: HashMap<CommunityId, Vec<RoleId>>,
        mut moderator: bool,
        after: u64,
        models: &HashMap<CommunityId, Arc<CommunityModel>>,
    ) -> (
        Vec<Arc<FeedEvent>>,
        HashSet<CommunityId>,
        HashMap<CommunityId, Vec<RoleId>>,
        bool,
    ) {
        let own = SubjectOwner::User(user);
        let mut reading: HashSet<CommunityId> = communities.into_iter().collect();
        for e in self.since(own, 0).take_while(|e| e.sequence <= after) {
            apply_membership(&mut reading, &mut roles, e);
            moderator = e.moderator.unwrap_or(moderator);
        }
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
                apply_membership(&mut reading, &mut roles, e);
                moderator = e.moderator.unwrap_or(moderator);
                true
            }
            SubjectOwner::Community(community) => {
                reading.contains(&community)
                    && may_read(
                        e,
                        e.access
                            .as_deref()
                            .or_else(|| models.get(&community).map(Arc::as_ref)),
                        user,
                        roles.get(&community),
                        moderator,
                    )
            }
        });
        (events, reading, roles, moderator)
    }
}

/// Applies what an event on the user's own subject changes about their memberships and roles.
fn apply_membership(
    reading: &mut HashSet<CommunityId>,
    roles: &mut HashMap<CommunityId, Vec<RoleId>>,
    event: &FeedEvent,
) {
    match event.membership {
        Some((community, true)) => {
            reading.insert(community);
        }
        Some((community, false)) => {
            reading.remove(&community);
            roles.remove(&community);
        }
        None => {}
    }
    if let Some((community, held)) = &event.roles {
        roles.insert(*community, held.clone());
    }
}

struct Connection {
    user: UserId,
    communities: HashSet<CommunityId>,
    /// The roles the user holds in each community besides everyone's.
    roles: HashMap<CommunityId, Vec<RoleId>>,
    /// Whether the user moderates the deployment.
    moderator: bool,
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

    /// Sends `event` to every connection that reads it and may view the channel it names, and
    /// applies any membership or role change it carries. Returns the connections that must be
    /// dropped: those whose queue is full.
    fn route(&mut self, event: &Arc<FeedEvent>) -> Vec<u64> {
        let readers = match event.owner {
            SubjectOwner::User(user) => self.by_user.get(&user),
            SubjectOwner::Community(community) => self.by_community.get(&community),
        };
        let Some(readers) = readers else {
            return Vec::new();
        };
        let readers = readers.clone();
        // Readers holding the same roles see the same; each set is decided once, apart from the
        // one who made what the event is about.
        let mut decided: HashMap<(bool, bool, Vec<RoleId>), bool> = HashMap::new();
        let mut slow = Vec::new();
        for id in &readers {
            let Some(connection) = self.connections.get(id) else {
                continue;
            };
            if let SubjectOwner::Community(community) = event.owner
                && (event.channel.is_some() || event.requires.is_some())
            {
                let model = event.access.as_deref();
                let roles = connection.roles.get(&community);
                let key = (
                    connection.moderator || model.is_some_and(|m| m.is_owner(connection.user)),
                    event.creator == Some(connection.user),
                    roles.cloned().unwrap_or_default(),
                );
                let visible = *decided.entry(key).or_insert_with(|| {
                    may_read(event, model, connection.user, roles, connection.moderator)
                });
                if !visible {
                    continue;
                }
            }
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
        if let SubjectOwner::User(_) = event.owner {
            for id in readers {
                if let Some((community, joined)) = event.membership {
                    if joined {
                        self.join(id, community);
                    } else {
                        self.leave(id, community);
                    }
                }
                if let Some(connection) = self.connections.get_mut(&id) {
                    if let Some((community, false)) = event.membership {
                        connection.roles.remove(&community);
                    }
                    if let Some((community, held)) = &event.roles {
                        connection.roles.insert(*community, held.clone());
                    }
                    if let Some(moderator) = event.moderator {
                        connection.moderator = moderator;
                    }
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
    /// Drops one connection, which then resumes: it joined a community whose model this
    /// server does not hold.
    Resync(u64),
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
            ShardCommand::Resync(id) => {
                routes.remove(id);
                metrics::counter!(aspen_metrics::api::EVENT_STREAMS_DROPPED, "reason" => "resync")
                    .increment(1);
            }
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
    let (membership, roles, moderator, change) = match owner {
        SubjectOwner::User(user) => {
            let (membership, roles) = own_membership(payload.get(), user);
            (membership, roles, moderation_change(payload.get()), None)
        }
        SubjectOwner::Community(_) => (None, None, None, ModelChange::read(payload.get())),
    };
    let ends_streams = matches!(owner, SubjectOwner::User(_))
        && payload.get().contains(r#""serverEvent":"accountBanned""#);
    let channel = message
        .headers
        .as_ref()
        .and_then(|headers| headers.get(CHANNEL_HEADER))
        .and_then(|value| value.as_str().parse().ok())
        .map(ChannelId);
    let header = |name: &str| {
        message
            .headers
            .as_ref()
            .and_then(|headers| headers.get(name))
            .map(|value| value.as_str().to_string())
    };
    let requires = header(REQUIRES_HEADER)
        .and_then(|name| name.parse::<Permission>().ok())
        .map(Permission::bits);
    let creator = header(CREATOR_HEADER)
        .and_then(|id| id.parse().ok())
        .map(UserId);
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
            roles,
            moderator,
            channel,
            requires,
            creator,
            change,
            access: None,
            published,
            ends_streams,
        },
        info.pending,
    ))
}

/// The dispatcher's account of which communities its connections read, so that it holds a
/// model for each and only those.
#[derive(Default)]
struct Models {
    models: HashMap<CommunityId, Arc<CommunityModel>>,
    /// How many registered connections read each community.
    readers: HashMap<CommunityId, usize>,
    /// Each registered connection's user and the communities it reads.
    connections: HashMap<u64, (UserId, HashSet<CommunityId>)>,
    by_user: HashMap<UserId, Vec<u64>>,
}

impl Models {
    /// Takes `snapshot` as `community`'s model unless one is held already, bringing it up to
    /// date with every retained event of the community.
    fn adopt(&mut self, retained: &Retained, community: CommunityId, snapshot: CommunityModel) {
        if self.models.contains_key(&community) {
            return;
        }
        let mut model = snapshot;
        for e in retained.since(SubjectOwner::Community(community), 0) {
            if let Some(change) = &e.change {
                model.apply(change);
            }
        }
        self.models.insert(community, Arc::new(model));
    }

    /// Lets go of the models no registered connection reads.
    fn release_unread(&mut self) {
        let readers = &self.readers;
        self.models
            .retain(|community, _| readers.get(community).is_some_and(|n| *n > 0));
    }

    fn read(&mut self, community: CommunityId) {
        *self.readers.entry(community).or_default() += 1;
    }

    fn unread(&mut self, community: CommunityId) {
        if let Some(n) = self.readers.get_mut(&community) {
            *n -= 1;
            if *n == 0 {
                self.readers.remove(&community);
                self.models.remove(&community);
            }
        }
    }

    fn register(&mut self, id: u64, user: UserId, communities: HashSet<CommunityId>) {
        for community in &communities {
            self.read(*community);
        }
        self.by_user.entry(user).or_default().push(id);
        self.connections.insert(id, (user, communities));
    }

    fn unregister(&mut self, id: u64) {
        let Some((user, communities)) = self.connections.remove(&id) else {
            return;
        };
        remove_from(&mut self.by_user, user, id);
        for community in communities {
            self.unread(community);
        }
    }

    /// Applies `event`'s change to its community's model and attaches the model as it now
    /// stands; follows the memberships it makes or ends. Returns the connections that joined
    /// a community with no model here, which must resume.
    fn follow(&mut self, event: &mut FeedEvent) -> Vec<u64> {
        match event.owner {
            SubjectOwner::Community(community) => {
                if let Some(model) = self.models.get_mut(&community) {
                    if let Some(change) = &event.change {
                        Arc::make_mut(model).apply(change);
                    }
                    event.access = Some(model.clone());
                }
                Vec::new()
            }
            SubjectOwner::User(user) => {
                let Some((community, joined)) = event.membership else {
                    return Vec::new();
                };
                let ids = self.by_user.get(&user).cloned().unwrap_or_default();
                let mut resync = Vec::new();
                for id in ids {
                    let Some((_, communities)) = self.connections.get_mut(&id) else {
                        continue;
                    };
                    if !joined {
                        if communities.remove(&community) {
                            self.unread(community);
                        }
                    } else if self.models.contains_key(&community) {
                        if communities.insert(community) {
                            self.read(community);
                        }
                    } else {
                        resync.push(id);
                    }
                }
                for id in &resync {
                    self.unregister(*id);
                }
                resync
            }
        }
    }
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
    let mut models = Models::default();
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
                let Some((mut event, pending)) = read(&message) else {
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
                    // The models missed the changes too; every connection registers afresh.
                    models = Models::default();
                }
                let resync = models.follow(&mut event);
                let event = Arc::new(event);
                for to in &shards {
                    tell(to, ShardCommand::Route(event.clone())).await;
                }
                for id in resync {
                    tell(shard(id), ShardCommand::Resync(id)).await;
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
                for (community, snapshot) in registration.models {
                    models.adopt(&retained, community, snapshot);
                }
                let (after, resumed) = retained.start_after(registration.resume_after);
                let (missed, communities, roles, moderator) = retained.catch_up(
                    registration.user,
                    registration.communities,
                    registration.roles,
                    registration.moderator,
                    after,
                    &models.models,
                );
                let missing: Vec<CommunityId> = communities
                    .iter()
                    .filter(|c| !models.models.contains_key(c))
                    .copied()
                    .collect();
                if !missing.is_empty() {
                    models.release_unread();
                    let _ = registration.outcome.send(Err(missing));
                    continue;
                }
                if !missed.is_empty()
                    && registration.deliveries.try_send(Delivery::CatchUp(missed)).is_err()
                {
                    models.release_unread();
                    continue;
                }
                if registration.outcome.send(Ok(resumed)).is_err() {
                    models.release_unread();
                    continue;
                }
                models.register(registration.id, registration.user, communities.clone());
                let connection = Connection {
                    user: registration.user,
                    communities,
                    roles,
                    moderator,
                    deliveries: registration.deliveries,
                };
                tell(shard(registration.id), ShardCommand::Add(registration.id, connection)).await;
            }
            Some(id) = unregister.recv() => {
                models.unregister(id);
                tell(shard(id), ShardCommand::Remove(id)).await;
            }
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
        Arc::new(plain(sequence, owner, membership))
    }

    /// An event to adjust before it is routed.
    fn plain(
        sequence: u64,
        owner: SubjectOwner,
        membership: Option<(CommunityId, bool)>,
    ) -> FeedEvent {
        FeedEvent {
            sequence,
            event_id: None,
            payload: RawValue::from_string(format!("{{\"n\":{sequence}}}")).unwrap(),
            owner,
            membership,
            roles: None,
            moderator: None,
            channel: None,
            requires: None,
            creator: None,
            change: None,
            access: None,
            published: Instant::now(),
            ends_streams: false,
        }
    }

    /// An event in `channel` of a community, routed with `model` attached.
    fn in_channel(
        sequence: u64,
        community: CommunityId,
        channel: ChannelId,
        model: &Arc<CommunityModel>,
    ) -> Arc<FeedEvent> {
        let mut e = plain(sequence, SubjectOwner::Community(community), None);
        e.channel = Some(channel);
        e.access = Some(model.clone());
        Arc::new(e)
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
        assert_eq!(own_membership(&join, user), (Some((community, true)), None));
        let leave = join.replace("create", "delete");
        assert_eq!(
            own_membership(&leave, user),
            (Some((community, false)), None)
        );
        assert_eq!(own_membership(&join, UserId::new()), (None, None));
        assert_eq!(
            moderation_change(
                r#"{"serverEvent":"deploymentAccessChanged","permissions":["viewDashboard","moderateCommunities"]}"#
            ),
            Some(true)
        );
        assert_eq!(
            moderation_change(r#"{"serverEvent":"deploymentAccessChanged","permissions":[]}"#),
            Some(false)
        );
        assert_eq!(
            own_membership(r#"{"serverEvent":"message","type":"create"}"#, user),
            (None, None)
        );
        let role = RoleId::new();
        let promoted = format!(
            r#"{{"serverEvent":"userCommunity","type":"update","user":"{}","community":"{}","roles":["{}"]}}"#,
            user.0, community.0, role.0
        );
        assert_eq!(
            own_membership(&promoted, user),
            (None, Some((community, vec![role])))
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
        let none = HashMap::new();
        let (missed, reading, _, _) =
            retained.catch_up(user, vec![kept, left], HashMap::new(), false, 0, &none);
        assert_eq!(sequences(&missed), vec![2, 3, 4, 5, 6]);
        assert_eq!(reading, HashSet::from([kept, joined]));
        // Resuming after the join, with a database read from before it was committed, still
        // reads the community joined.
        let (missed, reading, _, _) =
            retained.catch_up(user, vec![kept, left], HashMap::new(), false, 4, &none);
        assert_eq!(sequences(&missed), vec![5, 6]);
        assert_eq!(reading, HashSet::from([kept, joined]));
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
                roles: HashMap::new(),
                moderator: false,
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

    /// A model where everyone may view `open` and only `moderator` may view `hidden`.
    fn two_channels(
        community: CommunityId,
        open: ChannelId,
        hidden: ChannelId,
        moderator: RoleId,
    ) -> CommunityModel {
        use crate::app::permissions::Permissions;
        let everyone = RoleId::new();
        let mut model = CommunityModel::new(community);
        for change in [
            ModelChange::Role {
                id: everyone,
                permissions: Some(Permissions::MEMBER_TEMPLATE),
                everyone: Some(true),
            },
            ModelChange::Role {
                id: moderator,
                permissions: Some(Permissions::empty()),
                everyone: Some(false),
            },
            ModelChange::ChannelCategory {
                channel: open,
                category: None,
            },
            ModelChange::ChannelCategory {
                channel: hidden,
                category: None,
            },
            ModelChange::ChannelOverride {
                channel: hidden,
                role: everyone,
                set: Some((Permissions::empty(), Permissions::VIEW_CHANNEL)),
            },
            ModelChange::ChannelOverride {
                channel: hidden,
                role: moderator,
                set: Some((Permissions::VIEW_CHANNEL, Permissions::empty())),
            },
        ] {
            model.apply(&change);
        }
        model
    }

    #[test]
    fn routing_leaves_out_channels_a_reader_may_not_view() {
        let (member, moderator_user) = (UserId::new(), UserId::new());
        let community = CommunityId::new();
        let (open, hidden, moderator) = (ChannelId::new(), ChannelId::new(), RoleId::new());
        let model = Arc::new(two_channels(community, open, hidden, moderator));
        let mut routes = Routes::default();
        let (member_tx, mut member_rx) = mpsc::channel(8);
        let (moderator_tx, mut moderator_rx) = mpsc::channel(8);
        routes.add(
            1,
            Connection {
                user: member,
                communities: HashSet::from([community]),
                roles: HashMap::new(),
                moderator: false,
                deliveries: member_tx,
            },
        );
        routes.add(
            2,
            Connection {
                user: moderator_user,
                communities: HashSet::from([community]),
                roles: HashMap::from([(community, vec![moderator])]),
                moderator: false,
                deliveries: moderator_tx,
            },
        );
        routes.route(&in_channel(1, community, open, &model));
        routes.route(&in_channel(2, community, hidden, &model));
        let received = |rx: &mut mpsc::Receiver<Delivery>| {
            let mut sequences = Vec::new();
            while let Ok(Delivery::Live(e)) = rx.try_recv() {
                sequences.push(e.sequence);
            }
            sequences
        };
        assert_eq!(received(&mut member_rx), vec![1]);
        assert_eq!(received(&mut moderator_rx), vec![1, 2]);
        // Losing the role, by their own event, ends what it showed them.
        let mut demoted = plain(3, SubjectOwner::User(moderator_user), None);
        demoted.roles = Some((community, Vec::new()));
        routes.route(&Arc::new(demoted));
        routes.route(&in_channel(4, community, hidden, &model));
        assert_eq!(received(&mut moderator_rx), vec![3]);
        // Moderating the deployment, by their own event, shows every channel whatever the
        // roles.
        let mut promoted = plain(5, SubjectOwner::User(moderator_user), None);
        promoted.moderator = Some(true);
        routes.route(&Arc::new(promoted));
        routes.route(&in_channel(6, community, hidden, &model));
        assert_eq!(received(&mut moderator_rx), vec![5, 6]);
    }

    #[test]
    fn the_dispatcher_attaches_each_events_model_and_resyncs_unmodelled_joins() {
        let user = UserId::new();
        let (community, unmodelled) = (CommunityId::new(), CommunityId::new());
        let (open, hidden, moderator) = (ChannelId::new(), ChannelId::new(), RoleId::new());
        let retained = Retained::default();
        let mut models = Models::default();
        models.adopt(
            &retained,
            community,
            two_channels(community, open, hidden, moderator),
        );
        models.register(7, user, HashSet::from([community]));
        // Showing the hidden channel to everyone changes the model the next event carries.
        let everyone_view = ModelChange::ChannelOverride {
            channel: hidden,
            role: moderator,
            set: None,
        };
        let mut change = plain(1, SubjectOwner::Community(community), None);
        change.change = Some(everyone_view);
        assert!(models.follow(&mut change).is_empty());
        let attached = change.access.clone().expect("a model");
        assert!(!attached.can_view(UserId::new(), &[moderator], hidden));
        let mut join = plain(2, SubjectOwner::User(user), Some((unmodelled, true)));
        assert_eq!(models.follow(&mut join), vec![7]);
        // The resynced connection no longer counts as a reader, so its model goes.
        assert!(models.models.is_empty());
    }

    #[test]
    fn invite_events_reach_their_creator_and_whoever_manages_invites() {
        use crate::app::permissions::Permissions;
        let community = CommunityId::new();
        let (everyone, keeper) = (RoleId::new(), RoleId::new());
        let mut model = CommunityModel::new(community);
        for change in [
            ModelChange::Role {
                id: everyone,
                permissions: Some(Permissions::MEMBER_TEMPLATE),
                everyone: Some(true),
            },
            ModelChange::Role {
                id: keeper,
                permissions: Some(Permissions::MANAGE_INVITES),
                everyone: Some(false),
            },
        ] {
            model.apply(&change);
        }
        let model = Arc::new(model);
        let (creator, manager, bystander) = (UserId::new(), UserId::new(), UserId::new());
        let mut routes = Routes::default();
        let mut receivers = Vec::new();
        for (id, user, roles) in [
            (1, creator, vec![]),
            (2, manager, vec![keeper]),
            (3, bystander, vec![]),
        ] {
            let (tx, rx) = mpsc::channel(8);
            routes.add(
                id,
                Connection {
                    user,
                    communities: HashSet::from([community]),
                    roles: HashMap::from([(community, roles)]),
                    moderator: false,
                    deliveries: tx,
                },
            );
            receivers.push(rx);
        }
        let mut invite = plain(1, SubjectOwner::Community(community), None);
        invite.requires = Some(Permissions::MANAGE_INVITES);
        invite.creator = Some(creator);
        invite.access = Some(model);
        routes.route(&Arc::new(invite));
        let got: Vec<bool> = receivers
            .iter_mut()
            .map(|rx| rx.try_recv().is_ok())
            .collect();
        assert_eq!(got, vec![true, true, false]);
    }
}
