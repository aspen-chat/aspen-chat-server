//! One call from the client's side: everything the web client's `VoiceCall` decides, on the
//! voice server's signalling (`voice_protocol::signal`), where the server is the only offerer.
//!
//! Joining asks the API server for an offer (a token and the candidate voice servers), measures
//! the round trip to each candidate, and tries them nearest first: a signalling socket is
//! opened, the token presented, and if the server does not answer `ready` in time, or its media
//! never connects, the failure is reported to the API server and the next candidate is tried.
//! Once in, the server offers the peer connection; the engine answers each offer at once,
//! attaching the track of each source it sends and reading the track of each slot and video it
//! receives, and media on a new section flows once the answer is applied.
//!
//! A socket that drops is first taken back with `resume` on the same server, within the grace
//! the server keeps the participant for, media flowing meanwhile; only when that fails is the
//! channel joined again, after a random pause of up to a second so the participants of a lost
//! call do not all ask the API server at once. A call ended for being idle, or by the user, is
//! not rejoined.
//!
//! Everything runs on one task: commands from the engine's facade, frames from the sockets,
//! the peer connection's state changes, and the timers all arrive as events on one queue, so no
//! lock guards the call's state and every handler sees the whole of it.

use crate::api::{Api, ApiError, Candidate, Credentials, JoinOffer};
use crate::peer_connection::{
    Configuration, ConnectionState, DescriptionKind, Direction, PeerConnection,
    PeerConnectionError, PeerConnectionFactory, PeerConnectionObserver, SessionDescription,
    TrackId, TransceiverId,
};
use crate::signalling::{Incoming, Signalling, signalling_url};
use crate::state::{
    AudioSlot, CallObserver, CallState, CallStatus, EndedReason, ErrorKind, LocalSource,
    Participant, Refusal, RemoteVideo, SessionEndReason, ShareAudio,
};
use crate::transport::{Http, Socket, SocketFactory, TransportError};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use uuid::Uuid;
use voice_protocol::signal::{
    ClientMessage, ConnectionKind, KickReason, MediaSource, ParticipantInfo, RESUME_GRACE_SECONDS,
    Section, SectionContent, ServerMessage, SlotAssignment, WatchedVideo,
};
use voice_protocol::token::Grants;

/// How long a candidate has to accept the socket and answer `identify` (or `resume`) with
/// `ready`.
pub const READY_TIMEOUT: Duration = Duration::from_secs(6);
/// How long a candidate has to answer `GET /health` when the candidates are ranked.
pub const PING_TIMEOUT: Duration = Duration::from_secs(3);
/// How long the peer connection has to connect once the server has sent `ready`. The server
/// gives up after thirty seconds of its own; a client that gives up sooner tries the next
/// candidate while the user still waits.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// The most a lost call waits before joining again.
pub const REJOIN_DELAY_MAX: Duration = Duration::from_secs(1);
/// How long a rejoin that could not reach the server the API server still offered waits before
/// asking for an offer once more. It is the API server's `offer_silence_seconds`: a voice server
/// silent for that long is no longer offered, and a call bound to it is ended when someone next
/// asks to join its channel, so the second offer names a living server.
pub const DEAD_SERVER_RETRY: Duration = Duration::from_secs(60);
/// How long the server keeps a participant whose socket dropped, for `resume`.
pub const RESUME_GRACE: Duration = Duration::from_secs(RESUME_GRACE_SECONDS);

/// The platform's side of the network and of WebRTC.
pub struct Platform {
    pub http: Arc<dyn Http>,
    pub sockets: Arc<dyn SocketFactory>,
    pub peer_connections: Arc<dyn PeerConnectionFactory>,
}

pub struct Options {
    /// The deployment's `public_url`.
    pub api_url: String,
    pub credentials: Arc<dyn Credentials>,
    /// Whether plain `http:` voice servers at this machine's own addresses may be reached, as a
    /// development build allows; every other build reaches voice servers over HTTPS alone.
    pub loopback_plain_text: bool,
    /// Uniform in [0, 1); seeds the rejoin and resume pauses.
    pub random: Box<dyn Fn() -> f64 + Send>,
}

impl Options {
    /// Uniform in [0, 1) from the standard library's randomly keyed hasher, which is seeded from
    /// the operating system and needs no crate of its own: a pause spread by it is spread.
    pub fn system_random() -> f64 {
        use std::hash::{BuildHasher, RandomState};
        (RandomState::new().hash_one(0u8) >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// What the engine's facade asks of the call.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Joins a channel's call, leaving any current one first, with the microphone kept.
    Join {
        channel: Uuid,
    },
    Leave,
    SetMuted(bool),
    SetDeafened(bool),
    /// Follows a moderator's mute of the user being made or lifted, as the API server's
    /// `voiceMute` events say; the voice server applies it to the call itself.
    SetServerMuted(bool),
    /// The microphone track to send: given before joining, or to replace the one sent (a new
    /// input device); `None` closes it.
    SetMicrophone(Option<TrackId>),
    /// Sends a camera or a screen (its picture as `screen`, its sound as `screenAudio`) from a
    /// track the platform opened. A microphone given here is `SetMicrophone`.
    StartSource {
        source: MediaSource,
        track: TrackId,
    },
    StopSource(MediaSource),
    /// The videos to show, replacing what was watched before.
    SetWatched(Vec<WatchedVideo>),
    /// How loud `user`'s voice (`microphone`) or the sound of what they share (`screenAudio`) is
    /// to this user alone, 0 to 10, 1 as sent.
    SetUserVolume {
        user: Uuid,
        source: MediaSource,
        gain: f64,
    },
    /// Silences one person's voice or share sound for this user alone, keeping their volume.
    SetUserMuted {
        user: Uuid,
        source: MediaSource,
        muted: bool,
    },
    /// The API server's `voiceSessionEnded` event, which the app layer hands on.
    SessionEnded {
        session: Uuid,
        channel: Uuid,
        reason: SessionEndReason,
    },
    AcknowledgeEnd,
    ClearStateRefusal,
    ClearSourceRefusal,
    /// Leaves any call and ends the engine.
    Shutdown,
}

/// What went wrong with an attempt to join.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Failure {
    Offer(ApiError),
    Microphone,
    /// A voice server's own: its socket not opening, a fatal `error`, no `ready` in time, media
    /// that never connected. Reported, and the next candidate tried.
    Server(String),
    /// A candidate not reached over HTTPS, which is never tried nor blamed.
    Insecure(String),
    Refused(Refusal),
    Device(String),
}

impl Failure {
    fn kind(&self) -> ErrorKind {
        match self {
            Failure::Offer(_) => ErrorKind::Offer,
            Failure::Microphone => ErrorKind::Microphone,
            Failure::Server(_) | Failure::Insecure(_) => ErrorKind::Server,
            Failure::Refused(_) => ErrorKind::Refused,
            Failure::Device(_) => ErrorKind::Device,
        }
    }

    fn detail(&self) -> String {
        match self {
            Failure::Offer(e) => e.to_string(),
            Failure::Microphone => {
                "the call lets you speak and no microphone was given".to_string()
            }
            Failure::Server(detail) | Failure::Device(detail) => detail.clone(),
            Failure::Insecure(url) => format!("{url} is not reached over HTTPS"),
            Failure::Refused(refusal) => refusal.detail.clone(),
        }
    }

    fn retry_after_seconds(&self) -> Option<u64> {
        match self {
            Failure::Refused(refusal) => refusal.retry_after_seconds,
            _ => None,
        }
    }

