//! Telling event stream connections how many are online in the channels they view (`viewing`)
//! as it changes, in `channelOnline` frames, so a count shown stays current without being read
//! again.
//!
//! One router task per API server keeps which of its connections view which channels. It reads
//! the presence hints every server publishes (`app::presence_feed`), and every [`WINDOW`] finds
//! the communities viewed here that a hinted user belongs to, in one query, and the DMs viewed
//! here that one is a recipient of. Each such community's online members are read again
//! (`CommunityOnline`, which the counts read over REST then reuse), and each connection viewing
//! one of its channels is told its count, as the REST read counts it for its reader, when it
//! differs from what the connection was last told. So a community is read at most once a window
//! on each server where someone views it, however many view it and however often its members'
//! presence changes.
//!
//! Each counting checks again that every reader may view what they are told of, in one query
//! per community, so one who loses access is told nothing more of it. What changes a count but
//! no one's presence (a member joining or leaving, a role or an override) is not told; clients
//! read the count again now and then.

use super::{CommunityOnline, hidden_with, renew_community_online};
use crate::context::GlobalServerContext;
use crate::events::{ChannelHome, channel_home, dm_recipients};
use crate::{ChannelId, CommunityId, UserId};
use aspen_schema::{community_user, dm_recipient, user_block};
use aspen_wire::ephemeral::EphemeralEvent;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use futures_util::StreamExt;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{error, warn};

/// How long hints are gathered before what they may change is counted again: a community
/// viewed here is read at most once in it.
const WINDOW: Duration = Duration::from_secs(2);

/// The most hinted users held between countings; past it every community viewed here is
/// counted at the next, as though each had a hinted member.
const MAX_HINTED: usize = 65_536;

/// How many hinted users one query looks for among the members of the communities viewed here.
const MEMBERSHIP_BATCH: usize = 5_000;

/// The most communities counted in one window; the rest are counted in the next.
const COMMUNITIES_PER_WINDOW: usize = 500;

/// How many communities are counted at once, each holding a database connection meanwhile.
const AT_ONCE: usize = 4;

/// How many viewer and recipient pairs one visibility query decides.
const PAIR_BATCH: usize = 5_000;

/// How many frames may wait for one connection; past it the connection is told at the next
/// change.
const DELIVERY_QUEUE: usize = 4;

/// How long a channel a connection looked up is not looked up again. The connection takes at
/// most one `viewing` list a second (`api::event_stream`), so a client switching between lists
/// costs at most `event_feed::MAX_VIEWING` lookups in this time.
const LOOKED_UP_FOR: Duration = Duration::from_secs(5);

/// The server's end of telling online counts; its router starts with the first connection.
#[derive(Default)]
pub struct OnlineFeed {
    router: OnceLock<RouterHandle>,
}

#[derive(Clone)]
struct RouterHandle {
    registrations: mpsc::UnboundedSender<Registration>,
    next_id: Arc<AtomicU64>,
}

enum Registration {
    Add {
        id: u64,
        reader: UserId,
        deliveries: mpsc::Sender<EphemeralEvent>,
    },
    /// What the connection views now, replacing what it viewed.
    View {
        id: u64,
        viewed: Vec<Viewed>,
    },
    Remove(u64),
}

/// A channel a connection views, and where its count comes from.
#[derive(Clone)]
struct Viewed {
    channel: ChannelId,
    place: Place,
}

#[derive(Clone)]
enum Place {
    /// A community channel, or a thread of one, counted as `governing`.
    Community {
        community: CommunityId,
        governing: ChannelId,
    },
    /// A DM or group DM, or a thread of one, and its recipients as it was looked up, by which
    /// hints find it; its count reads them again.
    Direct {
        dm: ChannelId,
        recipients: Vec<UserId>,
    },
}

impl OnlineFeed {
    /// Registers an event stream connection of `reader`'s, which then says what it views.
    pub fn watch(&self, state: &GlobalServerContext, reader: UserId) -> OnlineWatch {
        let router = self.router.get_or_init(|| {
            let (registrations, registered) = mpsc::unbounded_channel();
            tokio::spawn(route(state.clone(), registered));
            RouterHandle {
                registrations,
                next_id: Arc::new(AtomicU64::new(0)),
            }
        });
        let id = router.next_id.fetch_add(1, Ordering::Relaxed);
        let (deliveries, received) = mpsc::channel(DELIVERY_QUEUE);
        let _ = router.registrations.send(Registration::Add {
            id,
            reader,
            deliveries,
        });
        // A list waiting is replaced by the next; a client sends one as its view changes.
        let (lists, incoming) = mpsc::channel(2);
        tokio::spawn(look_up(
            state.clone(),
            reader,
            id,
            router.registrations.clone(),
            incoming,
        ));
        OnlineWatch {
            id,
            lists,
            received,
            registrations: router.registrations.clone(),
        }
    }
}

