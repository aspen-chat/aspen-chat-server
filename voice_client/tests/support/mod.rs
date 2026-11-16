//! Stand-ins for the platform: an API server answering the join and failure routes and each
//! candidate's health check, voice servers scripted frame by frame in `voice_protocol::signal`,
//! a peer connection that records what it is asked and reports the states a test gives it, with
//! data channels whose other end the test plays, and files that record what is read and written.
//! `World` wires them to a `Call` running on the test's runtime.

#![allow(dead_code)]

use async_trait::async_trait;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;
use voice_client::api::Credentials;
use voice_client::call::{Call, Command, Options, Platform};
use voice_client::files::{FileError, FileSink, FileSource, Files};
use voice_client::peer_connection::{
    Configuration, ConnectionState, DataChannel, DataChannelObserver, DataChannelState,
    DescriptionKind, Direction, IceCandidate, MediaKind, PeerConnection, PeerConnectionError,
    PeerConnectionFactory, PeerConnectionKind, PeerConnectionObserver, SendEncoding,
    SenderParameters, SessionDescription, SignalingState, StatsOf, TrackId, TransceiverId,
};
use voice_client::sounds::SoundCue;
use voice_client::state::{CallObserver, CallState, CallStatus};
use voice_client::transfers::CHUNK_BYTES;
use voice_client::transport::{
    Http, HttpRequest, HttpResponse, Received, Socket, SocketFactory, TransportError,
};
use voice_protocol::signal::{
    ClientMessage, ConnectionKind, FileOffer, IceServer, MediaSource, ParticipantInfo, Section,
    SectionContent, ServerMessage, TransferMode, TransferRole,
};

pub const API_URL: &str = "https://chat.example.org";

/// Lets every task the call spawned run to its next wait.
pub async fn settle() {
    for _ in 0..64 {
        tokio::task::yield_now().await;
    }
}

/// Moves the paused clock on and lets what fired run.
pub async fn advance(by: Duration) {
    tokio::time::advance(by).await;
    settle().await;
}

// --- The API server -----------------------------------------------------------------------------

/// How a candidate answers `GET /health`.
#[derive(Clone, Copy, Debug)]
pub enum Health {
    /// `200` after this long.
    After(Duration),
    /// Never answers.
    Silent,
    /// `503` at once.
    Down,
}

#[derive(Default)]
pub struct FakeHttp {
    /// Answers to the join route, in order; the last is repeated.
    offers: Mutex<Vec<HttpResponse>>,
    offers_served: Mutex<usize>,
    pub health: Mutex<HashMap<String, Health>>,
    pub requests: Mutex<Vec<HttpRequest>>,
}

impl FakeHttp {
    pub fn answer_join(&self, response: HttpResponse) {
        self.offers.lock().unwrap().push(response);
    }

    /// Servers reported as failing, in order.
    pub fn failures_reported(&self) -> Vec<Uuid> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter_map(|request| {
                let rest = request
                    .url
                    .strip_prefix(&format!("{API_URL}/api/v1/voice-servers/"))?;
                let id = rest.strip_suffix("/failures")?;
                Uuid::parse_str(id).ok()
            })
            .collect()
    }

    pub fn offers_asked(&self) -> usize {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.url.ends_with("/voice/join"))
            .count()
    }

    pub fn health_asked(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter_map(|request| request.url.strip_suffix("/health").map(str::to_string))
            .collect()
    }
}

#[async_trait]
impl Http for FakeHttp {
    async fn request(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
        self.requests.lock().unwrap().push(request.clone());
        if let Some(server) = request.url.strip_suffix("/health") {
            let health = self
                .health
                .lock()
                .unwrap()
                .get(server)
                .copied()
                .unwrap_or(Health::After(Duration::ZERO));
            return match health {
                Health::After(wait) => {
                    tokio::time::sleep(wait).await;
                    Ok(plain(200, ""))
                }
                Health::Silent => std::future::pending().await,
                Health::Down => Ok(plain(503, "")),
            };
        }
        if request.url.ends_with("/voice/join") {
            let offers = self.offers.lock().unwrap();
            let mut served = self.offers_served.lock().unwrap();
            let answer = offers
                .get((*served).min(offers.len().saturating_sub(1)))
                .cloned();
            *served += 1;
            return Ok(answer.unwrap_or_else(|| plain(500, "no offer scripted")));
        }
        if request.url.ends_with("/failures") {
            return Ok(plain(
                200,
                r#"{"failures":1,"disabled":false,"counted":true}"#,
            ));
        }
        Ok(plain(404, ""))
    }
}

pub fn plain(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: Vec::new(),
        body: body.to_string(),
    }
}

/// A join offer naming `servers`, each a candidate with a fresh id at `url`.
pub struct Offer {
    pub channel: Uuid,
    pub candidates: Vec<(Uuid, String)>,
    pub token: String,
    pub speak: bool,
    pub share_screen: bool,
    pub camera: bool,
    pub transfer_files: bool,
    pub server_muted: bool,
}

impl Offer {
    pub fn to(channel: Uuid, urls: &[&str]) -> Self {
        Self {
            channel,
            candidates: urls
                .iter()
                .map(|url| (Uuid::now_v7(), url.to_string()))
                .collect(),
            token: "join-token".to_string(),
            speak: true,
            share_screen: true,
            camera: true,
            transfer_files: true,
            server_muted: false,
        }
    }