    /// Whether the voice servers offered failed to take the call, which another offer may
    /// change, rather than something no other offer would.
    fn is_servers(&self) -> bool {
        matches!(self, Failure::Server(_))
    }
}

/// What arrives on the call's queue besides commands.
enum Internal {
    Offered {
        generation: u64,
        result: Result<JoinOffer, ApiError>,
    },
    Ranked {
        generation: u64,
        candidates: Vec<Candidate>,
    },
    Opened {
        generation: u64,
        step: u64,
        result: Result<Arc<dyn Socket>, TransportError>,
    },
    ResumeOpened {
        generation: u64,
        step: u64,
        result: Result<Arc<dyn Socket>, TransportError>,
    },
    Frame {
        socket: u64,
        frame: ServerMessage,
    },
    SocketClosed {
        socket: u64,
    },
    ConnectionState {
        connection: u64,
        state: ConnectionState,
    },
    Timeout {
        generation: u64,
        step: u64,
        kind: Timer,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Timer {
    /// `READY_TIMEOUT` after a socket was asked for.
    Ready,
    /// `CONNECT_TIMEOUT` after `ready`.
    Connect,
    /// The random pause before a rejoin.
    RejoinDelay,
    /// `DEAD_SERVER_RETRY` before the second offer of a rejoin.
    DeadServerRetry,
    /// The random pause before trying `resume` again.
    ResumeRetry,
}

/// One signalling socket, numbered so frames from one the call has let go are told apart.
#[derive(Clone)]
struct Link {
    id: u64,
    signalling: Arc<Signalling>,
}

impl Link {
    /// A send that fails means the socket is closing, which `SocketClosed` tells of.
    async fn send(&self, frame: ClientMessage) {
        let _ = self.signalling.send(&frame).await;
    }

    fn close(&self) {
        self.signalling.close();
    }
}

/// The peer connection, numbered as `Link` is.
#[derive(Clone)]
struct Peer {
    id: u64,
    pc: Arc<dyn PeerConnection>,
}

struct Forwarder {
    connection: u64,
    internal: mpsc::UnboundedSender<Internal>,
}

impl PeerConnectionObserver for Forwarder {
    fn connection_state_changed(&self, state: ConnectionState) {
        let _ = self.internal.send(Internal::ConnectionState {
            connection: self.connection,
            state,
        });
    }
}

/// Why a channel is joined again without the user asking, and how far that has got.
#[derive(Clone, Copy, Debug)]
struct Rejoin {
    channel: Uuid,
    /// The server went away without a word, so the API server may still offer it: a rejoin that
    /// cannot reach the servers offered waits `DEAD_SERVER_RETRY` and asks once more.
    server_silent: bool,
    /// Whether that second offer has been asked for.
    retried: bool,
}

struct Trying {
    candidate: Candidate,
    link: Option<Link>,
}

/// A join or rejoin under way, from asking for the offer to the first connected candidate.
struct Attempt {
    rejoin: Option<Rejoin>,
    offer: Option<JoinOffer>,
    candidates: VecDeque<Candidate>,
    current: Option<Trying>,
    last_error: Option<Failure>,
}

struct Resuming {
    /// When the server's grace for the dropped socket runs out.
    deadline: Instant,
    link: Option<Link>,
}

/// The call from `ready` on: a place on one server, its socket, and its peer connection.
struct Live {
    server: Candidate,
    link: Link,
    connection: Option<Peer>,
    /// The secret `resume` presents, made anew by each `ready`.
    secret: String,
    sections: Vec<Section>,
    transceivers: HashMap<String, TransceiverId>,
    /// The answer to the call connection's latest offer, by revision. The server restates an
    /// offer still outstanding after a resume, with the same revision, and the peer connection
    /// has already applied it: the same answer is sent again and nothing is negotiated anew.
    answered: Option<(u64, String)>,
    /// The track on the microphone's sender: `None` while the user shows muted.
    microphone_attached: Option<TrackId>,
    /// The volume last set on each receiver, so one is set only when it changes.
    volumes_applied: HashMap<TransceiverId, f64>,
    resuming: Option<Resuming>,
}

impl Live {
    fn transceiver_of(&self, content: impl Fn(&SectionContent) -> bool) -> Option<TransceiverId> {
        self.sections
            .iter()
            .find(|section| content(&section.content))
            .and_then(|section| self.transceivers.get(&section.mid).copied())
    }

    fn sending_transceiver(&self, source: MediaSource) -> Option<TransceiverId> {
        self.transceiver_of(|content| *content == SectionContent::Sending { source })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Asked {
    muted: bool,
    deafened: bool,
}

#[derive(Clone, Copy, Debug)]
struct Volume {
    gain: f64,
    muted: bool,
}

pub struct Call {
    platform: Platform,
    api: Arc<Api>,
    loopback_plain_text: bool,
    random: Box<dyn Fn() -> f64 + Send>,
    observer: Arc<dyn CallObserver>,
    internal: mpsc::UnboundedSender<Internal>,
    internal_rx: Option<mpsc::UnboundedReceiver<Internal>>,
    state: CallState,
    published: CallState,
    /// Counts up with every join, rejoin, leave, and end, so the result of work started before
    /// is let go.
    generation: u64,
    /// Counts up with every phase a timer could outlive, so a timer of a phase that is over is
    /// let go.
    step: u64,
    next_id: u64,
    /// The mute and deafen the user last asked for. In a call, `state.muted` and
    /// `state.deafened` show the server's word, except that they show the user silenced the
    /// moment they ask to be: muting and deafening take effect at once (the microphone's track
    /// is taken off its sender here too, and the server never refuses them), while unmuting
    /// and undeafening show only once the server says it did them, so a refused one never shows
    /// the user audible when they are not.
    asked: Asked,
    microphone: Option<TrackId>,
    volumes: HashMap<(Uuid, MediaSource), Volume>,
    attempt: Option<Attempt>,
    pending_rejoin: Option<Rejoin>,
    live: Option<Live>,
}

impl Call {
    pub fn new(platform: Platform, options: Options, observer: Arc<dyn CallObserver>) -> Self {
        let (internal, internal_rx) = mpsc::unbounded_channel();
        let api = Arc::new(Api::new(
            platform.http.clone(),
            &options.api_url,
            options.credentials,
        ));
        Self {
            platform,
            api,
            loopback_plain_text: options.loopback_plain_text,
            random: options.random,
            observer,
            internal,
            internal_rx: Some(internal_rx),
            state: CallState::default(),
            published: CallState::default(),
            generation: 0,
            step: 0,
            next_id: 0,
            asked: Asked::default(),
            microphone: None,
            volumes: HashMap::new(),
            attempt: None,
            pending_rejoin: None,
            live: None,
        }
    }

    /// Runs the call until `Shutdown` or the commands' sender is dropped, leaving any call then.
    pub async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        let mut internal = self.internal_rx.take().expect("run is called once");
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Some(Command::Shutdown) | None => {
                        self.generation += 1;
                        self.teardown(false).await;
                        self.reset_to_idle();
                        self.publish();
                        return;
                    }
                    Some(command) => self.handle_command(command).await,
                },
                event = internal.recv() => match event {
                    Some(event) => self.handle_internal(event).await,
                    None => return,
                },
            }
            self.publish();
        }
    }

    fn publish(&mut self) {
        self.state.revision = self.published.revision;
        if self.state != self.published {
            self.state.revision += 1;
            self.published = self.state.clone();
            self.observer.state_changed(&self.published);
        }
    }

    fn id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    fn timer(&self, kind: Timer, after: Duration) {
        let internal = self.internal.clone();
        let (generation, step) = (self.generation, self.step);
        tokio::spawn(async move {
            tokio::time::sleep(after).await;
            let _ = internal.send(Internal::Timeout {
                generation,
                step,
                kind,
            });
        });
    }

    fn pause(&self) -> Duration {
        Duration::from_millis(((self.random)() * REJOIN_DELAY_MAX.as_millis() as f64) as u64)
    }

