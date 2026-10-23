//! What a held message waits for, and what it is: messages, and the other things that wait for
//! a channel's mana (`app::message::held`, `app::mana::queue`).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What a held message waits for before it is posted.
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
    schemars::JsonSchema,
    diesel::FromSqlRow,
    diesel::AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum HeldWaiting {
    /// Its attachments' previews, being made.
    Preview,
    /// Its channel's mana: the channel is busy, and it waits its turn.
    Mana,
}

crate::wire_name_traits!(HeldWaiting);
crate::text_sql_traits!(HeldWaiting);

/// What a held message is. Only a `message` waits for previews; the rest wait for mana.
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
    schemars::JsonSchema,
    diesel::FromSqlRow,
    diesel::AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum HeldKind {
    Message,
    /// A slash command, as its invocation was given.
    Command,
    /// An edit of the author's message `target`.
    Edit,
    /// A poll, as it was asked for; its duration counts from when it is posted.
    Poll,
    /// The echo of the author's thread reply `target` to the thread's parent.
    Echo,
}

crate::wire_name_traits!(HeldKind);
crate::text_sql_traits!(HeldKind);
