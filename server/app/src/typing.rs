//! Who is typing where. Nothing of it is stored: each word that someone is typing, or has
//! stopped, is published once on the core NATS subject [`TYPING_SUBJECT`], outside the event
//! stream's subjects, so JetStream neither keeps nor replays it, and every API server's event
//! feed routes it to its own connections as it routes the stream's events
//! (`app::event_feed`), to those who may view the channel.
//!
//! An event stream connection says its user is typing with a `typing` frame, which
//! [`Typist`] checks as posting is checked (`message::may_post`) before publishing it, and
//! says they stopped with `stoppedTyping`. A connection that ends says on its way out that its
//! user stopped wherever it said they were typing, so a lost connection is let go of at once
//! rather than after `TYPING_EXPIRY_SECONDS`.

use crate::context::GlobalServerContext;
use crate::events::{ChannelHome, channel_home, dm_recipients};
use crate::{ChannelId, CommunityId, UserId};
use aspen_schema::user_block;
pub use aspen_wire::ephemeral::{EphemeralEvent, TYPING_EXPIRY_SECONDS, TYPING_REFRESH_SECONDS};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tracing::{debug, warn};

/// The core NATS subject every API server publishes typing on and reads it from. It lies outside
/// `events::SUBJECT_ROOT`, whose subjects JetStream keeps.
pub const TYPING_SUBJECT: &str = "aspen.typing";

/// The least time between two `typing` frames for one channel that are published, and between
/// two checks of one channel; a client sends them every `TYPING_REFRESH_SECONDS`, and closer
/// ones are dropped.
const REFRESH_FLOOR: Duration = Duration::from_secs(TYPING_REFRESH_SECONDS - 1);

/// How long someone is shown typing after the last word that they are.
const EXPIRY: Duration = Duration::from_secs(TYPING_EXPIRY_SECONDS);

/// The most channels one connection may say its user is typing in at once; a person types in
/// one, and a few more cover switching between them before the last ones expire.
const MAX_CHANNELS: usize = 4;

/// How many frames may wait for a connection's typist; more are dropped, since the next
/// refresh says the same.
const QUEUE: usize = 8;

/// Who a word about typing in a channel reaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Audience {
    /// The members of a community who may view `governing` (the channel, or a thread's parent).
    Community {
        community: CommunityId,
        governing: ChannelId,
    },
    /// The people of a DM or group DM, or of the DM a thread is in.
    Direct { recipients: Vec<UserId> },
}

/// What one API server tells the others on [`TYPING_SUBJECT`]: the event, who it reaches, and
/// who it does not although the audience holds them (the typist, and anyone they block, from
/// whom a block keeps when they are about as it keeps their presence).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Relay {
    pub event: EphemeralEvent,
    pub audience: Audience,
    pub unseen_by: Vec<UserId>,
}

enum Frame {
    Typing(ChannelId),
    Stopped(ChannelId),
}

/// One event stream connection's typing: what it says, checked and published in the order it
/// says it by a task of its own, so a slow check holds up neither the connection's events nor
/// its later frames' order. Dropping it (the connection ending) says its user stopped wherever
/// they were still shown typing.
pub struct Typist {
    frames: mpsc::Sender<Frame>,
}

impl Typist {
    pub fn spawn(state: GlobalServerContext, user: UserId) -> Self {
        let (frames, received) = mpsc::channel(QUEUE);
        tokio::spawn(run(state, user, received));
        Typist { frames }
    }

    /// The user is typing in `channel`.
    pub fn typing(&self, channel: ChannelId) {
        let _ = self.frames.try_send(Frame::Typing(channel));
    }

    /// The user stopped typing in `channel`.
    pub fn stopped(&self, channel: ChannelId) {
        let _ = self.frames.try_send(Frame::Stopped(channel));
    }
}

/// How many words a connection's typist may check or publish in a burst, and how many it earns
/// back each second ([`Budget`]). A person types in one channel, refreshing every
/// `TYPING_REFRESH_SECONDS`, so this leaves room for switching between a few; it bounds each
/// connection's database checks and the relays it sends every API server.
const BUDGET_BURST: f64 = 4.0;
const BUDGET_PER_SECOND: f64 = 2.0;

/// A token bucket over a connection's checks and publishes.
struct Budget {
    tokens: f64,
    at: Instant,
}

impl Budget {
    fn new(now: Instant) -> Self {
        Budget {
            tokens: BUDGET_BURST,
            at: now,
        }
    }