    /// Wraps a socket and reads it until it closes, each frame arriving on the queue.
    fn link(&mut self, socket: Arc<dyn Socket>) -> Link {
        let link = Link {
            id: self.id(),
            signalling: Arc::new(Signalling::new(socket)),
        };
        let (reader, internal, id) = (link.signalling.clone(), self.internal.clone(), link.id);
        tokio::spawn(async move {
            loop {
                match reader.receive().await {
                    Incoming::Frame(frame) => {
                        let _ = internal.send(Internal::Frame { socket: id, frame });
                    }
                    Incoming::Unknown { .. } => {}
                    Incoming::Closed => {
                        let _ = internal.send(Internal::SocketClosed { socket: id });
                        return;
                    }
                }
            }
        });
        link
    }

    fn open(&self, url: String, step: u64, resume: bool) {
        let (sockets, internal, generation) = (
            self.platform.sockets.clone(),
            self.internal.clone(),
            self.generation,
        );
        tokio::spawn(async move {
            let result = sockets.connect(url, None).await;
            let _ = internal.send(if resume {
                Internal::ResumeOpened {
                    generation,
                    step,
                    result,
                }
            } else {
                Internal::Opened {
                    generation,
                    step,
                    result,
                }
            });
        });
    }

    // --- Commands ---------------------------------------------------------------------------

    async fn handle_command(&mut self, command: Command) {
        match command {
            Command::Join { channel } => self.join(channel).await,
            Command::Leave => self.leave().await,
            Command::SetMuted(muted) => {
                self.ask(Asked {
                    muted,
                    ..self.asked
                })
                .await
            }
            Command::SetDeafened(deafened) => {
                self.ask(Asked {
                    deafened,
                    ..self.asked
                })
                .await
            }
            Command::SetServerMuted(server_muted) => {
                if self.state.status != CallStatus::Idle {
                    self.state.server_muted = server_muted;
                }
            }
            Command::SetMicrophone(track) => self.set_microphone(track).await,
            Command::StartSource { source, track } => self.start_source(source, track).await,
            Command::StopSource(source) => self.stop_source(source).await,
            Command::SetWatched(video) => self.set_watched(video).await,
            Command::SetUserVolume { user, source, gain } => {
                self.volumes
                    .entry((user, source))
                    .or_insert(Volume {
                        gain: 1.0,
                        muted: false,
                    })
                    .gain = gain.clamp(0.0, 10.0);
                self.apply_volumes(user, source).await;
            }
            Command::SetUserMuted {
                user,
                source,
                muted,
            } => {
                self.volumes
                    .entry((user, source))
                    .or_insert(Volume {
                        gain: 1.0,
                        muted: false,
                    })
                    .muted = muted;
                self.apply_volumes(user, source).await;
            }
            Command::SessionEnded {
                session,
                channel,
                reason,
            } => self.session_ended(session, channel, reason).await,
            Command::AcknowledgeEnd => self.state.ended_reason = None,
            Command::ClearStateRefusal => self.state.state_refused = None,
            Command::ClearSourceRefusal => self.state.source_refused = None,
            Command::Shutdown => {}
        }
    }

    async fn join(&mut self, channel: Uuid) {
        // Already in, or on the way into, this very call: nothing to do.
        if self.state.channel == Some(channel)
            && matches!(
                self.state.status,
                CallStatus::Joining | CallStatus::Connected | CallStatus::Rejoining
            )
        {
            return;
        }
        // Moving from one call to another keeps the microphone open for the next one.
        if self.state.channel.is_some() {
            self.teardown(true).await;
            if self.state.channel != Some(channel) {
                self.state.watching.clear();
            }
        }
        self.generation += 1;
        self.pending_rejoin = None;
        self.state.status = CallStatus::Joining;
        self.state.channel = Some(channel);
        self.state.error_kind = None;
        self.state.error = None;
        self.state.retry_after_seconds = None;
        self.state.ended_reason = None;
        self.state.state_refused = None;
        self.state.source_refused = None;
        self.start_attempt(channel, None);
    }

    async fn leave(&mut self) {
        if self.state.status == CallStatus::Idle {
            return;
        }
        self.generation += 1;
        self.teardown(false).await;
        self.reset_to_idle();
    }

    /// Ends the call without the user asking, telling them why when there is a reason.
    async fn end(&mut self, reason: Option<EndedReason>) {
        self.generation += 1;
        self.teardown(false).await;
        self.reset_to_idle();
        self.state.ended_reason = reason;
    }

    /// The state outside a call: nothing of one, the mute and deafen lifted.
    fn reset_to_idle(&mut self) {
        self.state = CallState {
            revision: self.state.revision,
            microphone: self.state.microphone.clone(),
            ..CallState::default()
        };
        self.asked = Asked::default();
    }

    /// Asks for `asked` as the user's mute and deafen. Outside a connected call it simply is the
    /// state; in one, whatever it silences shows at once and whatever it lifts waits for the
    /// voice server (`participantState`).
    async fn ask(&mut self, asked: Asked) {
        self.asked = asked;
        self.state.state_refused = None;
        if self.state.status != CallStatus::Connected {
            self.state.muted = asked.muted;
            self.state.deafened = asked.deafened;
            return;
        }
        self.state.muted |= asked.muted;
        self.state.deafened |= asked.deafened;
        self.hold_microphone().await;
        self.send_state().await;
    }

    async fn send_state(&mut self) {
        if let Some(live) = &self.live {
            let link = live.link.clone();
            link.send(ClientMessage::SetState {
                muted: self.asked.muted,
                deafened: self.asked.deafened,
            })
            .await;
        }
    }

    /// The track the microphone's sender carries: the microphone, or nothing while the user
    /// shows muted, so nothing leaves the device whatever the server does with its pause.
    fn outgoing_microphone(&self) -> Option<TrackId> {
        if self.state.muted {
            None
        } else {
            self.microphone.clone()
        }
    }

    /// Keeps the microphone's sender carrying `outgoing_microphone`.
    async fn hold_microphone(&mut self) {
        let wanted = self.outgoing_microphone();
        let Some(live) = &mut self.live else {
            return;
        };
        let (Some(peer), Some(transceiver)) = (
            live.connection.clone(),
            live.sending_transceiver(MediaSource::Microphone),
        ) else {
            return;
        };
        if live.microphone_attached == wanted {
            return;
        }
        live.microphone_attached = wanted.clone();
        let _ = peer.pc.replace_track(transceiver, wanted).await;
    }

    async fn set_microphone(&mut self, track: Option<TrackId>) {
        self.microphone = track.clone();
        self.state.microphone = track.clone();
        if self.live.is_none() {
            return;
        }
        let sending = self
            .state
            .sources
            .iter()
            .position(|source| source.source == MediaSource::Microphone);
        match (track, sending) {
            (Some(track), Some(at)) => {
                self.state.sources[at].track = track;
                self.hold_microphone().await;
            }
            (Some(track), None) => {
                if self.state.grants.speak {
                    self.request_source(MediaSource::Microphone, track).await;
                }
            }
            (None, Some(_)) => self.stop_source(MediaSource::Microphone).await,
            (None, None) => {}
        }
    }

    /// Asks the server for a section to send `source` on; the offer that brings it takes the
    /// track.
    async fn request_source(&mut self, source: MediaSource, track: TrackId) {
        let Some(live) = &self.live else {
            return;
        };
        self.state.sources.push(LocalSource {
            source,
            track,
            live: false,
        });
        let link = live.link.clone();
        link.send(ClientMessage::Send {
            source,
            external: false,
        })
        .await;
    }

    async fn start_source(&mut self, source: MediaSource, track: TrackId) {
        if source == MediaSource::Microphone {
            self.set_microphone(Some(track)).await;
            return;
        }
        let refused = if self.state.status != CallStatus::Connected {
            Some("not in a call")
        } else if !self.state.grants.may_send(source) {
            Some("this call does not let you send it")
        } else if self.state.sources.iter().any(|s| s.source == source) {
            Some("already sending it")
        } else {
            None
        };
        if let Some(detail) = refused {
            self.state.source_refused = Some((
                source,
                Refusal {
                    detail: detail.to_string(),
                    retry_after_seconds: None,
                },
            ));
            return;
        }
        self.state.source_refused = None;
        self.request_source(source, track).await;
    }

