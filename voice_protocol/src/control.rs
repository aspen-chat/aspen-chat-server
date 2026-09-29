//! Messages between the API server and the voice servers, carried over core NATS.
//!
//! Every voice server publishes [`VoiceReport`]s on [`REPORT_SUBJECT`]; the API servers share a
//! queue group on it so exactly one of them acts on each report. The API server sends a
//! [`VoiceCommand`] to one voice server on that server's own subject from [`command_subject`].

use crate::signal::{TransferEnd, TransferMode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Where every voice server publishes its reports.
pub const REPORT_SUBJECT: &str = "aspen.voice.report";

/// The queue group the API servers subscribe to reports with, so a report is handled once
/// however many API servers run.
pub const REPORT_QUEUE_GROUP: &str = "aspen-api";

/// The subject one voice server listens on for commands.
pub fn command_subject(server: Uuid) -> String {
    format!("aspen.voice.command.{server}")
}

/// How often a voice server reports its load, whether or not anything changed. The API server
/// treats a server silent for several of these as gone and ends its sessions.
pub const LOAD_REPORT_INTERVAL_SECONDS: u64 = 15;

/// Something a voice server tells the API server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum VoiceReport {
    /// The periodic heartbeat: how many participants the server carries right now.
    Load { server: Uuid, participants: u32 },
    /// The first participant connected to a channel with no session, so one now exists on
    /// this server.
    SessionStarted {
        server: Uuid,
        session: Uuid,
        channel: Uuid,
    },
    /// A user's media is flowing.
    ParticipantJoined { session: Uuid, user: Uuid },
    /// A user disconnected or was removed.
    ParticipantLeft { session: Uuid, user: Uuid },
    /// A user started or stopped speaking, as the server's audio level observer sees it.
    Speaking {
        session: Uuid,
        user: Uuid,
        speaking: bool,
    },
    /// A user's mute or deafen state changed, whether by their own hand or a command.
    ParticipantState {
        session: Uuid,
        user: Uuid,
        muted: bool,
        deafened: bool,
        /// Whether the participant has a screen (or window, or game) video producer. Absent in
        /// a report from a voice server that predates screen sharing, which means not sharing.
        #[serde(default)]
        sharing_screen: bool,
    },
    /// The last participant left, or the server is shutting the session down.
    SessionEnded { session: Uuid },
    /// A participant offered a file to the call, for the deployment's record of transfers.
    /// `record` is the voice server's own id for the offer, time-ordered and never the id the
    /// client chose, so no client can make its offer collide with another in the record.
    FileOffered {
        channel: Uuid,
        record: Uuid,
        sender: Uuid,
        name: String,
        size: u64,
        allow_direct: bool,
        valid_for_seconds: u32,
    },
    /// A transfer of an offered file began.
    TransferStarted {
        record: Uuid,
        sender: Uuid,
        receiver: Uuid,
        mode: TransferMode,
    },
    /// A transfer ended, ended by `ended_by` (one of its two sides) for `reason`.
    TransferEnded {
        record: Uuid,
        receiver: Uuid,
        ended_by: Uuid,
        reason: TransferEnd,
    },
}

/// Something the API server asks a voice server to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum VoiceCommand {
    /// Stop or resume forwarding a user's audio to the others.
    Mute {
        session: Uuid,
        user: Uuid,
        muted: bool,
    },
    /// Disconnect a user from the session.
    Kick { session: Uuid, user: Uuid },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_are_tagged_by_type() {
        let report = VoiceReport::Speaking {
            session: Uuid::nil(),
            user: Uuid::nil(),
            speaking: true,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["type"], "speaking");
        assert_eq!(serde_json::from_value::<VoiceReport>(json).unwrap(), report);
    }
}
