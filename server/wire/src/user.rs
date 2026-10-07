//! What a user says they are up to, and whether they are online.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What a user says they are up to: a short line of text and, optionally, an emoji beside it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CustomStatus {
    pub text: String,
    /// A single emoji, or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
}

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    Serialize,
    utoipa::ToSchema,
    schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum UserOnlineStatus {
    Online,
    Offline,
    Away,
}
