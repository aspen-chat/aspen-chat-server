//! The signalling protocol between a client and a voice server, as JSON frames over a
//! WebSocket at the voice server's `/ws`. `voice_signal_schema.json`, written by the voice
//! server with `--gen-signal-schema`, describes every frame for the client's code generator.
//!
//! The flow: the client sends `identify` with its join token and receives `ready` with the
//! router's RTP capabilities and who is already in the call. It loads a mediasoup device from
//! those, sends `setCapabilities`, creates a send and a receive transport, connects them, and
//! produces its microphone. The server creates a consumer on the client's receive transport for
//! every producer of everyone else, present and future, announcing each with `newConsumer`;
//! the client acknowledges with `resumeConsumer` once it has set the consumer up. Everything
//! mediasoup-shaped (RTP parameters, ICE and DTLS parameters) crosses as opaque JSON that the
//! mediasoup libraries on each side understand.
//!
//! Files are offered to the call and sent between two participants over a WebRTC data channel
//! of their own, never through mediasoup. An offer (`offerFile`) stands for the time its sender
//! chose; each participant who accepts it (`acceptFile`) starts one transfer, in the mode they
//! chose among those the sender and this server allow. The server answers both people with
//! `transferStarting`, carrying the ICE servers for that transfer (its STUN, and its TURN relay
//! with credentials for this transfer alone), passes the peer connection's offer, answer, and
//! candidates between them (`transferSignal`), and ends the transfer when either side ends it
//! (`endTransfer`) or leaves. A transfer outlives its offer; the offer only bounds when it may
//! be accepted.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Which way media flows on a transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum TransportDirection {
    Send,
    Recv,
}

/// What a producer carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

/// The kind of media, as WebRTC names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Audio,
    Video,
}

/// One producer of a participant, as told to the others.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ProducerInfo {
    pub id: String,
    pub kind: MediaKind,
    pub source: MediaSource,
    pub paused: bool,
}

/// Someone in the call, as told to a newcomer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ParticipantInfo {
    pub user: Uuid,
    pub muted: bool,
    pub deafened: bool,
    pub speaking: bool,
    pub producers: Vec<ProducerInfo>,
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
    /// Must be the first frame: the join token from the API server.
    Identify {
        token: String,
    },
    /// The client's device capabilities, once it has loaded them from `ready`. The server
    /// creates consumers for the client only after this.
    SetCapabilities {
        rtp_capabilities: Value,
    },
    CreateTransport {
        direction: TransportDirection,
    },
    ConnectTransport {
        transport_id: String,
        dtls_parameters: Value,
    },
    Produce {
        transport_id: String,
        kind: MediaKind,
        source: MediaSource,
        rtp_parameters: Value,
    },
    CloseProducer {
        producer_id: String,
    },
    /// A producer fed by RTP the client sends itself rather than through a WebRTC transport:
    /// the desktop shell's game capture, whose helper encodes H.264 and sends SRTP straight to
    /// the voice server, and the benchmark's simulated participants. `screen` is H.264;
    /// `microphone` and `screenAudio` are Opus. The server answers `rtpProduced` with where and
    /// how to send.
    ProduceRtp {
        source: MediaSource,
    },
    /// Receive everyone else's media over plain RTP instead of WebRTC, for a client that is not
    /// a browser (the benchmark's simulated participants). The server creates a plain transport
    /// (SRTP, RTP and RTCP multiplexed, the client's address learned from the first packet it
    /// sends), answers `rtpConsuming`, and from then on the participant's consumers, announced
    /// with `newConsumer` as usual, are on it.
    ConsumeRtp,
    /// The client has set the consumer up and wants media on it.
    ResumeConsumer {
        consumer_id: String,
    },
    /// Muting pauses the microphone producer on the server; deafening pauses every consumer.
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
    /// The reply to `identify`.
    Ready {
        session: Uuid,
        user: Uuid,
        /// The router's RTP capabilities, for loading the client's mediasoup device.
        router_rtp_capabilities: Value,
        participants: Vec<ParticipantInfo>,
        /// The offers standing in the call.
        #[serde(default)]
        offers: Vec<FileOffer>,
        /// Every transfer under way in the call, one entry each.
        #[serde(default)]
        links: Vec<TransferLink>,
        #[serde(default)]
        transfers: TransferPolicy,
    },
    TransportCreated {
        direction: TransportDirection,
        id: String,
        ice_parameters: Value,
        ice_candidates: Value,
        dtls_parameters: Value,
    },
    TransportConnected {
        transport_id: String,
    },
    Produced {
        producer_id: String,
        source: MediaSource,
    },
    /// The answer to `produceRtp`: an H.264 producer awaiting SRTP at `ip`:`port` (RTP and
    /// RTCP multiplexed, so RTCP feedback comes back from the same address) with the given
    /// SSRC and payload type, encrypted both ways with the key. The client's own preview of
    /// what it sends arrives as a `newConsumer` naming the client itself.
    RtpProduced {
        producer_id: String,
        source: MediaSource,
        ip: String,
        port: u16,
        ssrc: u32,
        payload_type: u8,
        srtp_crypto_suite: String,
        srtp_key_base64: String,
    },
    /// Where media for a `consumeRtp` client comes from: the plain transport to send a first
    /// packet (any SRTCP) to, and the key that decrypts what arrives and encrypts what is sent.
    RtpConsuming {
        ip: String,
        port: u16,
        srtp_crypto_suite: String,
        srtp_key_base64: String,
    },
    /// A consumer the server created for one of another participant's producers. It starts
    /// paused; the client answers with `resumeConsumer`.
    NewConsumer {
        consumer_id: String,
        producer_id: String,
        user: Uuid,
        kind: MediaKind,
        source: MediaSource,
        rtp_parameters: Value,
        producer_paused: bool,
    },
    ConsumerClosed {
        consumer_id: String,
    },
    /// A producer the client consumes was paused or resumed at its source, such as by a mute.
    ProducerPaused {
        producer_id: String,
        paused: bool,
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
    /// last of these. Its producers of a source no longer allowed are already closed, and its
    /// file offers withdrawn when it may no longer transfer files; it stops sending them.
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
    /// The server is shutting down.
    ServerStopping,
    /// The user may no longer be in the call: they lost access to its channel, or left or were
    /// removed from where it is.
    AccessLost,
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
