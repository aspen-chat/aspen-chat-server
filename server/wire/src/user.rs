//! What a user says they are up to, and whether they are online.

use diesel::{AsExpression, FromSqlRow};
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
    /// Connected, and asking not to be disturbed: they are told of nothing and no call rings
    /// them.
    DoNotDisturb,
    /// Connected, but showing as offline to everyone else. Only the user themself is told this.
    Invisible,
}

/// What a user may choose to show of their presence in place of what their connections say
/// (`app::presence_override`).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Deserialize,
    Serialize,
    ToSchema,
    schemars::JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum PresenceOverride {
    /// Offline to everyone else while connected.
    Invisible,
    /// Away while connected, however recently they used Aspen.
    Away,
    /// Do not disturb while connected; and, connected or not, no notice, phone push, or ring
    /// reaches them.
    DoNotDisturb,
}

crate::wire_name_traits!(PresenceOverride);

crate::text_sql_traits!(PresenceOverride);