    pub fn response(&self) -> HttpResponse {
        let candidates: Vec<String> = self
            .candidates
            .iter()
            .map(|(id, url)| format!(r#"{{"id":"{id}","name":"{id}","url":"{url}"}}"#))
            .collect();
        plain(
            200,
            &format!(
                r#"{{"channelId":"{}","candidates":[{}],"token":"{}","expiresAt":"2026-01-01T00:00:00Z","speak":{},"shareScreen":{},"transferFiles":{},"useCamera":{},"serverMuted":{}}}"#,
                self.channel,
                candidates.join(","),
                self.token,
                self.speak,
                self.share_screen,
                self.transfer_files,
                self.camera,
                self.server_muted
            ),
        )
    }
}

pub fn problem(status: u16, code: &str, detail: &str) -> HttpResponse {
    plain(
        status,
        &format!(r#"{{"code":"{code}","title":"Refused","detail":"{detail}","status":{status}}}"#),
    )
}

pub struct Token;

#[async_trait]
impl Credentials for Token {
    async fn session_token(&self) -> Option<String> {
        Some("session-token".to_string())
    }

    async fn renewed_session_token(&self, _rejected: String) -> Option<String> {
        None
    }
}

// --- Voice servers --------------------------------------------------------------------------------

/// How a voice server takes a socket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behaviour {
    /// Accepts it; the test scripts every frame.
    Scripted,
    /// Refuses the connection.
    Refuse,
    /// Never answers the connection.
    Hang,
    /// Accepts it after this long.
    After(Duration),
}

struct Connection {
    to_client: mpsc::UnboundedSender<Received>,
    closed: AtomicBool,
}

/// One voice server, by its signalling URL.
pub struct FakeServer {
    pub behaviour: Mutex<Behaviour>,
    connections: Mutex<Vec<Arc<Connection>>>,
    received: Mutex<Vec<ClientMessage>>,
}

impl FakeServer {
    pub fn set(&self, behaviour: Behaviour) {
        *self.behaviour.lock().unwrap() = behaviour;
    }

    /// How many sockets have been opened to it.
    pub fn connections(&self) -> usize {
        self.connections.lock().unwrap().len()
    }

    fn latest(&self) -> Option<Arc<Connection>> {
        self.connections.lock().unwrap().last().cloned()
    }

    /// Sends a frame on the latest socket.
    pub fn send(&self, frame: &ServerMessage) {
        self.send_text(serde_json::to_string(frame).unwrap());
    }

    pub fn send_text(&self, text: String) {
        let connection = self.latest().expect("a socket to send on");
        assert!(
            !connection.closed.load(Ordering::SeqCst),
            "the latest socket is closed"
        );
        let _ = connection.to_client.send(Received::Text(text));
    }

    /// Closes the latest socket from the server's side.
    pub fn close(&self) {
        let connection = self.latest().expect("a socket to close");
        if !connection.closed.swap(true, Ordering::SeqCst) {
            let _ = connection.to_client.send(Received::Closed);
        }
    }

    pub fn is_closed(&self) -> bool {
        self.latest()
            .is_none_or(|connection| connection.closed.load(Ordering::SeqCst))
    }

    /// Every frame the client sent since the last call.
    pub fn received(&self) -> Vec<ClientMessage> {
        std::mem::take(&mut *self.received.lock().unwrap())
    }

    pub fn ready(
        &self,
        session: Uuid,
        user: Uuid,
        participants: Vec<ParticipantInfo>,
        secret: &str,
    ) {
        self.send(&ServerMessage::Ready {
            session,
            user,
            participants,
            audio_slots: 16,
            max_watched_video: 25,
            resume: secret.to_string(),
            offers: Vec::new(),
            links: Vec::new(),
            transfers: Default::default(),
        });
    }

    /// Offers `sections` on the call connection as revision `revision`; the SDP names the mids,
    /// which the fake peer connection reads.
    pub fn offer(&self, revision: u64, sections: Vec<Section>) {
        self.send(&ServerMessage::Offer {
            connection: ConnectionKind::Call,
            revision,
            sdp: sdp_of(&sections),
            sections,
        });
    }

    pub fn sections(&self, sections: Vec<Section>) {
        self.send(&ServerMessage::Sections {
            connection: ConnectionKind::Call,
            sections,
        });
    }

    /// Offers `sections` on the capture connection as revision `revision`.
    pub fn capture_offer(&self, revision: u64, sections: Vec<Section>) {
        self.send(&ServerMessage::Offer {
            connection: ConnectionKind::Capture,
            revision,
            sdp: sdp_of(&sections),
            sections,
        });
    }

    pub fn capture_sections(&self, sections: Vec<Section>) {
        self.send(&ServerMessage::Sections {
            connection: ConnectionKind::Capture,
            sections,
        });
    }
}

pub fn sdp_of(sections: &[Section]) -> String {
    let mids: Vec<&str> = sections.iter().map(|s| s.mid.as_str()).collect();
    format!("v=0 mids={}", mids.join(","))
}

pub fn slot(mid: &str, slot: u8) -> Section {
    Section {
        mid: mid.to_string(),
        kind: voice_protocol::signal::MediaKind::Audio,
        content: SectionContent::AudioSlot { slot },
    }
}

pub fn sending(mid: &str, source: MediaSource) -> Section {
    Section {
        mid: mid.to_string(),
        kind: source.kind(),
        content: SectionContent::Sending { source },
    }
}

pub fn video(mid: &str, user: Uuid, source: MediaSource) -> Section {
    Section {
        mid: mid.to_string(),
        kind: voice_protocol::signal::MediaKind::Video,
        content: SectionContent::Video { user, source },
    }
}

pub fn share_audio(mid: &str, user: Uuid) -> Section {
    Section {
        mid: mid.to_string(),
        kind: voice_protocol::signal::MediaKind::Audio,
        content: SectionContent::ShareAudio { user },
    }
}

#[derive(Default)]
pub struct FakeSockets {
    servers: Mutex<HashMap<String, Arc<FakeServer>>>,
    /// Every URL connected to, in order.
    pub connected: Mutex<Vec<String>>,
}

impl FakeSockets {
    /// The server at `url` (an HTTP address, as a candidate names it), made if there is none.
    pub fn server(&self, url: &str) -> Arc<FakeServer> {
        let ws = voice_client::signalling::signalling_url(url, true).expect("a reachable url");
        self.servers
            .lock()
            .unwrap()
            .entry(ws)
            .or_insert_with(|| {
                Arc::new(FakeServer {
                    behaviour: Mutex::new(Behaviour::Scripted),
                    connections: Mutex::new(Vec::new()),
                    received: Mutex::new(Vec::new()),
                })
            })
            .clone()
    }
}

struct FakeSocket {
    server: Arc<FakeServer>,
    connection: Arc<Connection>,
    from_server: tokio::sync::Mutex<mpsc::UnboundedReceiver<Received>>,
}

#[async_trait]
impl Socket for FakeSocket {
    async fn send_text(&self, text: String) -> Result<(), TransportError> {
        if self.connection.closed.load(Ordering::SeqCst) {
            return Err(TransportError("closed".to_string()));
        }
        let frame: ClientMessage = serde_json::from_str(&text).expect("a client frame");
        self.server.received.lock().unwrap().push(frame);
        Ok(())
    }

    async fn receive(&self) -> Received {
        self.from_server
            .lock()
            .await
            .recv()
            .await
            .unwrap_or(Received::Closed)
    }

    fn close(&self) {
        if !self.connection.closed.swap(true, Ordering::SeqCst) {
            let _ = self.connection.to_client.send(Received::Closed);
        }
    }
}

#[async_trait]
impl SocketFactory for FakeSockets {
    async fn connect(
        &self,
        url: String,
        bearer: Option<String>,
    ) -> Result<Arc<dyn Socket>, TransportError> {
        assert_eq!(bearer, None, "a voice server takes its token in a frame");
        self.connected.lock().unwrap().push(url.clone());
        let server = self.servers.lock().unwrap().get(&url).cloned();
        let Some(server) = server else {
            return Err(TransportError(format!("no voice server at {url}")));
        };
        let behaviour = *server.behaviour.lock().unwrap();
        match behaviour {
            Behaviour::Scripted => {}
            Behaviour::Refuse => return Err(TransportError("connection refused".to_string())),
            Behaviour::Hang => std::future::pending().await,
            Behaviour::After(wait) => tokio::time::sleep(wait).await,
        }
        let (to_client, from_server) = mpsc::unbounded_channel();
        let connection = Arc::new(Connection {
            to_client,
            closed: AtomicBool::new(false),
        });
        server.connections.lock().unwrap().push(connection.clone());
        Ok(Arc::new(FakeSocket {
            server,
            connection,
            from_server: tokio::sync::Mutex::new(from_server),
        }))
    }
}

// --- Peer connections -----------------------------------------------------------------------------

/// What a peer connection was asked, by mid where the operation names a transceiver.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    SetRemote(String),
    ReplaceTrack(String, Option<TrackId>),
    SetDirection(String, Direction),
    CreateAnswer,
    SetLocal(String),
    SetVolume(String, f64),
    SetEncoding(String, Option<u32>, Option<f64>),
    Close,
    /// A transfer's own offer, which the client makes; `CreateOffer` and `SetLocal` record it.
    CreateOffer,
    AddCandidate(IceCandidate),
    CreateDataChannel(String),
}

struct Inner {
    /// Transceivers in the order their mids first appeared, the index being the id.
    mids: Vec<String>,
    remote: Option<SessionDescription>,
    local: Option<SessionDescription>,
    state: ConnectionState,
    closed: bool,
    /// Fails the next `set_remote_description`.
    fail_remote: bool,
    /// Fails the next `create_data_channel`.
    fail_channel: bool,
    /// What `stats` answers: the W3C array as JSON.
    stats: String,
}

pub struct FakePeerConnection {
    pub id: usize,
    pub configuration: Configuration,
    /// What the engine made it for.
    pub kind: PeerConnectionKind,
    observer: Arc<dyn PeerConnectionObserver>,
    inner: Mutex<Inner>,
    parameters: Mutex<HashMap<TransceiverId, SenderParameters>>,
    pub ops: Mutex<Vec<Op>>,
    /// The data channel the engine opened on this connection, a transfer's sender's.
    pub channel: Mutex<Option<Arc<FakeDataChannel>>>,
}

impl FakePeerConnection {
    /// Reports a connection state, as the platform's observer would.
    pub fn report(&self, state: ConnectionState) {
        self.inner.lock().unwrap().state = state;
        self.observer.connection_state_changed(state);
    }