    /// Stops sending `source`: its sender carries nothing from now, and the server takes its
    /// section away in a later offer. The microphone stays held for the next time it may be
    /// sent.
    async fn stop_source(&mut self, source: MediaSource) {
        let Some(at) = self.state.sources.iter().position(|s| s.source == source) else {
            return;
        };
        self.state.sources.remove(at);
        let Some(live) = &mut self.live else {
            return;
        };
        let link = live.link.clone();
        if let (Some(peer), Some(transceiver)) =
            (live.connection.clone(), live.sending_transceiver(source))
        {
            if source == MediaSource::Microphone {
                live.microphone_attached = None;
            }
            let _ = peer.pc.replace_track(transceiver, None).await;
        }
        link.send(ClientMessage::StopSending { source }).await;
    }

    /// Asks for `video`, the first `max_watched_video` of it: the server gives sections for no
    /// more and leaves the rest out, so asking for them would only mislead the state.
    async fn set_watched(&mut self, mut video: Vec<WatchedVideo>) {
        if self.state.status == CallStatus::Connected
            && let Some(live) = &self.live
        {
            video.truncate(self.state.max_watched_video as usize);
            self.state.watching = video.clone();
            let link = live.link.clone();
            link.send(ClientMessage::Watch { video }).await;
        } else {
            self.state.watching = video;
        }
    }

    fn effective_volume(&self, user: Uuid, source: MediaSource) -> f64 {
        self.volumes
            .get(&(user, source))
            .map_or(1.0, |volume| if volume.muted { 0.0 } else { volume.gain })
    }

    /// Sets the volume of every receiver carrying `user`'s `source` now: the slots they are
    /// in, or the sound of their share.
    async fn apply_volumes(&mut self, user: Uuid, source: MediaSource) {
        let mids: Vec<String> = match source {
            MediaSource::Microphone => self
                .state
                .slots
                .iter()
                .filter(|slot| slot.user == Some(user))
                .filter_map(|slot| self.slot_mid(slot.slot))
                .collect(),
            MediaSource::ScreenAudio => self
                .live
                .iter()
                .flat_map(|live| &live.sections)
                .filter(|section| section.content == SectionContent::ShareAudio { user })
                .map(|section| section.mid.clone())
                .collect(),
            MediaSource::Screen | MediaSource::Camera => Vec::new(),
        };
        for mid in mids {
            self.apply_volume(&mid, user, source).await;
        }
    }

    fn slot_mid(&self, slot: u8) -> Option<String> {
        self.live.as_ref().and_then(|live| {
            live.sections
                .iter()
                .find(|section| section.content == SectionContent::AudioSlot { slot })
                .map(|section| section.mid.clone())
        })
    }

    async fn apply_volume(&mut self, mid: &str, user: Uuid, source: MediaSource) {
        let volume = self.effective_volume(user, source);
        let Some(live) = &mut self.live else {
            return;
        };
        let (Some(peer), Some(&transceiver)) =
            (live.connection.clone(), live.transceivers.get(mid))
        else {
            return;
        };
        // A receiver plays at 1 until told otherwise.
        if live
            .volumes_applied
            .get(&transceiver)
            .copied()
            .unwrap_or(1.0)
            == volume
        {
            return;
        }
        live.volumes_applied.insert(transceiver, volume);
        let _ = peer.pc.set_receiver_volume(transceiver, volume).await;
    }

    /// The API server ended a call. Only the end of this call's own session counts: the end of
    /// another session of the channel, as of the dead one a rejoin has already left behind,
    /// changes nothing, so a rejoin is not rejoined again when the API server ends the session
    /// it replaced.
    async fn session_ended(&mut self, session: Uuid, channel: Uuid, reason: SessionEndReason) {
        if self.state.channel != Some(channel)
            || self.state.session != Some(session)
            || self.state.status == CallStatus::Idle
        {
            return;
        }
        match reason {
            SessionEndReason::ServerLost | SessionEndReason::ServerRemoved => {
                self.rejoin(false).await
            }
            SessionEndReason::Idle => self.end(Some(EndedReason::Idle)).await,
            SessionEndReason::Empty => {}
        }
    }

    // --- Joining ----------------------------------------------------------------------------

    fn start_attempt(&mut self, channel: Uuid, rejoin: Option<Rejoin>) {
        self.step += 1;
        self.attempt = Some(Attempt {
            rejoin,
            offer: None,
            candidates: VecDeque::new(),
            current: None,
            last_error: None,
        });
        let (api, internal, generation) =
            (self.api.clone(), self.internal.clone(), self.generation);
        tokio::spawn(async move {
            let result = api.join_offer(channel).await;
            let _ = internal.send(Internal::Offered { generation, result });
        });
    }

    async fn offered(&mut self, result: Result<JoinOffer, ApiError>) {
        if self.attempt.is_none() {
            return;
        }
        let offer = match result {
            Ok(offer) => offer,
            Err(e) => return self.fail_attempt(Failure::Offer(e)).await,
        };
        self.state.grants = Grants {
            speak: offer.speak,
            share_screen: offer.share_screen,
            camera: offer.use_camera,
            transfer_files: offer.transfer_files,
        };
        self.state.server_muted = offer.server_muted;
        // Someone who may not speak joins to listen, and a microphone kept open from a call they
        // could speak in is let go; someone who may must have given one, since its absence is
        // this device's, never a voice server's, so no server is tried or blamed for it.
        if !offer.speak {
            self.microphone = None;
            self.state.microphone = None;
        } else if self.microphone.is_none() {
            return self.fail_attempt(Failure::Microphone).await;
        }
        // A server not reached over HTTPS is never tried, nor reported as failing: it is not at
        // fault.
        let mut reachable = Vec::new();
        let mut refused = None;
        for candidate in &offer.candidates {
            match signalling_url(&candidate.url, self.loopback_plain_text) {
                Ok(_) => reachable.push(candidate.clone()),
                Err(insecure) => {
                    refused.get_or_insert(insecure);
                }
            }
        }
        if reachable.is_empty()
            && let Some(insecure) = refused
        {
            return self.fail_attempt(Failure::Insecure(insecure.0)).await;
        }
        if let Some(attempt) = &mut self.attempt {
            attempt.offer = Some(offer);
        }
        if reachable.len() <= 1 {
            self.ranked(reachable).await;
            return;
        }
        let (api, internal, generation) =
            (self.api.clone(), self.internal.clone(), self.generation);
        tokio::spawn(async move {
            let pings: Vec<_> = reachable
                .iter()
                .map(|candidate| {
                    let (api, url) = (api.clone(), candidate.url.clone());
                    tokio::spawn(async move { api.voice_server_health(&url, PING_TIMEOUT).await })
                })
                .collect();
            let mut ranked: Vec<(Option<Duration>, Candidate)> = Vec::new();
            for (ping, candidate) in pings.into_iter().zip(reachable) {
                ranked.push((ping.await.unwrap_or(None), candidate));
            }
            // Nearest first; those that did not answer last, in their offered order.
            ranked.sort_by_key(|(latency, _)| (latency.is_none(), *latency));
            let _ = internal.send(Internal::Ranked {
                generation,
                candidates: ranked.into_iter().map(|(_, c)| c).collect(),
            });
        });
    }

    async fn ranked(&mut self, candidates: Vec<Candidate>) {
        let Some(attempt) = &mut self.attempt else {
            return;
        };
        attempt.candidates = candidates.into();
        self.next_candidate().await;
    }

