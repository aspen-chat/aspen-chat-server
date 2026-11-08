//! What a held message waits for, and what it is: messages, and the other things that wait for
//! a channel's mana (`app::message::held`, `app::mana::queue`); and a message scheduled for
//! later as its author's apps hold it (`app::message::scheduled`).

use crate::{AttachmentId, ChannelId, HeldMessageId, MessageId};
use chrono::{DateTime, Utc};
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
    /// The time its author scheduled it for, or, having failed, its author's decision.
    Scheduled,
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

/// How a scheduled message stands.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum ScheduledStatus {
    /// It is posted at `sendAt`.
    Scheduled,
    /// Its time came while its channel was busy, and it waits its turn for the channel's mana,
    /// however long that takes.
    WaitingForMana,
    /// It could not be posted, `failure` says why, and it is kept for its author to schedule
    /// again or delete.
    Failed,
}

/// A message its author scheduled for later (`app::message::scheduled`), which only they are
/// told of until it is posted. `scheduledMessageChanged` carries it whenever it changes;
/// `heldMessagePosted` names the message it becomes, `heldMessageCancelled` says it was deleted
/// before then, and `heldMessageFailed` that it was dropped because its author can no longer
/// see where it would go.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ScheduledMessage {
    pub id: HeldMessageId,
    /// Where it is posted: a channel, a thread, or a DM.
    pub channel_id: ChannelId,
    pub content: String,
    pub attachments: Vec<AttachmentId>,
    /// The message it replies to, which it is posted replying to.
    pub reply_to: Option<MessageId>,
    /// Whether it tags the author of `replyTo`.
    pub tag_replied: bool,
    /// When it is posted; once `status` is no longer `scheduled`, when it was to be.
    pub send_at: DateTime<Utc>,
    pub status: ScheduledStatus,
    /// Why it could not be posted, in the language it was scheduled in; with `status` `failed`
    /// only.
    pub failure: Option<String>,
}