    /// Reports a candidate of this connection's own, as the platform's observer would.
    pub fn gathered(&self, candidate: &str) {
        self.observer.ice_candidate(IceCandidate {
            candidate: candidate.to_string(),
            sdp_mid: Some("0".to_string()),
            sdp_m_line_index: Some(0),
            username_fragment: None,
        });
    }

    /// The other side opened `channel` on this connection, as the platform's observer would
    /// tell; the observer the engine answers reads the channel from then on.
    pub fn channel_opened(&self, channel: Arc<FakeDataChannel>) -> Arc<dyn DataChannelObserver> {
        self.observer.data_channel_opened(channel)
    }

    pub fn fail_next_remote(&self) {
        self.inner.lock().unwrap().fail_remote = true;
    }

    pub fn fail_next_channel(&self) {
        self.inner.lock().unwrap().fail_channel = true;
    }

    /// What `stats` answers from now on.
    pub fn set_stats(&self, json: &str) {
        self.inner.lock().unwrap().stats = json.to_string();
    }

    pub fn remote_description(&self) -> Option<SessionDescription> {
        self.inner.lock().unwrap().remote.clone()
    }

    /// The data channel the engine opened, which a transfer's sender does.
    pub fn data_channel(&self) -> Arc<FakeDataChannel> {
        self.channel
            .lock()
            .unwrap()
            .clone()
            .expect("the engine opened a data channel")
    }

