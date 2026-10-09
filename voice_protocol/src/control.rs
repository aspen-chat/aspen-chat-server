//! Messages between the API server and the voice servers, carried over NATS.
//!
//! Every voice server publishes its [`VoiceReport`]s to the JetStream stream [`REPORT_STREAM`].
//! Reports are spread over [`REPORT_PARTITIONS`] lanes by the channel they are about
//! ([`partition`]), and speaking changes go in lanes of their own, so each report's subject
//! ([`VoiceReport::subject`]) names its kind of lane, its lane, and its voice server. The API
//! servers read every lane through one durable consumer that hands out a report only once the
//! one before it has been applied, so the reports about one channel are applied once each, in
//! the order they were sent, however many API servers share the work, while different lanes
//! are applied side by side. A busy channel never holds up the others in its voice server
//! beyond its lane, and speaking changes, which come many times faster than anything else,
//! never hold up a call's joins and leaves. The API server sends a [`VoiceCommand`] to one voice
//! server over core NATS, on that server's own subject from [`command_subject`].

use crate::signal::{TransferEnd, TransferMode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The JetStream stream every voice server's reports are kept in until an API server has
/// applied them. The API servers create it.
pub const REPORT_STREAM: &str = "aspen_voice_reports";

/// What the subjects of every report but speaking changes start with.
pub const REPORT_SUBJECT_ROOT: &str = "aspen.voice.report";

/// What the subjects of speaking changes start with.
pub const SPEAKING_SUBJECT_ROOT: &str = "aspen.voice.speaking";

/// How many lanes reports are spread over, for each of the two kinds. Every report about a
/// channel goes in the same lane, so its calls' reports stay in order, a channel's next call
/// included. Both sides compute the lane, so this is part of the protocol: changing it while
/// reports are waiting would let a channel's reports be applied out of order.
pub const REPORT_PARTITIONS: u8 = 64;

/// The lane of reports about `key` (a channel, or a voice server for what is about the whole
/// server): its last byte, which is random in a UUIDv7, modulo [`REPORT_PARTITIONS`]. The API
/// server's database computes the same as `get_byte(uuid_send(key), 15) % 64`.
pub fn partition(key: Uuid) -> u8 {
    key.as_bytes()[15] % REPORT_PARTITIONS
}

pub fn report_subject(partition: u8, server: Uuid) -> String {
    format!("{REPORT_SUBJECT_ROOT}.{partition}.{server}")
}

pub fn speaking_subject(partition: u8, server: Uuid) -> String {
    format!("{SPEAKING_SUBJECT_ROOT}.{partition}.{server}")
}

/// The voice server a report's subject names, its last token. A voice server's NATS user may
/// publish only on subjects naming it, so the API server takes this, not anything the report
/// says, as where the report came from.
pub fn subject_server(subject: &str) -> Option<Uuid> {
    subject.rsplit('.').next()?.parse().ok()
}

/// Where a voice server asks the API servers for the key join tokens are signed with
/// (`token::sign`); any API server answers with a `TokenKey`. The request is a
/// `TokenKeyRequest`, or empty, which is answered with the key alone.
pub const TOKEN_KEY_SUBJECT: &str = "aspen.voice.token-key";

/// What a voice server sends on `TOKEN_KEY_SUBJECT`: the id it reports as, so the answer can say
/// whether that id is registered. The id is only asked about, never believed: the API servers
/// know a report's server by its subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenKeyRequest {
    pub server: Uuid,
}

/// The public half of the key the API servers sign join tokens with, as they answer a request on
/// `TOKEN_KEY_SUBJECT`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenKey {
    /// `token::key_id` of the key, as the tokens it signs name it.
    pub key_id: String,
    /// The Ed25519 public key, base64url without padding.
    pub public_key: String,
    /// Whether the id the `TokenKeyRequest` named is a registered voice server; absent when the
    /// request named none or the answering API server could not tell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registered: Option<bool>,
}

/// The subject one voice server listens on for commands.
pub fn command_subject(server: Uuid) -> String {
    format!("aspen.voice.command.{server}")
}