    async fn next_candidate(&mut self) {
        loop {
            let Some(attempt) = &mut self.attempt else {
                return;
            };
            let Some(candidate) = attempt.candidates.pop_front() else {
                let failure = attempt
                    .last_error
                    .take()
                    .unwrap_or_else(|| Failure::Server("no voice server could be reached".into()));
                return self.fail_attempt(failure).await;
            };
            let Ok(url) = signalling_url(&candidate.url, self.loopback_plain_text) else {
                continue;
            };
            attempt.current = Some(Trying {
                candidate,
                link: None,
            });
            self.step += 1;
            let step = self.step;
            self.open(url, step, false);
            self.timer(Timer::Ready, READY_TIMEOUT);
            return;
        }
    }

    async fn opened(&mut self, result: Result<Arc<dyn Socket>, TransportError>) {
        let socket = match result {
            Ok(socket) => socket,
            Err(e) => {
                return self
                    .candidate_failed(Failure::Server(format!(
                        "the voice server did not accept the connection: {e}"
                    )))
                    .await;
            }
        };
        let token = self
            .attempt
            .as_ref()
            .and_then(|attempt| attempt.offer.as_ref())
            .map(|offer| offer.token.clone());
        let (Some(token), Some(trying)) = (
            token,
            self.attempt.as_ref().and_then(|a| a.current.as_ref()),
        ) else {
            socket.close();
            return;
        };
        if trying.link.is_some() {
            socket.close();
            return;
        }
        let link = self.link(socket);
        link.send(ClientMessage::Identify { token }).await;
        if let Some(trying) = self.attempt.as_mut().and_then(|a| a.current.as_mut()) {
            trying.link = Some(link);
        }
    }

    /// What the candidate being tried sent before `ready`.
    async fn identifying_frame(&mut self, frame: ServerMessage) {
        match frame {
            ServerMessage::Ready {
                session,
                user,
                participants,
                audio_slots,
                max_watched_video,
                resume,
                ..
            } => {
                self.admitted(
                    session,
                    user,
                    participants,
                    (audio_slots, max_watched_video),
                    resume,
                )
                .await
            }
            // Turned away for joining too fast is a refusal to wait out, not the server failing,
            // and another server would refuse it no differently.
            ServerMessage::Error {
                detail,
                fatal: true,
                retry_after_seconds: Some(seconds),
                ..
            } => {
                self.fail_attempt(Failure::Refused(Refusal {
                    detail,
                    retry_after_seconds: Some(seconds),
                }))
                .await
            }
            ServerMessage::Error {
                detail,
                fatal: true,
                ..
            } => self.candidate_failed(Failure::Server(detail)).await,
            _ => {}
        }
    }

    /// `ready` arrived: the client has its place in the call. Its peer connection is made for
    /// the server to offer, and it asks to send its microphone, which the first offer or the
    /// next carries a section for.
    async fn admitted(
        &mut self,
        session: Uuid,
        user: Uuid,
        participants: Vec<ParticipantInfo>,
        (audio_slots, max_watched_video): (u8, u32),
        secret: String,
    ) {
        let Some(trying) = self.attempt.as_mut().and_then(|a| a.current.take()) else {
            return;
        };
        let Some(link) = trying.link else {
            return;
        };
        self.step += 1;
        self.state.session = Some(session);
        self.state.user = Some(user);
        self.state.participants = participants.into_iter().map(participant).collect();
        self.state.audio_slots = audio_slots;
        self.state.max_watched_video = max_watched_video;
        self.live = Some(Live {
            server: trying.candidate,
            link,
            connection: None,
            secret,
            sections: Vec::new(),
            transceivers: HashMap::new(),
            answered: None,
            microphone_attached: None,
            volumes_applied: HashMap::new(),
            resuming: None,
        });
        let id = self.id();
        let observer = Arc::new(Forwarder {
            connection: id,
            internal: self.internal.clone(),
        });
        match self
            .platform
            .peer_connections
            .create(Configuration::default(), observer)
            .await
        {
            Ok(pc) => {
                if let Some(live) = &mut self.live {
                    live.connection = Some(Peer { id, pc });
                }
            }
            Err(e) => {
                return self
                    .fail_attempt(Failure::Device(format!(
                        "the peer connection could not be made: {e}"
                    )))
                    .await;
            }
        }
        if self.state.grants.speak
            && let Some(microphone) = self.microphone.clone()
        {
            self.request_source(MediaSource::Microphone, microphone)
                .await;
        }
        self.timer(Timer::Connect, CONNECT_TIMEOUT);
    }

    /// The candidate being tried failed in a way that is its own: it is reported to the API
    /// server, whatever it set up is released (the microphone kept for the next), and the next
    /// candidate tried.
    async fn candidate_failed(&mut self, failure: Failure) {
        let Some(attempt) = &mut self.attempt else {
            return;
        };
        attempt.last_error = Some(failure);
        let mut server = None;
        if let Some(trying) = attempt.current.take() {
            if let Some(link) = trying.link {
                link.close();
            }
            server = Some(trying.candidate.id);
        }
        if server.is_none() {
            server = self.live.as_ref().map(|live| live.server.id);
        }
        self.release_live().await;
        if let Some(server) = server {
            let api = self.api.clone();
            tokio::spawn(async move {
                let _ = api.report_failure(server).await;
            });
        }
        self.next_candidate().await;
    }

    /// The attempt is over without a call. A rejoin after the server went silent, whose offer
    /// may have named the dead server still, asks for one more offer after `DEAD_SERVER_RETRY`
    /// when every server offered failed; everything else fails the call now.
    async fn fail_attempt(&mut self, failure: Failure) {
        let plan = self.attempt.as_ref().and_then(|attempt| attempt.rejoin);
        if let Some(plan) = plan
            && plan.server_silent
            && !plan.retried
            && failure.is_servers()
        {
            self.teardown(true).await;
            self.state.status = CallStatus::Rejoining;
            self.pending_rejoin = Some(Rejoin {
                retried: true,
                ..plan
            });
            self.timer(Timer::DeadServerRetry, DEAD_SERVER_RETRY);
            return;
        }
        self.teardown(false).await;
        self.state.status = CallStatus::Failed;
        self.state.error_kind = Some(failure.kind());
        self.state.error = Some(failure.detail());
        self.state.retry_after_seconds = failure.retry_after_seconds();
    }

    /// The peer connection connected while joining: the call is on.
    async fn finish_join(&mut self) {
        self.step += 1;
        self.attempt = None;
        self.pending_rejoin = None;
        self.state.status = CallStatus::Connected;
        self.state.error = None;
        self.state.error_kind = None;
        self.state.retry_after_seconds = None;
        self.send_state().await;
        if !self.state.watching.is_empty() {
            let video = self.state.watching.clone();
            self.set_watched(video).await;
        }
    }

    /// Joins the channel again after a random pause of at most `REJOIN_DELAY_MAX`. With
    /// `server_silent`, the server went away without a word, so the API server may still offer
    /// it, not yet knowing it is gone: a rejoin that fails to reach the servers offered then
    /// waits `DEAD_SERVER_RETRY` and asks for an offer once more before giving up.
    async fn rejoin(&mut self, server_silent: bool) {
        let Some(channel) = self.state.channel else {
            return;
        };
        if self.pending_rejoin.is_some() || self.attempt.is_some() {
            return;
        }
        self.teardown(true).await;
        self.generation += 1;
        self.state.status = CallStatus::Rejoining;
        self.pending_rejoin = Some(Rejoin {
            channel,
            server_silent,
            retried: false,
        });
        let pause = self.pause();
        self.timer(Timer::RejoinDelay, pause);
    }

