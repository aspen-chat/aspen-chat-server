//! Stand-ins for the platform: an API server answering the join and failure routes and each
//! candidate's health check, voice servers scripted frame by frame in `voice_protocol::signal`,
//! and a peer connection that records what it is asked and reports the states a test gives
//! it. `World` wires them to a `Call` running on the test's runtime.

#![allow(dead_code)]

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;
use voice_client::api::Credentials;
use voice_client::call::{Call, Command, Options, Platform};
use voice_client::peer_connection::{
    Configuration, ConnectionState, DescriptionKind, Direction, MediaKind, PeerConnection,
    PeerConnectionError, PeerConnectionFactory, PeerConnectionObserver, SendEncoding,
    SenderParameters, SessionDescription, SignalingState, StatsOf, TrackId, TransceiverId,
};
use voice_client::sounds::SoundCue;
use voice_client::state::{CallObserver, CallState, CallStatus};
use voice_client::transport::{
    Http, HttpRequest, HttpResponse, Received, Socket, SocketFactory, TransportError,
};
use voice_protocol::signal::{
    ClientMessage, ConnectionKind, MediaSource, ParticipantInfo, Section, SectionContent,
    ServerMessage,
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
                r#"{{"channelId":"{}","candidates":[{}],"token":"{}","expiresAt":"2026-01-01T00:00:00Z","speak":{},"shareScreen":{},"transferFiles":false,"useCamera":{},"serverMuted":{}}}"#,
                self.channel,
                candidates.join(","),
                self.token,
                self.speak,
                self.share_screen,
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
}

pub struct FakePeerConnection {
    pub id: usize,
    observer: Arc<dyn PeerConnectionObserver>,
    inner: Mutex<Inner>,
    parameters: Mutex<HashMap<TransceiverId, SenderParameters>>,
    pub ops: Mutex<Vec<Op>>,
}

impl FakePeerConnection {
    /// Reports a connection state, as the platform's observer would.
    pub fn report(&self, state: ConnectionState) {
        self.inner.lock().unwrap().state = state;
        self.observer.connection_state_changed(state);
    }

    pub fn fail_next_remote(&self) {
        self.inner.lock().unwrap().fail_remote = true;
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
    async fn create_offer(&self, _ice_restart: bool) -> Result<String, PeerConnectionError> {
        Err(PeerConnectionError("the client never offers".to_string()))
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
        assert_eq!(description.kind, DescriptionKind::Offer);
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
        Ok("[]".to_string())
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
        _configuration: Configuration,
        observer: Arc<dyn PeerConnectionObserver>,
    ) -> Result<Arc<dyn PeerConnection>, PeerConnectionError> {
        if self.fail_create.load(Ordering::SeqCst) {
            return Err(PeerConnectionError("no WebRTC".to_string()));
        }
        let mut created = self.created.lock().unwrap();
        let pc = Arc::new(FakePeerConnection {
            id: created.len() + 1,
            observer,
            inner: Mutex::new(Inner {
                mids: Vec::new(),
                remote: None,
                local: None,
                state: ConnectionState::New,
                closed: false,
                fail_remote: false,
            }),
            parameters: Mutex::new(HashMap::new()),
            ops: Mutex::new(Vec::new()),
        });
        created.push(pc.clone());
        Ok(pc)
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
        let snapshots = Arc::new(Snapshots::default());
        let platform = Platform {
            http: http.clone(),
            sockets: sockets.clone(),
            peer_connections: pcs.clone(),
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
