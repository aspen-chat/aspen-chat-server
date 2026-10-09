//! What the event stream tells of as it happens and never keeps: no sequence, no replay, and
//! nothing in the database.

use crate::user::UserStatusRecord;
use crate::{ChannelId, UserId};
use serde::{Deserialize, Serialize};

/// How often a client says again that its user is still typing in a channel, while they type.
pub const TYPING_REFRESH_SECONDS: u64 = 3;

/// How long a client shows someone as typing after the last word that they are: a client that
/// stops refreshing (its connection lost, its app closed) is let go of by everyone this long
/// after.
pub const TYPING_EXPIRY_SECONDS: u64 = 8;

/// One thing the event stream tells of as it happens, in an `ephemeral` frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EphemeralEvent {
    /// Someone is typing in a channel (`typing: true`), said again every
    /// `TYPING_REFRESH_SECONDS` while they go on and shown for `TYPING_EXPIRY_SECONDS` after
    /// the last time, or has stopped (`typing: false`): sent their message, emptied the box, or
    /// lost their connection.
    #[serde(rename_all = "camelCase")]
    Typing {
        channel_id: ChannelId,
        user_id: UserId,
        typing: bool,
    },
    /// The presence of users the connection watches (`watchPresence`), each as the reader may
    /// learn it, for those whose presence changed or who were newly watched. Changes are
    /// gathered for up to `PRESENCE_WINDOW_MILLIS` and told together, and only what differs
    /// from what the connection was last told of each.
    #[serde(rename_all = "camelCase")]
    Presence { statuses: Vec<UserStatusRecord> },
}

/// How long a server gathers changes to presence before telling its connections of them
/// together.
pub const PRESENCE_WINDOW_MILLIS: u64 = 1_000;

/// The most users one connection may watch the presence of (`watchPresence`); a longer list is
/// cut to this many, the first kept.
pub const MAX_WATCHED_PRESENCE: usize = 500;