    /// Releases the call's place on its server and everything of it: `leave` is sent, the
    /// sockets closed, the peer connection closed, and what the client sent and received
    /// forgotten, so the platform releases those tracks. The microphone is `teardown`'s.
    async fn release_live(&mut self) {
        let Some(live) = self.live.take() else {
            return;
        };
        live.link.send(ClientMessage::Leave).await;
        live.link.close();
        if let Some(resuming) = &live.resuming
            && let Some(link) = &resuming.link
        {
            link.close();
        }
        if let Some(peer) = &live.connection {
            peer.pc.close();
        }
        self.state.session = None;
        self.state.user = None;
        self.state.participants.clear();
        self.state.sources.clear();
        self.state.audio_slots = 0;
        self.state.max_watched_video = 0;
        self.state.slots.clear();
        self.state.videos.clear();
        self.state.share_audio.clear();
    }

    /// Releases everything the call holds; every timer and spawned step of it is let go.
    /// `keep_microphone` leaves the microphone held for the next attempt or call.
    async fn teardown(&mut self, keep_microphone: bool) {
        self.step += 1;
        self.pending_rejoin = None;
        if let Some(trying) = self.attempt.take().and_then(|attempt| attempt.current)
            && let Some(link) = trying.link
        {
            link.close();
        }
        self.release_live().await;
        if !keep_microphone {
            self.microphone = None;
            self.state.microphone = None;
        }
    }

    // --- Events -----------------------------------------------------------------------------

    async fn handle_internal(&mut self, event: Internal) {
        match event {
            Internal::Offered { generation, result } => {
                if generation == self.generation {
                    self.offered(result).await;
                }
            }
            Internal::Ranked {
                generation,
                candidates,
            } => {
                if generation == self.generation {
                    self.ranked(candidates).await;
                }
            }
            Internal::Opened {
                generation,
                step,
                result,
            } => {
                if generation == self.generation && step == self.step {
                    self.opened(result).await;
                } else if let Ok(socket) = result {
                    socket.close();
                }
            }
            Internal::ResumeOpened {
                generation,
                step,
                result,
            } => {
                if generation == self.generation && step == self.step {
                    self.resume_opened(result).await;
                } else if let Ok(socket) = result {
                    socket.close();
                }
            }
            Internal::Frame { socket, frame } => self.frame(socket, frame).await,
            Internal::SocketClosed { socket } => self.socket_closed(socket).await,
            Internal::ConnectionState { connection, state } => {
                self.connection_state(connection, state).await
            }
            Internal::Timeout {
                generation,
                step,
                kind,
            } => {
                if generation == self.generation && step == self.step {
                    self.timeout(kind).await;
                }
            }
        }
    }

    fn identifying_link(&self) -> Option<u64> {
        self.attempt
            .as_ref()?
            .current
            .as_ref()?
            .link
            .as_ref()
            .map(|link| link.id)
    }

    fn resuming_link(&self) -> Option<u64> {
        self.live
            .as_ref()?
            .resuming
            .as_ref()?
            .link
            .as_ref()
            .map(|link| link.id)
    }

    async fn frame(&mut self, socket: u64, frame: ServerMessage) {
        if self.identifying_link() == Some(socket) {
            self.identifying_frame(frame).await;
        } else if self
            .live
            .as_ref()
            .is_some_and(|live| live.link.id == socket)
        {
            self.live_frame(frame).await;
        } else if self.resuming_link() == Some(socket) {
            self.resume_frame(frame).await;
        }
    }

    async fn socket_closed(&mut self, socket: u64) {
        let closed = Failure::Server("the voice server closed the connection".to_string());
        if self.identifying_link() == Some(socket) {
            self.candidate_failed(closed).await;
        } else if self
            .live
            .as_ref()
            .is_some_and(|live| live.link.id == socket)
        {
            if self.attempt.is_some() {
                self.candidate_failed(closed).await;
            } else if self
                .live
                .as_ref()
                .is_some_and(|live| live.resuming.is_none())
            {
                self.start_resume().await;
            }
        } else if self.resuming_link() == Some(socket) {
            self.resume_step_failed().await;
        }
    }

    async fn connection_state(&mut self, connection: u64, state: ConnectionState) {
        let Some(live) = &mut self.live else {
            return;
        };
        if live.connection.as_ref().map(|peer| peer.id) != Some(connection) {
            return;
        }
        let joining = self.attempt.is_some();
        let resuming = live.resuming.is_some();
        let link = live.link.clone();
        match state {
            ConnectionState::Connected => {
                if joining {
                    self.finish_join().await;
                }
            }
            ConnectionState::Failed => {
                if joining {
                    self.candidate_failed(Failure::Server(
                        "media could not reach the voice server".to_string(),
                    ))
                    .await;
                } else {
                    // Media lost mid-call: the call is joined again rather than left silent.
                    self.rejoin(false).await;
                }
            }
            // A phone moving between networks: the server offers an ICE restart when it asks,
            // and the next offer carries new credentials. Media that then fails rejoins.
            ConnectionState::Disconnected => {
                if !joining && !resuming {
                    link.send(ClientMessage::RestartIce {
                        connection: ConnectionKind::Call,
                    })
                    .await;
                }
            }
            ConnectionState::New | ConnectionState::Connecting | ConnectionState::Closed => {}
        }
    }

    async fn timeout(&mut self, kind: Timer) {
        match kind {
            Timer::Ready => {
                if self.attempt.is_some() {
                    self.candidate_failed(Failure::Server(
                        "the voice server did not answer in time".to_string(),
                    ))
                    .await;
                } else if self.live.as_ref().is_some_and(|l| l.resuming.is_some()) {
                    self.resume_step_failed().await;
                }
            }
            // Only while joining: the join's end moves the step on, so this never fires after.
            Timer::Connect => {
                if self.attempt.is_some() {
                    self.candidate_failed(Failure::Server(
                        "media could not reach the voice server in time".to_string(),
                    ))
                    .await;
                }
            }
            Timer::RejoinDelay | Timer::DeadServerRetry => {
                if let Some(plan) = self.pending_rejoin.take() {
                    self.start_attempt(plan.channel, Some(plan));
                }
            }
            Timer::ResumeRetry => self.attempt_resume(),
        }
    }

    // --- In the call ------------------------------------------------------------------------

    async fn live_frame(&mut self, frame: ServerMessage) {
        match frame {
            ServerMessage::Offer {
                connection: ConnectionKind::Call,
                revision,
                sdp,
                sections,
            } => self.apply_offer(revision, sdp, sections).await,
            ServerMessage::Sections {
                connection: ConnectionKind::Call,
                sections,
            } => {
                let Some(peer) = self.live.as_ref().and_then(|live| live.connection.clone()) else {
                    return;
                };
                self.sections_applied(sections, &peer).await;
            }
            // The game capture helper's connection is the desktop shell's.
            ServerMessage::Offer {
                connection: ConnectionKind::Capture,
                ..
            }
            | ServerMessage::Sections {
                connection: ConnectionKind::Capture,
                ..
            } => {}
            ServerMessage::Sending { source } => {
                if let Some(sending) = self.state.sources.iter_mut().find(|s| s.source == source) {
                    sending.live = true;
                }
            }
            ServerMessage::Sources { user, sources } => {
                if let Some(participant) = self.participant(user) {
                    participant.sources = sources;
                }
            }
            ServerMessage::Slots { assignments } => self.assign_slots(assignments).await,
            ServerMessage::ParticipantJoined {
                user,
                muted,
                deafened,
            } => {
                // A user joining again from another socket replaces their participant, whose
                // sources the server tells as gone.
                self.state.participants.retain(|p| p.user != user);
                self.state.participants.push(Participant {
                    user,
                    muted,
                    deafened,
                    speaking: false,
                    sources: Vec::new(),
                });
            }
            ServerMessage::ParticipantLeft { user } => {
                self.state.participants.retain(|p| p.user != user);
            }
            ServerMessage::ParticipantState {
                user,
                muted,
                deafened,
            } => {
                if self.state.user == Some(user) {
                    // The server's word on this client's own state, a moderator's mute
                    // included. One answering an earlier request never shows the user less
                    // silenced than they have since asked to be.
                    self.state.muted = muted || self.asked.muted;
                    self.state.deafened = deafened || self.asked.deafened;
                    self.hold_microphone().await;
                } else if let Some(participant) = self.participant(user) {
                    participant.muted = muted;
                    participant.deafened = deafened;
                }
            }
            ServerMessage::Speaking { user, speaking } => {
                if let Some(participant) = self.participant(user) {
                    participant.speaking = speaking;
                }
            }
            ServerMessage::Kicked { reason } => self.kicked(reason).await,
            ServerMessage::GrantsChanged { grants } => self.grants_changed(grants).await,
            ServerMessage::Error {
                detail,
                fatal: true,
                ..
            } => {
                self.generation += 1;
                let channel = self.state.channel;
                self.teardown(false).await;
                self.reset_to_idle();
                self.state.channel = channel;
                self.state.status = CallStatus::Failed;
                self.state.error_kind = Some(ErrorKind::Server);
                self.state.error = Some(detail);
            }
            ServerMessage::Error {
                detail,
                fatal: false,
                retry_after_seconds,
                refused,
            } => {
                self.refused(refused.as_deref(), detail, retry_after_seconds)
                    .await
            }
            // Files are the transfers piece's; a `ready` on an open socket answers nothing.
            ServerMessage::Ready { .. }
            | ServerMessage::FileOffered { .. }
            | ServerMessage::FileWithdrawn { .. }
            | ServerMessage::TransferStarting { .. }
            | ServerMessage::TransferSignal { .. }
            | ServerMessage::TransferEnded { .. }
            | ServerMessage::TransferLinkChanged { .. } => {}
        }
    }

