//! Telling those viewing a channel the level of its mana while it is below full. Nothing of it
//! is kept: each level is published on the core NATS subject [`MANA_SUBJECT`] as a relay, as
//! typing is (`app::typing::Relay`), and every API server's event feed routes it to its own
//! connections viewing the channel that may read it.
//!
//! A pool spent from is told of at most once each [`EVERY`] across the fleet (a Valkey key each
//! pool's first spend in the window sets), and a spend inside the window has its server tell
//! the level as it is when the window ends, so the last spend of a burst always reaches viewers.
//! Between levels a client animates the recharge itself, which is predictable.

use super::{Level, MILLI, Pool};
use crate::ChannelId;
use crate::context::GlobalServerContext;
use crate::events::{ChannelHome, channel_home, dm_recipients};
use crate::typing::{Audience, EphemeralEvent, Relay};
use fred::interfaces::KeysInterface;
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use tracing::warn;

/// The core NATS subject every API server publishes mana levels on and reads them from. It
/// lies outside `events::SUBJECT_ROOT`, whose subjects JetStream keeps.
pub const MANA_SUBJECT: &str = "aspen.mana";

/// The least time between two levels of one pool told to its viewers.
pub const EVERY: Duration = Duration::from_millis(500);

/// The pools this server will tell the level of when their window ends.
static TRAILING: LazyLock<Mutex<HashSet<ChannelId>>> = LazyLock::new(Default::default);

/// Tells viewers of `pool` its `level` after a spend, or as the window ends.
pub async fn changed(state: &GlobalServerContext, pool: &Pool, level: Level) {
    let first: Result<Option<String>, _> = state
        .valkey
        .set(
            format!("mana:{}:told", pool.channel.0),
            1,
            Some(fred::types::Expiration::PX(
                i64::try_from(EVERY.as_millis()).unwrap_or(i64::MAX),
            )),
            Some(fred::types::SetOptions::NX),
            false,
        )
        .await;
    if !matches!(first, Ok(None)) {
        tell(state.clone(), *pool, level);
        return;
    }
    if !TRAILING
        .lock()
        .expect("the trailing pools' lock")
        .insert(pool.channel)
    {
        return;
    }
    let (state, pool) = (state.clone(), *pool);
    tokio::spawn(async move {
        tokio::time::sleep(EVERY).await;
        TRAILING
            .lock()
            .expect("the trailing pools' lock")
            .remove(&pool.channel);
        if let Some(level) = super::level(&state, &pool).await {
            tell(state, pool, level);
        }
    });
}

/// The level of `pool` for one viewer starting to view it, when it is below full.
async fn now(state: &GlobalServerContext, pool: &Pool, queued: bool) -> Option<EphemeralEvent> {
    let level = super::level(state, pool).await?;
    (level.level < i64::from(pool.settings.max) * MILLI).then(|| event(pool, level, queued))
}

/// The level of `channel`'s mana for a connection of `user`'s starting to view it: when it is
/// below full, and they may read the channel.
async fn for_viewer(
    state: &GlobalServerContext,
    user: crate::UserId,
    channel: ChannelId,
) -> Option<EphemeralEvent> {
    let (pool, queued) = {
        let mut conn = state.connection_pool.get().await.ok()?;
        crate::permissions::channel_access(state, conn.as_mut(), user, channel)
            .await
            .ok()?;
        let pool = super::pool_of(state, conn.as_mut(), channel).await.ok()??;
        let queued = super::queue::waiting_in(conn.as_mut(), channel)
            .await
            .ok()?;
        (pool, queued)
    };
    now(state, &pool, queued).await
}

/// One event stream connection's task telling it the mana of the channels it starts to view,
/// below full, so a bar already showing appears at once; spends since are routed as relays.
/// Dropping it, with the connection, ends the task.
pub struct Viewer {
    lists: tokio::sync::mpsc::Sender<Vec<ChannelId>>,
}

impl Viewer {
    /// Starts the task; what it finds comes out of the receiver, for the connection to send.
    pub fn spawn(
        state: GlobalServerContext,
        user: crate::UserId,
    ) -> (Self, tokio::sync::mpsc::Receiver<EphemeralEvent>) {
        // A list waiting is replaced by the next; a client sends one as its view changes.
        let (lists, mut incoming) = tokio::sync::mpsc::channel::<Vec<ChannelId>>(2);
        let (told, levels) = tokio::sync::mpsc::channel(8);
        tokio::spawn(async move {
            let mut viewed: HashSet<ChannelId> = HashSet::new();
            while let Some(list) = incoming.recv().await {
                let now: HashSet<ChannelId> = list.into_iter().collect();
                for channel in now.difference(&viewed) {
                    if let Some(event) = for_viewer(&state, user, *channel).await
                        && told.send(event).await.is_err()
                    {
                        return;
                    }
                }
                viewed = now;
            }
        });
        (Self { lists }, levels)
    }

    /// The channels the connection's client views now (its `viewing` frame), cut as the
    /// event feed cuts them.
    pub fn viewing(&self, channels: Vec<ChannelId>) {
        let _ = self.lists.try_send(channels);
    }
}

fn event(pool: &Pool, level: Level, queued: bool) -> EphemeralEvent {
    EphemeralEvent::Mana {
        channel_id: pool.channel,
        level: level.level,
        max: pool.settings.max,
        recharge: pool.settings.recharge,
        at: chrono::DateTime::from_timestamp_millis(level.at_ms).unwrap_or_default(),
        queued,
    }
}

/// Publishes the level, with its own database connection, apart from the spender's, which may
/// hold one.
fn tell(state: GlobalServerContext, pool: Pool, level: Level) {
    tokio::spawn(async move {
        let told = async {
            let mut conn = state.connection_pool.get().await?;
            let queued = super::queue::waiting_in(conn.as_mut(), pool.channel).await?;
            let audience = match channel_home(&state, conn.as_mut(), pool.channel).await? {
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
            Ok::<_, crate::Error>((audience, queued))
        };
        let (audience, queued) = match told.await {
            Ok(told) => told,
            Err(e) => {
                warn!(error = %e, "could not find who views a channel to tell its mana");
                return;
            }
        };
        let relay = Relay {
            event: event(&pool, level, queued),
            audience,
            unseen_by: Vec::new(),
        };
        let payload = match serde_json::to_vec(&relay) {
            Ok(payload) => payload,
            Err(e) => {
                warn!(error = %e, "could not write a mana relay");
                return;
            }
        };
        if let Err(e) = state
            .nats_context
            .client()
            .publish(MANA_SUBJECT, payload.into())
            .await
        {
            warn!(error = %e, "could not publish a channel's mana");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_subject_lies_outside_what_jetstream_keeps() {
        assert!(!MANA_SUBJECT.starts_with(crate::events::SUBJECT_ROOT));
    }
}
