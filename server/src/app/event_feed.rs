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
    CATEGORY_HEADER, CHANNEL_HEADER, CREATOR_HEADER, REQUIRES_HEADER, SubjectOwner, memberships,
    subject_owner,
};
use crate::app::permissions::{Permission, Permissions};
use crate::app::visibility::{CommunityModel, ModelChange, member_roles};
use crate::app::{
    self, ASPEN_NATS_STREAM_NAME, CategoryId, ChannelId, CommunityId, RoleId, UserId,
};
use async_nats::jetstream;
use async_nats::jetstream::consumer::pull::{Ordered, OrderedConfig};
use async_nats::jetstream::consumer::{DeliverPolicy, ReplayPolicy};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::value::RawValue;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
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
    /// The category whose own overrides decide who receives the event (`CATEGORY_HEADER`).
    category: Option<CategoryId>,
    /// A community permission the event needs besides membership (Manage invites for an
    /// invite's), and the member who receives it without that permission (the invite's creator).
    requires: Option<Permissions>,
    creator: Option<UserId>,
    /// On a community's subject, the change the event makes to who may view what.
    change: Option<ModelChange>,
    /// The community's model as it stood after this event, when the dispatcher held one. Set
    /// as the event is followed, or for one retained before the dispatcher held the model, as
    /// the model is adopted.
    access: OnceLock<Arc<CommunityModel>>,
    /// For an event that changes who may view the channel or category it names, the model as it
    /// stood before: those who could view it then receive it too, so they learn they no longer
    /// can.
    before: OnceLock<Arc<CommunityModel>>,
    /// When it was published, on this process's clock.
    published: Instant,
    /// On a user's own subject, that the connections receiving it end once it is written to
    /// them, and why.
    ends: Option<StreamEnd>,
    /// On a user's own subject, the sign-ins of theirs that ended: only their connections
    /// receive the event.
    sign_ins: Option<EndedSignIns>,
    /// That what was announced on its subject may not have happened (`app::events::settle`):
    /// on a community's, the model is dropped and its readers resume; on a user's, their
    /// connections resume.
    resync: bool,
    /// On a user's own subject, that their email account changed (`app::email`): whether it now
    /// holds an address it has not verified, which a connection checks against the deployment's
    /// settings.
    email_unverified: Option<bool>,
}

impl FeedEvent {
    /// Whether a connection that delivers this event closes after it, and why.
    pub fn ends(&self) -> Option<StreamEnd> {
        self.ends
    }

    /// When it tells its user that their email account changed, whether the account now holds
    /// an address it has not verified.
    pub fn email_unverified(&self) -> Option<bool> {
        self.email_unverified
    }

    /// Whether a connection of `sign_in` receives this event: every connection of its user does,
    /// except that an end of sign-ins reaches only theirs.
    fn reaches(&self, sign_in: &str) -> bool {
        self.sign_ins
            .as_ref()
            .is_none_or(|ended| ended.covers(sign_in))
    }
}

/// Why a connection closes after an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    /// Its user was banned from the deployment (`app::user_ban`).
    Banned,
    /// Its sign-in ended (`app::login`): signed out, or revoked.
    SignedOut,
}

/// The sign-ins a `signInsEnded` event ends, by `app::login::sign_in_id`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct EndedSignIns {
    /// The one sign-in that ended; without it, every one but `kept` did.
    ended: Option<String>,
    kept: Option<String>,
}

impl EndedSignIns {
    fn covers(&self, sign_in: &str) -> bool {
        match &self.ended {
            Some(ended) => ended == sign_in,
            None => self.kept.as_deref() != Some(sign_in),
        }
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
    /// How many streams each user and each client address hold open here, against the caps.
    pub caps: StreamCaps,
}

/// How many event streams each user and each client address hold open on this server, and the
/// most either may (`[limits] max_event_streams_per_user` and `max_event_streams_per_address`).
/// Counted in this process alone: a count kept elsewhere would outlive a server that stopped
/// without giving its streams back, while this one cannot, and a client holding streams on every
/// API server is still bounded by their number.
#[derive(Clone)]
pub struct StreamCaps {
    counts: Arc<Mutex<StreamCounts>>,
    per_user: usize,
    per_address: usize,
}

#[derive(Default)]
struct StreamCounts {
    users: HashMap<UserId, usize>,
    addresses: HashMap<String, usize>,
}

#[derive(Clone, PartialEq, Eq)]
enum StreamHolder {
    User(UserId),
    Address(String),
}

/// One stream's place under a cap, given back when dropped.
pub struct StreamHold {
    counts: Arc<Mutex<StreamCounts>>,
    holder: StreamHolder,
}

impl StreamCaps {
    pub fn new(limits: &crate::aspen_config::LimitsConfig) -> Self {
        Self {
            counts: Arc::default(),
            per_user: limits.max_event_streams_per_user.max(1),
            per_address: limits.max_event_streams_per_address.max(1),
        }
    }

