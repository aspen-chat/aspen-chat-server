//! The peer connection as the engine drives it: what each platform's WebRTC must do for the
//! engine, and no more. The iOS layer implements it over `RTCPeerConnection` and the Android layer
//! over `org.webrtc.PeerConnection`; media (tracks, frames, devices) stays on their side, named
//! here by opaque ids. Everything that reaches the network goes through this interface, so a test
//! can stand in for the platform and replay what a real one answered. The enums and records
//! here cross to Swift and Kotlin as they are; the traits cross through `bindings`, those the
//! platform implements (`PeerConnectionFactory`, `PeerConnection`, `DataChannel`) and those the
//! engine implements and hands to it (`PeerConnectionObserver`, `DataChannelObserver`) alike.
//!
//! The call's own connection is offered by the voice server and carries media alone. A file
//! transfer's connection is between two clients (`transfers`): the sender offers it, both
//! trickle their ICE candidates through the voice server, and it carries one data channel.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum MediaKind {
    Audio,
    Video,
}

/// A platform's track: a microphone or camera it opened, or a track a receiver gave it. The
/// engine passes these on and never looks inside.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TrackId(pub String);

/// A transceiver of one peer connection, as the platform numbers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TransceiverId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum DescriptionKind {
    Offer,
    Answer,
    /// Takes back a local offer the remote end never answered, returning the connection to
    /// `Stable`; it carries no SDP.
    Rollback,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct SessionDescription {
    #[serde(rename = "type")]
    pub kind: DescriptionKind,
    /// Empty for a `Rollback`.
    #[serde(default)]
    pub sdp: String,
}

/// Where the offer-answer exchange stands, as `RTCSignalingState` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "kebab-case")]
pub enum SignalingState {
    Stable,
    HaveLocalOffer,
    HaveRemoteOffer,
    HaveLocalPranswer,
    HaveRemotePranswer,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    SendRecv,
    SendOnly,
    RecvOnly,
    Inactive,
}

/// One encoding a sender is asked for, as `RTCRtpEncodingParameters` has it. The engine asks for
/// one encoding per producer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct SendEncoding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bitrate: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_framerate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_resolution_down_by: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
}

/// A sender's parameters as `getParameters` gives them and `setParameters` takes them back:
/// its encodings, which is all the engine changes, and the rest carried as it came, since
/// `setParameters` refuses parameters that are not the last ones read.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SenderParameters {
    pub encodings: Vec<SendEncoding>,
    /// Everything else `getParameters` returned, as JSON, handed back unchanged.
    pub rest: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

/// How a peer connection is made: always unified plan, one ICE and DTLS association for every
/// section (`max-bundle`), RTCP multiplexed, as mediasoup requires of a client.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct Configuration {
    pub ice_servers: Vec<IceServer>,
    /// `relay` to use a TURN relay alone; `all` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ice_transport_policy: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionState {
    New,
    Connecting,
    Connected,
    Disconnected,
    Failed,
    Closed,
}

/// Whose statistics are asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "camelCase")]
pub enum StatsOf {
    Connection,
    Sender(TransceiverId),
    Receiver(TransceiverId),
}

/// An ICE candidate as `RTCIceCandidateInit` carries it, which is how a transfer's two sides
/// exchange them through the voice server, a browser among them. A browser's last candidate may
/// be the end-of-candidates mark, an empty `candidate`, which the engine never hands to the
/// platform: native WebRTC parses it as no candidate and refuses it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct IceCandidate {
    pub candidate: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdp_mid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdp_m_line_index: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username_fragment: Option<String>,
}

/// Where a data channel stands, as `RTCDataChannelState` names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, uniffi::Enum)]
#[serde(rename_all = "lowercase")]
pub enum DataChannelState {
    Connecting,
    Open,
    Closing,
    Closed,
}

/// What the platform's WebRTC refused or could not do, with the platform's own words.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[error("{0}")]
pub struct PeerConnectionError(pub String);

/// What the engine hears of a data channel without asking. Each message arrives whole, in the
/// order sent, binary and text told apart as the sender sent them.
pub trait DataChannelObserver: Send + Sync {
    fn state_changed(&self, state: DataChannelState);
    fn binary(&self, data: Vec<u8>);
    fn text(&self, text: String);
    /// The bytes waiting to be sent fell to the threshold `set_buffered_amount_low_threshold`
    /// gave, or below it, from above.
    fn buffered_amount_low(&self);
}

/// One data channel, ordered and reliable: the engine's file transfers want every byte, in
/// order, and ask for no other kind. `send_binary` and `send_text` queue a message and fail only
/// when the channel is not open; `buffered_amount` is what is queued and not yet sent, which the
/// sender keeps between two marks by pausing until `buffered_amount_low`. Every method may be
/// called from the engine's thread while the platform's WebRTC thread calls the observer.
pub trait DataChannel: Send + Sync {
    fn send_binary(&self, data: Vec<u8>) -> Result<(), PeerConnectionError>;
    fn send_text(&self, text: String) -> Result<(), PeerConnectionError>;
    fn buffered_amount(&self) -> u64;
    fn set_buffered_amount_low_threshold(&self, bytes: u64);
    fn state(&self) -> DataChannelState;
    /// Closes the channel, sending what it holds first. Closing a closed channel does nothing.
    fn close(&self);
}

