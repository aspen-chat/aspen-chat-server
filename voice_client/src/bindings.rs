//! The engine as Swift and Kotlin reach it, through UniFFI. Most of what crosses is the engine's
//! own types, marked where they are defined (`state`, `peer_connection`, `transport`, `api`);
//! this module holds what UniFFI cannot carry as the engine writes it, each mirrored at the
//! boundary and converted here, so the engine keeps its shape:
//!
//! - `Uuid` crosses as its text, `TrackId` as its string, and `TransceiverId` as its number
//!   (custom types), and the `voice_protocol` types the snapshot carries are declared remote.
//! - `CallState` crosses as it is, and beside it as JSON (`CallObserver::state_changed`'s `json`,
//!   `Engine::state_json`), which the apps hand their web views untouched: its shape is
//!   `call_bridge_schema.json`, which `state` describes. `HttpRequest` and `HttpResponse` carry
//!   their headers as `Header` records rather than pairs; `SenderParameters` carries `rest` as
//!   JSON text rather than a value.
//! - `TransportError` and `PeerConnectionError` are error enums of one variant, `Failed`, with
//!   the platform's words as `detail` (not `message`, which Kotlin's exceptions already have),
//!   since UniFFI's errors are enums and a flat one cannot be raised by foreign code; both take
//!   an exception the platform did not declare as that detail too.
//! - `Options` has no `random`: the engine draws from `call::Options::system_random`.
//! - `SocketFactory::connect` and `PeerConnectionFactory::create` answer a record holding what
//!   they made (`ConnectedSocket`, `CreatedPeerConnection`) rather than the trait object.
//! - `Socket::close` and `PeerConnection::close` are `close_socket` and `close_peer_connection`,
//!   since the Kotlin classes UniFFI generates are `AutoCloseable`.
//! - The traits the platforms implement (`Http`, `Socket`, `SocketFactory`, `PeerConnection`,
//!   `PeerConnectionFactory`, `CallObserver`) are declared again here over the mirrored types,
//!   and each foreign implementation is wrapped to implement the engine's trait.
//!
//! Every trait method is called on the engine's thread. A foreign implementation of a method
//! that returns no `Result` must not throw: UniFFI has nowhere to put the exception, and the
//! engine's thread ends. An `Engine` must not be released from inside `CallObserver`, which runs
//! on the thread releasing it waits for.

use crate::api::Credentials;
use crate::call;
use crate::engine;
use crate::peer_connection::{
    self, Configuration, ConnectionState, Direction, MediaKind, PeerConnectionObserver,
    SendEncoding, SessionDescription, SignalingState, StatsOf, TrackId, TransceiverId,
};
use crate::sounds::SoundCue;
use crate::state::{self, CallState, SessionEndReason};
use crate::transport::{self, Received};
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;
use voice_protocol::signal::Grants;
use voice_protocol::signal::{MediaSource, SourceInfo, WatchedVideo};

uniffi::custom_type!(Uuid, String, {
    remote,
    lower: |id| id.to_string(),
    try_lift: |text| Ok(Uuid::parse_str(&text)?),
});

uniffi::custom_type!(TrackId, String, {
    lower: |track| track.0,
    try_lift: |text| Ok(TrackId(text)),
});

uniffi::custom_type!(TransceiverId, u32, {
    lower: |transceiver| transceiver.0,
    try_lift: |number| Ok(TransceiverId(number)),
});

/// What a participant sends.
#[uniffi::remote(Enum)]
pub enum MediaSource {
    Microphone,
    Screen,
    ScreenAudio,
    Camera,
}

/// One source a participant sends, as told to the others.
#[uniffi::remote(Record)]
pub struct SourceInfo {
    pub source: MediaSource,
    /// Paused at the server: a muted microphone, or anything of a participant on hold.
    pub paused: bool,
}

/// A video a client shows, and wants a section for.
#[uniffi::remote(Record)]
pub struct WatchedVideo {
    pub user: Uuid,
    pub source: MediaSource,
}

/// What the channel lets the user do.
#[uniffi::remote(Record)]
pub struct Grants {
    pub speak: bool,
    pub share_screen: bool,
    pub camera: bool,
    pub transfer_files: bool,
}

/// Reads each snapshot as the call changes, as the record and as the same snapshot's JSON, for
/// the app's web view, and plays each of the call's sounds as a change cues it
/// (`sounds::SoundCue`), after the snapshot it follows from, through the call's own audio path.
/// Called on the engine's thread; keep it short.
#[uniffi::export(with_foreign)]
pub trait CallObserver: Send + Sync {
    fn state_changed(&self, state: CallState, json: String);
    fn play_sound(&self, cue: SoundCue);
}