    pub fn ops(&self) -> Vec<Op> {
        std::mem::take(&mut *self.ops.lock().unwrap())
    }

    pub fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    fn mid(&self, transceiver: TransceiverId) -> String {
        self.inner.lock().unwrap().mids[transceiver.0 as usize].clone()
    }

    fn op(&self, op: Op) {
        self.ops.lock().unwrap().push(op);
    }

    /// The track the receiver of `mid` gives.
    pub fn received(&self, mid: &str) -> TrackId {
        TrackId(format!("recv:{}:{mid}", self.id))
    }
}

#[async_trait]
impl PeerConnection for FakePeerConnection {
    /// The call connection is offered by the server alone; a transfer's connection is offered by
    /// its sender, which this answers with an offer naming the connection.
    async fn create_offer(&self, _ice_restart: bool) -> Result<String, PeerConnectionError> {
        self.op(Op::CreateOffer);
        Ok(format!("offer from pc {}", self.id))
    }

    async fn create_answer(&self) -> Result<String, PeerConnectionError> {
        self.op(Op::CreateAnswer);
        let inner = self.inner.lock().unwrap();
        Ok(format!("answer mids={}", inner.mids.join(",")))
    }

    async fn set_local_description(
        &self,
        description: SessionDescription,
    ) -> Result<(), PeerConnectionError> {
        self.op(Op::SetLocal(description.sdp.clone()));
        self.inner.lock().unwrap().local = Some(description);
        Ok(())
    }

    async fn set_remote_description(
        &self,
        description: SessionDescription,
    ) -> Result<(), PeerConnectionError> {
        self.op(Op::SetRemote(description.sdp.clone()));
        let mut inner = self.inner.lock().unwrap();
        if std::mem::take(&mut inner.fail_remote) {
            return Err(PeerConnectionError("bad sdp".to_string()));
        }
        // The server's offers name their mids; a transfer's descriptions name none.
        if description.kind == DescriptionKind::Offer {
            assert!(
                description.sdp.contains("mids=") || inner.remote.is_none(),
                "a second offer on a transfer's connection"
            );
        }
        let mids = description
            .sdp
            .split_once("mids=")
            .map(|(_, mids)| mids)
            .unwrap_or("");
        for mid in mids.split(',').filter(|m| !m.is_empty()) {
            if !inner.mids.iter().any(|m| m == mid) {
                inner.mids.push(mid.to_string());
            }
        }
        inner.remote = Some(description);
        Ok(())
    }

    async fn add_ice_candidate(&self, candidate: IceCandidate) -> Result<(), PeerConnectionError> {
        self.op(Op::AddCandidate(candidate));
        Ok(())
    }

    async fn create_data_channel(
        &self,
        label: String,
        observer: Arc<dyn DataChannelObserver>,
    ) -> Result<Arc<dyn DataChannel>, PeerConnectionError> {
        self.op(Op::CreateDataChannel(label.clone()));
        if std::mem::take(&mut self.inner.lock().unwrap().fail_channel) {
            return Err(PeerConnectionError("no data channels".to_string()));
        }
        let channel = Arc::new(FakeDataChannel::new(label));
        *channel.observer.lock().unwrap() = Some(observer);
        *self.channel.lock().unwrap() = Some(channel.clone());
        Ok(channel)
    }

    fn local_description(&self) -> Option<SessionDescription> {
        self.inner.lock().unwrap().local.clone()
    }

