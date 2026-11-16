//! What the interface layer reads of a call: one whole snapshot on every change, numbered, with
//! tracks as the platform's ids. It is what the web client's `VoiceCallState` carries where that
//! still applies, and what the new signalling adds: who is in the call, which audio slot carries
//! whom, and which videos are watched. The snapshot and its parts cross to Swift and Kotlin as
//! they are (`uniffi::Enum`, `uniffi::Record`), and to the apps' web views as JSON
//! (`Engine::state_json`, the observer's `json`), whose shape is `call_bridge_schema.json`,
//! written from `CallState`'s `JsonSchema` by the `call-bridge-schema` binary and read by the
//! client's code generator: camelCase names, as every schema the client is generated from.

use crate::peer_connection::TrackId;
use crate::sounds::SoundCue;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use voice_protocol::signal::Grants;
use voice_protocol::signal::{
    MediaSource, SourceInfo, TransferLink, TransferMode, TransferRole, WatchedVideo,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum CallStatus {
    Idle,
    Joining,
    Connected,
    /// Joining the channel again without the user asking, after its server or the connection
    /// was lost, or a dropped socket could not be taken back (`resume`) within the server's
    /// grace; a resume within it keeps the call `Connected`.
    Rejoining,
    Failed,
}

/// What failed when `status` is `Failed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// The API server refused the join offer (`error` is the Problem's text).
    Offer,
    /// The channel lets the user speak and no microphone track was given.
    Microphone,
    /// Every voice server offered failed to take the call.
    Server,
    /// The voice server refused one of the call's requests, most often for coming too fast.
    Refused,
    /// This device could not set the call up: the peer connection could not be made or the
    /// offer not applied, which no voice server is blamed for.
    Device,
}

/// Why the call ended without the user asking, until `acknowledge_end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum EndedReason {
    /// A moderator removed the user.
    Kicked,
    /// The user may no longer be in the call.
    AccessLost,
    /// The API server ended the call for going a day without two people.
    Idle,
}

/// Why the API server ended a call, as its `voiceSessionEnded` event says
/// (`aspen_wire::VoiceSessionEndReason`, which the app layer hands on).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum SessionEndReason {
    Empty,
    ServerLost,
    Idle,
    ServerRemoved,
}

/// Someone else in the call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub user: Uuid,
    /// Muted by themself or a moderator; the server tells the two apart to no one.
    pub muted: bool,
    pub deafened: bool,
    /// On hold, as the server says: nothing of theirs is sent, nor anything to them.
    pub held: bool,
    pub speaking: bool,
    pub sources: Vec<SourceInfo>,
}

/// One of the audio sections the client receives, and whose microphone it carries now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AudioSlot {
    pub slot: u8,
    /// Whose voice plays from it; `None` while it is silent. Follows the server's `slots`
    /// frames, so a person's volume and mute-for-me are applied to whichever slot they are in.
    pub user: Option<Uuid>,
    /// The platform's track for it, once the offer that added its section has been answered.
    pub track: Option<TrackId>,
}

/// A video the client watches and has a section for: a camera or a shared screen. The client's
/// own share, watched for its preview, is one too.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct RemoteVideo {
    pub user: Uuid,
    pub source: MediaSource,
    pub track: TrackId,
}

/// The sound of a shared screen the client watches. Only others' shares have one: the server
/// gives no section for the sound of the client's own share, which it hears at its source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ShareAudio {
    pub user: Uuid,
    pub track: TrackId,
}

/// Something this client sends, or has asked to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct LocalSource {
    pub source: MediaSource,
    pub track: TrackId,
    /// Whether its media has arrived at the server (`sending`), which is when the others are
    /// told of it.
    pub live: bool,
    /// Sent on the capture connection, a second peer connection of its own, as the phones'
    /// shares are (`call::Command::StartSource`), rather than on the call's.
    pub external: bool,
}

/// A request the voice server would not honour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub detail: String,
    /// How long until the same request would be taken, when it was refused for coming too fast.
    pub retry_after_seconds: Option<u64>,
}

/// A source the voice server refused to take, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SourceRefusal {
    pub source: MediaSource,
    pub refusal: Refusal,
}

/// A file offered in the call, this device's own included, while it may still be accepted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct FileOfferState {
    pub id: Uuid,
    pub from: Uuid,
    /// As the sender named it; the page shows and saves it without format and control
    /// characters (`plainFileName`).
    pub name: String,
    pub size: u64,
    pub allow_direct: bool,
    /// When the offer can no longer be accepted, in milliseconds since the Unix epoch on this
    /// device's clock, counted from when the server's frame arrived; the page judges it with the
    /// same clock, which is also the engine's for taking the offer down.
    pub expires_at_ms: u64,
    /// Whether this device made it.
    pub own: bool,
}

/// Where a transfer stands: connecting, moving bytes, or ended as `TransferEnd` says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum TransferStatus {
    Connecting,
    Moving,
    Completed,
    Cancelled,
    Failed,
    Left,
    NotPermitted,
}

/// How a transfer's bytes travel, whatever its receiver preferred: straight between the two
/// devices, or through the voice server's relay; read from the connection's selected candidate
/// pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum TransferRoute {
    Direct,
    Relayed,
}

/// Who ended a transfer: this device, or the other side (the server speaking for it, when it
/// left or lost the grant).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum EndedBy {
    #[serde(rename = "self")]
    ThisDevice,
    Peer,
}