/// What the subjects of a voice server's replies start with, rather than NATS's shared `_INBOX`,
/// so a NATS user for the server may be allowed its own replies and no one else's.
pub fn inbox_prefix(server: Uuid) -> String {
    format!("{VOICE_INBOXES}.{server}")
}

/// What every voice server's inbox prefix (`inbox_prefix`) starts with.
const VOICE_INBOXES: &str = "_INBOX_voice";

/// Whether `subject` is a reply subject in some voice server's inbox, or in `server`'s when it is
/// given. NATS lets a requester name any reply subject, whatever its own permissions, so an API
/// server answers a voice server only where this holds: otherwise a voice server could have it
/// publish, with its wider permissions, on any subject at all.
pub fn is_voice_inbox(subject: &str, server: Option<Uuid>) -> bool {
    let rest = match server {
        Some(server) => subject.strip_prefix(&inbox_prefix(server)),
        None => subject.strip_prefix(VOICE_INBOXES),
    };
    rest.is_some_and(|rest| rest.len() > 1 && rest.starts_with('.'))
}

/// The subjects naming `server` that its NATS user must be allowed: those it publishes on, and
/// those it subscribes to. The rest of its permissions name no server
/// (`docs/operators/installing/6-voice-servers.md`).
pub fn own_subjects(server: Uuid) -> ([String; 2], [String; 2]) {
    (
        [
            format!("{REPORT_SUBJECT_ROOT}.*.{server}"),
            format!("{SPEAKING_SUBJECT_ROOT}.*.{server}"),
        ],
        [
            command_subject(server),
            format!("{}.>", inbox_prefix(server)),
        ],
    )
}

/// How often a voice server reports its load, whether or not anything changed. The API server
/// treats a server silent for several of these as gone and ends its sessions.
pub const LOAD_REPORT_INTERVAL_SECONDS: u64 = 15;

/// How often a voice server reports every call it holds as it stands ([`VoiceReport::SessionSnapshot`]
/// and [`VoiceReport::SessionsHeld`]), which repairs whatever a lost report left wrong. It also
/// does so whenever it reconnects to NATS, since reports may have been lost while it was away.
pub const SNAPSHOT_INTERVAL_SECONDS: u64 = 60;

/// Someone in a call as a snapshot shows them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantSnapshot {
    pub user: Uuid,
    pub muted: bool,
    pub deafened: bool,
    pub sharing_screen: bool,
}

/// Something a voice server tells the API server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, strum::IntoStaticStr)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[strum(serialize_all = "camelCase")]
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
    ParticipantJoined {
        session: Uuid,
        channel: Uuid,
        user: Uuid,
        /// The sign-in their join token was issued to (`JoinClaims::sign_in`), which the API
        /// server checks is still live, so a token issued before its sign-in ended and used
        /// after cannot keep them in the call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sign_in: Option<String>,
    },
    /// A user disconnected or was removed.
    ParticipantLeft {
        session: Uuid,
        channel: Uuid,
        user: Uuid,
    },
    /// A user started or stopped speaking, as the server's audio level observer sees it.
    Speaking {
        session: Uuid,
        channel: Uuid,
        user: Uuid,
        speaking: bool,
    },
    /// A user's mute or deafen state changed, whether by their own hand or a command.
    ParticipantState {
        session: Uuid,
        channel: Uuid,
        user: Uuid,
        muted: bool,
        deafened: bool,
        /// Whether the participant has a screen (or window, or game) video producer. Absent in
        /// a report from a voice server that predates screen sharing, which means not sharing.
        #[serde(default)]
        sharing_screen: bool,
    },
    /// The last participant left, or the server is shutting the session down.
    SessionEnded { session: Uuid, channel: Uuid },
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
        channel: Uuid,
        record: Uuid,
        sender: Uuid,
        receiver: Uuid,
        mode: TransferMode,
    },
    /// A transfer ended, ended by `ended_by` (one of its two sides) for `reason`.
    TransferEnded {
        channel: Uuid,
        record: Uuid,
        receiver: Uuid,
        ended_by: Uuid,
        reason: TransferEnd,
    },
    /// One call this server holds and everyone in it, as they stand. Part of a snapshot: one of
    /// these for every call, then a [`VoiceReport::SessionsHeld`] for every lane, all sent
    /// together, so no other report comes between them and each says what its call or lane is
    /// as of its place in the order.
    SessionSnapshot {
        server: Uuid,
        session: Uuid,
        channel: Uuid,
        participants: Vec<ParticipantSnapshot>,
    },
    /// Every call this server holds in the channels of lane `partition`, ending a snapshot. A
    /// call recorded on this server in that lane and not listed is one it no longer has. It is
    /// sent for every lane, those with no calls included, and in its own lane, so it is applied
    /// after every report about those calls sent before it.
    SessionsHeld {
        server: Uuid,
        partition: u8,
        sessions: Vec<Uuid>,
    },
}