/// One event stream connection's watch on the counts of what it views. Dropping it ends the
/// watch.
pub struct OnlineWatch {
    id: u64,
    lists: mpsc::Sender<Vec<ChannelId>>,
    received: mpsc::Receiver<EphemeralEvent>,
    registrations: mpsc::UnboundedSender<Registration>,
}

impl OnlineWatch {
    /// The channels the connection's client views now (its `viewing` frame), cut as the event
    /// feed cuts them.
    pub fn viewing(&self, channels: Vec<ChannelId>) {
        let _ = self.lists.try_send(channels);
    }

    /// The next `channelOnline` to tell the connection of.
    pub async fn next(&mut self) -> Option<EphemeralEvent> {
        self.received.recv().await
    }
}

impl Drop for OnlineWatch {
    fn drop(&mut self) {
        let _ = self.registrations.send(Registration::Remove(self.id));
    }
}

/// One connection's task finding where each channel it views is counted from, among those its
/// reader may view, and handing that to the router.
async fn look_up(
    state: GlobalServerContext,
    reader: UserId,
    id: u64,
    registrations: mpsc::UnboundedSender<Registration>,
    mut lists: mpsc::Receiver<Vec<ChannelId>>,
) {
    let mut known: HashMap<ChannelId, (Instant, Option<Place>)> = HashMap::new();
    while let Some(list) = lists.recv().await {
        let at = Instant::now();
        known.retain(|_, (when, _)| at.duration_since(*when) < LOOKED_UP_FOR);
        let mut viewed = Vec::new();
        for channel in list {
            let place = match known.get(&channel) {
                Some((_, place)) => place.clone(),
                None => {
                    let place = place_of(&state, reader, channel).await;
                    known.insert(channel, (at, place.clone()));
                    place
                }
            };
            if let Some(place) = place {
                viewed.push(Viewed { channel, place });
            }
        }
        if registrations
            .send(Registration::View { id, viewed })
            .is_err()
        {
            return;
        }
    }
}

/// Where `channel`'s count comes from, when `reader` may view it.
async fn place_of(
    state: &GlobalServerContext,
    reader: UserId,
    channel: ChannelId,
) -> Option<Place> {
    let mut conn = state.connection_pool.get().await.ok()?;
    crate::permissions::channel_access(state, conn.as_mut(), reader, channel)
        .await
        .ok()?;
    match channel_home(state, conn.as_mut(), channel).await.ok()? {
        ChannelHome::Community {
            community,
            governing,
        } => Some(Place::Community {
            community,
            governing,
        }),
        ChannelHome::Direct(dm) => Some(Place::Direct {
            dm,
            recipients: dm_recipients(conn.as_mut(), dm).await.ok()?,
        }),
    }
}

struct Connection {
    reader: UserId,
    deliveries: mpsc::Sender<EphemeralEvent>,
    viewed: Vec<Viewed>,
    /// The count each channel it views was last told as.
    told: HashMap<ChannelId, u32>,
}

/// A count to tell one connection of a channel it views.
struct Told {
    connection: u64,
    channel: ChannelId,
    online: u32,
}

/// One connection's view of a community channel, to count.
struct Reading {
    connection: u64,
    reader: UserId,
    channel: ChannelId,
    governing: ChannelId,
}

/// One connection's view of a DM, to count.
struct DirectReading {
    connection: u64,
    reader: UserId,
    channel: ChannelId,
    dm: ChannelId,
}

/// What a counting reads and counts.
struct Counting {
    communities: Vec<(CommunityId, Vec<Reading>)>,
    direct: Vec<DirectReading>,
}

/// What the router's task in flight found.
enum Outcome {
    /// The communities viewed here that a hinted user belongs to.
    Found(HashSet<CommunityId>),
    Counted(Vec<Told>),
}