    fn participant(&mut self, user: Uuid) -> Option<&mut Participant> {
        self.state.participants.iter_mut().find(|p| p.user == user)
    }

    /// A request the server would not honour, named by the type of the frame that asked it.
    async fn refused(&mut self, request: Option<&str>, detail: String, retry_after: Option<u64>) {
        let refusal = Refusal {
            detail,
            retry_after_seconds: retry_after,
        };
        match request {
            // Only unmuting or undeafening is ever refused: the user stays as they are shown.
            Some("setState") => {
                self.asked = Asked {
                    muted: self.state.muted,
                    deafened: self.state.deafened,
                };
                self.state.state_refused = Some(refusal);
            }
            // The source last asked for, whose section the server will not offer.
            Some("send") => {
                if let Some(at) = self.state.sources.iter().rposition(|s| !s.live) {
                    let source = self.state.sources.remove(at).source;
                    self.state.source_refused = Some((source, refusal));
                }
            }
            _ => {}
        }
    }

    async fn kicked(&mut self, reason: KickReason) {
        match reason {
            // The server is going down, or the call goes on elsewhere: the join offer names
            // the server to go to. A connection that failed at the server (an offer unanswered,
            // media that never came) is rejoined the same way.
            KickReason::ServerStopping | KickReason::ConnectionFailed => self.rejoin(false).await,
            KickReason::Kicked => self.end(Some(EndedReason::Kicked)).await,
            KickReason::AccessLost => self.end(Some(EndedReason::AccessLost)).await,
            // Another of the user's own clients took the call over, or the sign-in it joined
            // with ended, which signs this client out too: neither needs a notice.
            KickReason::Replaced | KickReason::SignedOut => self.end(None).await,
        }
    }

    /// What the client may now do in the call, once the permissions behind it change: what it
    /// may no longer send stops (the server has already stopped it), and a microphone it may now
    /// send starts.
    async fn grants_changed(&mut self, grants: Grants) {
        self.state.grants = grants;
        if !grants.share_screen {
            self.stop_source(MediaSource::Screen).await;
            self.stop_source(MediaSource::ScreenAudio).await;
        }
        if !grants.camera {
            self.stop_source(MediaSource::Camera).await;
        }
        let sending_microphone = self
            .state
            .sources
            .iter()
            .any(|s| s.source == MediaSource::Microphone);
        if !grants.speak {
            if sending_microphone {
                self.stop_source(MediaSource::Microphone).await;
            }
            self.microphone = None;
            self.state.microphone = None;
        } else if !sending_microphone
            && self.state.status == CallStatus::Connected
            && let Some(microphone) = self.microphone.clone()
        {
            self.request_source(MediaSource::Microphone, microphone)
                .await;
            self.send_state().await;
        }
    }

    /// Answers the server's offer, as the web client's `VoiceCall` does: each section the client
    /// sends is set `sendonly` with its track before the answer is made, so the answer carries
    /// them; the answer is applied; the encodings of a screen or camera sent are bounded, which
    /// the applied answer makes the sender's; the answer is sent; and only then are the sections
    /// taken as the connection's, so what the state shows is what media flows on. Offers arrive
    /// one at a time, since the server waits for each answer before its next.
    async fn apply_offer(&mut self, revision: u64, sdp: String, sections: Vec<Section>) {
        let Some(live) = &self.live else {
            return;
        };
        if let Some((answered, answer)) = &live.answered
            && *answered == revision
        {
            let (link, answer) = (live.link.clone(), answer.clone());
            link.send(ClientMessage::Answer {
                connection: ConnectionKind::Call,
                revision,
                sdp: answer,
            })
            .await;
            return;
        }
        let Some(peer) = live.connection.clone() else {
            return;
        };
        let outgoing: HashMap<MediaSource, Option<TrackId>> = self
            .state
            .sources
            .iter()
            .map(|s| (s.source, Some(s.track.clone())))
            .chain([(MediaSource::Microphone, self.outgoing_microphone())])
            .collect();
        let result: Result<String, PeerConnectionError> = async {
            let mut encodings = Vec::new();
            peer.pc
                .set_remote_description(SessionDescription {
                    kind: DescriptionKind::Offer,
                    sdp,
                })
                .await?;
            for section in &sections {
                let Some(transceiver) = peer.pc.transceiver_by_mid(&section.mid) else {
                    return Err(PeerConnectionError(format!(
                        "the offer's section {} has no transceiver",
                        section.mid
                    )));
                };
                if let SectionContent::Sending { source } = &section.content {
                    // The server receives on a section it offers for what this client sends.
                    peer.pc
                        .set_direction(transceiver, Direction::SendOnly)
                        .await?;
                    let track = outgoing.get(source).cloned().flatten();
                    peer.pc.replace_track(transceiver, track).await?;
                    if let Some(encoding) = encoding_of(*source) {
                        encodings.push((transceiver, encoding));
                    }
                }
            }
            let answer = peer.pc.create_answer().await?;
            peer.pc
                .set_local_description(SessionDescription {
                    kind: DescriptionKind::Answer,
                    sdp: answer.clone(),
                })
                .await?;
            for (transceiver, encoding) in encodings {
                encode(&peer, transceiver, encoding).await?;
            }
            Ok(answer)
        }
        .await;
        match result {
            Ok(answer) => {
                let Some(live) = &mut self.live else {
                    return;
                };
                live.answered = Some((revision, answer.clone()));
                live.microphone_attached =
                    outgoing.get(&MediaSource::Microphone).cloned().flatten();
                let link = live.link.clone();
                link.send(ClientMessage::Answer {
                    connection: ConnectionKind::Call,
                    revision,
                    sdp: answer,
                })
                .await;
                self.sections_applied(sections, &peer).await;
            }
            Err(e) => {
                let detail = format!("the offer could not be applied: {e}");
                if self.attempt.is_some() {
                    self.fail_attempt(Failure::Device(detail)).await;
                } else {
                    self.rejoin(false).await;
                }
            }
        }
    }