struct ForeignObserver(Arc<dyn CallObserver>);

impl state::CallObserver for ForeignObserver {
    fn state_changed(&self, state: &state::CallState) {
        self.0.state_changed(state.clone(), state_json(state));
    }

    fn play_sound(&self, cue: SoundCue) {
        self.0.play_sound(cue);
    }
}

/// `state` as the web view reads it (`call_bridge_schema.json`). The snapshot is plain data,
/// with no map keyed by anything but a string, so writing it cannot fail.
fn state_json(state: &state::CallState) -> String {
    serde_json::to_string(state).expect("the call's state is plain data")
}

/// What the platform's network stack could not do, in its own words (`transport::TransportError`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum TransportError {
    #[error("{detail}")]
    Failed { detail: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for TransportError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            detail: error.reason,
        }
    }
}

impl From<TransportError> for transport::TransportError {
    fn from(error: TransportError) -> Self {
        let TransportError::Failed { detail } = error;
        Self(detail)
    }
}

/// What the platform's WebRTC refused or could not do (`peer_connection::PeerConnectionError`).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum PeerConnectionError {
    #[error("{detail}")]
    Failed { detail: String },
}

impl From<uniffi::UnexpectedUniFFICallbackError> for PeerConnectionError {
    fn from(error: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed {
            detail: error.reason,
        }
    }
}

impl From<PeerConnectionError> for peer_connection::PeerConnectionError {
    fn from(error: PeerConnectionError) -> Self {
        let PeerConnectionError::Failed { detail } = error;
        Self(detail)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct Header {
    pub name: String,
    pub value: String,
}

/// `transport::HttpRequest`, with its headers as records.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HttpRequest {
    /// `GET` or `POST`.
    pub method: String,
    pub url: String,
    pub headers: Vec<Header>,
    /// Sent as the request body, UTF-8; `None` sends none.
    pub body: Option<String>,
}

impl From<transport::HttpRequest> for HttpRequest {
    fn from(request: transport::HttpRequest) -> Self {
        Self {
            method: request.method,
            url: request.url,
            headers: request
                .headers
                .into_iter()
                .map(|(name, value)| Header { name, value })
                .collect(),
            body: request.body,
        }
    }
}

/// `transport::HttpResponse`, with its headers as records.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<Header>,
    /// The body as text; every answer the engine reads is JSON.
    pub body: String,
}

impl From<HttpResponse> for transport::HttpResponse {
    fn from(response: HttpResponse) -> Self {
        Self {
            status: response.status,
            headers: response
                .headers
                .into_iter()
                .map(|header| (header.name, header.value))
                .collect(),
            body: response.body,
        }
    }
}

/// Makes HTTP requests (`transport::Http`): the platform follows no redirects for the engine and
/// adds no credentials of its own.
#[uniffi::export(with_foreign)]
#[async_trait]
pub trait Http: Send + Sync {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, TransportError>;
}

struct ForeignHttp(Arc<dyn Http>);

#[async_trait]
impl transport::Http for ForeignHttp {
    async fn request(
        &self,
        request: transport::HttpRequest,
    ) -> Result<transport::HttpResponse, transport::TransportError> {
        Ok(self.0.request(request.into()).await?.into())
    }
}

/// One WebSocket (`transport::Socket`). The platform answers the server's pings itself.
#[uniffi::export(with_foreign)]
#[async_trait]
pub trait Socket: Send + Sync {
    /// Sends one text frame; an error means the socket is closed or closing.
    async fn send_text(&self, text: String) -> Result<(), TransportError>;
    /// The next text frame, or `Closed` once there will be no more. Binary frames are skipped.
    /// Called by one reader at a time.
    async fn receive(&self) -> Received;
    /// Closes the socket; `receive` then answers `Closed`. Closing a closed socket does nothing.
    /// (Not `close`: the Kotlin class UniFFI generates for a foreign trait is `AutoCloseable`,
    /// whose `close` releases the handle.)
    fn close_socket(&self);
}

struct ForeignSocket(Arc<dyn Socket>);

#[async_trait]
impl transport::Socket for ForeignSocket {
    async fn send_text(&self, text: String) -> Result<(), transport::TransportError> {
        Ok(self.0.send_text(text).await?)
    }