#[derive(Default)]
struct Router {
    connections: HashMap<u64, Connection>,
    /// The connections viewing a channel of each community.
    by_community: HashMap<CommunityId, HashSet<u64>>,
    /// The connections viewing a DM each user is a recipient of.
    by_recipient: HashMap<UserId, HashSet<u64>>,
    /// Users hinted since the communities they belong to were last looked for.
    hinted: HashSet<UserId>,
    /// More users were hinted than `MAX_HINTED`: every community viewed here is to be counted.
    overflowed: bool,
    /// Communities to count, found or carried over from a full window.
    due: HashSet<CommunityId>,
    /// Connections whose DMs are to be counted.
    due_direct: HashSet<u64>,
}

impl Router {
    fn register(&mut self, registration: Registration) {
        match registration {
            Registration::Add {
                id,
                reader,
                deliveries,
            } => {
                self.connections.insert(
                    id,
                    Connection {
                        reader,
                        deliveries,
                        viewed: Vec::new(),
                        told: HashMap::new(),
                    },
                );
            }
            Registration::View { id, viewed } => {
                if !self.connections.contains_key(&id) {
                    return;
                }
                self.unindex(id);
                let connection = self.connections.get_mut(&id).expect("checked above");
                let channels: HashSet<ChannelId> = viewed.iter().map(|v| v.channel).collect();
                connection
                    .told
                    .retain(|channel, _| channels.contains(channel));
                connection.viewed = viewed;
                self.index(id);
            }
            Registration::Remove(id) => {
                self.unindex(id);
                self.connections.remove(&id);
                self.due_direct.remove(&id);
            }
        }
    }

    fn index(&mut self, id: u64) {
        let Some(connection) = self.connections.get(&id) else {
            return;
        };
        for viewed in &connection.viewed {
            match &viewed.place {
                Place::Community { community, .. } => {
                    self.by_community.entry(*community).or_default().insert(id);
                }
                Place::Direct { recipients, .. } => {
                    for recipient in recipients {
                        self.by_recipient.entry(*recipient).or_default().insert(id);
                    }
                }
            }
        }
    }

    fn unindex(&mut self, id: u64) {
        let Some(connection) = self.connections.get(&id) else {
            return;
        };
        for viewed in &connection.viewed {
            match &viewed.place {
                Place::Community { community, .. } => {
                    if let Some(ids) = self.by_community.get_mut(community) {
                        ids.remove(&id);
                        if ids.is_empty() {
                            self.by_community.remove(community);
                        }
                    }
                }
                Place::Direct { recipients, .. } => {
                    for recipient in recipients {
                        if let Some(ids) = self.by_recipient.get_mut(recipient) {
                            ids.remove(&id);
                            if ids.is_empty() {
                                self.by_recipient.remove(recipient);
                            }
                        }
                    }
                }
            }
        }
    }

    fn hint(&mut self, users: Vec<UserId>) {
        for user in users {
            if let Some(ids) = self.by_recipient.get(&user) {
                self.due_direct.extend(ids);
            }
            if self.overflowed {
                continue;
            }
            if self.hinted.len() >= MAX_HINTED {
                self.overflowed = true;
                self.hinted.clear();
                continue;
            }
            self.hinted.insert(user);
        }
    }

    /// The hinted users and the communities viewed here to look for them in, when any were
    /// hinted; every community viewed here is due at once when too many were.
    fn take_hinted(&mut self) -> Option<(Vec<UserId>, Vec<CommunityId>)> {
        if self.overflowed {
            self.overflowed = false;
            self.due.extend(self.by_community.keys());
            return None;
        }
        if self.hinted.is_empty() || self.by_community.is_empty() {
            self.hinted.clear();
            return None;
        }
        Some((
            self.hinted.drain().collect(),
            self.by_community.keys().copied().collect(),
        ))
    }

