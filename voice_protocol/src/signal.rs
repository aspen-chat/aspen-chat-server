//! The signalling protocol between a client and a voice server, as JSON frames over a
//! WebSocket at the voice server's `/ws`. `voice_signal_schema.json`, written by the voice
//! server with `--gen-signal-schema`, describes every frame for the client's code generator.
//!
//! The flow: the client sends `identify` with its join token and receives `ready`: who is in
//! the call, what each sends, the most audio slots it receives, and a secret to resume with.
//! The server is the only side that offers. It then offers the client's peer connection
//! (`offer`), and the client answers (`answer`). Whatever the client wants changed is a request
//! (`send` a source, `watch` video, `restartIce`), which the server makes and offers. A
//! connection never has two offers outstanding, so neither side handles a collision.
//!
//! Every offer names what each of its media sections carries (`sections`): a source the client
//! sends, an audio slot, or another participant's video or a share's sound. A client receives an
//! audio slot for each other microphone up to a fixed number, which the server fills with whoever
//! is loudest and says whom each carries (`slots`) as it changes; and a section for each video it
//! watches. The desktop
//! shell's game capture helper holds a peer connection of its own (`ConnectionKind::Capture`),
//! negotiated the same way through the client.
//!
//! A client whose socket closes without `leave` stays in the call for `RESUME_GRACE`, its media
//! flowing, and takes its place back with `resume` and the secret `ready` gave.
//!
//! Files are offered to the call and sent between two participants over a WebRTC data channel
//! of their own, never through the server's media threads. An offer (`offerFile`) stands for the
//! time its sender chose; each participant who accepts it (`acceptFile`) starts one transfer, in
//! the mode they chose among those the sender and this server allow. The server answers both people with
//! `transferStarting`, carrying the ICE servers for that transfer (its STUN, and its TURN relay
//! with credentials for this transfer alone), passes the peer connection's offer, answer, and
//! candidates between them (`transferSignal`), and ends the transfer when either side ends it
//! (`endTransfer`) or leaves. A transfer outlives its offer; the offer only bounds when it may
//! be accepted.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// How long a participant whose socket closed without `leave` stays in the call, media
/// flowing, waiting to be resumed.
pub const RESUME_GRACE_SECONDS: u64 = 20;

/// What a participant sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MediaSource {
    /// The participant's voice.
    Microphone,
    /// A shared screen, window, or game, as video.
    Screen,
    /// The audio that goes with a shared screen or game.
    ScreenAudio,
    /// The participant's camera, as video.
    Camera,
}

impl MediaSource {
    /// The kind of media this source is sent as, which decides the kind of the section offered
    /// for it: what a participant may send, and what a mute pauses, is decided by source, so a
    /// source must mean what it says.
    pub fn kind(self) -> MediaKind {
        match self {
            MediaSource::Microphone | MediaSource::ScreenAudio => MediaKind::Audio,
            MediaSource::Screen | MediaSource::Camera => MediaKind::Video,
        }
    }
}

/// The kind of media, as WebRTC names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Audio,
    Video,
}

/// Which of a participant's peer connections a frame is about: the one every participant holds,
/// or the one the desktop shell's game capture helper holds for what it sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ConnectionKind {
    Call,
    Capture,
}

/// What one media section of a peer connection carries, so a client can place each track it
/// receives and attach each it sends. Told with the offer that adds the section, never after.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "carries",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum SectionContent {
    /// The client sends this source on it.
    Sending { source: MediaSource },
    /// One of the call's audio slots, which carries whichever microphone `slots` names.
    AudioSlot { slot: u8 },
    /// Another participant's video (`screen` or `camera`), or the client's own when it watches a
    /// share it sends from outside the browser.
    Video { user: Uuid, source: MediaSource },
    /// The sound of a shared screen the client watches.
    ShareAudio { user: Uuid },
}

/// One media section of an offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    /// The section's `a=mid` in the SDP.
    pub mid: String,
    pub kind: MediaKind,
    #[serde(flatten)]
    pub content: SectionContent,
}