    /// Takes one token, answering whether there was one.
    fn take(&mut self, now: Instant) -> bool {
        let earned = now.duration_since(self.at).as_secs_f64() * BUDGET_PER_SECOND;
        self.tokens = (self.tokens + earned).min(BUDGET_BURST);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Who a word about one channel reaches, as a check found it.
#[derive(Clone)]
struct Found {
    audience: Audience,
    unseen_by: Vec<UserId>,
}

/// What a connection's typist knows of one channel it was told of. It outlives a stop and a
/// refusal, so neither lets the next `typing` check the channel again sooner than
/// [`REFRESH_FLOOR`]. An entry is made only by a check, which takes a token of the [`Budget`],
/// and is forgotten [`EXPIRY`] after it was last checked or shown, so a typist holds at most
/// what its budget lets it check in that time.
struct Known {
    /// When it was last checked.
    checked: Instant,
    /// Who the check found the word reaches; `None` when it refused.
    found: Option<Found>,
    /// When it last published that the user is typing there, while they are shown typing.
    shown: Option<Instant>,
}

impl Known {
    fn fresh(&self, now: Instant) -> bool {
        now.duration_since(self.checked) < REFRESH_FLOOR
    }

    fn last_used(&self) -> Instant {
        self.shown
            .map_or(self.checked, |shown| shown.max(self.checked))
    }
}

async fn run(state: GlobalServerContext, user: UserId, mut frames: mpsc::Receiver<Frame>) {
    let mut known: HashMap<ChannelId, Known> = HashMap::new();
    let mut budget = Budget::new(Instant::now());
    while let Some(frame) = frames.recv().await {
        let now = Instant::now();
        known.retain(|_, k| now.duration_since(k.last_used()) < EXPIRY);
        for k in known.values_mut() {
            if k.shown.is_some_and(|at| now.duration_since(at) >= EXPIRY) {
                k.shown = None;
            }
        }
        match frame {
            Frame::Typing(channel) => {
                let entry = known.get(&channel);
                if entry.is_some_and(|k| {
                    k.shown
                        .is_some_and(|at| now.duration_since(at) < REFRESH_FLOOR)
                        || (k.found.is_none() && k.fresh(now))
                }) {
                    continue;
                }
                let showing = entry.is_some_and(|k| k.shown.is_some());
                if !showing && known.values().filter(|k| k.shown.is_some()).count() >= MAX_CHANNELS
                {
                    continue;
                }
                if !budget.take(now) {
                    continue;
                }
                let cached = entry.filter(|k| k.fresh(now)).and_then(|k| k.found.clone());
                let found = match cached {
                    Some(found) => found,
                    None => {
                        let found = check(&state, user, channel).await;
                        let checked = Instant::now();
                        match found {
                            Some(found) => {
                                known.insert(
                                    channel,
                                    Known {
                                        checked,
                                        found: Some(found.clone()),
                                        shown: None,
                                    },
                                );
                                found
                            }
                            None => {
                                // Refused or failed. What was shown lapses by the expiry rather
                                // than being withdrawn: the audience was read at least a refresh
                                // ago, and may hold people who may no longer be told of the
                                // channel.
                                known.insert(
                                    channel,
                                    Known {
                                        checked,
                                        found: None,
                                        shown: None,
                                    },
                                );
                                continue;
                            }
                        }
                    }
                };
                publish(&state, user, channel, true, &found).await;
                if let Some(k) = known.get_mut(&channel) {
                    k.shown = Some(Instant::now());
                }
            }
            Frame::Stopped(channel) => {
                stop(&state, user, channel, &mut known, &mut budget).await;
            }
        }
    }
    // The connection ended.
    let shown: Vec<ChannelId> = known
        .iter()
        .filter(|(_, k)| k.shown.is_some())
        .map(|(channel, _)| *channel)
        .collect();
    for channel in shown {
        stop(&state, user, channel, &mut known, &mut budget).await;
    }
}

/// Says `user` stopped typing in `channel`, if they are shown typing there. The stop goes to the
/// audience the last check found when that check is fresh; otherwise the channel is checked again
/// when the budget allows and the stop goes to whom that finds, and when it does not, or refuses,
/// nothing is said and what was shown lapses by the expiry. So a stop never reaches people read
/// into the audience longer ago than [`REFRESH_FLOOR`].
async fn stop(
    state: &GlobalServerContext,
    user: UserId,
    channel: ChannelId,
    known: &mut HashMap<ChannelId, Known>,
    budget: &mut Budget,
) {
    let now = Instant::now();
    let Some(k) = known.get_mut(&channel) else {
        return;
    };
    if k.shown.take().is_none() {
        return;
    }
    let found = match k.found.clone().filter(|_| k.fresh(now)) {
        Some(found) => found,
        None => {
            if !budget.take(now) {
                return;
            }
            let found = check(state, user, channel).await;
            k.checked = Instant::now();
            k.found.clone_from(&found);
            match found {
                Some(found) => found,
                None => return,
            }
        }
    };
    publish(state, user, channel, false, &found).await;
}

/// Who sees `user` typing in `channel`, or `None` when they may not say so there or the check
/// failed. A check that takes longer than a refresh is overtaken by the next one.
async fn check(state: &GlobalServerContext, user: UserId, channel: ChannelId) -> Option<Found> {
    let checked = tokio::time::timeout(
        Duration::from_secs(TYPING_REFRESH_SECONDS),
        audience(state, user, channel),
    )
    .await;
    match checked {
        Ok(Ok((audience, unseen_by))) => Some(Found {
            audience,
            unseen_by,
        }),
        Ok(Err(e)) => {
            debug!(error = %e, "typing refused");
            None
        }
        Err(_) => {
            warn!("checking who sees someone typing took longer than a refresh");
            None
        }
    }
}

/// Who sees `user` typing in `channel`, once it is checked that they may post there.
async fn audience(
    state: &GlobalServerContext,
    user: UserId,
    channel: ChannelId,
) -> crate::Result<(Audience, Vec<UserId>)> {
    let mut conn = state.connection_pool.get().await?;
    crate::message::may_type(state, conn.as_mut(), user, channel).await?;
    let audience = match channel_home(state, conn.as_mut(), channel).await? {
        ChannelHome::Community {
            community,
            governing,
        } => Audience::Community {
            community,
            governing,
        },
        ChannelHome::Direct(dm) => Audience::Direct {
            recipients: dm_recipients(conn.as_mut(), dm).await?,
        },
    };
    let mut unseen_by = blocked_in(conn.as_mut(), user, &audience).await?;
    unseen_by.push(user);
    Ok((audience, unseen_by))
}

/// Those `user` blocks whom `audience` holds: the members of its community, or the people of
/// its DM. A relay is published every few seconds while someone types, so it names only those
/// it could reach rather than everyone they block.
async fn blocked_in(
    conn: &mut AsyncPgConnection,
    user: UserId,
    audience: &Audience,
) -> crate::Result<Vec<UserId>> {
    let blocked = user_block::table
        .select(user_block::blocked)
        .filter(user_block::blocker.eq(user));
    Ok(match audience {
        Audience::Community { community, .. } => {
            blocked
                .filter(diesel::dsl::exists(
                    aspen_schema::community_user::table.filter(
                        aspen_schema::community_user::community
                            .eq(*community)
                            .and(aspen_schema::community_user::user.eq(user_block::blocked)),
                    ),
                ))
                .load(conn)
                .await?
        }
        Audience::Direct { recipients } => {
            blocked
                .filter(user_block::blocked.eq_any(recipients))
                .load(conn)
                .await?
        }
    })
}

/// Tells every API server. Best-effort: a word lost is said again at the next refresh, or
/// lapses on its own.
async fn publish(
    state: &GlobalServerContext,
    user: UserId,
    channel: ChannelId,
    typing: bool,
    found: &Found,
) {
    let relay = Relay {
        event: EphemeralEvent::Typing {
            channel_id: channel,
            user_id: user,
            typing,
        },
        audience: found.audience.clone(),
        unseen_by: found.unseen_by.clone(),
    };
    let payload = match serde_json::to_vec(&relay) {
        Ok(payload) => payload,
        Err(e) => {
            warn!(error = %e, "could not write a typing relay");
            return;
        }
    };
    if let Err(e) = state
        .nats_context
        .client()
        .publish(TYPING_SUBJECT, payload.into())
        .await
    {
        warn!(error = %e, "could not publish typing");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subject_lies_outside_what_jetstream_keeps() {
        assert!(!TYPING_SUBJECT.starts_with(crate::events::SUBJECT_ROOT));
    }

    #[test]
    fn the_budget_allows_a_burst_then_its_rate() {
        let start = Instant::now();
        let mut budget = Budget::new(start);
        for _ in 0..4 {
            assert!(budget.take(start));
        }
        assert!(!budget.take(start));
        let half_second = start + Duration::from_millis(500);
        assert!(budget.take(half_second));
        assert!(!budget.take(half_second));
        let much_later = start + Duration::from_secs(60);
        for _ in 0..4 {
            assert!(budget.take(much_later));
        }
        assert!(!budget.take(much_later));
    }

    #[test]
    fn relays_read_back_as_written() {
        let relay = Relay {
            event: EphemeralEvent::Typing {
                channel_id: ChannelId(uuid::Uuid::now_v7()),
                user_id: UserId(uuid::Uuid::now_v7()),
                typing: true,
            },
            audience: Audience::Direct {
                recipients: vec![UserId(uuid::Uuid::now_v7())],
            },
            unseen_by: vec![UserId(uuid::Uuid::now_v7())],
        };
        let written = serde_json::to_string(&relay).unwrap();
        assert_eq!(serde_json::from_str::<Relay>(&written).unwrap(), relay);
        assert!(written.contains(r#""type":"typing""#));
    }
}