    /// What is due now, at most `COMMUNITIES_PER_WINDOW` communities, the rest left due.
    fn take_counting(&mut self) -> Option<Counting> {
        self.due
            .retain(|community| self.by_community.contains_key(community));
        let communities: Vec<CommunityId> = self
            .due
            .iter()
            .copied()
            .take(COMMUNITIES_PER_WINDOW)
            .collect();
        for community in &communities {
            self.due.remove(community);
        }
        let communities: Vec<(CommunityId, Vec<Reading>)> = communities
            .into_iter()
            .map(|community| {
                let readings = self.by_community[&community]
                    .iter()
                    .flat_map(|id| {
                        let connection = &self.connections[id];
                        connection
                            .viewed
                            .iter()
                            .filter_map(move |viewed| match viewed.place {
                                Place::Community {
                                    community: of,
                                    governing,
                                } if of == community => Some(Reading {
                                    connection: *id,
                                    reader: connection.reader,
                                    channel: viewed.channel,
                                    governing,
                                }),
                                _ => None,
                            })
                    })
                    .collect();
                (community, readings)
            })
            .collect();
        let direct: Vec<DirectReading> = self
            .due_direct
            .drain()
            .filter_map(|id| self.connections.get(&id).map(|c| (id, c)))
            .flat_map(|(id, connection)| {
                connection
                    .viewed
                    .iter()
                    .filter_map(move |viewed| match viewed.place {
                        Place::Direct { dm, .. } => Some(DirectReading {
                            connection: id,
                            reader: connection.reader,
                            channel: viewed.channel,
                            dm,
                        }),
                        Place::Community { .. } => None,
                    })
            })
            .collect();
        (!communities.is_empty() || !direct.is_empty()).then_some(Counting {
            communities,
            direct,
        })
    }

    /// Tells each connection still viewing the channel its count, where it differs from what
    /// it was last told.
    fn tell(&mut self, counts: Vec<Told>) {
        for Told {
            connection,
            channel,
            online,
        } in counts
        {
            let Some(connection) = self.connections.get_mut(&connection) else {
                continue;
            };
            if !connection.viewed.iter().any(|v| v.channel == channel)
                || connection.told.get(&channel) == Some(&online)
            {
                continue;
            }
            let event = EphemeralEvent::ChannelOnline {
                channel_id: channel,
                online,
            };
            if connection.deliveries.try_send(event).is_ok() {
                connection.told.insert(channel, online);
            }
        }
    }
}

async fn route(
    state: GlobalServerContext,
    mut registrations: mpsc::UnboundedReceiver<Registration>,
) {
    let nats = state.nats_context.client();
    let mut hints = crate::presence_feed::subscribe_hints(&nats).await;
    let mut router = Router::default();
    let mut window = tokio::time::interval(WINDOW);
    window.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut working: Option<tokio::task::JoinHandle<crate::Result<Outcome>>> = None;
    loop {
        tokio::select! {
            registration = registrations.recv() => {
                // The feed is gone: the server is stopping.
                let Some(registration) = registration else {
                    return;
                };
                router.register(registration);
            },
            message = hints.next() => {
                let Some(message) = message else {
                    error!("the presence hint subscription ended, subscribing again");
                    hints = crate::presence_feed::subscribe_hints(&nats).await;
                    continue;
                };
                match serde_json::from_slice::<Vec<UserId>>(&message.payload) {
                    Ok(users) => router.hint(users),
                    Err(e) => warn!("a presence hint could not be read: {e}"),
                }
            },
            _ = window.tick(), if working.is_none() => {
                if let Some((users, communities)) = router.take_hinted() {
                    let state = state.clone();
                    working = Some(tokio::spawn(async move {
                        find(&state, users, communities).await.map(Outcome::Found)
                    }));
                } else if let Some(counting) = router.take_counting() {
                    let state = state.clone();
                    working = Some(tokio::spawn(async move {
                        Ok(Outcome::Counted(count(&state, counting).await))
                    }));
                }
            },
            done = async { working.as_mut().expect("guarded").await }, if working.is_some() => {
                working = None;
                match done {
                    Ok(Ok(Outcome::Found(communities))) => {
                        router.due.extend(communities);
                        // Counted at once rather than a window later; the next look for
                        // hinted users still waits for the window.
                        if let Some(counting) = router.take_counting() {
                            let state = state.clone();
                            working = Some(tokio::spawn(async move {
                                Ok(Outcome::Counted(count(&state, counting).await))
                            }));
                        }
                    }
                    Ok(Ok(Outcome::Counted(counts))) => router.tell(counts),
                    Ok(Err(e)) => warn!(error = %e, "could not find the communities of presence hints"),
                    Err(e) => error!("counting who is online in viewed channels failed: {e}"),
                }
            },
        }
    }
}

/// Which of `communities` any of `users` belongs to.
async fn find(
    state: &GlobalServerContext,
    users: Vec<UserId>,
    communities: Vec<CommunityId>,
) -> crate::Result<HashSet<CommunityId>> {
    let mut conn = state.connection_pool.get().await?;
    let mut found = HashSet::new();
    for batch in users.chunks(MEMBERSHIP_BATCH) {
        let of: Vec<CommunityId> = community_user::table
            .select(community_user::community)
            .distinct()
            .filter(community_user::community.eq_any(&communities))
            .filter(community_user::user.eq_any(batch))
            .load(conn.as_mut())
            .await?;
        found.extend(of);
    }
    Ok(found)
}