    async fn receive(&self) -> Received {
        self.0.receive().await
    }

    fn close(&self) {
        self.0.close_socket();
    }
}

/// A socket a `SocketFactory` opened. A record rather than the socket itself, since an async
/// foreign method cannot return a trait object: the generated code infers a lifetime for it
/// that nothing implements.
#[derive(uniffi::Record)]
pub struct ConnectedSocket {
    pub socket: Arc<dyn Socket>,
}

/// Opens WebSockets (`transport::SocketFactory`).
#[uniffi::export(with_foreign)]
#[async_trait]
pub trait SocketFactory: Send + Sync {
    /// Connects to `url` (`ws:` or `wss:`), sending `Authorization: Bearer <bearer>` with the
    /// upgrade when one is given, and returns once the socket is open.
    async fn connect(
        &self,
        url: String,
        bearer: Option<String>,
    ) -> Result<ConnectedSocket, TransportError>;
}

struct ForeignSockets(Arc<dyn SocketFactory>);

#[async_trait]
impl transport::SocketFactory for ForeignSockets {
    async fn connect(
        &self,
        url: String,
        bearer: Option<String>,
    ) -> Result<Arc<dyn transport::Socket>, transport::TransportError> {
        let connected = self.0.connect(url, bearer).await?;
        Ok(Arc::new(ForeignSocket(connected.socket)))
    }
}

/// `peer_connection::SenderParameters`, with everything but the encodings as JSON text.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SenderParameters {
    pub encodings: Vec<SendEncoding>,
    /// Everything else `getParameters` returned, as JSON, handed back unchanged by
    /// `set_sender_parameters`; empty when the platform keeps the rest itself.
    pub rest: String,
}

impl From<peer_connection::SenderParameters> for SenderParameters {
    fn from(parameters: peer_connection::SenderParameters) -> Self {
        Self {
            encodings: parameters.encodings,
            rest: if parameters.rest.is_null() {
                String::new()
            } else {
                parameters.rest.to_string()
            },
        }
    }
}

impl TryFrom<SenderParameters> for peer_connection::SenderParameters {
    type Error = peer_connection::PeerConnectionError;

    fn try_from(parameters: SenderParameters) -> Result<Self, Self::Error> {
        let rest = if parameters.rest.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_str(&parameters.rest).map_err(|error| {
                peer_connection::PeerConnectionError(format!(
                    "the sender parameters' rest is not JSON: {error}"
                ))
            })?
        };
        Ok(Self {
            encodings: parameters.encodings,
            rest,
        })
    }
}

/// A peer connection a `PeerConnectionFactory` made; a record for the reason `ConnectedSocket`
/// is one.
#[derive(uniffi::Record)]
pub struct CreatedPeerConnection {
    pub connection: Arc<dyn PeerConnection>,
}

/// Makes peer connections (`peer_connection::PeerConnectionFactory`). One per platform process.
#[uniffi::export(with_foreign)]
#[async_trait]
pub trait PeerConnectionFactory: Send + Sync {
    async fn create(
        &self,
        configuration: Configuration,
        observer: Arc<dyn PeerConnectionObserver>,
    ) -> Result<CreatedPeerConnection, PeerConnectionError>;
}

struct ForeignPeerConnections(Arc<dyn PeerConnectionFactory>);

#[async_trait]
impl peer_connection::PeerConnectionFactory for ForeignPeerConnections {
    async fn create(
        &self,
        configuration: Configuration,
        observer: Arc<dyn PeerConnectionObserver>,
    ) -> Result<Arc<dyn peer_connection::PeerConnection>, peer_connection::PeerConnectionError>
    {
        let created = self.0.create(configuration, observer).await?;
        Ok(Arc::new(ForeignPeerConnection(created.connection)))
    }
}