    /// A place for one more stream of `user`'s, or `None` when they hold as many as they may.
    pub fn hold_user(&self, user: UserId) -> Option<StreamHold> {
        self.hold(StreamHolder::User(user), self.per_user)
    }

    /// A place for one more stream from the client address `address` (as the rate limits count
    /// it, `aspen_limits::ClientAddresses::key`), or `None` when it holds as many as it may.
    pub fn hold_address(&self, address: String) -> Option<StreamHold> {
        self.hold(StreamHolder::Address(address), self.per_address)
    }

    fn hold(&self, holder: StreamHolder, cap: usize) -> Option<StreamHold> {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        let count = match &holder {
            StreamHolder::User(user) => counts.users.entry(*user).or_default(),
            StreamHolder::Address(address) => counts.addresses.entry(address.clone()).or_default(),
        };
        if *count >= cap {
            return None;
        }
        *count += 1;
        Some(StreamHold {
            counts: self.counts.clone(),
            holder,
        })
    }
}

impl Drop for StreamHold {
    fn drop(&mut self) {
        let mut counts = self.counts.lock().unwrap_or_else(|e| e.into_inner());
        fn give_back<K: std::hash::Hash + Eq>(map: &mut HashMap<K, usize>, key: &K) {
            if let Some(count) = map.get_mut(key) {
                *count -= 1;
                if *count == 0 {
                    map.remove(key);
                }
            }
        }
        match &self.holder {
            StreamHolder::User(user) => give_back(&mut counts.users, user),
            StreamHolder::Address(address) => give_back(&mut counts.addresses, address),
        }
    }
}

struct Register {
    id: u64,
    user: UserId,
    /// The sign-in the connection belongs to (`app::login::sign_in_id`).
    sign_in: String,
    communities: Vec<CommunityId>,
    roles: HashMap<CommunityId, Vec<RoleId>>,
    moderator: bool,
    /// Models of the communities, as the database had them, for those the dispatcher lacks.
    models: HashMap<CommunityId, CommunityModel>,
    resume_after: Option<u64>,
    deliveries: mpsc::Sender<Delivery>,
    outcome: oneshot::Sender<Outcome>,
}

/// What became of a registration.
enum Outcome {
    /// Registered; whether `resume_after` was honoured.
    Registered(bool),
    /// The communities the connection reads once caught up whose models it must load before
    /// registering again.
    Load(Vec<CommunityId>),
    /// Refused: the stream retains an event before the resumed position that ends it.
    Ended(StreamEnd),
}

/// Why a connection could not be registered.
#[derive(Debug)]
pub enum Refused {
    /// Its sign-in ended, or its account was banned, by an event the stream still retains.
    Ended(StreamEnd),
    Failed(app::Error),
}

impl From<app::Error> for Refused {
    fn from(e: app::Error) -> Self {
        Refused::Failed(e)
    }
}

/// How many times a connection loads models it turns out to need before giving up. Each retry
/// is for a community joined in the moments since the last.
const REGISTER_ATTEMPTS: usize = 3;

impl EventFeed {
    /// A feed that reads no events, for a server that holds no event stream connections (a
    /// private worker, `app::context::Role`). A connection registered with it is refused.
    pub fn idle() -> Self {
        let (registrations, _) = mpsc::channel(1);
        let (unregister, _) = mpsc::unbounded_channel();
        Self {
            registrations,
            unregister,
            next_id: Arc::new(AtomicU64::new(0)),
            queue_size: 1,
            caps: StreamCaps::new(&Default::default()),
        }
    }