/// Which microphone one audio slot carries from now on; `None` leaves it silent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SlotAssignment {
    pub slot: u8,
    pub user: Option<Uuid>,
}

/// One source a participant sends, as told to the others: only once its media has arrived.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub source: MediaSource,
    /// Paused at the server: a muted microphone.
    pub paused: bool,
}

/// A video a client shows, and wants a section for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WatchedVideo {
    pub user: Uuid,
    pub source: MediaSource,
}

/// Someone in the call, as told to a newcomer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantInfo {
    pub user: Uuid,
    pub muted: bool,
    pub deafened: bool,
    pub speaking: bool,
    pub sources: Vec<SourceInfo>,
}

/// How a transfer travels, as the person receiving it chose.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TransferMode {
    /// Straight between the two devices when a hole-punched connection can be made, which lets
    /// each learn the other's address; through the relay otherwise, when this server relays.
    DirectPreferred,
    /// Through this server's relay alone, so neither side learns the other's address.
    RelayOnly,
}

/// Which end of a transfer a participant is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TransferRole {
    Sender,
    Receiver,
}

/// Why a transfer ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TransferEnd {
    /// Every byte arrived.
    Completed,
    /// One side cancelled it.
    Cancelled,
    /// The connection could not be made or broke.
    Failed,
    /// The other side left the call. Only the server says this.
    Left,
    /// The sender may no longer offer files in the call (Transfer files). Only the server says
    /// this, to both sides.
    NotPermitted,
}

/// Why an offer stopped standing. Transfers it started go on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum OfferEnd {
    /// Its sender withdrew it.
    Withdrawn,
    /// The time its sender gave it ran out.
    Expired,
    /// Its sender left the call.
    Left,
    /// Its sender may no longer offer files in the call.
    NotPermitted,
}

/// A file offered to the others in the call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FileOffer {
    pub id: Uuid,
    pub from: Uuid,
    pub name: String,
    /// In bytes, as the sender stated it.
    pub size: u64,
    /// Whether the sender lets people who accept connect to them directly.
    pub allow_direct: bool,
    /// How much longer the offer may be accepted, counted from when this frame was sent, so
    /// that clocks need not agree.
    pub expires_in_ms: u64,
}

/// A transfer under way in the call, as everyone in it is told: who sends to whom, and nothing
/// of what or how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TransferLink {
    pub sender: Uuid,
    pub receiver: Uuid,
}

/// Whether this server relays transfers, and how fast. A server that says nothing of it relays
/// nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TransferPolicy {
    /// Every relayed transfer on this server together is held to this many megabits a second;
    /// absent when this server does not relay transfers, which leaves direct transfers alone.
    pub relay_mbps: Option<u32>,
}

