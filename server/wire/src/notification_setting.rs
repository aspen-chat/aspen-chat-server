//! How much of a channel, community, or DM someone is told of.

use diesel::{AsExpression, FromSqlRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// How much of a community or channel to be told of.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum NotificationLevel {
    /// Every message.
    All,
    /// Only messages that tag the user: by name, through a role they hold, or as everyone.
    Tags,
    /// Nothing, not even tags.
    Nothing,
}

crate::wire_name_traits!(NotificationLevel);

crate::text_sql_traits!(NotificationLevel);