    /// Starts the dispatcher on `context`'s event stream, routing through `shards` tasks.
    /// `queue_size` bounds each connection's queue, and `caps` how many each user and address
    /// may hold.
    pub fn start(
        context: jetstream::Context,
        queue_size: usize,
        shards: usize,
        caps: StreamCaps,
    ) -> Self {
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
            caps,
        }
    }
}

/// Registers a connection of `user`'s, of the sign-in `sign_in` (`app::login::sign_in_id`),
/// resuming after `resume_after` when that is still retained.
pub async fn subscribe(
    state: &GlobalServerContext,
    user: UserId,
    sign_in: String,
    resume_after: Option<u64>,
) -> Result<Subscription, Refused> {
    let mut conn = state
        .connection_pool
        .get()
        .await
        .map_err(app::Error::from)?;
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
                sign_in: sign_in.clone(),
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
            Outcome::Registered(resumed) => {
                return Ok(Subscription {
                    resumed,
                    deliveries,
                    _registration: Registration {
                        id,
                        unregister: feed.unregister.clone(),
                    },
                });
            }
            Outcome::Load(missing) => {
                models = CommunityModel::load(conn.as_mut(), &missing).await?
            }
            Outcome::Ended(end) => return Err(Refused::Ended(end)),
        }
    }
    Err(app::Error::EventFeedStopped.into())
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
/// longer do, parsed only for an event of that kind.
fn moderation_change(kind: Option<&str>, payload: &str) -> Option<bool> {
    if kind != Some("deploymentAccessChanged") {
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
/// may view the one it names, by `model` or, for an event that changes who may, by the model
/// before it (`event.before`); and likewise for the category it names. A channel a model does
/// not know (one not yet made, or deleted) nobody views by it. A deployment moderator reads
/// everything. Anything restricted reaches no one without a model.
fn may_read(
    event: &FeedEvent,
    model: Option<&CommunityModel>,
    user: UserId,
    roles: Option<&Vec<RoleId>>,
    moderator: bool,
) -> bool {
    if moderator
        || (event.requires.is_none() && event.channel.is_none() && event.category.is_none())
    {
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
    let views = |model: &CommunityModel, channel: ChannelId| {
        model.knows(channel) && model.can_view(user, roles, channel)
    };
    let views_category = |model: &CommunityModel, category: CategoryId| {
        model.can_view_category(user, roles, category)
    };
    // A deleted category's overrides are gone from the model after it, which would show the
    // deletion to everyone; it reaches those who could view the category before.
    let deleted = matches!(event.change, Some(ModelChange::CategoryDeleted(_)));
    event.channel.is_none_or(|channel| {
        views(model, channel)
            || event
                .before
                .get()
                .is_some_and(|before| views(before, channel))
    }) && event.category.is_none_or(|category| {
        (!deleted && views_category(model, category))
            || event
                .before
                .get()
                .is_some_and(|before| views_category(before, category))
    })
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
            .map_or(self.last_sequence.saturating_add(1), |(_, _, sequence)| {
                *sequence
            })
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
    /// in-memory stream was recreated and its sequence restarted). `resume_after` is the
    /// client's to choose, so it is compared without arithmetic that could overflow.
    fn start_after(&self, resume_after: Option<u64>) -> (u64, bool) {
        let before_first = self.first_sequence().saturating_sub(1);
        match resume_after {
            Some(after) if after >= before_first && after <= self.last_sequence => (after, true),
            _ => (before_first, false),
        }
    }

    /// What a connection of `user`'s, of the sign-in `sign_in`, reading what `reading` says
    /// missed after `after`, and what it reads once it has caught up.
    /// The database was read at some moment in the retained window, possibly before changes
    /// published just ahead of it were committed, so every membership and role change of the
    /// user's retained up to `after` is applied first (each sets a value, so applying one the
    /// database already shows changes nothing), and those after it as they are passed. Channel
    /// events are kept by the model attached to each, or `models`' for one routed before the
    /// dispatcher held its community's.
    ///
    /// The session was checked against the database just as possibly before an end of the
    /// sign-in or a ban was committed, so a retained event at or before `after` that ends the
    /// connection refuses it, with why: resuming past it is not a way around it. One after
    /// `after` is in the catch-up, which closes the connection once written.
    fn catch_up(
        &self,
        user: UserId,
        sign_in: &str,
        reading: Reading,
        after: u64,
        models: &HashMap<CommunityId, Arc<CommunityModel>>,
    ) -> Result<(Vec<Arc<FeedEvent>>, Reading), StreamEnd> {
        let own = SubjectOwner::User(user);
        let Reading {
            communities: mut reading,
            mut roles,
            mut moderator,
        } = reading;
        for e in self.since(own, 0).take_while(|e| e.sequence <= after) {
            if let Some(end) = e.ends
                && e.reaches(sign_in)
            {
                return Err(end);
            }
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
                e.reaches(sign_in)
            }
            SubjectOwner::Community(community) => {
                reading.contains(&community)
                    && may_read(
                        e,
                        e.access
                            .get()
                            .map(Arc::as_ref)
                            .or_else(|| models.get(&community).map(Arc::as_ref)),
                        user,
                        roles.get(&community),
                        moderator,
                    )
            }
        });
        let caught_up = Reading {
            communities: reading,
            roles,
            moderator,
        };
        Ok((events, caught_up))
    }
}

/// What a connection reads: its user's communities, the roles they hold in each besides
/// everyone's, and whether they moderate the deployment.
struct Reading {
    communities: HashSet<CommunityId>,
    roles: HashMap<CommunityId, Vec<RoleId>>,
    moderator: bool,
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
    sign_in: String,
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
            if !event.reaches(&connection.sign_in) {
                continue;
            }
            if let SubjectOwner::Community(community) = event.owner
                && (event.channel.is_some() || event.category.is_some() || event.requires.is_some())
            {
                let model = event.access.get().map(Arc::as_ref);
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

/// An event's kind: its payload's own `serverEvent`, at the top. Searching the text for the tag
/// would not do, since an event may carry JSON of someone else's making (a plugin event's
/// `payload`) with any tag inside it, and the kinds read here end streams.
fn server_event_of(payload: &str) -> Option<String> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Tag {
        server_event: String,
    }
    serde_json::from_str::<Tag>(payload)
        .ok()
        .map(|tag| tag.server_event)
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
    let kind = server_event_of(payload.get());
    let kind = kind.as_deref();
    let (membership, roles, moderator, change) = match owner {
        SubjectOwner::User(user) => {
            let (membership, roles) = own_membership(payload.get(), user);
            (
                membership,
                roles,
                moderation_change(kind, payload.get()),
                None,
            )
        }
        SubjectOwner::Community(_) => (None, None, None, ModelChange::read(payload.get())),
    };
    let own = matches!(owner, SubjectOwner::User(_));
    let sign_ins = (own && kind == Some("signInsEnded"))
        .then(|| serde_json::from_str::<EndedSignIns>(payload.get()).ok())
        .flatten();
    let ends = if own && kind == Some("accountBanned") {
        Some(StreamEnd::Banned)
    } else if sign_ins.is_some() {
        Some(StreamEnd::SignedOut)
    } else {
        None
    };
    let resync = match owner {
        SubjectOwner::Community(_) => kind == Some("communityResync"),
        SubjectOwner::User(_) => kind == Some("userResync"),
    };
    let email_unverified = (own && kind == Some("emailAccountChanged"))
        .then(|| {
            #[derive(Deserialize)]
            struct Changed {
                unverified: bool,
            }
            serde_json::from_str::<Changed>(payload.get()).ok()
        })
        .flatten()
        .map(|changed| changed.unverified);
    let channel = message
        .headers
        .as_ref()
        .and_then(|headers| headers.get(CHANNEL_HEADER))
        .and_then(|value| value.as_str().parse().ok())
        .map(ChannelId);
    let category = message
        .headers
        .as_ref()
        .and_then(|headers| headers.get(CATEGORY_HEADER))
        .and_then(|value| value.as_str().parse().ok())
        .map(CategoryId);
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
            category,
            requires,
            creator,
            change,
            access: OnceLock::new(),
            before: OnceLock::new(),
            published,
            ends,
            sign_ins,
            resync,
            email_unverified,
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
    /// date with every retained event of the community and attaching to each one routed
    /// without a model the model as it stood there, so a catch-up judges it by the permissions
    /// of its moment rather than today's.
    fn adopt(&mut self, retained: &Retained, community: CommunityId, snapshot: CommunityModel) {
        if self.models.contains_key(&community) {
            return;
        }
        let mut model = Arc::new(snapshot);
        for e in retained.since(SubjectOwner::Community(community), 0) {
            apply_and_attach(&mut model, e);
        }
        self.models.insert(community, model);
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
    /// stands; follows the memberships it makes or ends. Returns the connections that must
    /// resume: those that joined a community with no model here, on a community's resync every
    /// one reading the community, whose model is dropped, and on a user's, every one of theirs,
    /// whose communities and roles were followed from events that may not have happened.
    fn follow(&mut self, event: &mut FeedEvent) -> Vec<u64> {
        match event.owner {
            SubjectOwner::Community(community) if event.resync => {
                let resync: Vec<u64> = self
                    .connections
                    .iter()
                    .filter(|(_, (_, communities))| communities.contains(&community))
                    .map(|(id, _)| *id)
                    .collect();
                for id in &resync {
                    self.unregister(*id);
                }
                self.models.remove(&community);
                resync
            }
            SubjectOwner::Community(community) => {
                if let Some(model) = self.models.get_mut(&community) {
                    apply_and_attach(model, event);
                }
                Vec::new()
            }
            SubjectOwner::User(user) if event.resync => {
                let resync = self.by_user.get(&user).cloned().unwrap_or_default();
                for id in &resync {
                    self.unregister(*id);
                }
                resync
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

/// Applies `event`'s change to `model` and attaches the model to the event as it then stands,
/// and, when the change concerns the channel or category the event names, as it stood before.
/// Events already given models keep them.
fn apply_and_attach(model: &mut Arc<CommunityModel>, event: &FeedEvent) {
    if let Some(change) = &event.change {
        if event.channel.is_some() || event.category.is_some() {
            let _ = event.before.set(model.clone());
        }
        Arc::make_mut(model).apply(change);
    }
    let _ = event.access.set(model.clone());
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
                            start_sequence: retained.last_sequence.saturating_add(1),
                        };
                        (messages, _) = reopen(&context, policy).await;
                        continue;
                    }
                };
                let Some((mut event, pending)) = read(&message) else {
                    continue;
                };
                let last = retained.last_sequence;
                if last != 0 && last.checked_add(1) != Some(event.sequence) {
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
                let reading = Reading {
                    communities: registration.communities.into_iter().collect(),
                    roles: registration.roles,
                    moderator: registration.moderator,
                };
                let caught_up = retained.catch_up(
                    registration.user,
                    &registration.sign_in,
                    reading,
                    after,
                    &models.models,
                );
                let (missed, Reading { communities, roles, moderator }) = match caught_up {
                    Ok(caught_up) => caught_up,
                    Err(end) => {
                        models.release_unread();
                        let _ = registration.outcome.send(Outcome::Ended(end));
                        continue;
                    }
                };
                let missing: Vec<CommunityId> = communities
                    .iter()
                    .filter(|c| !models.models.contains_key(c))
                    .copied()
                    .collect();
                if !missing.is_empty() {
                    models.release_unread();
                    let _ = registration.outcome.send(Outcome::Load(missing));
                    continue;
                }
                if !missed.is_empty()
                    && registration.deliveries.try_send(Delivery::CatchUp(missed)).is_err()
                {
                    models.release_unread();
                    continue;
                }
                if registration.outcome.send(Outcome::Registered(resumed)).is_err() {
                    models.release_unread();
                    continue;
                }
                models.register(registration.id, registration.user, communities.clone());
                let connection = Connection {
                    user: registration.user,
                    sign_in: registration.sign_in,
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
            category: None,
            requires: None,
            creator: None,
            change: None,
            access: OnceLock::new(),
            before: OnceLock::new(),
            published: Instant::now(),
            ends: None,
            sign_ins: None,
            resync: false,
            email_unverified: None,
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
        let _ = e.access.set(model.clone());
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
        let moderation =
            |payload: &str| moderation_change(server_event_of(payload).as_deref(), payload);
        assert_eq!(
            moderation(
                r#"{"serverEvent":"deploymentAccessChanged","permissions":["viewDashboard","moderateCommunities"]}"#
            ),
            Some(true)
        );
        assert_eq!(
            moderation(r#"{"serverEvent":"deploymentAccessChanged","permissions":[]}"#),
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
        let reading = || Reading {
            communities: HashSet::from([kept, left]),
            roles: HashMap::new(),
            moderator: false,
        };
        let (missed, caught_up) = retained.catch_up(user, "", reading(), 0, &none).unwrap();
        assert_eq!(sequences(&missed), vec![2, 3, 4, 5, 6]);
        assert_eq!(caught_up.communities, HashSet::from([kept, joined]));
        // Resuming after the join, with a database read from before it was committed, still
        // reads the community joined.
        let (missed, caught_up) = retained.catch_up(user, "", reading(), 4, &none).unwrap();
        assert_eq!(sequences(&missed), vec![5, 6]);
        assert_eq!(caught_up.communities, HashSet::from([kept, joined]));
    }

    #[test]
    fn streams_are_capped_per_user_and_per_address() {
        let caps = StreamCaps::new(&crate::aspen_config::LimitsConfig {
            max_event_streams_per_user: 2,
            max_event_streams_per_address: 3,
            ..Default::default()
        });
        let (user, other) = (UserId::new(), UserId::new());
        let first = caps.hold_user(user).unwrap();
        let _second = caps.hold_user(user).unwrap();
        assert!(caps.hold_user(user).is_none());
        assert!(caps.hold_user(other).is_some());
        drop(first);
        assert!(caps.hold_user(user).is_some());
        let address = || "192.0.2.1".to_string();
        let held: Vec<_> = (0..3)
            .map(|_| caps.hold_address(address()).unwrap())
            .collect();
        assert!(caps.hold_address(address()).is_none());
        drop(held);
        assert!(caps.counts.lock().unwrap().addresses.is_empty());
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
        assert_eq!(retained.start_after(Some(u64::MAX)), (9, false));
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
                sign_in: String::new(),
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
        two_channels_with(community, open, hidden, moderator, RoleId::new())
    }

    /// `two_channels`, with `everyone` as everyone's role.
    fn two_channels_with(
        community: CommunityId,
        open: ChannelId,
        hidden: ChannelId,
        moderator: RoleId,
        everyone: RoleId,
    ) -> CommunityModel {
        use crate::app::permissions::Permissions;
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
    fn an_end_of_sign_ins_reaches_and_ends_only_theirs() {
        let user = UserId::new();
        let mut routes = Routes::default();
        let mut receivers = Vec::new();
        for (id, sign_in) in [(1, "a"), (2, "b"), (3, "c")] {
            let (tx, rx) = mpsc::channel(8);
            routes.add(
                id,
                Connection {
                    user,
                    sign_in: sign_in.to_string(),
                    communities: HashSet::new(),
                    roles: HashMap::new(),
                    moderator: false,
                    deliveries: tx,
                },
            );
            receivers.push(rx);
        }
        let received = |receivers: &mut Vec<mpsc::Receiver<Delivery>>| -> Vec<bool> {
            receivers
                .iter_mut()
                .map(|rx| match rx.try_recv() {
                    Ok(Delivery::Live(e)) => e.ends() == Some(StreamEnd::SignedOut),
                    _ => false,
                })
                .collect()
        };
        let ended = |sequence, ended: Option<&str>, kept: Option<&str>| {
            let mut e = plain(sequence, SubjectOwner::User(user), None);
            e.sign_ins = Some(EndedSignIns {
                ended: ended.map(str::to_string),
                kept: kept.map(str::to_string),
            });
            e.ends = Some(StreamEnd::SignedOut);
            Arc::new(e)
        };
        // Signing out of one ends its streams alone.
        routes.route(&ended(1, Some("b"), None));
        assert_eq!(received(&mut receivers), vec![false, true, false]);
        // Changing the password ends every other.
        routes.route(&ended(2, None, Some("a")));
        assert_eq!(received(&mut receivers), vec![false, true, true]);
    }

    #[test]
    fn resuming_past_an_end_of_the_sign_in_or_a_ban_is_refused() {
        let user = UserId::new();
        let own = SubjectOwner::User(user);
        let mut retained = Retained::default();
        retained.push(event(1, own, None));
        let mut signed_out = plain(2, own, None);
        signed_out.sign_ins = Some(EndedSignIns {
            ended: Some("b".to_string()),
            kept: None,
        });
        signed_out.ends = Some(StreamEnd::SignedOut);
        retained.push(Arc::new(signed_out));
        retained.push(event(3, own, None));
        let none = HashMap::new();
        let reading = || Reading {
            communities: HashSet::new(),
            roles: HashMap::new(),
            moderator: false,
        };
        let refused = |sign_in, after| retained.catch_up(user, sign_in, reading(), after, &none);
        assert_eq!(refused("b", 2).err(), Some(StreamEnd::SignedOut));
        assert_eq!(refused("b", 3).err(), Some(StreamEnd::SignedOut));
        // Before it, the end is in the catch-up, which closes the connection.
        let (missed, _) = refused("b", 1).unwrap();
        assert_eq!(sequences(&missed), vec![2, 3]);
        // Another sign-in's end does not concern this one.
        let (missed, _) = refused("a", 2).unwrap();
        assert_eq!(sequences(&missed), vec![3]);
        let mut banned = plain(4, own, None);
        banned.ends = Some(StreamEnd::Banned);
        retained.push(Arc::new(banned));
        assert_eq!(
            retained.catch_up(user, "a", reading(), 4, &none).err(),
            Some(StreamEnd::Banned)
        );
    }

    #[test]
    fn a_resync_drops_the_model_and_its_readers() {
        let community = CommunityId::new();
        let other = CommunityId::new();
        let mut models = Models::default();
        models
            .models
            .insert(community, Arc::new(CommunityModel::new(community)));
        models
            .models
            .insert(other, Arc::new(CommunityModel::new(other)));
        models.register(1, UserId::new(), HashSet::from([community, other]));
        models.register(2, UserId::new(), HashSet::from([other]));
        let mut resync = plain(1, SubjectOwner::Community(community), None);
        resync.resync = true;
        assert_eq!(models.follow(&mut resync), vec![1]);
        assert!(!models.models.contains_key(&community));
        // The other community is still read, by the connection that stays.
        assert!(models.models.contains_key(&other));
        assert_eq!(models.readers.get(&other), Some(&1));
    }

    #[test]
    fn a_users_resync_drops_their_connections_alone() {
        let community = CommunityId::new();
        let user = UserId::new();
        let mut models = Models::default();
        models
            .models
            .insert(community, Arc::new(CommunityModel::new(community)));
        models.register(1, user, HashSet::from([community]));
        models.register(2, user, HashSet::new());
        models.register(3, UserId::new(), HashSet::from([community]));
        let mut resync = plain(1, SubjectOwner::User(user), None);
        resync.resync = true;
        let mut dropped = models.follow(&mut resync);
        dropped.sort();
        assert_eq!(dropped, vec![1, 2]);
        // The community is still read, by the other user's connection.
        assert!(models.models.contains_key(&community));
        assert_eq!(models.readers.get(&community), Some(&1));
    }

    #[test]
    fn a_change_to_who_may_view_reaches_those_on_either_side_of_it() {
        let member = UserId::new();
        let community = CommunityId::new();
        let (open, hidden, moderator) = (ChannelId::new(), ChannelId::new(), RoleId::new());
        let everyone = RoleId::new();
        let mut models = Models::default();
        models.models.insert(
            community,
            Arc::new(two_channels_with(
                community, open, hidden, moderator, everyone,
            )),
        );
        models.readers.insert(community, 1);
        let mut routes = Routes::default();
        let (tx, mut rx) = mpsc::channel(8);
        routes.add(
            1,
            Connection {
                user: member,
                sign_in: String::new(),
                communities: HashSet::from([community]),
                roles: HashMap::new(),
                moderator: false,
                deliveries: tx,
            },
        );
        let mut followed = |sequence: u64, channel: ChannelId, change: Option<ModelChange>| {
            let mut e = plain(sequence, SubjectOwner::Community(community), None);
            e.channel = Some(channel);
            e.change = change;
            models.follow(&mut e);
            Arc::new(e)
        };
        let hide = |channel| ModelChange::ChannelOverride {
            channel,
            role: everyone,
            set: Some((Permissions::empty(), Permissions::VIEW_CHANNEL)),
        };
        // Hiding the open channel reaches the member, who could view it until then, and then
        // nothing more of it does.
        let hiding = followed(1, open, Some(hide(open)));
        let after = followed(2, open, None);
        // Moving the hidden one into no category changes nothing for them and is not sent.
        let moved = followed(
            3,
            hidden,
            Some(ModelChange::ChannelCategory {
                channel: hidden,
                category: None,
            }),
        );
        // A channel made hidden, its override published ahead of its creation, never reaches
        // them, though the model did not know it before.
        let new = ChannelId::new();
        let override_first = followed(4, new, Some(hide(new)));
        let created = followed(
            5,
            new,
            Some(ModelChange::ChannelCategory {
                channel: new,
                category: None,
            }),
        );
        // Clearing the override on the hidden channel lets them view it, and they receive it.
        let shown = followed(
            6,
            hidden,
            Some(ModelChange::ChannelOverride {
                channel: hidden,
                role: everyone,
                set: None,
            }),
        );
        // Deleting a channel reaches only those who could view it.
        let deleted = followed(7, new, Some(ModelChange::ChannelDeleted(new)));
        for e in [
            hiding,
            after,
            moved,
            override_first,
            created,
            shown,
            deleted,
        ] {
            routes.route(&e);
        }
        let mut sequences = Vec::new();
        while let Ok(Delivery::Live(e)) = rx.try_recv() {
            sequences.push(e.sequence);
        }
        assert_eq!(sequences, vec![1, 6]);
    }

    #[test]
    fn a_categorys_events_reach_those_its_own_overrides_let_view_it() {
        let member = UserId::new();
        let community = CommunityId::new();
        let (open, hidden, moderator) = (ChannelId::new(), ChannelId::new(), RoleId::new());
        let everyone = RoleId::new();
        let mut models = Models::default();
        models.models.insert(
            community,
            Arc::new(two_channels_with(
                community, open, hidden, moderator, everyone,
            )),
        );
        models.readers.insert(community, 1);
        let mut routes = Routes::default();
        let (tx, mut rx) = mpsc::channel(16);
        routes.add(
            1,
            Connection {
                user: member,
                sign_in: String::new(),
                communities: HashSet::from([community]),
                roles: HashMap::new(),
                moderator: false,
                deliveries: tx,
            },
        );
        let mut followed = |sequence: u64, category: CategoryId, change: Option<ModelChange>| {
            let mut e = plain(sequence, SubjectOwner::Community(community), None);
            e.category = Some(category);
            e.change = change;
            models.follow(&mut e);
            Arc::new(e)
        };
        let set = |category, set| ModelChange::CategoryOverride {
            category,
            role: everyone,
            set,
        };
        let hide = Some((Permissions::empty(), Permissions::VIEW_CHANNEL));
        let (kept, other) = (CategoryId::new(), CategoryId::new());
        let events = [
            // Made with no overrides, everyone may learn of it.
            followed(1, kept, None),
            // Hiding it reaches those who could view it, and then nothing more of it does.
            followed(2, kept, Some(set(kept, hide))),
            followed(3, kept, None),
            // Showing it again reaches them, about a category they did not have.
            followed(4, kept, Some(set(kept, None))),
            followed(5, kept, Some(set(kept, hide))),
            // Deleting a hidden category, whose overrides go with it, does not show it.
            followed(6, kept, Some(ModelChange::CategoryDeleted(kept))),
            // Deleting one they could view reaches them.
            followed(7, other, Some(ModelChange::CategoryDeleted(other))),
        ];
        for e in &events {
            routes.route(e);
        }
        let mut sequences = Vec::new();
        while let Ok(Delivery::Live(e)) = rx.try_recv() {
            sequences.push(e.sequence);
        }
        assert_eq!(sequences, vec![1, 2, 4, 5, 7]);
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
                sign_in: String::new(),
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
                sign_in: String::new(),
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
        let attached = change.access.get().cloned().expect("a model");
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
                    sign_in: String::new(),
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
        let _ = invite.access.set(model);
        routes.route(&Arc::new(invite));
        let got: Vec<bool> = receivers
            .iter_mut()
            .map(|rx| rx.try_recv().is_ok())
            .collect();
        assert_eq!(got, vec![true, true, false]);
    }

    #[test]
    fn an_events_kind_is_its_own_tag_not_one_it_carries() {
        let event = crate::api::message_enum::server_event::ServerEvent::PluginEvent {
            plugin: "example".into(),
            kind: "anything".into(),
            channel: None,
            community: None,
            payload: serde_json::json!({ "serverEvent": "communityResync" }),
        };
        let payload = serde_json::to_string(&event).unwrap();
        assert!(payload.contains(r#""serverEvent":"communityResync""#));
        assert_eq!(server_event_of(&payload).as_deref(), Some("pluginEvent"));
    }
}