/// A transfer this device is one side of, kept once ended until dismissed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct TransferState {
    pub offer: Uuid,
    pub peer: Uuid,
    pub role: TransferRole,
    pub mode: TransferMode,
    pub name: String,
    pub size: u64,
    /// Bytes sent, or received, so far; moved on at most every `transfers::PROGRESS_INTERVAL`,
    /// and whenever anything else about the transfer changes.
    pub bytes: u64,
    pub status: TransferStatus,
    /// The route in use, or last used once ended; `None` before the connection came up.
    pub route: Option<TransferRoute>,
    /// Who ended it, once it has ended.
    pub ended_by: Option<EndedBy>,
    /// Why it failed, in the platform's or the engine's words, when it did and the reason is
    /// this device's to know: the file could not be read or written, the connection broke.
    pub failure: Option<String>,
}

/// Which file command of this device's failed before any transfer came of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum FileCommand {
    /// Offering: the picked file could not be opened, or the voice server refused the offer.
    Offer,
    /// Accepting: the chosen place could not be opened, or the voice server refused the
    /// acceptance.
    Accept,
}

/// A file command that failed, with why, until the next file command or `clear_file_failure`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct FileFailure {
    pub command: FileCommand,
    pub offer: Uuid,
    /// The file's name as offered, for the page to name it by.
    pub name: String,
    /// Why, in the platform's or the voice server's words.
    pub detail: String,
}

/// The call's files as this device sees them: what the web client's `FilesState` carries, and
/// `failure`, which only an engine has, since only it can fail to open a file.
#[derive(
    Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, uniffi::Record,
)]
#[serde(rename_all = "camelCase")]
pub struct FilesState {
    /// Offers that may still be accepted, this device's own included, in the order they came.
    pub offers: Vec<FileOfferState>,
    /// Transfers this device is one side of, live ones first, then the ended until dismissed.
    pub transfers: Vec<TransferState>,
    /// Every transfer under way in the call, one entry each, whoever its two sides are.
    pub links: Vec<TransferLink>,
    /// The megabits a second every relayed transfer on this voice server shares; `None` when it
    /// does not relay.
    pub relay_mbps: Option<u32>,
    pub failure: Option<FileFailure>,
}

/// The whole snapshot, the root of `call_bridge_schema.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct CallState {
    /// Counts up with every snapshot that differs from the one before.
    pub revision: u64,
    pub status: CallStatus,
    /// The channel being joined or in; kept while `Failed`, so the reason can be shown there.
    pub channel: Option<Uuid>,
    /// The call's session at the API server, from `ready`; `None` until then.
    pub session: Option<Uuid>,
    /// The user the voice server admitted, from `ready`.
    pub user: Option<Uuid>,
    /// In a call, the server's word, except that the user shows silenced the moment they ask
    /// to be; outside one, what they last asked for.
    pub muted: bool,
    pub deafened: bool,
    /// Whether the call is on hold (`set_held`), as a phone puts it while a phone call has its
    /// audio: the voice server sends none of the user's media and none to them, and their mute
    /// and deafen stay as they are, at the server and here, for when it ends. Shown the moment
    /// it is asked, like a mute; its end waits for the server's word.
    pub held: bool,
    /// Whether a moderator's mute of the user stands in the call's community.
    pub server_muted: bool,
    /// What the channel lets the user do, from the join offer and then `grantsChanged`.
    pub grants: Grants,
    pub participants: Vec<Participant>,
    /// The microphone track the engine holds, sent while the user may speak and is not muted.
    /// Kept across a move between calls and a rejoin; dropped once the user may not speak in
    /// the call joined, which tells the platform to release it.
    pub microphone: Option<TrackId>,
    pub sources: Vec<LocalSource>,
    /// How many audio slots the server fills at most (`ready`); `slots` grows towards it.
    pub audio_slots: u8,
    /// The most videos the server gives sections for (`ready`); a `watch` is cut to it.
    pub max_watched_video: u32,
    pub slots: Vec<AudioSlot>,
    /// The videos the client asked to show, sent as `watch`, the first `max_watched_video` of
    /// them once that is known; the server gives a section for each that exists, as `videos`
    /// and `share_audio` then show.
    pub watching: Vec<WatchedVideo>,
    pub videos: Vec<RemoteVideo>,
    pub share_audio: Vec<ShareAudio>,
    pub error_kind: Option<ErrorKind>,
    pub error: Option<String>,
    /// When the voice server refused a request for coming too fast, how many seconds until it
    /// would take it.
    pub retry_after_seconds: Option<u64>,
    /// The voice server refused to unmute or undeafen the user, who stays as `muted` and
    /// `deafened` show, until they try again, dismiss it, or the call ends.
    pub state_refused: Option<Refusal>,
    /// The voice server refused a source the client asked to send, until the next is asked.
    pub source_refused: Option<SourceRefusal>,
    pub ended_reason: Option<EndedReason>,
    /// The call's files: offers, this device's transfers, everyone's links, and the relay's
    /// limit (`transfers`).
    pub files: FilesState,
}

impl Default for CallState {
    fn default() -> Self {
        Self {
            revision: 0,
            status: CallStatus::Idle,
            channel: None,
            session: None,
            user: None,
            muted: false,
            deafened: false,
            held: false,
            server_muted: false,
            grants: Grants::default(),
            participants: Vec::new(),
            microphone: None,
            sources: Vec::new(),
            audio_slots: 0,
            max_watched_video: 0,
            slots: Vec::new(),
            watching: Vec::new(),
            videos: Vec::new(),
            share_audio: Vec::new(),
            error_kind: None,
            error: None,
            retry_after_seconds: None,
            state_refused: None,
            source_refused: None,
            ended_reason: None,
            files: FilesState::default(),
        }
    }
}

/// Reads each snapshot as the call changes, and plays each of the call's sounds as the change
/// cues it (`sounds`), after the snapshot it follows from. Called on the engine's thread; keep
/// it short.
pub trait CallObserver: Send + Sync {
    fn state_changed(&self, state: &CallState);
    fn play_sound(&self, cue: SoundCue);
}
