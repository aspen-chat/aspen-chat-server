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

/// Frames a client sends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
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
    /// The client has set the consumer up and wants media on it.
    ResumeConsumer {
        consumer_id: String,
    },
    /// Muting pauses the microphone producer on the server; deafening pauses every consumer.
    SetState {
        muted: bool,
        deafened: bool,
    },
    Leave,
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
    /// A request could not be honoured; the connection stays open unless `fatal`.
    Error {
        detail: String,
        fatal: bool,
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
}

/// The whole protocol, the root of `voice_signal_schema.json`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VoiceSignalProtocol {
    pub client_message: ClientMessage,
    pub server_message: ServerMessage,
}
