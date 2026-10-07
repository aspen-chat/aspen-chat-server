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

/// The least time between two `typing` frames for one channel that are published; a client
/// sends them every `TYPING_REFRESH_SECONDS`, and closer ones are dropped.
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

/// A channel the connection said its user is typing in.
struct Shown {
    /// When it last published that they are.
    at: Instant,
    audience: Audience,
    unseen_by: Vec<UserId>,
}

async fn run(state: GlobalServerContext, user: UserId, mut frames: mpsc::Receiver<Frame>) {
    let mut shown: HashMap<ChannelId, Shown> = HashMap::new();
    while let Some(frame) = frames.recv().await {
        let now = Instant::now();
        shown.retain(|_, s| now.duration_since(s.at) < EXPIRY);
        match frame {
            Frame::Typing(channel) => {
                if let Some(s) = shown.get(&channel) {
                    if now.duration_since(s.at) < REFRESH_FLOOR {
                        continue;
                    }
                } else if shown.len() >= MAX_CHANNELS {
                    continue;
                }
                // A check that takes longer than a refresh is overtaken by the next one.
                let checked = tokio::time::timeout(
                    Duration::from_secs(TYPING_REFRESH_SECONDS),
                    audience(&state, user, channel),
                )
                .await;
                let (audience, unseen_by) = match checked {
                    Ok(Ok(found)) => found,
                    Ok(Err(e)) => {
                        // Refused (no longer allowed to post there) or failed: whatever was
                        // shown lapses.
                        debug!(error = %e, "typing refused");
                        if let Some(s) = shown.remove(&channel) {
                            publish(&state, user, channel, false, s.audience, s.unseen_by).await;
                        }
                        continue;
                    }
                    Err(_) => {
                        warn!("checking who sees someone typing took longer than a refresh");
                        continue;
                    }
                };
                publish(
                    &state,
                    user,
                    channel,
                    true,
                    audience.clone(),
                    unseen_by.clone(),
                )
                .await;
                shown.insert(
                    channel,
                    Shown {
                        at: Instant::now(),
                        audience,
                        unseen_by,
                    },
                );
            }
            Frame::Stopped(channel) => {
                if let Some(s) = shown.remove(&channel) {
                    publish(&state, user, channel, false, s.audience, s.unseen_by).await;
                }
            }
        }
    }
    // The connection ended.
    let now = Instant::now();
    for (channel, s) in shown {
        if now.duration_since(s.at) < EXPIRY {
            publish(&state, user, channel, false, s.audience, s.unseen_by).await;
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
    crate::message::may_post(state, conn.as_mut(), user, channel).await?;
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
    let mut unseen_by = blocked_by(conn.as_mut(), user).await?;
    unseen_by.push(user);
    Ok((audience, unseen_by))
}

/// Everyone `user` blocks.
async fn blocked_by(conn: &mut AsyncPgConnection, user: UserId) -> crate::Result<Vec<UserId>> {
    Ok(user_block::table
        .select(user_block::blocked)
        .filter(user_block::blocker.eq(user))
        .load(conn)
        .await?)
}

/// Tells every API server. Best-effort: a word lost is said again at the next refresh, or
/// lapses on its own.
async fn publish(
    state: &GlobalServerContext,
    user: UserId,
    channel: ChannelId,
    typing: bool,
    audience: Audience,
    unseen_by: Vec<UserId>,
) {
    let relay = Relay {
        event: EphemeralEvent::Typing {
            channel_id: channel,
            user_id: user,
            typing,
        },
        audience,
        unseen_by,
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