    fn signaling_state(&self) -> SignalingState {
        SignalingState::Stable
    }

    async fn add_transceiver(
        &self,
        _kind: MediaKind,
        _track: Option<TrackId>,
        _direction: Direction,
        _stream_id: Option<String>,
        _send_encodings: Vec<SendEncoding>,
    ) -> Result<TransceiverId, PeerConnectionError> {
        Err(PeerConnectionError(
            "the client adds no transceivers of its own".to_string(),
        ))
    }

    fn transceiver_mid(&self, transceiver: TransceiverId) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .mids
            .get(transceiver.0 as usize)
            .cloned()
    }

    fn transceiver_by_mid(&self, mid: &str) -> Option<TransceiverId> {
        self.inner
            .lock()
            .unwrap()
            .mids
            .iter()
            .position(|m| m == mid)
            .map(|at| TransceiverId(at as u32))
    }

    async fn set_direction(
        &self,
        transceiver: TransceiverId,
        direction: Direction,
    ) -> Result<(), PeerConnectionError> {
        self.op(Op::SetDirection(self.mid(transceiver), direction));
        Ok(())
    }

    async fn stop_transceiver(
        &self,
        _transceiver: TransceiverId,
    ) -> Result<(), PeerConnectionError> {
        Ok(())
    }

    async fn remove_track(&self, _transceiver: TransceiverId) -> Result<(), PeerConnectionError> {
        Ok(())
    }

    async fn replace_track(
        &self,
        transceiver: TransceiverId,
        track: Option<TrackId>,
    ) -> Result<(), PeerConnectionError> {
        self.op(Op::ReplaceTrack(self.mid(transceiver), track));
        Ok(())
    }

    /// One encoding, as a negotiated sender has, with whatever was last set on it.
    async fn sender_parameters(
        &self,
        transceiver: TransceiverId,
    ) -> Result<SenderParameters, PeerConnectionError> {
        Ok(self
            .parameters
            .lock()
            .unwrap()
            .get(&transceiver)
            .cloned()
            .unwrap_or(SenderParameters {
                encodings: vec![SendEncoding::default()],
                rest: serde_json::Value::Null,
            }))
    }

    async fn set_sender_parameters(
        &self,
        transceiver: TransceiverId,
        parameters: SenderParameters,
    ) -> Result<(), PeerConnectionError> {
        let first = parameters.encodings.first();
        self.op(Op::SetEncoding(
            self.mid(transceiver),
            first.and_then(|e| e.max_bitrate),
            first.and_then(|e| e.max_framerate),
        ));
        self.parameters
            .lock()
            .unwrap()
            .insert(transceiver, parameters);
        Ok(())
    }

    fn receiver_track(&self, transceiver: TransceiverId) -> Option<TrackId> {
        Some(self.received(&self.mid(transceiver)))
    }

    async fn set_receiver_volume(
        &self,
        transceiver: TransceiverId,
        volume: f64,
    ) -> Result<(), PeerConnectionError> {
        self.op(Op::SetVolume(self.mid(transceiver), volume));
        Ok(())
    }

    async fn set_configuration(&self, _: Configuration) -> Result<(), PeerConnectionError> {
        Ok(())
    }

    async fn stats(&self, _: StatsOf) -> Result<String, PeerConnectionError> {
        let inner = self.inner.lock().unwrap();
        if inner.closed {
            return Err(PeerConnectionError("closed".to_string()));
        }
        Ok(inner.stats.clone())
    }

    fn connection_state(&self) -> ConnectionState {
        self.inner.lock().unwrap().state
    }

    fn close(&self) {
        self.op(Op::Close);
        self.inner.lock().unwrap().closed = true;
    }
}

#[derive(Default)]
pub struct FakePeerConnections {
    pub created: Mutex<Vec<Arc<FakePeerConnection>>>,
    pub fail_create: AtomicBool,
}

impl FakePeerConnections {
    /// The peer connection made last.
    pub fn latest(&self) -> Arc<FakePeerConnection> {
        self.created
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a peer connection")
    }

    pub fn count(&self) -> usize {
        self.created.lock().unwrap().len()
    }
}

#[async_trait]
impl PeerConnectionFactory for FakePeerConnections {
    async fn create(
        &self,
        kind: PeerConnectionKind,
        configuration: Configuration,
        observer: Arc<dyn PeerConnectionObserver>,
    ) -> Result<Arc<dyn PeerConnection>, PeerConnectionError> {
        if self.fail_create.load(Ordering::SeqCst) {
            return Err(PeerConnectionError("no WebRTC".to_string()));
        }
        let mut created = self.created.lock().unwrap();
        let pc = Arc::new(FakePeerConnection {
            id: created.len() + 1,
            configuration,
            kind,
            observer,
            inner: Mutex::new(Inner {
                mids: Vec::new(),
                remote: None,
                local: None,
                state: ConnectionState::New,
                closed: false,
                fail_remote: false,
                fail_channel: false,
                stats: "[]".to_string(),
            }),
            parameters: Mutex::new(HashMap::new()),
            ops: Mutex::new(Vec::new()),
            channel: Mutex::new(None),
        });
        created.push(pc.clone());
        Ok(pc)
    }
}

