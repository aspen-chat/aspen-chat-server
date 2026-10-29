//! Telling event stream connections of presence as it changes. Presence itself is read from
//! Valkey (`app::user_status`); this tells those who show someone that their presence may have
//! changed, gathered so that a burst of changes costs a few reads and frames, not one each.
//!
//! - **Hints.** Whatever may change someone's presence says so with [`PresenceFeed::changed`]:
//!   their coming online or becoming active again (`app::user_status`), their choosing a status
//!   (`app::presence_override`), a block (`app::block`). A hint names only the user; one task per
//!   server gathers the hints made meanwhile and publishes them together on the core NATS subject
//!   [`PRESENCE_SUBJECT`], outside the event stream's, so JetStream neither keeps nor replays
//!   them.
//! - **Expiries.** What changes presence by running out (a connection's key, activity's, a
//!   timed override) is hinted when it runs out, by a timer ([`PresenceFeed::expires`]) kept on
//!   the server that set it, and on every server whose router read the user's presence for a
//!   telling, from how long each key had left then (`user_status::raw_statuses_with_expiries`).
//!   So the timers live wherever someone watches the user, and a server restarting loses only
//!   its own: the next telling anywhere sets them again, and so does each renewal of the user's
//!   `online` key wherever their devices are (`user_status::mark_user_online_id`). A key renewed
//!   elsewhere meanwhile makes a hint that changes nothing, which is then told to no one, and the
//!   telling it causes sets the timer again from the renewed key.
//! - **Watching.** Each event stream connection says which users it shows (`watchPresence`, at
//!   most [`MAX_WATCHED_PRESENCE`]), kept by one router task per server. The users of its first
//!   list are told only of changes: its client reads them whole over REST once the list is taken
//!   up, a read the request path can refuse when busy, where telling them would put every
//!   reconnecting connection's whole list on the router at once. Users a later list adds are
//!   told as they are, as far as the connection's budget allows ([`ADDED_PER_MINUTE`]). Each list
//!   taken up is answered with `presenceWatching`, which the client waits for before that read,
//!   so every change is in the read or told after it.
//! - **Telling.** The router gathers the hints for users watched here, and newly watched users,
//!   for up to [`PRESENCE_WINDOW_MILLIS`] from the first, then reads all their presence in one
//!   batched read in a task of its own, decides in one query which watchers may learn it (as
//!   `user_status::presence_visible` does: the user themself, those sharing a community or DM
//!   with them, a bot's owner, and never anyone they block; [`Decisions`] keeps the answers a
//!   little while), and sends each connection one `presence` frame holding what differs from
//!   what it was last told. Someone a watcher may no longer learn the presence of is told to them
//!   as offline.
//!
//! A hint lost (a full queue, NATS dropping a core message) leaves a watcher behind until the
//! next change, the next of its own timers, or its client's slower full read.

use crate::UserId;
use crate::database::Pool;
use crate::user_status::{presence_visible_pairs, raw_statuses_with_expiries, seen_by_others};
use aspen_wire::ephemeral::EphemeralEvent;
pub use aspen_wire::ephemeral::{MAX_WATCHED_PRESENCE, PRESENCE_WINDOW_MILLIS};
use aspen_wire::user::{UserOnlineStatus, UserStatusRecord};
use futures_util::StreamExt;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;
use tracing::{error, warn};

/// The core NATS subject every API server publishes presence hints on and reads them from: a
/// JSON array of user ids. It lies outside `events::SUBJECT_ROOT`, whose subjects JetStream
/// keeps.
pub const PRESENCE_SUBJECT: &str = "aspen.presence";

/// How long changes are gathered before connections are told of them.
const WINDOW: Duration = Duration::from_millis(PRESENCE_WINDOW_MILLIS);

/// How many hints may wait to be published; more are dropped.
const HINT_QUEUE: usize = 65_536;

/// The most users one published hint names.
const HINT_BATCH: usize = 4_096;

/// How many expiries may wait for the timer task; more are dropped.
const EXPIRY_QUEUE: usize = 65_536;

/// How many users a connection's watch lists after its first may add in a burst, and how many it
/// earns back each minute. Past it the rest of a list are taken up as the budget allows, a
/// telling at a time, so a client changing lists fast costs the router no more than one changing
/// them as a person scrolls.
const ADDED_BURST: f64 = MAX_WATCHED_PRESENCE as f64;
const ADDED_PER_MINUTE: f64 = 1_000.0;

/// How long a decision of whether a viewer may learn someone's presence is kept, and how many
/// are ([`Decisions`]).
const DECIDED_FOR: Duration = Duration::from_secs(10);
const DECISIONS_KEPT: u64 = 200_000;

/// How many `presence` frames may wait for one connection; past it the connection is told again
/// at the next change.
const DELIVERY_QUEUE: usize = 4;

/// The most viewer and user pairs one visibility query decides.
const PAIR_BATCH: usize = 5_000;

/// The most connection and user pairs one telling decides, ten visibility queries' worth; the
/// rest wait for the next window, so a burst (a popular user's change watched by thousands here,
/// many connections changing their watch lists) never holds a database connection for long.
/// Changes are told before pairs carried over or newly watched.
const MAX_PAIRS_PER_TELLING: usize = 10 * PAIR_BATCH;