/// What the engine hears of a peer connection without asking.
pub trait PeerConnectionObserver: Send + Sync {
    fn connection_state_changed(&self, state: ConnectionState);
    /// The connection gathered a candidate of its own, for the other side.
    fn ice_candidate(&self, candidate: IceCandidate);
    /// The other side opened a data channel on this connection. The observer answered reads
    /// the channel from then on, the messages that arrived before it was asked for included,
    /// in order.
    fn data_channel_opened(&self, channel: Arc<dyn DataChannel>) -> Arc<dyn DataChannelObserver>;
}

/// What a peer connection is made for, which decides the WebRTC factory the platform makes it
/// from. A WebRTC factory has one audio capture for every connection it makes, so the call's
/// own connection (`Call`), whose audio capture is the microphone, and the capture connection
/// (`Capture`, `voice_protocol::signal::ConnectionKind::Capture`), which carries a share sent
/// from outside the call's own and whose audio capture is the share's sound, come from
/// factories of their own. A transfer's connection (`Transfer`) carries one data channel and
/// no media, so the call's factory makes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum PeerConnectionKind {
    Call,
    Capture,
    Transfer,
}

/// Makes peer connections. One per platform process.
#[async_trait]
pub trait PeerConnectionFactory: Send + Sync {
    async fn create(
        &self,
        kind: PeerConnectionKind,
        configuration: Configuration,
        observer: Arc<dyn PeerConnectionObserver>,
    ) -> Result<Arc<dyn PeerConnection>, PeerConnectionError>;
}

/// One peer connection. Each method is one W3C operation, named as it is there, so the iOS and
/// Android implementations are a line or two each and what a test replays is what a browser
/// would have been asked.
#[async_trait]
pub trait PeerConnection: Send + Sync {
    async fn create_offer(&self, ice_restart: bool) -> Result<String, PeerConnectionError>;
    async fn create_answer(&self) -> Result<String, PeerConnectionError>;
    async fn set_local_description(
        &self,
        description: SessionDescription,
    ) -> Result<(), PeerConnectionError>;
    async fn set_remote_description(
        &self,
        description: SessionDescription,
    ) -> Result<(), PeerConnectionError>;
    /// Adds a candidate of the other side's, after its description.
    async fn add_ice_candidate(&self, candidate: IceCandidate) -> Result<(), PeerConnectionError>;
    /// Opens an ordered, reliable data channel for the other side to receive, read by `observer`
    /// from the start. Negotiated by the next offer: a transfer's sender opens its channel before
    /// offering.
    async fn create_data_channel(
        &self,
        label: String,
        observer: Arc<dyn DataChannelObserver>,
    ) -> Result<Arc<dyn DataChannel>, PeerConnectionError>;
    /// The description last set locally, which has the mids `set_local_description` assigned.
    fn local_description(&self) -> Option<SessionDescription>;
    fn signaling_state(&self) -> SignalingState;
    /// Adds a transceiver of `kind`, sending `track` when one is given, in the stream
    /// `stream_id` names, with the encodings asked of its sender.
    async fn add_transceiver(
        &self,
        kind: MediaKind,
        track: Option<TrackId>,
        direction: Direction,
        stream_id: Option<String>,
        send_encodings: Vec<SendEncoding>,
    ) -> Result<TransceiverId, PeerConnectionError>;
    /// The transceiver's mid, `None` until a description gave it one.
    fn transceiver_mid(&self, transceiver: TransceiverId) -> Option<String>;
    /// The transceiver with this mid. A remote offer's new sections make their transceivers as
    /// `set_remote_description` applies it, with the offer's mids, so the engine finds each
    /// between applying the offer and making the answer: a track and direction set then shape
    /// the answer, where ones set after it would need another negotiation.
    fn transceiver_by_mid(&self, mid: &str) -> Option<TransceiverId>;
    async fn set_direction(
        &self,
        transceiver: TransceiverId,
        direction: Direction,
    ) -> Result<(), PeerConnectionError>;
    async fn stop_transceiver(&self, transceiver: TransceiverId)
    -> Result<(), PeerConnectionError>;
    /// `removeTrack` of the transceiver's sender.
    async fn remove_track(&self, transceiver: TransceiverId) -> Result<(), PeerConnectionError>;
    /// `replaceTrack` on the transceiver's sender; `None` sends nothing.
    async fn replace_track(
        &self,
        transceiver: TransceiverId,
        track: Option<TrackId>,
    ) -> Result<(), PeerConnectionError>;
    async fn sender_parameters(
        &self,
        transceiver: TransceiverId,
    ) -> Result<SenderParameters, PeerConnectionError>;
    async fn set_sender_parameters(
        &self,
        transceiver: TransceiverId,
        parameters: SenderParameters,
    ) -> Result<(), PeerConnectionError>;
    /// The track the transceiver's receiver plays or draws, which the platform owns.
    fn receiver_track(&self, transceiver: TransceiverId) -> Option<TrackId>;
    /// How loud the transceiver's received audio plays, 0 to 10 as WebRTC's audio tracks take
    /// it (`RTCAudioTrack.source.volume`, `AudioTrack.setVolume`), 1 as received. Per-person
    /// volume and mute-for-me are applied here, to the slot or share sound that carries them.
    async fn set_receiver_volume(
        &self,
        transceiver: TransceiverId,
        volume: f64,
    ) -> Result<(), PeerConnectionError>;
    async fn set_configuration(
        &self,
        configuration: Configuration,
    ) -> Result<(), PeerConnectionError>;
    /// The W3C statistics report as JSON: an array of its stats objects.
    async fn stats(&self, of: StatsOf) -> Result<String, PeerConnectionError>;
    fn connection_state(&self) -> ConnectionState;
    fn close(&self);
}