impl VoiceReport {
    /// The subject this report is published on: its lane by the channel it is about, or by
    /// the server for a report about the whole server, among speaking changes or the rest.
    pub fn subject(&self, server: Uuid) -> String {
        match self {
            VoiceReport::Speaking { channel, .. } => speaking_subject(partition(*channel), server),
            VoiceReport::Load { .. } => report_subject(partition(server), server),
            VoiceReport::SessionsHeld { partition, .. } => report_subject(*partition, server),
            VoiceReport::SessionStarted { channel, .. }
            | VoiceReport::ParticipantJoined { channel, .. }
            | VoiceReport::ParticipantLeft { channel, .. }
            | VoiceReport::ParticipantState { channel, .. }
            | VoiceReport::SessionEnded { channel, .. }
            | VoiceReport::FileOffered { channel, .. }
            | VoiceReport::TransferStarted { channel, .. }
            | VoiceReport::TransferEnded { channel, .. }
            | VoiceReport::SessionSnapshot { channel, .. } => {
                report_subject(partition(*channel), server)
            }
        }
    }
}

/// Something the API server asks a voice server to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum VoiceCommand {
    /// Server-mute or unmute a user. While server-muted their microphone is not forwarded,
    /// whatever they ask; unmuting lifts only this mute, not one they set themself.
    Mute {
        session: Uuid,
        user: Uuid,
        muted: bool,
    },
    /// Disconnect a user from the session, telling them why: by a moderator when no reason is
    /// given.
    Kick {
        session: Uuid,
        user: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<crate::signal::KickReason>,
    },
    /// What a user may now do in the session: producers it no longer allows are closed and
    /// offers withdrawn, and they are told.
    Grant {
        session: Uuid,
        user: Uuid,
        grants: crate::token::Grants,
    },
    /// Close the session's room, telling everyone in it the server is closing the call
    /// (`KickReason::ServerStopping`), so their clients rejoin wherever the channel's call is
    /// recorded. Sent for a room the API server will not record because the channel's call goes
    /// on on another server that is still reporting.
    Close { session: Uuid },
    /// Some of a user's sign-ins ended, as `signInsEnded` says: `ended` alone, or without it
    /// every one but `kept`. Their participant in the session leaves (`KickReason::SignedOut`)
    /// if it joined on a token of one of those sign-ins, or of none (a bot's) when every
    /// sign-in but `kept` ended.
    EndSignIns {
        session: Uuid,
        user: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ended: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kept: Option<String>,
    },
}