/// A viewer and a user whose presence they watch.
type ViewerPair = (UserId, UserId);

/// What a telling reads: each user's presence, how long what makes it has left, and the viewer
/// pairs that may learn it.
type Read = (
    HashMap<UserId, UserOnlineStatus>,
    Vec<(UserId, Expiry, Duration)>,
    HashSet<ViewerPair>,
);

/// A little after a key's expiry, so the hint finds it gone.
const EXPIRY_SLACK: Duration = Duration::from_secs(1);

/// When to hint what runs out `after` from now.
fn expiry_at(after: Duration) -> Instant {
    Instant::now() + after + EXPIRY_SLACK
}

/// What runs out and so may change someone's presence when it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Expiry {
    /// Their connection's key, `user:{uuid}:online`: they go offline.
    Connection,
    /// Their activity's key, `user:{uuid}:active`: they go away.
    Activity,
    /// A timed presence override.
    Chosen,
}

/// The server's end of presence telling; cheap to clone.
#[derive(Clone)]
pub struct PresenceFeed {
    hints: mpsc::Sender<UserId>,
    /// `None` on a server that routes nothing (`Role::PrivateWorker`).
    expiries: Option<mpsc::Sender<(UserId, Expiry, Instant)>>,
    router: Option<RouterHandle>,
}

#[derive(Clone)]
struct RouterHandle {
    registrations: mpsc::UnboundedSender<Registration>,
    /// Each connection's latest watch list not yet taken by the router, one slot per connection,
    /// so however fast connections send lists the router holds one each, and none crowds out
    /// another's.
    lists: Arc<Mutex<HashMap<u64, Vec<UserId>>>>,
    /// Woken when a list is put in `lists`.
    listed: Arc<Notify>,
    next_id: Arc<AtomicU64>,
}

enum Registration {
    Add {
        id: u64,
        viewer: UserId,
        deliveries: mpsc::Sender<EphemeralEvent>,
    },
    Remove(u64),
}

impl PresenceFeed {
    /// Starts publishing hints and, with `route`, the timers and the router for this server's
    /// event stream connections.
    pub fn start(
        nats: async_nats::Client,
        valkey: fred::clients::Client,
        pool: Pool,
        route: bool,
    ) -> Self {
        let (hints, published) = mpsc::channel(HINT_QUEUE);
        tokio::spawn(publish_hints(nats.clone(), published));
        if !route {
            return PresenceFeed {
                hints,
                expiries: None,
                router: None,
            };
        }
        let (expiries, asked) = mpsc::channel(EXPIRY_QUEUE);
        tokio::spawn(keep_expiries(asked, hints.clone()));
        let (registrations, registered) = mpsc::unbounded_channel();
        let lists = Arc::new(Mutex::new(HashMap::new()));
        let listed = Arc::new(Notify::new());
        tokio::spawn(route_presence(
            nats,
            valkey,
            pool,
            expiries.clone(),
            registered,
            lists.clone(),
            listed.clone(),
        ));
        PresenceFeed {
            hints,
            expiries: Some(expiries),
            router: Some(RouterHandle {
                registrations,
                lists,
                listed,
                next_id: Arc::new(AtomicU64::new(0)),
            }),
        }
    }

    /// Says `user`'s presence may have changed. Best effort: a full queue drops it.
    pub fn changed(&self, user: UserId) {
        let _ = self.hints.try_send(user);
    }

    /// Says what of `user`'s presence runs out `after` from now, when a hint will be made. Each
    /// replaces the last for the same user and kind.
    pub fn expires(&self, user: UserId, kind: Expiry, after: Duration) {
        if let Some(expiries) = &self.expiries {
            let _ = expiries.try_send((user, kind, expiry_at(after)));
        }
    }

    /// Registers an event stream connection of `viewer`'s, which then says whom it watches.
    pub fn watch(&self, viewer: UserId) -> PresenceWatch {
        let (deliveries, received) = mpsc::channel(DELIVERY_QUEUE);
        let Some(router) = &self.router else {
            return PresenceWatch {
                id: 0,
                received,
                router: None,
            };
        };
        let id = router.next_id.fetch_add(1, Ordering::Relaxed);
        let _ = router.registrations.send(Registration::Add {
            id,
            viewer,
            deliveries,
        });
        PresenceWatch {
            id,
            received,
            router: Some(router.clone()),
        }
    }
}

/// One event stream connection's watch on presence. Dropping it ends the watch.
pub struct PresenceWatch {
    id: u64,
    received: mpsc::Receiver<EphemeralEvent>,
    router: Option<RouterHandle>,
}

impl PresenceWatch {
    /// The users the connection shows, replacing those it named before; beyond
    /// [`MAX_WATCHED_PRESENCE`] the rest are left out. A list the router has not taken yet is
    /// replaced by this one.
    pub fn set(&self, mut users: Vec<UserId>) {
        users.truncate(MAX_WATCHED_PRESENCE);
        if let Some(router) = &self.router {
            router
                .lists
                .lock()
                .expect("the presence watch lists' lock")
                .insert(self.id, users);
            router.listed.notify_one();
        }
    }