    /// Takes `sections` as what the connection carries: the tracks of the slots and videos
    /// received are read, and a source whose section the server took away (its grant gone) is
    /// forgotten.
    async fn sections_applied(&mut self, sections: Vec<Section>, peer: &Peer) {
        let Some(live) = &mut self.live else {
            return;
        };
        let had: Vec<MediaSource> = live
            .sections
            .iter()
            .filter_map(|s| match s.content {
                SectionContent::Sending { source } => Some(source),
                _ => None,
            })
            .collect();
        live.transceivers = sections
            .iter()
            .filter_map(|s| Some((s.mid.clone(), peer.pc.transceiver_by_mid(&s.mid)?)))
            .collect();
        live.sections = sections.clone();
        let mut videos = Vec::new();
        let mut share_audio = Vec::new();
        let mut volumes = Vec::new();
        for section in &sections {
            let Some(&transceiver) = live.transceivers.get(&section.mid) else {
                continue;
            };
            match &section.content {
                SectionContent::Sending { .. } => {}
                SectionContent::AudioSlot { slot } => {
                    let at = usize::from(*slot);
                    while self.state.slots.len() <= at {
                        let slot = self.state.slots.len() as u8;
                        self.state.slots.push(AudioSlot {
                            slot,
                            user: None,
                            track: None,
                        });
                    }
                    self.state.slots[at].track = peer.pc.receiver_track(transceiver);
                    if let Some(user) = self.state.slots[at].user {
                        volumes.push((section.mid.clone(), user, MediaSource::Microphone));
                    }
                }
                SectionContent::Video { user, source } => {
                    if let Some(track) = peer.pc.receiver_track(transceiver) {
                        videos.push(RemoteVideo {
                            user: *user,
                            source: *source,
                            track,
                        });
                    }
                }
                SectionContent::ShareAudio { user } => {
                    if let Some(track) = peer.pc.receiver_track(transceiver) {
                        share_audio.push(ShareAudio { user: *user, track });
                    }
                    volumes.push((section.mid.clone(), *user, MediaSource::ScreenAudio));
                }
            }
        }
        self.state.videos = videos;
        self.state.share_audio = share_audio;
        for source in had {
            let still = sections
                .iter()
                .any(|s| s.content == SectionContent::Sending { source });
            if !still {
                self.state.sources.retain(|s| s.source != source);
            }
        }
        for (mid, user, source) in volumes {
            self.apply_volume(&mid, user, source).await;
        }
    }

    /// Whom each slot carries from now on. A slot the offers have not yet given a section is
    /// told of first, so its entry exists, with no track, until the offer adds its section.
    async fn assign_slots(&mut self, assignments: Vec<SlotAssignment>) {
        for assignment in &assignments {
            let at = usize::from(assignment.slot);
            while self.state.slots.len() <= at {
                let slot = self.state.slots.len() as u8;
                self.state.slots.push(AudioSlot {
                    slot,
                    user: None,
                    track: None,
                });
            }
            self.state.slots[at].user = assignment.user;
        }
        for assignment in assignments {
            if let (Some(user), Some(mid)) = (assignment.user, self.slot_mid(assignment.slot)) {
                self.apply_volume(&mid, user, MediaSource::Microphone).await;
            }
        }
    }

    // --- Resuming ---------------------------------------------------------------------------

    /// The socket dropped without `leave`: the participant stays in the call for `RESUME_GRACE`,
    /// media flowing, and is taken back with `resume` on a new socket to the same server.
    async fn start_resume(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        live.resuming = Some(Resuming {
            deadline: Instant::now() + RESUME_GRACE,
            link: None,
        });
        self.state.status = CallStatus::Rejoining;
        self.attempt_resume();
    }

    fn attempt_resume(&mut self) {
        let Some(live) = &self.live else {
            return;
        };
        let Ok(url) = signalling_url(&live.server.url, self.loopback_plain_text) else {
            return;
        };
        self.step += 1;
        self.open(url, self.step, true);
        self.timer(Timer::Ready, READY_TIMEOUT);
    }

    async fn resume_opened(&mut self, result: Result<Arc<dyn Socket>, TransportError>) {
        let socket = match result {
            Ok(socket) => socket,
            Err(_) => return self.resume_step_failed().await,
        };
        let Some(secret) = self
            .live
            .as_ref()
            .filter(|live| live.resuming.is_some())
            .map(|live| live.secret.clone())
        else {
            socket.close();
            return;
        };
        let link = self.link(socket);
        link.send(ClientMessage::Resume { secret }).await;
        if let Some(resuming) = self.live.as_mut().and_then(|live| live.resuming.as_mut()) {
            resuming.link = Some(link);
        }
    }

    async fn resume_frame(&mut self, frame: ServerMessage) {
        match frame {
            ServerMessage::Ready {
                session,
                user,
                participants,
                audio_slots,
                max_watched_video,
                resume,
                ..
            } => {
                let Some(live) = &mut self.live else {
                    return;
                };
                let Some(link) = live.resuming.take().and_then(|r| r.link) else {
                    return;
                };
                self.step += 1;
                let old = std::mem::replace(&mut live.link, link);
                old.close();
                live.secret = resume;
                self.state.session = Some(session);
                self.state.user = Some(user);
                self.state.participants = participants.into_iter().map(participant).collect();
                self.state.audio_slots = audio_slots;
                self.state.max_watched_video = max_watched_video;
                self.state.status = CallStatus::Connected;
            }
            // The server would not take the participant back: the grace ran out, or the secret
            // was used. It is alive, so the channel is joined again at once.
            ServerMessage::Error { fatal: true, .. } => {
                if let Some(live) = &mut self.live
                    && let Some(resuming) = live.resuming.take()
                    && let Some(link) = resuming.link
                {
                    link.close();
                }
                self.rejoin(false).await;
            }
            _ => {}
        }
    }

    /// A resume did not reach the server. It is tried again after a random pause while the next
    /// try can still finish within the grace; past that the channel is joined again, as after a
    /// server that went silent.
    async fn resume_step_failed(&mut self) {
        let Some(live) = &mut self.live else {
            return;
        };
        let Some(resuming) = &mut live.resuming else {
            return;
        };
        if let Some(link) = resuming.link.take() {
            link.close();
        }
        let deadline = resuming.deadline;
        self.step += 1;
        let pause = self.pause();
        if Instant::now() + pause + READY_TIMEOUT <= deadline {
            self.timer(Timer::ResumeRetry, pause);
        } else {
            self.rejoin(true).await;
        }
    }
}

/// How a sent video is bounded: a shared screen as one layer allowed up to 25 Mbps at 60 frames
/// a second, a ceiling the connection's bandwidth estimate brings down, and a camera up to 4 Mbps
/// at 30, which a 1080p camera fills well. Sound is left to the encoder.
fn encoding_of(source: MediaSource) -> Option<(u32, f64)> {
    match source {
        MediaSource::Screen => Some((25_000_000, 60.0)),
        MediaSource::Camera => Some((4_000_000, 30.0)),
        MediaSource::Microphone | MediaSource::ScreenAudio => None,
    }
}

/// Bounds what the sender's encoder may spend, as `encoding` says, when it is not so already.
/// A sender with no encoding yet is left alone: the applied answer gives it one.
async fn encode(
    peer: &Peer,
    transceiver: TransceiverId,
    (max_bitrate, max_framerate): (u32, f64),
) -> Result<(), PeerConnectionError> {
    let mut parameters = peer.pc.sender_parameters(transceiver).await?;
    let Some(first) = parameters.encodings.first_mut() else {
        return Ok(());
    };
    if first.max_bitrate == Some(max_bitrate) && first.max_framerate == Some(max_framerate) {
        return Ok(());
    }
    first.max_bitrate = Some(max_bitrate);
    first.max_framerate = Some(max_framerate);
    peer.pc.set_sender_parameters(transceiver, parameters).await
}

fn participant(info: ParticipantInfo) -> Participant {
    Participant {
        user: info.user,
        muted: info.muted,
        deafened: info.deafened,
        speaking: info.speaking,
        sources: info.sources,
    }
}
