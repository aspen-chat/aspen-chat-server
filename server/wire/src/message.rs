//! What kind a message is.

use diesel::deserialize::FromSql;
use diesel::pg::Pg;
use diesel::serialize::{IsNull, Output, ToSql};
use diesel::{AsExpression, FromSqlRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::Write;
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

impl ToSql<aspen_schema::sql_types::MessageKind, Pg> for MessageKind {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        out.write_all(match self {
            MessageKind::Standard => b"standard",
            MessageKind::Poll => b"poll",
            MessageKind::PollClosed => b"poll_closed",
            MessageKind::ThreadEcho => b"thread_echo",
            MessageKind::Call => b"call",
            MessageKind::MissedCall => b"missed_call",
            MessageKind::Command => b"command",
            MessageKind::Warning => b"warning",
            MessageKind::MemberJoined => b"member_joined",
            MessageKind::CommunityGrew => b"community_grew",
        })?;
        Ok(IsNull::No)
    }
}

impl FromSql<aspen_schema::sql_types::MessageKind, Pg> for MessageKind {
    fn from_sql(
        bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
    ) -> diesel::deserialize::Result<Self> {
        match bytes.as_bytes() {
            b"standard" => Ok(MessageKind::Standard),
            b"poll" => Ok(MessageKind::Poll),
            b"poll_closed" => Ok(MessageKind::PollClosed),
            b"thread_echo" => Ok(MessageKind::ThreadEcho),
            b"call" => Ok(MessageKind::Call),
            b"command" => Ok(MessageKind::Command),
            b"missed_call" => Ok(MessageKind::MissedCall),
            b"warning" => Ok(MessageKind::Warning),
            b"member_joined" => Ok(MessageKind::MemberJoined),
            b"community_grew" => Ok(MessageKind::CommunityGrew),
            _ => Err(format!(
                "Unrecognized enum variant: {:?}",
                String::from_utf8_lossy(bytes.as_bytes())
            )
            .into()),
        }
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