    /// The next `presence` or `presenceWatching` to tell the connection of; never on a server
    /// that routes nothing.
    pub async fn next(&mut self) -> Option<EphemeralEvent> {
        if self.router.is_none() {
            return std::future::pending().await;
        }
        self.received.recv().await
    }
}

impl Drop for PresenceWatch {
    fn drop(&mut self) {
        if let Some(router) = &self.router {
            let _ = router.registrations.send(Registration::Remove(self.id));
        }
    }
}

/// Publishes the hints made on this server, those made meanwhile together.
async fn publish_hints(nats: async_nats::Client, mut hints: mpsc::Receiver<UserId>) {
    let mut batch: HashSet<UserId> = HashSet::new();
    while let Some(user) = hints.recv().await {
        batch.insert(user);
        while batch.len() < HINT_BATCH {
            match hints.try_recv() {
                Ok(user) => {
                    batch.insert(user);
                }
                Err(_) => break,
            }
        }
        let users: Vec<UserId> = batch.drain().collect();
        let payload = match serde_json::to_vec(&users) {
            Ok(payload) => payload,
            Err(e) => {
                warn!("a presence hint could not be written: {e}");
                continue;
            }
        };
        if let Err(e) = nats.publish(PRESENCE_SUBJECT, payload.into()).await {
            warn!("a presence hint could not be published: {e}");
        }
    }
}

/// When each user's keys and timed overrides run out, as set on this server or read for its
/// watchers: one entry per user and kind, the latest set.
#[derive(Default)]
struct Expiries {
    at: HashMap<(UserId, Expiry), Instant>,
    due: BTreeSet<(Instant, UserId, Expiry)>,
}

impl Expiries {
    fn set(&mut self, user: UserId, kind: Expiry, at: Instant) {
        if let Some(previous) = self.at.insert((user, kind), at) {
            self.due.remove(&(previous, user, kind));
        }
        self.due.insert((at, user, kind));
    }

    fn next(&self) -> Option<Instant> {
        self.due.first().map(|(at, _, _)| *at)
    }

    /// Takes every entry due by `now`, answering their users.
    fn take_due(&mut self, now: Instant) -> HashSet<UserId> {
        let mut users = HashSet::new();
        while let Some(&(at, user, kind)) = self.due.first() {
            if at > now {
                break;
            }
            self.due.pop_first();
            self.at.remove(&(user, kind));
            users.insert(user);
        }
        users
    }
}

