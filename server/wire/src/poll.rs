//! Polls' options, tallies, and write-ins.

use crate::UserId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// The tally for one option of a poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PollOptionResult {
    pub count: u32,
    /// Who voted for the option, oldest vote first. Absent on an anonymous poll.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voters: Option<Vec<UserId>>,
}

/// One choice on a poll: its text and, optionally, an emoji shown beside it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PollOption {
    pub label: String,
    /// A single emoji, or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

/// One of the calling user's votes: the option they chose on a poll.
/// An answer a voter added to a poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PollWriteIn {
    pub label: String,
    /// Who wrote it in. Absent on an anonymous poll, where it would say how they voted, and
    /// once their account is gone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub written_by: Option<UserId>,
}