/// One ICE server for a transfer's peer connection, as `RTCIceServer` takes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// Frames a client sends.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    JsonSchema,
    strum::IntoStaticStr,
    strum::VariantNames,
)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[strum(serialize_all = "camelCase")]
pub enum ClientMessage {
    /// Must be the first frame, or `resume`: the join token from the API server.
    Identify {
        token: String,
    },
    /// In place of `identify`, within `RESUME_GRACE_SECONDS` of the socket closing: takes back
    /// the participant the secret was given to (`ready`'s `resume`). The server answers with
    /// `ready`, a new secret in it, and each connection's outstanding offer again, or its
    /// sections (`sections`) where none is outstanding.
    Resume {
        secret: String,
    },
    /// The answer to the offer of `revision` on `connection`.
    Answer {
        connection: ConnectionKind,
        revision: u64,
        sdp: String,
    },
    /// Asks to send `source`: the server offers a section for it, which the client answers with
    /// its track. `external` sends it from the game capture helper's connection.
    Send {
        source: MediaSource,
        #[serde(default)]
        external: bool,
    },
    StopSending {
        source: MediaSource,
    },
    /// The video the client shows, replacing what it watched before: the server offers a
    /// section for each (and for a screen's sound), and takes away those no longer named.
    /// The client's own share is named for its preview.
    Watch {
        video: Vec<WatchedVideo>,
    },
    /// Whose voice, and whose share's sound, the client has silenced (blocked, or set to
    /// nothing), replacing what it said before: the server puts no one in `voices` in the
    /// client's audio slots and offers no share sound of anyone in `shares`, so nothing of theirs
    /// reaches it, not even for a moment as a slot changes hands. Each list is cut to the call's
    /// most participants.
    Silence {
        #[serde(default)]
        voices: Vec<Uuid>,
        #[serde(default)]
        shares: Vec<Uuid>,
    },
    /// Stops or resumes one section the client receives, with no renegotiation: video off
    /// screen, someone muted for this client alone.
    SetReceiving {
        mid: String,
        receiving: bool,
    },
    /// Asks for an ICE restart, as after a change of network: the next offer carries new ICE
    /// credentials.
    RestartIce {
        connection: ConnectionKind,
    },
    /// Muting pauses the microphone at the server; deafening stops the audio slots and the
    /// shares' sound sent to the client, not video. A moderator's mute stands over this one:
    /// unmuting leaves the microphone paused while it does, and the `participantState` that
    /// answers says so.
    SetState {
        muted: bool,
        deafened: bool,
    },
    /// Offers a file to everyone else in the call, under an id the client chose, for
    /// `valid_for_seconds`. Takes the join token's Transfer files grant.
    OfferFile {
        offer: Uuid,
        name: String,
        size: u64,
        allow_direct: bool,
        valid_for_seconds: u32,
    },
    /// Takes back one of the client's own offers. Transfers it started go on.
    WithdrawFile {
        offer: Uuid,
    },
    /// Accepts an offer that still stands, starting a transfer from its sender to the client.
    AcceptFile {
        offer: Uuid,
        mode: TransferMode,
    },
    /// Part of the transfer's peer connection (its offer, answer, or a candidate), for the
    /// other side; the server passes it on unread.
    TransferSignal {
        offer: Uuid,
        peer: Uuid,
        signal: Value,
    },
    /// Ends one transfer, telling the other side why: `completed`, `cancelled`, or `failed`.
    EndTransfer {
        offer: Uuid,
        peer: Uuid,
        reason: TransferEnd,
    },
    Leave,
}

impl ClientMessage {
    /// Every frame type's name, its `type` on the wire. `strum` names them, and a test holds
    /// its names to the ones `serde` gives.
    pub const KINDS: &'static [&'static str] = <Self as strum::VariantNames>::VARIANTS;

    /// This frame's type, as on the wire.
    pub fn kind(&self) -> &'static str {
        self.into()
    }
}