/// Hints each expiry asked for when it comes.
async fn keep_expiries(
    mut asked: mpsc::Receiver<(UserId, Expiry, Instant)>,
    hints: mpsc::Sender<UserId>,
) {
    let mut expiries = Expiries::default();
    loop {
        let next = expiries.next();
        tokio::select! {
            ask = asked.recv() => {
                let Some((user, kind, at)) = ask else {
                    return;
                };
                expiries.set(user, kind, at);
            }
            () = async {
                match next {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                for user in expiries.take_due(Instant::now()) {
                    let _ = hints.try_send(user);
                }
            }
        }
    }
}

/// One watching connection.
struct Watcher {
    viewer: UserId,
    deliveries: mpsc::Sender<EphemeralEvent>,
    /// Each user watched, and what the connection was last told of them; `None` until told.
    told: HashMap<UserId, Option<UserOnlineStatus>>,
    /// The watch list it named last, taken up at the next telling.
    next: Option<Vec<UserId>>,
    /// Whether a watch list of its has been taken up. The users of its first are told only of
    /// changes, since its client reads them whole once the list is taken up; those added by later
    /// lists are told as they are.
    watching: bool,
    /// How many users its later lists may still add ([`ADDED_BURST`]), as of `budget_at`.
    budget: f64,
    budget_at: Instant,
}

impl Watcher {
    /// How many users its lists may add now, whole.
    fn may_add(&mut self, now: Instant) -> usize {
        let earned = now.duration_since(self.budget_at).as_secs_f64() * ADDED_PER_MINUTE / 60.0;
        self.budget = (self.budget + earned).min(ADDED_BURST);
        self.budget_at = now;
        self.budget as usize
    }
}

/// Recent decisions of whether a viewer may learn someone's presence, so a connection naming
/// users it named a moment ago does not have them decided again. A decision is used only when it
/// was made after the last hint for the user (`hinted`): whatever changes who may learn it and is
/// hinted (a block, a change of presence) is decided afresh at once. What changes it unhinted
/// (joining or leaving a community, a DM) reaches a user named again within [`DECIDED_FOR`].
struct Decisions {
    decided: moka::sync::Cache<(UserId, UserId), (bool, Instant)>,
    hinted: moka::sync::Cache<UserId, Instant>,
}

impl Default for Decisions {
    fn default() -> Self {
        fn cache<K, V>() -> moka::sync::Cache<K, V>
        where
            K: std::hash::Hash + Eq + Send + Sync + 'static,
            V: Clone + Send + Sync + 'static,
        {
            moka::sync::Cache::builder()
                .max_capacity(DECISIONS_KEPT)
                .time_to_live(DECIDED_FOR)
                .build()
        }
        Decisions {
            decided: cache(),
            hinted: cache(),
        }
    }
}

impl Decisions {
    fn hinted(&self, user: UserId, at: Instant) {
        self.hinted.insert(user, at);
    }

    /// Splits `pairs` into those decided since their user's last hint, with the ones visible, and
    /// those to decide.
    fn split(&self, pairs: Vec<(UserId, UserId)>) -> (HashSet<ViewerPair>, Vec<ViewerPair>) {
        let mut visible = HashSet::new();
        let mut undecided = Vec::new();
        for pair in pairs {
            let hinted = self.hinted.get(&pair.1);
            match self.decided.get(&pair) {
                Some((seen, at)) if hinted.is_none_or(|hinted| at > hinted) => {
                    if seen {
                        visible.insert(pair);
                    }
                }
                _ => undecided.push(pair),
            }
        }
        (visible, undecided)
    }

    /// Keeps the decisions of `pairs` begun `at`, `visible` holding those that may.
    fn note(&self, pairs: &[(UserId, UserId)], visible: &HashSet<(UserId, UserId)>, at: Instant) {
        for pair in pairs {
            self.decided.insert(*pair, (visible.contains(pair), at));
        }
    }
}

/// Who watches whom on this server, and what is waiting to be told.
#[derive(Default)]
struct Router {
    watchers: HashMap<u64, Watcher>,
    by_user: HashMap<UserId, HashSet<u64>>,
    /// Users watched here whose presence may have changed.
    changed: HashSet<UserId>,
    /// Pairs to tell whatever they are: users a connection added to its watch after its first
    /// list, and pairs left over from a telling cut short, failed, or not delivered.
    fresh: HashSet<(u64, UserId)>,
    /// Connections whose latest watch list was taken up and who are yet to be told so
    /// (`presenceWatching`).
    unacknowledged: HashSet<u64>,
    /// When the gathered changes are told; `None` with none waiting.
    tell_at: Option<Instant>,
    decisions: Decisions,
}

impl Router {
    fn due(&mut self) {
        self.tell_at.get_or_insert_with(|| Instant::now() + WINDOW);
    }

    fn add(&mut self, id: u64, viewer: UserId, deliveries: mpsc::Sender<EphemeralEvent>) {
        self.watchers.insert(
            id,
            Watcher {
                viewer,
                deliveries,
                told: HashMap::new(),
                next: None,
                watching: false,
                budget: ADDED_BURST,
                budget_at: Instant::now(),
            },
        );
    }

    fn remove(&mut self, id: u64) {
        let Some(watcher) = self.watchers.remove(&id) else {
            return;
        };
        for user in watcher.told.keys() {
            self.unwatch(id, *user);
        }
    }

    fn unwatch(&mut self, id: u64, user: UserId) {
        if let Some(ids) = self.by_user.get_mut(&user) {
            ids.remove(&id);
            if ids.is_empty() {
                self.by_user.remove(&user);
            }
        }
    }

    fn named(&mut self, id: u64, users: Vec<UserId>) {
        if let Some(watcher) = self.watchers.get_mut(&id) {
            watcher.next = Some(users);
            self.due();
        }
    }

    fn hinted(&mut self, users: Vec<UserId>) {
        let now = Instant::now();
        for user in users {
            self.decisions.hinted(user, now);
            if self.by_user.contains_key(&user) {
                self.changed.insert(user);
                self.due();
            }
        }
    }

    /// Takes up each connection's latest watch list, one per telling however many it sent. The
    /// users a list adds are told as they are, except those of a connection's first, which its
    /// client reads whole. A later list adds users as far as its connection's budget allows
    /// ([`ADDED_PER_MINUTE`]), in the order it names them; the rest wait for the next telling,
    /// and the list is acknowledged once it is taken up whole.
    fn take_up_watches(&mut self) {
        let now = Instant::now();
        let named: Vec<(u64, Vec<UserId>)> = self
            .watchers
            .iter_mut()
            .filter_map(|(id, watcher)| watcher.next.take().map(|users| (*id, users)))
            .collect();
        let mut waiting = false;
        for (id, mut users) in named {
            users.truncate(MAX_WATCHED_PRESENCE);
            let wanted: HashSet<UserId> = users.iter().copied().collect();
            let Some(watcher) = self.watchers.get_mut(&id) else {
                continue;
            };
            let dropped: Vec<UserId> = watcher
                .told
                .keys()
                .filter(|user| !wanted.contains(user))
                .copied()
                .collect();
            for user in &dropped {
                watcher.told.remove(user);
            }
            let mut seen = HashSet::new();
            let mut added: Vec<UserId> = users
                .iter()
                .copied()
                .filter(|user| !watcher.told.contains_key(user) && seen.insert(*user))
                .collect();
            let tell_added = watcher.watching;
            let whole = if tell_added {
                let may = watcher.may_add(now);
                let whole = added.len() <= may;
                added.truncate(may);
                watcher.budget -= added.len() as f64;
                whole
            } else {
                true
            };
            for user in &added {
                watcher.told.insert(*user, None);
            }
            watcher.watching = true;
            if !whole {
                // A list named since replaces this one in `named`.
                watcher.next = Some(users);
                waiting = true;
            }
            for user in dropped {
                self.unwatch(id, user);
            }
            for user in added {
                self.by_user.entry(user).or_default().insert(id);
                if tell_added {
                    self.fresh.insert((id, user));
                }
            }
            if whole {
                self.unacknowledged.insert(id);
            }
        }
        if waiting {
            self.due();
        }
        self.acknowledge();
    }

    /// Tells each connection whose watch list was taken up that it was. One whose queue is full
    /// is told at the next telling.
    fn acknowledge(&mut self) {
        let mut left = HashSet::new();
        for id in std::mem::take(&mut self.unacknowledged) {
            let Some(watcher) = self.watchers.get(&id) else {
                continue;
            };
            if let Err(mpsc::error::TrySendError::Full(_)) = watcher
                .deliveries
                .try_send(EphemeralEvent::PresenceWatching)
            {
                left.insert(id);
            }
        }
        if !left.is_empty() {
            self.unacknowledged = left;
            self.due();
        }
    }

    /// The connection and user pairs to tell of now, at most [`MAX_PAIRS_PER_TELLING`]; the
    /// rest are kept for the next telling, as newly watched pairs, which are told whatever
    /// differs from what their connections were last told just as changed ones are.
    fn gathered(&mut self) -> HashSet<(u64, UserId)> {
        self.tell_at = None;
        self.take_up_watches();
        let mut pairs = HashSet::new();
        let mut in_order = Vec::new();
        for user in std::mem::take(&mut self.changed) {
            if let Some(ids) = self.by_user.get(&user) {
                for id in ids {
                    if pairs.insert((*id, user)) {
                        in_order.push((*id, user));
                    }
                }
            }
        }
        for pair in std::mem::take(&mut self.fresh) {
            if pairs.insert(pair) {
                in_order.push(pair);
            }
        }
        if in_order.len() > MAX_PAIRS_PER_TELLING {
            for pair in in_order.split_off(MAX_PAIRS_PER_TELLING) {
                pairs.remove(&pair);
                self.fresh.insert(pair);
            }
            self.due();
        }
        pairs
    }

    /// The viewer and user pairs among `pairs` whose visibility must be decided: all but each
    /// viewer's own.
    fn viewer_pairs(&self, pairs: &HashSet<(u64, UserId)>) -> Vec<(UserId, UserId)> {
        let unique: HashSet<(UserId, UserId)> = pairs
            .iter()
            .filter_map(|(id, user)| self.watchers.get(id).map(|w| (w.viewer, *user)))
            .filter(|(viewer, user)| viewer != user)
            .collect();
        unique.into_iter().collect()
    }

    /// Tells each connection of what in `pairs` differs from what it was last told: `statuses`
    /// as each user is told their own, shown to others as they may learn it (`visible`).
    fn tell(
        &mut self,
        pairs: HashSet<(u64, UserId)>,
        statuses: &HashMap<UserId, UserOnlineStatus>,
        visible: &HashSet<(UserId, UserId)>,
    ) {
        let mut out: HashMap<u64, Vec<UserStatusRecord>> = HashMap::new();
        for (id, user) in pairs {
            let Some(watcher) = self.watchers.get_mut(&id) else {
                continue;
            };
            // No longer watched since it was gathered.
            let Some(told) = watcher.told.get_mut(&user) else {
                continue;
            };
            let own = statuses
                .get(&user)
                .copied()
                .unwrap_or(UserOnlineStatus::Offline);
            let status = if watcher.viewer == user {
                own
            } else if visible.contains(&(watcher.viewer, user)) {
                seen_by_others(own)
            } else {
                UserOnlineStatus::Offline
            };
            if *told != Some(status) {
                *told = Some(status);
                out.entry(id).or_default().push(UserStatusRecord {
                    id: user,
                    online_status: status,
                });
            }
        }
        for (id, statuses) in out {
            let Some(watcher) = self.watchers.get_mut(&id) else {
                continue;
            };
            if let Err(mpsc::error::TrySendError::Full(EphemeralEvent::Presence { statuses })) =
                watcher
                    .deliveries
                    .try_send(EphemeralEvent::Presence { statuses })
            {
                // Told again at the next telling.
                for status in statuses {
                    watcher.told.insert(status.id, None);
                    self.fresh.insert((id, status.id));
                }
                self.due();
            }
        }
    }
}

/// Reads the presence of `users`, how long what makes it has left, and which viewers may learn
/// it.
async fn read(
    valkey: &fred::clients::Client,
    pool: &Pool,
    users: Vec<UserId>,
    viewer_pairs: Vec<(UserId, UserId)>,
) -> crate::Result<Read> {
    let (statuses, expiries) = raw_statuses_with_expiries(valkey, users).await?;
    let mut visible = HashSet::new();
    if !viewer_pairs.is_empty() {
        let mut conn = pool.get().await?;
        for batch in viewer_pairs.chunks(PAIR_BATCH) {
            visible.extend(presence_visible_pairs(conn.as_mut(), batch).await?);
        }
    }
    Ok((statuses, expiries, visible))
}

/// Subscribes to presence hints, retried until NATS answers.
async fn subscribe_hints(nats: &async_nats::Client) -> async_nats::Subscriber {
    let mut wait = Duration::from_millis(250);
    loop {
        match nats.subscribe(PRESENCE_SUBJECT).await {
            Ok(subscriber) => return subscriber,
            Err(e) => {
                error!("could not subscribe to presence hints, retrying: {e}");
                tokio::time::sleep(wait).await;
                wait = (wait * 2).min(Duration::from_secs(5));
            }
        }
    }
}

/// A telling's read, done by a task of its own so the router goes on taking hints, lists, and
/// connections meanwhile.
struct Reading {
    pairs: HashSet<(u64, UserId)>,
    /// The viewer and user pairs [`Decisions`] knew to be visible.
    known: HashSet<(UserId, UserId)>,
    /// Those it decided, and when it began.
    decided: Vec<(UserId, UserId)>,
    began: Instant,
    read: crate::Result<Read>,
}

/// Applies a connection's registration, answering `false` when the feed is gone.
fn register(router: &mut Router, registration: Option<Registration>) -> bool {
    match registration {
        Some(Registration::Add {
            id,
            viewer,
            deliveries,
        }) => router.add(id, viewer, deliveries),
        Some(Registration::Remove(id)) => router.remove(id),
        None => return false,
    }
    true
}

/// This server's router: keeps who watches whom, gathers hints, and tells. Each telling's read
/// says how long what makes each user's presence has left, and a timer for each is kept here
/// (`expiries`), so the hint when it runs out is made wherever the user is watched.
async fn route_presence(
    nats: async_nats::Client,
    valkey: fred::clients::Client,
    pool: Pool,
    expiries: mpsc::Sender<(UserId, Expiry, Instant)>,
    mut registrations: mpsc::UnboundedReceiver<Registration>,
    lists: Arc<Mutex<HashMap<u64, Vec<UserId>>>>,
    listed: Arc<Notify>,
) {
    let mut hints = subscribe_hints(&nats).await;
    let mut router = Router::default();
    let mut reading: Option<tokio::task::JoinHandle<Reading>> = None;
    loop {
        // One telling at a time: the next waits for this one's read.
        let tell_at = router.tell_at.filter(|_| reading.is_none());
        tokio::select! {
            registration = registrations.recv() => {
                // The feed is gone: the server is stopping.
                if !register(&mut router, registration) {
                    return;
                }
            },
            () = listed.notified() => {
                // A connection is registered before it names a list, so taking the
                // registrations first finds every list's connection.
                while let Ok(registration) = registrations.try_recv() {
                    register(&mut router, Some(registration));
                }
                let named = std::mem::take(
                    &mut *lists.lock().expect("the presence watch lists' lock"),
                );
                for (id, users) in named {
                    router.named(id, users);
                }
            },
            done = async {
                match &mut reading {
                    Some(task) => task.await,
                    None => std::future::pending().await,
                }
            } => {
                reading = None;
                let Reading { pairs, mut known, decided, began, read } = match done {
                    Ok(done) => done,
                    Err(e) => {
                        error!("reading presence for its watchers failed: {e}");
                        continue;
                    }
                };
                match read {
                    Ok((statuses, runs_out, visible)) => {
                        for (user, kind, left) in runs_out {
                            let _ = expiries.try_send((user, kind, expiry_at(left)));
                        }
                        router.decisions.note(&decided, &visible, began);
                        known.extend(visible);
                        router.tell(pairs, &statuses, &known);
                    }
                    Err(e) => {
                        warn!(error = %e, "presence could not be read to tell its watchers");
                        router.fresh.extend(pairs);
                        router.due();
                    }
                }
            },
            message = hints.next() => {
                let Some(message) = message else {
                    error!("the presence hint subscription ended, subscribing again");
                    hints = subscribe_hints(&nats).await;
                    continue;
                };
                match serde_json::from_slice::<Vec<UserId>>(&message.payload) {
                    Ok(users) => router.hinted(users),
                    Err(e) => warn!("a presence hint could not be read: {e}"),
                }
            }
            () = async {
                match tell_at {
                    Some(at) => tokio::time::sleep_until(at).await,
                    None => std::future::pending().await,
                }
            } => {
                let pairs = router.gathered();
                if pairs.is_empty() {
                    continue;
                }
                let users: Vec<UserId> = pairs
                    .iter()
                    .map(|(_, user)| *user)
                    .collect::<HashSet<_>>()
                    .into_iter()
                    .collect();
                let (known, decided) = router.decisions.split(router.viewer_pairs(&pairs));
                let (valkey, pool) = (valkey.clone(), pool.clone());
                reading = Some(tokio::spawn(async move {
                    let began = Instant::now();
                    let read = read(&valkey, &pool, users, decided.clone()).await;
                    Reading { pairs, known, decided, began, read }
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(n: u128) -> UserId {
        UserId(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn an_expiry_set_again_replaces_the_last() {
        let mut expiries = Expiries::default();
        let now = Instant::now();
        expiries.set(user(1), Expiry::Activity, now + Duration::from_secs(5));
        expiries.set(user(1), Expiry::Activity, now + Duration::from_secs(10));
        expiries.set(user(2), Expiry::Connection, now + Duration::from_secs(7));
        assert_eq!(expiries.next(), Some(now + Duration::from_secs(7)));
        assert!(expiries.take_due(now + Duration::from_secs(6)).is_empty());
        assert_eq!(
            expiries.take_due(now + Duration::from_secs(7)),
            HashSet::from([user(2)])
        );
        assert_eq!(
            expiries.take_due(now + Duration::from_secs(10)),
            HashSet::from([user(1)])
        );
        assert_eq!(expiries.next(), None);
    }

    /// The next `presence` frame waiting for a connection, past any `presenceWatching`.
    fn next_told(received: &mut mpsc::Receiver<EphemeralEvent>) -> Option<Vec<UserStatusRecord>> {
        loop {
            match received.try_recv().ok()? {
                EphemeralEvent::Presence { statuses } => return Some(statuses),
                EphemeralEvent::PresenceWatching => continue,
                EphemeralEvent::Typing { .. }
                | EphemeralEvent::Mana { .. }
                | EphemeralEvent::PluginCatalogueChanged => {
                    panic!("a relay from the presence router")
                }
            }
        }
    }

    #[test]
    fn each_watch_list_taken_up_is_acknowledged_even_past_a_full_queue() {
        let (mut router, mut received) = router_with(&[(0, user(1))]);
        router.named(0, vec![user(2)]);
        let _ = router.gathered();
        assert_eq!(
            received[0].try_recv().unwrap(),
            EphemeralEvent::PresenceWatching
        );
        assert!(received[0].try_recv().is_err());
        // A queue full of frames: the acknowledgement waits for the next telling.
        for _ in 0..DELIVERY_QUEUE {
            router.watchers[&0]
                .deliveries
                .try_send(EphemeralEvent::Presence { statuses: vec![] })
                .unwrap();
        }
        router.named(0, vec![user(3)]);
        let _ = router.gathered();
        assert!(router.tell_at.is_some());
        while received[0].try_recv().is_ok() {}
        let _ = router.gathered();
        assert_eq!(
            received[0].try_recv().unwrap(),
            EphemeralEvent::PresenceWatching
        );
        assert!(router.unacknowledged.is_empty());
    }

    fn router_with(viewers: &[(u64, UserId)]) -> (Router, Vec<mpsc::Receiver<EphemeralEvent>>) {
        let mut router = Router::default();
        let mut received = Vec::new();
        for (id, viewer) in viewers {
            let (deliveries, rx) = mpsc::channel(DELIVERY_QUEUE);
            router.add(*id, *viewer, deliveries);
            received.push(rx);
        }
        (router, received)
    }

    #[tokio::test]
    async fn watchers_are_told_what_changed_as_they_may_learn_it() {
        let (alice, bob, carol) = (user(1), user(2), user(3));
        let (mut router, mut received) = router_with(&[(0, alice), (1, bob)]);
        // Their first lists, whose users their clients read whole.
        router.named(0, vec![]);
        router.named(1, vec![]);
        assert!(router.gathered().is_empty());
        router.named(0, vec![bob, carol, alice]);
        router.named(1, vec![alice]);
        let pairs = router.gathered();
        assert_eq!(pairs.len(), 4);
        // Alice blocked Bob: Bob may not learn hers. Alice may learn Bob's, not Carol's.
        let visible = HashSet::from([(alice, bob)]);
        let statuses = HashMap::from([
            (alice, UserOnlineStatus::Invisible),
            (bob, UserOnlineStatus::DoNotDisturb),
            (carol, UserOnlineStatus::Online),
        ]);
        router.tell(pairs, &statuses, &visible);
        let mut told = next_told(&mut received[0]).unwrap();
        told.sort_by_key(|s| s.id);
        assert_eq!(
            told,
            vec![
                UserStatusRecord {
                    id: alice,
                    online_status: UserOnlineStatus::Invisible
                },
                UserStatusRecord {
                    id: bob,
                    online_status: UserOnlineStatus::DoNotDisturb
                },
                UserStatusRecord {
                    id: carol,
                    online_status: UserOnlineStatus::Offline
                },
            ]
        );
        assert_eq!(
            next_told(&mut received[1]).unwrap(),
            vec![UserStatusRecord {
                id: alice,
                online_status: UserOnlineStatus::Offline
            }]
        );

        // A hint that changes nothing tells nobody; one that does tells only what changed.
        router.hinted(vec![alice, bob]);
        let pairs = router.gathered();
        let statuses = HashMap::from([
            (alice, UserOnlineStatus::Invisible),
            (bob, UserOnlineStatus::Away),
        ]);
        router.tell(pairs, &statuses, &visible);
        assert_eq!(
            next_told(&mut received[0]).unwrap(),
            vec![UserStatusRecord {
                id: bob,
                online_status: UserOnlineStatus::Away
            }]
        );
        assert!(next_told(&mut received[1]).is_none());
    }

    #[test]
    fn a_telling_decides_at_most_its_share_changes_first() {
        let viewers: Vec<(u64, UserId)> = (0..MAX_PAIRS_PER_TELLING as u64 / 100 + 1)
            .map(|id| (id, user(1_000_000 + u128::from(id))))
            .collect();
        let (mut router, _received) = router_with(&viewers);
        let known = user(999);
        for (id, _) in &viewers {
            router.named(*id, if *id == 0 { vec![known] } else { vec![] });
        }
        let _ = router.gathered();
        // A crowd's watch lists, and a change to someone already watched, in one window.
        for (id, _) in &viewers {
            let mut watched: Vec<UserId> = (0..100).map(user).collect();
            if *id == 0 {
                watched.push(known);
            }
            router.named(*id, watched);
        }
        router.hinted(vec![known]);
        let first = router.gathered();
        assert_eq!(first.len(), MAX_PAIRS_PER_TELLING);
        assert!(first.contains(&(0, known)));
        assert_eq!(
            router.fresh.len(),
            viewers.len() * 100 + 1 - MAX_PAIRS_PER_TELLING
        );
        assert!(router.tell_at.is_some());
        let second = router.gathered();
        assert_eq!(
            second.len(),
            viewers.len() * 100 + 1 - MAX_PAIRS_PER_TELLING
        );
        assert!(first.is_disjoint(&second));
        assert!(router.fresh.is_empty());
    }

    #[tokio::test]
    async fn a_connections_first_list_is_told_only_of_changes() {
        let (alice, bob, carol) = (user(1), user(2), user(3));
        let (mut router, mut received) = router_with(&[(0, alice)]);
        router.named(0, vec![bob]);
        assert!(router.gathered().is_empty());
        let visible = HashSet::from([(alice, bob), (alice, carol)]);
        let online = |users: &[UserId]| {
            users
                .iter()
                .map(|user| (*user, UserOnlineStatus::Online))
                .collect::<HashMap<_, _>>()
        };
        router.hinted(vec![bob]);
        let pairs = router.gathered();
        router.tell(pairs, &online(&[bob]), &visible);
        assert_eq!(
            next_told(&mut received[0]).unwrap(),
            vec![UserStatusRecord {
                id: bob,
                online_status: UserOnlineStatus::Online
            }]
        );
        // A user a later list adds is told as they are.
        router.named(0, vec![bob, carol]);
        let pairs = router.gathered();
        assert_eq!(pairs, HashSet::from([(0, carol)]));
        router.tell(pairs, &online(&[bob, carol]), &visible);
        assert_eq!(
            next_told(&mut received[0]).unwrap(),
            vec![UserStatusRecord {
                id: carol,
                online_status: UserOnlineStatus::Online
            }]
        );
    }

    #[test]
    fn later_lists_add_users_within_the_budget() {
        let (mut router, mut received) = router_with(&[(0, user(1))]);
        router.named(0, vec![]);
        let _ = router.gathered();
        while received[0].try_recv().is_ok() {}
        let first: Vec<UserId> = (100..100 + MAX_WATCHED_PRESENCE as u128)
            .map(user)
            .collect();
        router.named(0, first);
        assert_eq!(router.gathered().len(), MAX_WATCHED_PRESENCE);
        assert_eq!(
            received[0].try_recv().unwrap(),
            EphemeralEvent::PresenceWatching
        );
        // Another whole list at once: the budget is spent, so it waits, unanswered.
        let second: Vec<UserId> = (10_000..10_000 + MAX_WATCHED_PRESENCE as u128)
            .map(user)
            .collect();
        router.named(0, second.clone());
        let pairs = router.gathered();
        assert!(pairs.len() < 10, "{} added", pairs.len());
        assert!(received[0].try_recv().is_err());
        assert!(router.watchers[&0].next.is_some());
        assert!(router.tell_at.is_some());
        // The users it kept are those it names first.
        for (_, added) in &pairs {
            assert!(second[..10].contains(added));
        }
    }

    #[test]
    fn a_decision_kept_is_used_until_its_user_is_hinted() {
        let decisions = Decisions::default();
        let (alice, bob, carol) = (user(1), user(2), user(3));
        let began = Instant::now();
        decisions.note(
            &[(alice, bob), (alice, carol)],
            &HashSet::from([(alice, bob)]),
            began,
        );
        let (visible, undecided) =
            decisions.split(vec![(alice, bob), (alice, carol), (bob, alice)]);
        assert_eq!(visible, HashSet::from([(alice, bob)]));
        assert_eq!(undecided, vec![(bob, alice)]);
        decisions.hinted(bob, began + Duration::from_millis(1));
        let (visible, undecided) = decisions.split(vec![(alice, bob), (alice, carol)]);
        assert!(visible.is_empty());
        assert_eq!(undecided, vec![(alice, bob)]);
    }

    #[test]
    fn a_hint_for_someone_nobody_watches_waits_for_nothing() {
        let (mut router, _received) = router_with(&[(0, user(1))]);
        router.hinted(vec![user(9)]);
        assert!(router.tell_at.is_none());
        router.named(0, vec![user(9)]);
        let _ = router.gathered();
        router.hinted(vec![user(9)]);
        assert!(router.tell_at.is_some());
    }

    #[test]
    fn leaving_or_unwatching_stops_the_telling() {
        let (mut router, _received) = router_with(&[(0, user(1)), (1, user(2))]);
        router.named(0, vec![user(3)]);
        router.named(1, vec![user(3)]);
        let _ = router.gathered();
        router.named(0, vec![]);
        let _ = router.gathered();
        assert_eq!(router.by_user.get(&user(3)), Some(&HashSet::from([1])));
        router.remove(1);
        assert!(router.by_user.is_empty());
    }
}