/// One peer connection (`peer_connection::PeerConnection`): each method is one W3C operation,
/// named as it is there; see that trait for what each must do.
#[uniffi::export(with_foreign)]
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
    fn local_description(&self) -> Option<SessionDescription>;
    fn signaling_state(&self) -> SignalingState;
    async fn add_transceiver(
        &self,
        kind: MediaKind,
        track: Option<TrackId>,
        direction: Direction,
        stream_id: Option<String>,
        send_encodings: Vec<SendEncoding>,
    ) -> Result<TransceiverId, PeerConnectionError>;
    fn transceiver_mid(&self, transceiver: TransceiverId) -> Option<String>;
    fn transceiver_by_mid(&self, mid: String) -> Option<TransceiverId>;
    async fn set_direction(
        &self,
        transceiver: TransceiverId,
        direction: Direction,
    ) -> Result<(), PeerConnectionError>;
    async fn stop_transceiver(&self, transceiver: TransceiverId)
    -> Result<(), PeerConnectionError>;
    async fn remove_track(&self, transceiver: TransceiverId) -> Result<(), PeerConnectionError>;
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
    fn receiver_track(&self, transceiver: TransceiverId) -> Option<TrackId>;
    async fn set_receiver_volume(
        &self,
        transceiver: TransceiverId,
        volume: f64,
    ) -> Result<(), PeerConnectionError>;
    async fn set_configuration(
        &self,
        configuration: Configuration,
    ) -> Result<(), PeerConnectionError>;
    async fn stats(&self, of: StatsOf) -> Result<String, PeerConnectionError>;
    fn connection_state(&self) -> ConnectionState;
    /// (Not `close`, for the reason `Socket::close_socket` is not.)
    fn close_peer_connection(&self);
}

struct ForeignPeerConnection(Arc<dyn PeerConnection>);

type PeerConnectionResult<T> = Result<T, peer_connection::PeerConnectionError>;

#[async_trait]
impl peer_connection::PeerConnection for ForeignPeerConnection {
    async fn create_offer(&self, ice_restart: bool) -> PeerConnectionResult<String> {
        Ok(self.0.create_offer(ice_restart).await?)
    }

    async fn create_answer(&self) -> PeerConnectionResult<String> {
        Ok(self.0.create_answer().await?)
    }

    async fn set_local_description(
        &self,
        description: SessionDescription,
    ) -> PeerConnectionResult<()> {
        Ok(self.0.set_local_description(description).await?)
    }

    async fn set_remote_description(
        &self,
        description: SessionDescription,
    ) -> PeerConnectionResult<()> {
        Ok(self.0.set_remote_description(description).await?)
    }

    fn local_description(&self) -> Option<SessionDescription> {
        self.0.local_description()
    }

    fn signaling_state(&self) -> SignalingState {
        self.0.signaling_state()
    }

    async fn add_transceiver(
        &self,
        kind: MediaKind,
        track: Option<TrackId>,
        direction: Direction,
        stream_id: Option<String>,
        send_encodings: Vec<SendEncoding>,
    ) -> PeerConnectionResult<TransceiverId> {
        Ok(self
            .0
            .add_transceiver(kind, track, direction, stream_id, send_encodings)
            .await?)
    }

    fn transceiver_mid(&self, transceiver: TransceiverId) -> Option<String> {
        self.0.transceiver_mid(transceiver)
    }

    fn transceiver_by_mid(&self, mid: &str) -> Option<TransceiverId> {
        self.0.transceiver_by_mid(mid.to_string())
    }

    async fn set_direction(
        &self,
        transceiver: TransceiverId,
        direction: Direction,
    ) -> PeerConnectionResult<()> {
        Ok(self.0.set_direction(transceiver, direction).await?)
    }

    async fn stop_transceiver(&self, transceiver: TransceiverId) -> PeerConnectionResult<()> {
        Ok(self.0.stop_transceiver(transceiver).await?)
    }

    async fn remove_track(&self, transceiver: TransceiverId) -> PeerConnectionResult<()> {
        Ok(self.0.remove_track(transceiver).await?)
    }

    async fn replace_track(
        &self,
        transceiver: TransceiverId,
        track: Option<TrackId>,
    ) -> PeerConnectionResult<()> {
        Ok(self.0.replace_track(transceiver, track).await?)
    }

    async fn sender_parameters(
        &self,
        transceiver: TransceiverId,
    ) -> PeerConnectionResult<peer_connection::SenderParameters> {
        self.0.sender_parameters(transceiver).await?.try_into()
    }

    async fn set_sender_parameters(
        &self,
        transceiver: TransceiverId,
        parameters: peer_connection::SenderParameters,
    ) -> PeerConnectionResult<()> {
        Ok(self
            .0
            .set_sender_parameters(transceiver, parameters.into())
            .await?)
    }

    fn receiver_track(&self, transceiver: TransceiverId) -> Option<TrackId> {
        self.0.receiver_track(transceiver)
    }

    async fn set_receiver_volume(
        &self,
        transceiver: TransceiverId,
        volume: f64,
    ) -> PeerConnectionResult<()> {
        Ok(self.0.set_receiver_volume(transceiver, volume).await?)
    }