/// Each reading's count; a community or the DMs that cannot be counted are left out, to be
/// counted at the next change.
async fn count(state: &GlobalServerContext, counting: Counting) -> Vec<Told> {
    let mut told: Vec<Told> = futures_util::stream::iter(counting.communities)
        .map(|(community, readings)| async move {
            count_community(state, community, readings)
                .await
                .unwrap_or_else(|e| {
                    warn!(error = %e, "could not count who is online in a community's channels");
                    Vec::new()
                })
        })
        .buffer_unordered(AT_ONCE)
        .flat_map(futures_util::stream::iter)
        .collect()
        .await;
    match count_direct(state, counting.direct).await {
        Ok(direct) => told.extend(direct),
        Err(e) => warn!(error = %e, "could not count who is online in DMs"),
    }
    told
}

async fn count_community(
    state: &GlobalServerContext,
    community: CommunityId,
    readings: Vec<Reading>,
) -> crate::Result<Vec<Told>> {
    let online: Arc<CommunityOnline> = renew_community_online(state, community).await?;
    let mut conn = state.connection_pool.get().await?;
    let pairs: Vec<(UserId, ChannelId)> = readings
        .iter()
        .map(|reading| (reading.reader, reading.governing))
        .collect();
    let allowed = crate::visibility::may_view(conn.as_mut(), community, &pairs).await?;
    let readers: Vec<UserId> = pairs.iter().map(|(reader, _)| *reader).collect();
    let listed: Vec<UserId> = online.online.iter().copied().collect();
    let mut blockers: HashMap<UserId, HashSet<UserId>> = HashMap::new();
    if !listed.is_empty() {
        for (blocked, blocker) in user_block::table
            .select((user_block::blocked, user_block::blocker))
            .filter(user_block::blocked.eq_any(&readers))
            .filter(user_block::blocker.eq_any(&listed))
            .load::<(UserId, UserId)>(conn.as_mut())
            .await?
        {
            if blocked != blocker {
                blockers.entry(blocked).or_default().insert(blocker);
            }
        }
    }
    let mut told = Vec::with_capacity(readings.len());
    for reading in readings {
        if !allowed.contains(&(reading.reader, reading.governing)) {
            continue;
        }
        let hidden = match blockers.get(&reading.reader) {
            Some(theirs) => {
                hidden_with(conn.as_mut(), community, reading.governing, theirs).await?
            }
            None => 0,
        };
        told.push(Told {
            connection: reading.connection,
            channel: reading.channel,
            online: online.count_for(reading.reader, reading.governing, hidden),
        });
    }
    Ok(told)
}