// --- Data channels --------------------------------------------------------------------------------

/// A data channel whose other end the test plays: what the engine sends is recorded, what the
/// test sends arrives through the observer the engine gave (a sender's, kept from
/// `create_data_channel`; a receiver's, answered by `channel_opened`). The buffered amount is
/// whatever the test sets, so backpressure can be staged.
pub struct FakeDataChannel {
    pub label: String,
    pub observer: Mutex<Option<Arc<dyn DataChannelObserver>>>,
    state: Mutex<DataChannelState>,
    pub buffered: AtomicU64,
    pub threshold: AtomicU64,
    /// Binary messages the engine sent, in order.
    pub sent: Mutex<Vec<Vec<u8>>>,
    /// Text messages the engine sent, in order.
    pub texts: Mutex<Vec<String>>,
    pub closed: AtomicBool,
}

impl FakeDataChannel {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            observer: Mutex::new(None),
            state: Mutex::new(DataChannelState::Connecting),
            buffered: AtomicU64::new(0),
            threshold: AtomicU64::new(0),
            sent: Mutex::new(Vec::new()),
            texts: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
        }
    }

    fn observer(&self) -> Arc<dyn DataChannelObserver> {
        self.observer
            .lock()
            .unwrap()
            .clone()
            .expect("the engine reads this channel")
    }

    /// The channel opens, as the platform would tell.
    pub fn open(&self) {
        *self.state.lock().unwrap() = DataChannelState::Open;
        self.observer().state_changed(DataChannelState::Open);
    }

    /// The other side closed the channel.
    pub fn closed_by_peer(&self) {
        *self.state.lock().unwrap() = DataChannelState::Closed;
        self.observer().state_changed(DataChannelState::Closed);
    }

    /// The other side sent `data`.
    pub fn deliver(&self, data: Vec<u8>) {
        self.observer().binary(data);
    }

    /// The other side sent `text`.
    pub fn deliver_text(&self, text: &str) {
        self.observer().text(text.to_string());
    }

    /// The bytes waiting fell to the threshold, as the platform would tell.
    pub fn drained(&self, to: u64) {
        self.buffered.store(to, Ordering::SeqCst);
        self.observer().buffered_amount_low();
    }

    /// Every byte the engine sent, joined.
    pub fn sent_bytes(&self) -> Vec<u8> {
        self.sent.lock().unwrap().concat()
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }
}

impl DataChannel for FakeDataChannel {
    fn send_binary(&self, data: Vec<u8>) -> Result<(), PeerConnectionError> {
        if *self.state.lock().unwrap() != DataChannelState::Open {
            return Err(PeerConnectionError("the channel is not open".to_string()));
        }
        self.sent.lock().unwrap().push(data);
        Ok(())
    }

    fn send_text(&self, text: String) -> Result<(), PeerConnectionError> {
        if *self.state.lock().unwrap() != DataChannelState::Open {
            return Err(PeerConnectionError("the channel is not open".to_string()));
        }
        self.texts.lock().unwrap().push(text);
        Ok(())
    }

    fn buffered_amount(&self) -> u64 {
        self.buffered.load(Ordering::SeqCst)
    }

    fn set_buffered_amount_low_threshold(&self, bytes: u64) {
        self.threshold.store(bytes, Ordering::SeqCst);
    }

    fn state(&self) -> DataChannelState {
        *self.state.lock().unwrap()
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        *self.state.lock().unwrap() = DataChannelState::Closed;
    }
}

// --- Files ----------------------------------------------------------------------------------------

/// A picked file: its bytes, and how many times it has been opened.
pub struct FakeFile {
    pub bytes: Vec<u8>,
    pub opened: AtomicU64,
    /// What `read` answers, when set: the file could not be read.
    pub fail_read: AtomicBool,
    /// What `size` answers in place of the bytes' length, when set: a file that changed.
    pub claimed_size: Mutex<Option<u64>>,
}

/// One opening of a `FakeFile`, read from its start.
pub struct FakeSource {
    file: Arc<FakeFile>,
    position: Mutex<u64>,
    pub closed: AtomicBool,
}

impl FakeSource {
    /// How far it has been read.
    pub fn position(&self) -> u64 {
        *self.position.lock().unwrap()
    }
}

#[async_trait]
impl FileSource for FakeSource {
    fn size(&self) -> u64 {
        self.file
            .claimed_size
            .lock()
            .unwrap()
            .unwrap_or(self.file.bytes.len() as u64)
    }

