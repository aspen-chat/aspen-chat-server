//! What kind a message is.

use diesel::deserialize::FromSql;
use diesel::pg::Pg;
use diesel::serialize::{IsNull, Output, ToSql};
use diesel::{AsExpression, FromSqlRow};
use serde::{Deserialize, Serialize};
use std::io::Write;

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
    utoipa::ToSchema,
    schemars::JsonSchema,
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
            _ => Err(format!(
                "Unrecognized enum variant: {:?}",
                String::from_utf8_lossy(bytes.as_bytes())
            )
            .into()),
        }
    }
}