/// Each DM reading's count, from its recipients read again: a reader no longer among them is
/// told nothing.
async fn count_direct(
    state: &GlobalServerContext,
    readings: Vec<DirectReading>,
) -> crate::Result<Vec<Told>> {
    if readings.is_empty() {
        return Ok(Vec::new());
    }
    let dms: Vec<ChannelId> = readings
        .iter()
        .map(|r| r.dm)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let mut conn = state.connection_pool.get().await?;
    let mut recipients: HashMap<ChannelId, Vec<UserId>> = HashMap::new();
    for (dm, user) in dm_recipient::table
        .select((dm_recipient::channel, dm_recipient::user))
        .filter(dm_recipient::channel.eq_any(&dms))
        .load::<(ChannelId, UserId)>(conn.as_mut())
        .await?
    {
        recipients.entry(dm).or_default().push(user);
    }
    let readings: Vec<(DirectReading, &Vec<UserId>)> = readings
        .into_iter()
        .filter_map(|reading| {
            let of = recipients.get(&reading.dm)?;
            of.contains(&reading.reader).then_some((reading, of))
        })
        .collect();
    let pairs: Vec<(UserId, UserId)> = readings
        .iter()
        .flat_map(|(reading, of)| {
            of.iter()
                .filter(|user| **user != reading.reader)
                .map(|user| (reading.reader, *user))
        })
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let mut visible = HashSet::new();
    for batch in pairs.chunks(PAIR_BATCH) {
        visible.extend(crate::user_status::presence_visible_pairs(conn.as_mut(), batch).await?);
    }
    drop(conn);
    let asked: Vec<UserId> = visible
        .iter()
        .map(|(_, user)| *user)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let online = crate::user_status::online_among(state, asked).await?;
    Ok(readings
        .into_iter()
        .map(|(reading, of)| {
            let others = of
                .iter()
                .filter(|user| {
                    **user != reading.reader
                        && visible.contains(&(reading.reader, **user))
                        && online.contains(user)
                })
                .count() as u32;
            Told {
                connection: reading.connection,
                channel: reading.channel,
                online: others + 1,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn user(n: u128) -> UserId {
        UserId(Uuid::from_u128(n))
    }

    fn channel(n: u128) -> ChannelId {
        ChannelId(Uuid::from_u128(n))
    }

    fn community(n: u128) -> CommunityId {
        CommunityId(Uuid::from_u128(n))
    }

    fn connected(router: &mut Router, id: u64, reader: UserId) -> mpsc::Receiver<EphemeralEvent> {
        let (deliveries, received) = mpsc::channel(DELIVERY_QUEUE);
        router.register(Registration::Add {
            id,
            reader,
            deliveries,
        });
        received
    }

    #[test]
    fn a_hinted_recipient_makes_their_dms_viewed_here_due() {
        let mut router = Router::default();
        let _received = connected(&mut router, 1, user(1));
        router.register(Registration::View {
            id: 1,
            viewed: vec![Viewed {
                channel: channel(10),
                place: Place::Direct {
                    dm: channel(10),
                    recipients: vec![user(1), user(2)],
                },
            }],
        });
        router.hint(vec![user(3)]);
        assert!(router.due_direct.is_empty());
        router.hint(vec![user(2)]);
        let counting = router.take_counting().expect("the DM is due");
        assert_eq!(counting.direct.len(), 1);
        assert_eq!(counting.direct[0].dm, channel(10));
    }

    #[test]
    fn a_view_replaced_leaves_nothing_of_the_last_indexed() {
        let mut router = Router::default();
        let _received = connected(&mut router, 1, user(1));
        let viewing = |id: u128| Viewed {
            channel: channel(id),
            place: Place::Community {
                community: community(id),
                governing: channel(id),
            },
        };
        router.register(Registration::View {
            id: 1,
            viewed: vec![viewing(10)],
        });
        router.register(Registration::View {
            id: 1,
            viewed: vec![viewing(20)],
        });
        assert!(!router.by_community.contains_key(&community(10)));
        assert!(router.by_community.contains_key(&community(20)));
        router.register(Registration::Remove(1));
        assert!(router.by_community.is_empty());
        assert!(router.connections.is_empty());
    }

    #[test]
    fn too_many_hints_make_every_viewed_community_due() {
        let mut router = Router::default();
        let _received = connected(&mut router, 1, user(1));
        router.register(Registration::View {
            id: 1,
            viewed: vec![Viewed {
                channel: channel(10),
                place: Place::Community {
                    community: community(10),
                    governing: channel(10),
                },
            }],
        });
        router.hint((0..=MAX_HINTED as u128).map(|n| user(1_000 + n)).collect());
        assert!(router.take_hinted().is_none());
        let counting = router.take_counting().expect("the community is due");
        assert_eq!(counting.communities.len(), 1);
        assert_eq!(counting.communities[0].1.len(), 1);
    }

    #[tokio::test]
    async fn a_count_is_told_once_until_it_changes() {
        let mut router = Router::default();
        let mut received = connected(&mut router, 1, user(1));
        router.register(Registration::View {
            id: 1,
            viewed: vec![Viewed {
                channel: channel(10),
                place: Place::Community {
                    community: community(10),
                    governing: channel(10),
                },
            }],
        });
        let told = |online| Told {
            connection: 1,
            channel: channel(10),
            online,
        };
        router.tell(vec![told(3)]);
        router.tell(vec![told(3)]);
        router.tell(vec![told(4)]);
        // A channel no longer viewed is told nothing.
        router.register(Registration::View {
            id: 1,
            viewed: Vec::new(),
        });
        router.tell(vec![told(5)]);
        let mut counts = Vec::new();
        while let Ok(event) = received.try_recv() {
            counts.push(event);
        }
        assert_eq!(
            counts,
            [3, 4].map(|online| EphemeralEvent::ChannelOnline {
                channel_id: channel(10),
                online,
            })
        );
    }
}
