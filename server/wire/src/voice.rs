//! Why a call ended.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Why a call ended, carried by the `voiceSessionEnded` event so a client can tell its user.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum VoiceSessionEndReason {
    /// The last participant left.
    Empty,
    /// The voice server stopped reporting and the call was ended for it.
    ServerLost,
    /// The call went a day without ever holding two people, so it was ended to free the
    /// voice server; the lone participant is shown a dialog saying so.
    Idle,
    /// An operator removed the voice server.
    ServerRemoved,
}