/// Frames a voice server sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ServerMessage {
    /// The reply to `identify` and to `resume`.
    Ready {
        session: Uuid,
        user: Uuid,
        participants: Vec<ParticipantInfo>,
        /// The most audio slots the client's offers carry: one from the start, and one more for
        /// each other microphone in the call, up to this, never fewer than before.
        audio_slots: u8,
        /// The most videos a `watch` is given sections for; those past it are left out.
        max_watched_video: u32,
        /// The secret to `resume` with, made anew with each `ready`.
        resume: String,
        /// The offers standing in the call.
        #[serde(default)]
        offers: Vec<FileOffer>,
        /// Every transfer under way in the call, one entry each.
        #[serde(default)]
        links: Vec<TransferLink>,
        #[serde(default)]
        transfers: TransferPolicy,
    },
    /// An offer for one of the client's connections, which it answers with `answer`;
    /// `sections` names what every section of it carries.
    Offer {
        connection: ConnectionKind,
        revision: u64,
        sdp: String,
        sections: Vec<Section>,
    },
    /// What every section of a connection carries, after a `resume`, where no offer is
    /// outstanding to say so.
    Sections {
        connection: ConnectionKind,
        sections: Vec<Section>,
    },
    /// A source the client asked to send carries media at the server, and is sent on.
    Sending {
        source: MediaSource,
    },
    /// What a participant sends now, whenever it changes.
    Sources {
        user: Uuid,
        sources: Vec<SourceInfo>,
    },
    /// Whom the client's audio slots carry from now on, those that changed.
    Slots {
        assignments: Vec<SlotAssignment>,
    },
    ParticipantJoined {
        user: Uuid,
        muted: bool,
        deafened: bool,
    },
    ParticipantLeft {
        user: Uuid,
    },
    ParticipantState {
        user: Uuid,
        muted: bool,
        deafened: bool,
    },
    Speaking {
        user: Uuid,
        speaking: bool,
    },
    /// The server closed the client's place in the call.
    Kicked {
        reason: KickReason,
    },
    /// A file offered to the call, the client's own included.
    FileOffered {
        offer: FileOffer,
    },
    /// An offer no longer stands.
    FileWithdrawn {
        offer: Uuid,
        reason: OfferEnd,
    },
    /// A transfer begins between the client and `peer`: the sender opens a peer connection
    /// with these ICE servers (only the relay's in `relayOnly`) and a data channel, and the
    /// receiver answers.
    TransferStarting {
        offer: Uuid,
        peer: Uuid,
        role: TransferRole,
        mode: TransferMode,
        /// The file's name and size as offered, which the transfer keeps however long it
        /// outlives the offer.
        name: String,
        size: u64,
        ice_servers: Vec<IceServer>,
    },
    /// Part of `peer`'s side of the transfer's peer connection.
    TransferSignal {
        offer: Uuid,
        peer: Uuid,
        signal: Value,
    },
    /// The transfer between the client and `peer` ended; the client closes it at once.
    TransferEnded {
        offer: Uuid,
        peer: Uuid,
        reason: TransferEnd,
    },
    /// To everyone in the call: a transfer between two participants began (`active`) or ended.
    /// Sent once for each transfer, so two between the same people are two starts and two ends.
    TransferLinkChanged {
        link: TransferLink,
        active: bool,
    },
    /// What the client may now do in the call, which changed since it joined or since the
    /// last of these. Its sources no longer allowed are already stopped, and its file offers
    /// withdrawn when it may no longer transfer files; it stops sending them.
    GrantsChanged {
        grants: crate::token::Grants,
    },
    /// A request could not be honoured; the connection stays open unless `fatal`.
    Error {
        detail: String,
        fatal: bool,
        /// Set when the request was refused for coming too fast: how many seconds until the
        /// same request would be taken.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_seconds: Option<u64>,
        /// The `type` of the client frame this answers, when it answers one: a client tells
        /// by it which of its requests was refused (a `setState` unmuting, for instance, so it
        /// goes on showing itself muted).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refused: Option<String>,
    },
}

/// Why a server closed a client's place in the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum KickReason {
    /// A moderator removed the user.
    Kicked,
    /// The same user connected again from elsewhere.
    Replaced,
    /// The server is closing the call: it is shutting down, or the call goes on on another
    /// server. The client rejoins, and its join offer names the server to go to.
    ServerStopping,
    /// The user may no longer be in the call: they lost access to its channel, or left or were
    /// removed from where it is.
    AccessLost,
    /// The client's peer connection failed at the server: an offer went unanswered, an answer
    /// could not be applied, or it never connected. The client rejoins.
    ConnectionFailed,
    /// The sign-in the user joined with ended: they signed out there, or ended it from another
    /// device, changed their password, or (for a bot) had its token replaced.
    SignedOut,
}

/// The whole protocol, the root of `voice_signal_schema.json`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSignalProtocol {
    pub client_message: ClientMessage,
    pub server_message: ServerMessage,
}

#[cfg(test)]
mod client_message_kinds {
    use super::ClientMessage;

    /// `KINDS` is exactly the `type` values the schema allows.
    #[test]
    fn kinds_are_the_wire_types() {
        let schema = serde_json::to_value(schemars::schema_for!(ClientMessage)).unwrap();
        let mut wire: Vec<String> = schema["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .map(|variant| {
                variant["properties"]["type"]["const"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        wire.sort();
        let mut kinds: Vec<String> = ClientMessage::KINDS.iter().map(|k| k.to_string()).collect();
        kinds.sort();
        assert_eq!(wire, kinds);
    }
}