    async fn read(&self, max_bytes: u64) -> Result<Vec<u8>, FileError> {
        if self.file.fail_read.load(Ordering::SeqCst) {
            return Err(FileError("the disk refused".to_string()));
        }
        let mut position = self.position.lock().unwrap();
        let start = (*position as usize).min(self.file.bytes.len());
        let end = (start + max_bytes as usize).min(self.file.bytes.len());
        *position = end as u64;
        Ok(self.file.bytes[start..end].to_vec())
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

/// A place the user chose: what was written to it, in order, and how it ended.
#[derive(Default)]
pub struct FakeSink {
    pub written: Mutex<Vec<Vec<u8>>>,
    pub finished: AtomicBool,
    pub discarded: AtomicBool,
    /// Fails every `write`.
    pub fail_write: AtomicBool,
    /// Fails `finish`.
    pub fail_finish: AtomicBool,
}

impl FakeSink {
    pub fn written_bytes(&self) -> Vec<u8> {
        self.written.lock().unwrap().concat()
    }

    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    pub fn is_discarded(&self) -> bool {
        self.discarded.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl FileSink for FakeSink {
    async fn write(&self, data: Vec<u8>) -> Result<(), FileError> {
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(FileError("the disk is full".to_string()));
        }
        self.written.lock().unwrap().push(data);
        Ok(())
    }

    async fn finish(&self) -> Result<(), FileError> {
        if self.fail_finish.load(Ordering::SeqCst) {
            return Err(FileError("the file could not be closed".to_string()));
        }
        self.finished.store(true, Ordering::SeqCst);
        Ok(())
    }

    async fn discard(&self) {
        self.discarded.store(true, Ordering::SeqCst);
    }
}

/// The platform's files: picked files by reference and chosen places by destination, each given
/// by the test before the engine asks for it; a reference or destination it was not given fails.
#[derive(Default)]
pub struct FakeFiles {
    files: Mutex<HashMap<String, Arc<FakeFile>>>,
    sinks: Mutex<HashMap<String, Arc<FakeSink>>>,
    /// Every source opened, in order.
    pub sources: Mutex<Vec<Arc<FakeSource>>>,
    /// Every reference released, in order.
    pub released: Mutex<Vec<String>>,
    /// Makes every `create_sink` take a second, as a picker's place on slow storage might.
    pub slow_sinks: AtomicBool,
}

impl FakeFiles {
    /// A picked file at `reference` holding `bytes`.
    pub fn add(&self, reference: &str, bytes: Vec<u8>) -> Arc<FakeFile> {
        let file = Arc::new(FakeFile {
            bytes,
            opened: AtomicU64::new(0),
            fail_read: AtomicBool::new(false),
            claimed_size: Mutex::new(None),
        });
        self.files
            .lock()
            .unwrap()
            .insert(reference.to_string(), file.clone());
        file
    }

    /// A place the user chose at `destination`.
    pub fn add_destination(&self, destination: &str) -> Arc<FakeSink> {
        let sink = Arc::new(FakeSink::default());
        self.sinks
            .lock()
            .unwrap()
            .insert(destination.to_string(), sink.clone());
        sink
    }

    pub fn released(&self) -> Vec<String> {
        self.released.lock().unwrap().clone()
    }

    /// The source opened last.
    pub fn latest_source(&self) -> Arc<FakeSource> {
        self.sources
            .lock()
            .unwrap()
            .last()
            .cloned()
            .expect("a file was opened")
    }
}

#[async_trait]
impl Files for FakeFiles {
    async fn open(&self, reference: String) -> Result<Arc<dyn FileSource>, FileError> {
        let file = self.files.lock().unwrap().get(&reference).cloned();
        let Some(file) = file else {
            return Err(FileError(format!("no file at {reference}")));
        };
        file.opened.fetch_add(1, Ordering::SeqCst);
        let source = Arc::new(FakeSource {
            file,
            position: Mutex::new(0),
            closed: AtomicBool::new(false),
        });
        self.sources.lock().unwrap().push(source.clone());
        Ok(source)
    }

    fn release(&self, reference: String) {
        self.released.lock().unwrap().push(reference);
    }

    async fn create_sink(&self, destination: String) -> Result<Arc<dyn FileSink>, FileError> {
        if self.slow_sinks.load(Ordering::SeqCst) {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let sink = self.sinks.lock().unwrap().get(&destination).cloned();
        match sink {
            Some(sink) => Ok(sink),
            None => Err(FileError(format!("no place at {destination}"))),
        }
    }
}

// --- The observer and the world ---------------------------------------------------------------------

/// Every snapshot published, and every sound cued, each in order.
#[derive(Default)]
pub struct Snapshots(pub Mutex<Vec<CallState>>, pub Mutex<Vec<SoundCue>>);

impl CallObserver for Snapshots {
    fn state_changed(&self, state: &CallState) {
        self.0.lock().unwrap().push(state.clone());
    }

    fn play_sound(&self, cue: SoundCue) {
        self.1.lock().unwrap().push(cue);
    }
}

pub struct World {
    pub http: Arc<FakeHttp>,
    pub sockets: Arc<FakeSockets>,
    pub pcs: Arc<FakePeerConnections>,
    pub files: Arc<FakeFiles>,
    pub snapshots: Arc<Snapshots>,
    pub commands: mpsc::UnboundedSender<Command>,
    pub channel: Uuid,
    pub user: Uuid,
    pub session: Uuid,
}

pub const MICROPHONE: &str = "mic-1";

impl World {
    /// A call on the test's runtime, with the random pauses at `random` of their most.
    pub fn new(random: f64) -> Self {
        let http = Arc::new(FakeHttp::default());
        let sockets = Arc::new(FakeSockets::default());
        let pcs = Arc::new(FakePeerConnections::default());
        let files = Arc::new(FakeFiles::default());
        let snapshots = Arc::new(Snapshots::default());
        let platform = Platform {
            http: http.clone(),
            sockets: sockets.clone(),
            peer_connections: pcs.clone(),
            files: files.clone(),
        };
        let options = Options {
            api_url: API_URL.to_string(),
            credentials: Arc::new(Token),
            loopback_plain_text: true,
            random: Box::new(move || random),
        };
        let call = Call::new(platform, options, snapshots.clone());
        let (commands, receiver) = mpsc::unbounded_channel();
        tokio::spawn(call.run(receiver));
        Self {
            http,
            sockets,
            pcs,
            files,
            snapshots,
            commands,
            channel: Uuid::now_v7(),
            user: Uuid::now_v7(),
            session: Uuid::now_v7(),
        }
    }

    pub fn send(&self, command: Command) {
        self.commands.send(command).expect("the call runs");
    }

    /// The latest snapshot, or the state before any.
    pub fn state(&self) -> CallState {
        self.snapshots
            .0
            .lock()
            .unwrap()
            .last()
            .cloned()
            .unwrap_or_default()
    }

    pub fn microphone() -> TrackId {
        TrackId(MICROPHONE.to_string())
    }

    /// Every sound cued since the last call.
    pub fn sounds(&self) -> Vec<SoundCue> {
        std::mem::take(&mut *self.snapshots.1.lock().unwrap())
    }

    /// Gives the microphone and joins `channel`, letting the offer be asked for and the
    /// candidates ranked (their health answered at once unless scripted otherwise).
    pub async fn join(&self, offer: &Offer) {
        self.http.answer_join(offer.response());
        self.send(Command::SetMicrophone(Some(Self::microphone())));
        self.send(Command::Join {
            channel: offer.channel,
        });
        settle().await;
    }

    /// Admits the client at `server`, offers one audio slot, and connects: the happy path from
    /// `identify` to `Connected`. Returns every frame the client sent after `identify`.
    pub async fn admit(&self, server: &FakeServer) -> Vec<ClientMessage> {
        let received = server.received();
        assert!(
            received.contains(&ClientMessage::Identify {
                token: "join-token".to_string()
            }),
            "identify is the first frame: {received:?}"
        );
        server.ready(self.session, self.user, Vec::new(), "secret-1");
        settle().await;
        server.offer(1, vec![slot("0", 0)]);
        settle().await;
        self.pcs.latest().report(ConnectionState::Connected);
        settle().await;
        assert_eq!(self.state().status, CallStatus::Connected);
        server.received()
    }

    /// Joins and gets connected at one server.
    pub async fn connected(&self, url: &str) -> (Offer, Arc<FakeServer>) {
        let offer = Offer::to(self.channel, &[url]);
        let server = self.sockets.server(url);
        self.join(&offer).await;
        let frames = self.admit(&server).await;
        assert!(frames.contains(&ClientMessage::SetState {
            muted: false,
            deafened: false,
            held: false
        }));
        (offer, server)
    }
}

// --- The voice server's transfer frames, and the other side of a transfer ------------------

/// The ICE servers a `transferStarting` names: a STUN server and a TURN relay.
pub fn ice() -> Vec<IceServer> {
    vec![
        IceServer {
            urls: vec!["stun:voice.example:3478".to_string()],
            username: None,
            credential: None,
        },
        IceServer {
            urls: vec!["turn:voice.example:3478?transport=udp".to_string()],
            username: Some("u".to_string()),
            credential: Some("c".to_string()),
        },
    ]
}

/// A `fileOffered` frame.
pub fn offered(
    id: Uuid,
    from: Uuid,
    name: &str,
    size: u64,
    allow_direct: bool,
    ms: u64,
) -> ServerMessage {
    ServerMessage::FileOffered {
        offer: FileOffer {
            id,
            from,
            name: name.to_string(),
            size,
            allow_direct,
            expires_in_ms: ms,
        },
    }
}

/// A `transferStarting` frame with `ice()`'s servers.
pub fn starting(
    offer: Uuid,
    peer: Uuid,
    role: TransferRole,
    mode: TransferMode,
    name: &str,
    size: u64,
) -> ServerMessage {
    ServerMessage::TransferStarting {
        offer,
        peer,
        role,
        mode,
        name: name.to_string(),
        size,
        ice_servers: ice(),
    }
}

/// A `transferSignal` frame carrying `signal`.
pub fn signal(offer: Uuid, peer: Uuid, signal: Value) -> ServerMessage {
    ServerMessage::TransferSignal {
        offer,
        peer,
        signal,
    }
}

/// A transfer signal carrying a session description.
pub fn description(kind: &str, sdp: &str) -> Value {
    json!({ "kind": "description", "description": { "type": kind, "sdp": sdp } })
}

/// Opens the sender's channel on the receiver's connection, as the platform would on the other
/// side's channel arriving, and opens it.
pub fn channel_arrives(pc: &FakePeerConnection) -> Arc<FakeDataChannel> {
    let channel = Arc::new(FakeDataChannel::new("file"));
    let observer = pc.channel_opened(channel.clone());
    *channel.observer.lock().unwrap() = Some(observer);
    channel.open();
    channel
}

pub fn deliver_all(channel: &FakeDataChannel, bytes: &[u8]) {
    for chunk in bytes.chunks(CHUNK_BYTES as usize) {
        channel.deliver(chunk.to_vec());
    }
}
