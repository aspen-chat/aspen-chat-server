//! What the interface layer reads of a call: one whole snapshot on every change, numbered, with
//! tracks as the platform's ids. It is what the web client's `VoiceCallState` carries where that
//! still applies, and what the new signalling adds: who is in the call, which audio slot carries
//! whom, and which videos are watched. Its parts cross to Swift and Kotlin as they are
//! (`uniffi::Enum`, `uniffi::Record`); the snapshot itself crosses as `bindings::CallState`.

use crate::peer_connection::TrackId;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use voice_protocol::signal::Grants;
use voice_protocol::signal::{MediaSource, SourceInfo, WatchedVideo};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum CallStatus {
    Idle,
    Joining,
    Connected,
    /// Between calls on the same channel without the user asking: taking a dropped socket back
    /// (`resume`), or joining the channel again after its server or the connection was lost.
    Rejoining,
    Failed,
}

/// What failed when `status` is `Failed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum SessionEndReason {
    Empty,
    ServerLost,
    Idle,
    ServerRemoved,
}

/// Someone else in the call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    pub user: Uuid,
    /// Muted by themself or a moderator; the server tells the two apart to no one.
    pub muted: bool,
    pub deafened: bool,
    pub speaking: bool,
    pub sources: Vec<SourceInfo>,
}

/// One of the audio sections the client receives, and whose microphone it carries now.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct RemoteVideo {
    pub user: Uuid,
    pub source: MediaSource,
    pub track: TrackId,
}

/// The sound of a shared screen the client watches. Only others' shares have one: the server
/// gives no section for the sound of the client's own share, which it hears at its source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct ShareAudio {
    pub user: Uuid,
    pub track: TrackId,
}

/// Something this client sends, or has asked to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct LocalSource {
    pub source: MediaSource,
    pub track: TrackId,
    /// Whether its media has arrived at the server (`sending`), which is when the others are
    /// told of it.
    pub live: bool,
}

/// A request the voice server would not honour.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct Refusal {
    pub detail: String,
    /// How long until the same request would be taken, when it was refused for coming too fast.
    pub retry_after_seconds: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// The mute and deafen the user themself last asked for, apart from the server's word, so a
    /// hold (the system taking the call's audio) restores exactly what they asked rather than
    /// asking a moderator's mute of them again.
    pub asked_muted: bool,
    pub asked_deafened: bool,
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
    pub source_refused: Option<(MediaSource, Refusal)>,
    pub ended_reason: Option<EndedReason>,
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
            asked_muted: false,
            asked_deafened: false,
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
        }
    }
}

/// Reads each snapshot as the call changes. Called on the engine's thread; keep it short.
pub trait CallObserver: Send + Sync {
    fn state_changed(&self, state: &CallState);
}