    async fn set_configuration(&self, configuration: Configuration) -> PeerConnectionResult<()> {
        Ok(self.0.set_configuration(configuration).await?)
    }

    async fn stats(&self, of: StatsOf) -> PeerConnectionResult<String> {
        Ok(self.0.stats(of).await?)
    }

    fn connection_state(&self) -> ConnectionState {
        self.0.connection_state()
    }

    fn close(&self) {
        self.0.close_peer_connection();
    }
}

/// The platform's implementations the engine runs on (`call::Platform`).
#[derive(uniffi::Record)]
pub struct Platform {
    pub http: Arc<dyn Http>,
    pub sockets: Arc<dyn SocketFactory>,
    pub peer_connections: Arc<dyn PeerConnectionFactory>,
}

/// `call::Options`; the engine draws its random numbers itself.
#[derive(uniffi::Record)]
pub struct Options {
    /// The deployment's `public_url`.
    pub api_url: String,
    pub credentials: Arc<dyn Credentials>,
    /// Whether plain `http:` voice servers at this machine's own addresses may be reached, as a
    /// development build allows; every other build reaches voice servers over HTTPS alone.
    pub loopback_plain_text: bool,
}

/// `engine::Engine` as the apps hold it: its methods queue the call's commands from any thread
/// and never block, and `state` is the latest snapshot the observer was given. Releasing the
/// last reference leaves any call and stops the engine's thread, waiting for it.
#[derive(uniffi::Object)]
pub struct Engine(engine::Engine);

#[uniffi::export]
impl Engine {
    /// Starts the engine's thread. `observer` is called on that thread with every change.
    #[uniffi::constructor]
    pub fn start(
        platform: Platform,
        options: Options,
        observer: Arc<dyn CallObserver>,
    ) -> Arc<Self> {
        let platform = call::Platform {
            http: Arc::new(ForeignHttp(platform.http)),
            sockets: Arc::new(ForeignSockets(platform.sockets)),
            peer_connections: Arc::new(ForeignPeerConnections(platform.peer_connections)),
        };
        let options = call::Options {
            api_url: options.api_url,
            credentials: options.credentials,
            loopback_plain_text: options.loopback_plain_text,
            random: Box::new(call::Options::system_random),
        };
        Arc::new(Self(engine::Engine::start(
            platform,
            options,
            Arc::new(ForeignObserver(observer)),
        )))
    }

    pub fn state(&self) -> CallState {
        self.0.state()
    }

    /// The latest snapshot as the web view reads it, the JSON the observer is given beside each.
    pub fn state_json(&self) -> String {
        state_json(&self.0.state())
    }

    pub fn join(&self, channel: Uuid) {
        self.0.join(channel);
    }

    pub fn leave(&self) {
        self.0.leave();
    }

    pub fn set_muted(&self, muted: bool) {
        self.0.set_muted(muted);
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.0.set_deafened(deafened);
    }

    /// Puts the call on hold or takes it off, as the system does while a phone call has the
    /// audio (CallKit's `CXSetHeldCallAction`, Telecom's `onSetInactive`).
    pub fn set_held(&self, held: bool) {
        self.0.set_held(held);
    }

    pub fn set_server_muted(&self, server_muted: bool) {
        self.0.set_server_muted(server_muted);
    }

    pub fn set_microphone(&self, track: Option<TrackId>) {
        self.0.set_microphone(track);
    }

    pub fn start_source(&self, source: MediaSource, track: TrackId) {
        self.0.start_source(source, track);
    }

    pub fn stop_source(&self, source: MediaSource) {
        self.0.stop_source(source);
    }

    pub fn set_watched(&self, video: Vec<WatchedVideo>) {
        self.0.set_watched(video);
    }

    pub fn set_user_volume(&self, user: Uuid, source: MediaSource, gain: f64) {
        self.0.set_user_volume(user, source, gain);
    }

    pub fn set_user_muted(&self, user: Uuid, source: MediaSource, muted: bool) {
        self.0.set_user_muted(user, source, muted);
    }

    pub fn session_ended(&self, session: Uuid, channel: Uuid, reason: SessionEndReason) {
        self.0.session_ended(session, channel, reason);
    }

    pub fn acknowledge_end(&self) {
        self.0.acknowledge_end();
    }

    pub fn clear_state_refusal(&self) {
        self.0.clear_state_refusal();
    }

    pub fn clear_source_refusal(&self) {
        self.0.clear_source_refusal();
    }
}