impl VoiceCommand {
    /// Whether `EndSignIns` with `ended` and `kept` ends the participant who joined on a token
    /// of `sign_in`.
    pub fn ends_sign_in(ended: Option<&str>, kept: Option<&str>, sign_in: Option<&str>) -> bool {
        match ended {
            Some(ended) => sign_in == Some(ended),
            None => kept.is_none() || sign_in != kept,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ending_sign_ins_reaches_the_participants_of_those_sign_ins() {
        let ends = VoiceCommand::ends_sign_in;
        // One sign-in ended: only a participant of it leaves.
        assert!(ends(Some("a"), None, Some("a")));
        assert!(!ends(Some("a"), None, Some("b")));
        assert!(!ends(Some("a"), None, None));
        // Every one but `kept`: everyone else leaves, a bot's participant included.
        assert!(!ends(None, Some("a"), Some("a")));
        assert!(ends(None, Some("a"), Some("b")));
        assert!(ends(None, Some("a"), None));
        // Every one.
        assert!(ends(None, None, Some("a")));
        assert!(ends(None, None, None));
    }

    #[test]
    fn replies_go_only_to_voice_servers_inboxes() {
        let server = Uuid::from_u128(1);
        let other = Uuid::from_u128(2);
        let reply = format!("{}.abc.1", inbox_prefix(server));
        assert!(is_voice_inbox(&reply, Some(server)));
        assert!(is_voice_inbox(&reply, None));
        assert!(!is_voice_inbox(&reply, Some(other)));
        // A prefix alone, or one running into another server's id, is no inbox.
        assert!(!is_voice_inbox(&inbox_prefix(server), Some(server)));
        assert!(!is_voice_inbox(
            &format!("{}0.x", inbox_prefix(server)),
            Some(server)
        ));
        assert!(!is_voice_inbox("_INBOX_voice", None));
        assert!(!is_voice_inbox("_INBOX_voicex.y", None));
        // The subjects the API servers act on are refused.
        assert!(!is_voice_inbox("aspen.plugins.changed", None));
        assert!(!is_voice_inbox("$KV.aspen_rate_limits.suspension", None));
        assert!(!is_voice_inbox("_INBOX.abc", None));
    }

    #[test]
    fn reports_are_tagged_by_type() {
        let report = VoiceReport::Speaking {
            session: Uuid::nil(),
            channel: Uuid::nil(),
            user: Uuid::nil(),
            speaking: true,
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["type"], "speaking");
        assert_eq!(serde_json::from_value::<VoiceReport>(json).unwrap(), report);
    }

    #[test]
    fn snapshots_name_their_fields_in_camel_case() {
        let report = VoiceReport::SessionSnapshot {
            server: Uuid::nil(),
            session: Uuid::nil(),
            channel: Uuid::nil(),
            participants: vec![ParticipantSnapshot {
                user: Uuid::nil(),
                muted: true,
                deafened: false,
                sharing_screen: true,
            }],
        };
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["type"], "sessionSnapshot");
        assert_eq!(json["participants"][0]["sharingScreen"], true);
        assert_eq!(serde_json::from_value::<VoiceReport>(json).unwrap(), report);
        let held = serde_json::to_value(VoiceReport::SessionsHeld {
            server: Uuid::nil(),
            partition: 3,
            sessions: vec![],
        })
        .unwrap();
        assert_eq!(held["type"], "sessionsHeld");
    }

    #[test]
    fn reports_go_in_the_lane_of_their_channel_and_speaking_in_its_own() {
        let server = Uuid::now_v7();
        let channel = Uuid::now_v7();
        let lane = partition(channel);
        assert!(lane < REPORT_PARTITIONS);
        let joined = VoiceReport::ParticipantJoined {
            session: Uuid::now_v7(),
            channel,
            user: Uuid::now_v7(),
            sign_in: None,
        };
        assert_eq!(
            joined.subject(server),
            format!("{REPORT_SUBJECT_ROOT}.{lane}.{server}")
        );
        let speaking = VoiceReport::Speaking {
            session: Uuid::now_v7(),
            channel,
            user: Uuid::now_v7(),
            speaking: true,
        };
        assert_eq!(
            speaking.subject(server),
            format!("{SPEAKING_SUBJECT_ROOT}.{lane}.{server}")
        );
        let load = VoiceReport::Load {
            server,
            participants: 0,
        };
        assert_eq!(
            load.subject(server),
            report_subject(partition(server), server)
        );
    }

    #[test]
    fn a_subject_names_its_server_last() {
        let server = Uuid::now_v7();
        assert_eq!(subject_server(&report_subject(3, server)), Some(server));
        assert_eq!(subject_server(&speaking_subject(63, server)), Some(server));
        assert_eq!(subject_server("aspen.voice.report.3.nonsense"), None);
    }

    #[test]
    fn the_lane_is_the_last_byte_modulo_the_lanes() {
        let key = Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdc7);
        assert_eq!(partition(key), 0xc7 % REPORT_PARTITIONS);
    }
}
