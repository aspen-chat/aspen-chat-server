//! What kind a message is.

use diesel::deserialize::FromSql;
use diesel::pg::Pg;
use diesel::serialize::{IsNull, Output, ToSql};
use diesel::{AsExpression, FromSqlRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::Write;
use strum::VariantArray as _;
use utoipa::ToSchema;

/// What a message is. Both poll kinds and echoes carry no `content`; the client renders the poll
/// kinds from the poll record the message's `poll` field names, and an echo from its reply.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    Serialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
    strum::VariantArray,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = aspen_schema::sql_types::MessageKind)]
pub enum MessageKind {
    /// Text written by its author.
    Standard,
    /// The message a poll was opened with.
    Poll,
    /// The system message announcing a poll's outcome; its `author` is the poll's creator.
    PollClosed,
    /// A thread reply shown in the thread's parent channel, by reference: `echoOf` names the
    /// reply, and the echo has no content of its own. Its `author` is the reply's.
    ThreadEcho,
    /// The system message recording that a DM's call ended; `callSeconds` says how long it
    /// lasted and its `author` is who started it. It has no content of its own.
    Call,
    /// The system message recording that a DM's call ended without anyone joining whoever
    /// started it, its `author`. It has no content of its own.
    MissedCall,
    /// A bot command its `author` invoked, as it was sent (`/name` and its arguments), to the
    /// bot `commandBot` names (see `app::bot_command`).
    Command,
    /// A moderator's warning to the person `warning` names, sent for the deployment's
    /// moderators by the system account in its DM with them: the reviewer's words as
    /// `content`, about what `warning` holds (see `app::report`).
    Warning,
    /// The system message announcing that its `author` joined the community, posted in its
    /// default channel (see `app::default_channel`). It has no content of its own.
    MemberJoined,
    /// The system message announcing that the community has become large, posted in its
    /// default channel by the system account and tagging everyone (see `app::large_community`).
    /// It has no content of its own; its `growth` says what changed.
    CommunityGrew,
}

impl MessageKind {
    /// Whether a message of this kind may tell anyone of itself: wake a phone, sound, count
    /// in the activity feed. The rest only record something: an echo says nothing of its own,
    /// a call's record follows the ring that already told everyone of the call, and a command
    /// is for its bot, whose answer is what tells. This is the one list of them; a message's
    /// record carries the answer as `notifies`.
    pub const fn notifies(self) -> bool {
        match self {
            MessageKind::Standard
            | MessageKind::Poll
            | MessageKind::PollClosed
            | MessageKind::Warning
            | MessageKind::MemberJoined
            | MessageKind::CommunityGrew => true,
            MessageKind::ThreadEcho
            | MessageKind::Call
            | MessageKind::MissedCall
            | MessageKind::Command => false,
        }
    }

    /// The variant's label in the database's `message_kind` enum, read in both directions.
    const fn sql_name(self) -> &'static str {
        match self {
            MessageKind::Standard => "standard",
            MessageKind::Poll => "poll",
            MessageKind::PollClosed => "poll_closed",
            MessageKind::ThreadEcho => "thread_echo",
            MessageKind::Call => "call",
            MessageKind::MissedCall => "missed_call",
            MessageKind::Command => "command",
            MessageKind::Warning => "warning",
            MessageKind::MemberJoined => "member_joined",
            MessageKind::CommunityGrew => "community_grew",
        }
    }
}

impl ToSql<aspen_schema::sql_types::MessageKind, Pg> for MessageKind {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        out.write_all(self.sql_name().as_bytes())?;
        Ok(IsNull::No)
    }
}

impl FromSql<aspen_schema::sql_types::MessageKind, Pg> for MessageKind {
    fn from_sql(
        bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
    ) -> diesel::deserialize::Result<Self> {
        Self::VARIANTS
            .iter()
            .copied()
            .find(|variant| variant.sql_name().as_bytes() == bytes.as_bytes())
            .ok_or_else(|| {
                format!(
                    "Unrecognized enum variant: {:?}",
                    String::from_utf8_lossy(bytes.as_bytes())
                )
                .into()
            })
    }
}

/// What changed when a community became large, carried by its `CommunityGrew` message so each
/// reader's client can say so in their own language.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[diesel(sql_type = diesel::sql_types::Jsonb)]
#[serde(rename_all = "camelCase")]
pub struct CommunityGrowth {
    /// How many members the community had when it crossed the deployment's threshold: the
    /// threshold, or more when a join before could not make it large.
    pub members: u32,
    /// Whether Mention everyone was taken from the everyone role. It is not when the role
    /// had already lost it.
    pub mention_everyone_taken: bool,
}

crate::jsonb_sql_traits!(CommunityGrowth);

#[cfg(test)]
mod tests {
    use super::*;

    /// What only records something told of already tells no one; what announces something
    /// with no text of its own tells as any message does.
    #[test]
    fn records_do_not_notify_and_announcements_do() {
        assert!(MessageKind::Standard.notifies());
        assert!(MessageKind::PollClosed.notifies());
        assert!(MessageKind::MemberJoined.notifies());
        assert!(!MessageKind::ThreadEcho.notifies());
        assert!(!MessageKind::MissedCall.notifies());
        assert!(!MessageKind::Command.notifies());
    }
}
