//! One media thread: its sockets, the connections of the calls on it, and forwarding between
//! them.
//!
//! The thread waits in `poll` on its sockets, a waker the rooms write to with each command, and
//! the earliest timeout any connection wants. A datagram goes to the connection its source
//! address was learned for, or, the first time, to the one whose ICE username fragment its STUN
//! request names.
//!
//! Forwarding is packet for packet. Each section a participant receives is an `Out` with its own
//! SSRC; its sequence numbers and RTP timestamps run on unbroken when the source it carries
//! changes (an audio slot moving to another speaker, a video resumed), rebased at the new
//! source's first packet. A video `Out` starts on a keyframe, asking its source for one.

use crate::connection::Connection;
use crate::keyframe::is_keyframe;
use crate::levels::{INTERVAL, Levels};
use crate::net::{self, Listener, Stream};
use crate::{CallId, Command, Event, Events, Failure, MediaConfig, ParticipantId};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::io;
use std::net::SocketAddr;
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixDatagram;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};
use str0m::media::{KeyframeRequestKind, Mid};
use str0m::net::{Protocol, Receive, Transmit};
use str0m::rtp::{ExtensionValues, RtpPacket, RtpWrite, SeqNo};
use str0m::{Event as RtcEvent, IceConnectionState, Input};
use uuid::Uuid;
use voice_protocol::signal::{
    ConnectionKind, MediaKind, MediaSource, SectionContent, SlotAssignment,
};

/// How long an offer may go unanswered before its connection counts as failed.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a connection has to complete ICE and DTLS, so a client cannot hold one it never
/// uses: the worst networks a call works over take a few seconds.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// The least time between keyframe requests to one source, however many of its watchers ask.
const KEYFRAME_REQUEST_INTERVAL: Duration = Duration::from_millis(300);
/// The longest the thread sleeps with nothing to do, so a missed wake costs little.
const MAX_SLEEP: Duration = Duration::from_millis(100);
/// Sources in the order a connection's sections are offered.
const SOURCES: [MediaSource; 4] = [
    MediaSource::Microphone,
    MediaSource::Camera,
    MediaSource::Screen,
    MediaSource::ScreenAudio,
];

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Census {
    pub calls: usize,
    pub participants: usize,
    pub connections: usize,
    pub sections: usize,
    /// Bytes the thread has allocated and not freed, by jemalloc's count of its own
    /// allocations: what its connections hold, whatever else the process allocates. `None`
    /// without the `jemalloc` feature.
    pub allocated_bytes: Option<u64>,
}

/// Raised when any media thread ends, which the server treats as fatal.
#[derive(Default)]
pub struct Stopped {
    raised: AtomicBool,
    notify: tokio::sync::Notify,
}

impl Stopped {
    fn raise(&self) {
        self.raised.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub async fn wait(&self) {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.raised.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }
}

/// Raises `Stopped` when the thread's body ends, by returning or by panicking.
struct RaiseOnDrop(Arc<Stopped>);

impl Drop for RaiseOnDrop {
    fn drop(&mut self) {
        self.0.raise();
    }
}

pub struct Handle {
    commands: mpsc::Sender<Command>,
    waker: UnixDatagram,
}

impl Handle {
    pub fn start(config: &MediaConfig, index: usize, stopped: Arc<Stopped>) -> io::Result<Self> {
        // Port 0 gives every thread a free port of its own, as tests want.
        let port = if config.first_port == 0 {
            0
        } else {
            u16::try_from(index)
                .ok()
                .and_then(|i| config.first_port.checked_add(i))
                .ok_or_else(|| io::Error::other("the media threads' ports run past 65535"))?
        };
        let listeners = config
            .addresses
            .iter()
            .map(|address| Listener::bind(address.ip, address.announced, port))
            .collect::<io::Result<Vec<_>>>()?;
        let (waker, woken) = UnixDatagram::pair()?;
        waker.set_nonblocking(true)?;
        woken.set_nonblocking(true)?;
        let (commands, received) = mpsc::channel();
        let thread = Thread::new(config.clone(), listeners, woken, received);
        std::thread::Builder::new()
            .name(format!("aspen-media-{index}"))
            .spawn(move || {
                let _raise = RaiseOnDrop(stopped);
                thread.run();
            })?;
        Ok(Self { commands, waker })
    }

    pub fn send(&self, command: Command) -> bool {
        if self.commands.send(command).is_err() {
            return false;
        }
        // A full buffer means a wake is already waiting.
        let _ = self.waker.send(&[0]);
        true
    }
}

/// What this thread has allocated less what it has freed, by jemalloc's counters for it.
#[cfg(feature = "jemalloc")]
fn thread_allocated_bytes() -> Option<u64> {
    let allocated = tikv_jemalloc_ctl::thread::allocatedp::read().ok()?.get();
    let freed = tikv_jemalloc_ctl::thread::deallocatedp::read().ok()?.get();
    Some(allocated.saturating_sub(freed))
}

#[cfg(not(feature = "jemalloc"))]
fn thread_allocated_bytes() -> Option<u64> {
    None
}

type SourceKey = (ParticipantId, MediaSource);
/// The sections a connection should hold, by kind and what each carries.
type Wanted = Vec<(MediaKind, SectionContent)>;

/// A section a receiver is sent, and where its numbering stands.
struct Out {
    content: SectionContent,
    kind: MediaKind,
    /// The source it carries now; `None` until its first packet, and again whenever it must
    /// start over (a new source, a resume).
    source: Option<SourceKey>,
    /// A packet's sequence number less this is its number here.
    base: u64,
    last_seq: Option<u64>,
    ts_offset: u32,
    last_ts: u32,
    last_at: Option<Instant>,
    /// Whether the client wants it (`setReceiving`).
    receiving: bool,
    /// The offer that added it; nothing is sent on it before that offer's answer.
    revision: u64,
}

struct Sending {
    external: bool,
    paused: bool,
    live: bool,
    ssrc: Option<u32>,
    levels: Levels,
    last_keyframe_request: Option<Instant>,
}

struct Participant {
    call: CallId,
    user: Uuid,
    events: Events,
    conn: usize,
    capture: Option<usize>,
    sending: HashMap<MediaSource, Sending>,
    watch: Vec<(Uuid, MediaSource)>,
    outs: HashMap<Mid, Out>,
    /// The participant each audio slot carries.
    slots: Vec<Option<ParticipantId>>,
    deafened: bool,
    speaking: bool,
    /// The sources whose routes name this participant as a receiver.
    routed: Vec<SourceKey>,
}

impl Participant {
    fn tell(&self, event: Event) {
        let _ = self.events.send(event);
    }

    fn conn_of(&self, kind: ConnectionKind) -> Option<usize> {
        match kind {
            ConnectionKind::Call => Some(self.conn),
            ConnectionKind::Capture => self.capture,
        }
    }
}

#[derive(Default)]
struct Call {
    /// In the order they joined.
    participants: Vec<ParticipantId>,
    by_user: HashMap<Uuid, ParticipantId>,
}

#[derive(Clone, Copy)]
struct Route {
    receiver: ParticipantId,
    mid: Mid,
}

/// Waits until one of `fds` is ready or `timeout` passes. Connections' timeouts are often less
/// than a millisecond away: on Linux `ppoll` waits to the nanosecond, and elsewhere `poll` waits
/// the whole millisecond rather than none, either way never spinning until a timeout is due.
fn sleep_on(fds: &mut [libc::pollfd], timeout: Duration) {
    #[cfg(target_os = "linux")]
    {
        let timeout = libc::timespec {
            tv_sec: timeout.as_secs() as libc::time_t,
            tv_nsec: libc::c_long::from(timeout.subsec_nanos()),
        };
        // SAFETY: `fds` is a valid array of `fds.len()` pollfds, and `timeout` a timespec,
        // both outliving the call.
        unsafe {
            libc::ppoll(
                fds.as_mut_ptr(),
                fds.len() as libc::nfds_t,
                &timeout,
                std::ptr::null(),
            )
        };
    }
    #[cfg(not(target_os = "linux"))]
    {
        let millis = timeout.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32;
        // SAFETY: `fds` is a valid array of `fds.len()` pollfds that outlives the call.
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, millis) };
    }
}

struct Thread {
    config: MediaConfig,
    listeners: Vec<Listener>,
    woken: UnixDatagram,
    commands: mpsc::Receiver<Command>,
    streams: Vec<Option<Stream>>,
    tcp_by_peer: HashMap<SocketAddr, usize>,
    conns: Vec<Option<Connection>>,
    /// The timeout each connection last put on the heap, so an unchanged one is not pushed again.
    queued: Vec<Option<Instant>>,
    reported: Vec<bool>,
    by_addr: HashMap<(Protocol, SocketAddr, SocketAddr), usize>,
    by_ufrag: HashMap<String, usize>,
    timers: BinaryHeap<Reverse<(Instant, usize)>>,
    participants: HashMap<ParticipantId, Participant>,
    calls: HashMap<CallId, Call>,
    routes: HashMap<SourceKey, Vec<Route>>,
    next_tick: Instant,
    /// Connections written to since they were last polled, each once (`marked`).
    dirty: Vec<usize>,
    marked: Vec<bool>,
}

impl Thread {
    fn new(
        config: MediaConfig,
        listeners: Vec<Listener>,
        woken: UnixDatagram,
        commands: mpsc::Receiver<Command>,
    ) -> Self {
        Self {
            config,
            listeners,
            woken,
            commands,
            streams: Vec::new(),
            tcp_by_peer: HashMap::new(),
            conns: Vec::new(),
            queued: Vec::new(),
            reported: Vec::new(),
            by_addr: HashMap::new(),
            by_ufrag: HashMap::new(),
            timers: BinaryHeap::new(),
            participants: HashMap::new(),
            calls: HashMap::new(),
            routes: HashMap::new(),
            next_tick: Instant::now() + INTERVAL,
            dirty: Vec::new(),
            marked: Vec::new(),
        }
    }

    fn run(mut self) {
        let mut buf = vec![0u8; 2048];
        loop {
            let mut wake = [0u8; 64];
            while self.woken.recv(&mut wake).is_ok() {}
            loop {
                match self.commands.try_recv() {
                    Ok(command) => self.command(command),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            }
            self.receive_udp(&mut buf);
            self.accept_tcp();
            self.receive_tcp();
            let now = Instant::now();
            self.expire(now);
            if now >= self.next_tick {
                self.tick(now);
                self.next_tick = (self.next_tick + INTERVAL).max(now);
            }
            self.flush_dirty();
            self.wait();
        }
    }

    // --- Commands ---------------------------------------------------------------------------

    fn command(&mut self, command: Command) {
        match command {
            Command::Join {
                call,
                participant,
                user,
                events,
            } => self.join(call, participant, user, events),
            Command::Leave { participant } => self.leave(participant),
            Command::Send {
                participant,
                source,
                external,
            } => self.send(participant, source, external),
            Command::StopSending {
                participant,
                source,
            } => self.stop_sending(participant, source),
            Command::Answer {
                participant,
                connection,
                revision,
                sdp,
            } => self.answer(participant, connection, revision, &sdp),
            Command::Watch { participant, video } => {
                let Some(p) = self.participants.get_mut(&participant) else {
                    return;
                };
                p.watch = video;
                p.watch.truncate(self.config.max_watched_video);
                self.settle(participant);
            }
            Command::SetReceiving {
                participant,
                mid,
                receiving,
            } => {
                let Some(p) = self.participants.get_mut(&participant) else {
                    return;
                };
                if let Some(out) = p.outs.get_mut(&Mid::from(mid.as_str())) {
                    if receiving && !out.receiving {
                        // Resumed video waits for a keyframe; resumed audio rebases.
                        out.source = None;
                    }
                    out.receiving = receiving;
                }
            }
            Command::SetSourcePaused {
                participant,
                source,
                paused,
            } => self.set_paused(participant, source, paused),
            Command::SetDeafened {
                participant,
                deafened,
            } => {
                if let Some(p) = self.participants.get_mut(&participant) {
                    p.deafened = deafened;
                    for out in p.outs.values_mut() {
                        out.source = None;
                    }
                }
            }
            Command::RestartIce {
                participant,
                connection,
            } => {
                let Some(index) = self
                    .participants
                    .get(&participant)
                    .and_then(|p| p.conn_of(connection))
                else {
                    return;
                };
                if let Some(conn) = self.conns[index].as_mut() {
                    conn.restart_ice = true;
                }
                self.settle(participant);
            }
            Command::Restate { participant } => self.restate(participant),
            Command::Census { reply } => {
                let _ = reply.send(self.census());
            }
        }
    }

    fn census(&self) -> Census {
        Census {
            calls: self.calls.len(),
            participants: self.participants.len(),
            connections: self.conns.iter().flatten().count(),
            sections: self.conns.iter().flatten().map(|c| c.sections.len()).sum(),
            allocated_bytes: thread_allocated_bytes(),
        }
    }

    fn candidates(&self) -> Vec<str0m::Candidate> {
        self.listeners
            .iter()
            .flat_map(Listener::candidates)
            .collect()
    }

    fn add_connection(&mut self, participant: ParticipantId, kind: ConnectionKind) -> usize {
        let conn = Connection::new(participant, kind, &self.candidates());
        let index = match self.conns.iter().position(Option::is_none) {
            Some(free) => free,
            None => {
                self.conns.push(None);
                self.queued.push(None);
                self.reported.push(false);
                self.conns.len() - 1
            }
        };
        self.by_ufrag.insert(conn.ufrag.clone(), index);
        self.conns[index] = Some(conn);
        self.queued[index] = None;
        self.reported[index] = false;
        index
    }

    fn remove_connection(&mut self, index: usize) {
        if let Some(conn) = self.conns[index].take() {
            self.by_ufrag.remove(&conn.ufrag);
        }
        self.by_addr.retain(|_, at| *at != index);
        self.queued[index] = None;
    }

    fn join(&mut self, call: CallId, participant: ParticipantId, user: Uuid, events: Events) {
        if self.participants.contains_key(&participant) {
            return;
        }
        let conn = self.add_connection(participant, ConnectionKind::Call);
        self.participants.insert(
            participant,
            Participant {
                call,
                user,
                events,
                conn,
                capture: None,
                sending: HashMap::new(),
                watch: Vec::new(),
                outs: HashMap::new(),
                // One slot from the start: a connection needs a section to connect at all.
                slots: vec![None; usize::from(self.config.audio_slots.min(1))],
                deafened: false,
                speaking: false,
                routed: Vec::new(),
            },
        );
        let entry = self.calls.entry(call).or_default();
        entry.participants.push(participant);
        entry.by_user.insert(user, participant);
        self.settle(participant);
        self.assign_slots(call);
    }

    fn leave(&mut self, participant: ParticipantId) {
        let Some(p) = self.participants.remove(&participant) else {
            return;
        };
        self.remove_connection(p.conn);
        if let Some(capture) = p.capture {
            self.remove_connection(capture);
        }
        for source in SOURCES {
            self.routes.remove(&(participant, source));
        }
        for key in &p.routed {
            if let Some(routes) = self.routes.get_mut(key) {
                routes.retain(|r| r.receiver != participant);
            }
        }
        let Some(call) = self.calls.get_mut(&p.call) else {
            return;
        };
        call.participants.retain(|other| *other != participant);
        if call.by_user.get(&p.user) == Some(&participant) {
            call.by_user.remove(&p.user);
        }
        if call.participants.is_empty() {
            self.calls.remove(&p.call);
            return;
        }
        let others = call.participants.clone();
        for other in others {
            self.settle(other);
        }
        self.assign_slots(p.call);
    }

    fn send(&mut self, participant: ParticipantId, source: MediaSource, external: bool) {
        let Some(p) = self.participants.get_mut(&participant) else {
            return;
        };
        if p.sending.contains_key(&source) {
            return;
        }
        p.sending.insert(
            source,
            Sending {
                external,
                paused: false,
                live: false,
                ssrc: None,
                levels: Levels::default(),
                last_keyframe_request: None,
            },
        );
        if external && p.capture.is_none() {
            let capture = self.add_connection(participant, ConnectionKind::Capture);
            if let Some(p) = self.participants.get_mut(&participant) {
                p.capture = Some(capture);
            }
        }
        self.sources_changed(participant, source);
    }

    fn stop_sending(&mut self, participant: ParticipantId, source: MediaSource) {
        let Some(p) = self.participants.get_mut(&participant) else {
            return;
        };
        if p.sending.remove(&source).is_none() {
            return;
        }
        let silenced = p.speaking && !p.sending.values().any(|s| s.levels.speaking());
        if silenced {
            p.speaking = false;
            p.tell(Event::Speaking {
                participant,
                speaking: false,
            });
        }
        self.routes.remove(&(participant, source));
        // The capture connection goes with the last source the helper sends.
        if let Some(p) = self.participants.get_mut(&participant)
            && !p.sending.values().any(|s| s.external)
            && let Some(capture) = p.capture.take()
        {
            self.remove_connection(capture);
        }
        self.sources_changed(participant, source);
    }

    fn restate(&mut self, participant: ParticipantId) {
        let Some(p) = self.participants.get(&participant) else {
            return;
        };
        for index in [Some(p.conn), p.capture].into_iter().flatten() {
            let Some(conn) = self.conns[index].as_ref() else {
                continue;
            };
            let sections = conn.sections_for_client();
            p.tell(match &conn.offered_sdp {
                Some(sdp) => Event::Offer {
                    participant,
                    connection: conn.kind,
                    revision: conn.revision,
                    sdp: sdp.clone(),
                    sections,
                },
                None => Event::Sections {
                    participant,
                    connection: conn.kind,
                    sections,
                },
            });
        }
    }

    /// After a participant starts or stops sending `source`: its own connections, everyone who
    /// could watch it, and the slots when it is a microphone.
    fn sources_changed(&mut self, participant: ParticipantId, source: MediaSource) {
        self.settle(participant);
        let Some(call) = self.participants.get(&participant).map(|p| p.call) else {
            return;
        };
        if source.kind() == MediaKind::Video || source == MediaSource::ScreenAudio {
            let others = self
                .calls
                .get(&call)
                .map(|c| c.participants.clone())
                .unwrap_or_default();
            for other in others.into_iter().filter(|o| *o != participant) {
                self.settle(other);
            }
        }
        if source == MediaSource::Microphone {
            self.assign_slots(call);
        }
    }

    fn set_paused(&mut self, participant: ParticipantId, source: MediaSource, paused: bool) {
        let Some(p) = self.participants.get_mut(&participant) else {
            return;
        };
        let Some(sending) = p.sending.get_mut(&source) else {
            return;
        };
        if sending.paused == paused {
            return;
        }
        sending.paused = paused;
        if paused {
            sending.levels.silence();
            if p.speaking && !p.sending.values().any(|s| s.levels.speaking()) {
                p.speaking = false;
                p.tell(Event::Speaking {
                    participant,
                    speaking: false,
                });
            }
        }
        // Whoever receives it starts over: video waits for a keyframe, audio rebases.
        let routes = self
            .routes
            .get(&(participant, source))
            .cloned()
            .unwrap_or_default();
        for route in routes {
            if let Some(out) = self
                .participants
                .get_mut(&route.receiver)
                .and_then(|r| r.outs.get_mut(&route.mid))
            {
                out.source = None;
            }
        }
        if !paused && source.kind() == MediaKind::Video {
            self.request_keyframe((participant, source), KeyframeRequestKind::Pli);
        }
        // A microphone paused or resumed changes who is loudest now, not at the next interval.
        if source == MediaSource::Microphone
            && let Some(call) = self.participants.get(&participant).map(|p| p.call)
        {
            self.assign_slots(call);
        }
    }

    fn answer(
        &mut self,
        participant: ParticipantId,
        connection: ConnectionKind,
        revision: u64,
        sdp: &str,
    ) {
        let Some(index) = self
            .participants
            .get(&participant)
            .and_then(|p| p.conn_of(connection))
        else {
            return;
        };
        let Some(conn) = self.conns[index].as_mut() else {
            return;
        };
        if let Err(reason) = conn.accept_answer(revision, sdp) {
            if let Some(p) = self.participants.get(&participant) {
                p.tell(Event::Failed {
                    participant,
                    connection,
                    reason,
                });
            }
            return;
        }
        self.mark(index);
        // Changes made while the offer waited go out now.
        self.settle(participant);
    }

    // --- Negotiation ------------------------------------------------------------------------

    /// The sections a participant's call connection and capture connection should hold.
    fn wanted(&self, participant: ParticipantId) -> (Wanted, Wanted) {
        let Some(p) = self.participants.get(&participant) else {
            return (Vec::new(), Vec::new());
        };
        let mut call = Vec::new();
        let mut capture = Vec::new();
        for slot in 0..p.slots.len() as u8 {
            call.push((MediaKind::Audio, SectionContent::AudioSlot { slot }));
        }
        for source in SOURCES {
            if let Some(sending) = p.sending.get(&source) {
                let wanted = (source.kind(), SectionContent::Sending { source });
                if sending.external {
                    capture.push(wanted);
                } else {
                    call.push(wanted);
                }
            }
        }
        let members = self.calls.get(&p.call);
        for &(user, source) in &p.watch {
            let Some(owner) = members
                .and_then(|c| c.by_user.get(&user))
                .and_then(|id| self.participants.get(id))
            else {
                continue;
            };
            if !owner.sending.contains_key(&source) || source.kind() != MediaKind::Video {
                continue;
            }
            call.push((MediaKind::Video, SectionContent::Video { user, source }));
            // A sharer watching its own share (the preview of one sent from outside the
            // browser) is not sent its sound back.
            if source == MediaSource::Screen
                && user != p.user
                && owner.sending.contains_key(&MediaSource::ScreenAudio)
            {
                call.push((MediaKind::Audio, SectionContent::ShareAudio { user }));
            }
        }
        (call, capture)
    }

    /// Offers whatever a participant's connections lack, where no offer is outstanding, and
    /// brings its outs and routes in line with the sections offered.
    fn settle(&mut self, participant: ParticipantId) {
        let (call_wanted, capture_wanted) = self.wanted(participant);
        let Some((conn, capture)) = self
            .participants
            .get(&participant)
            .map(|p| (p.conn, p.capture))
        else {
            return;
        };
        let now = Instant::now();
        for (index, wanted, kind) in [
            (Some(conn), call_wanted, ConnectionKind::Call),
            (capture, capture_wanted, ConnectionKind::Capture),
        ] {
            let Some(index) = index else { continue };
            let Some(connection) = self.conns[index].as_mut() else {
                continue;
            };
            let ufrag = connection.ufrag.clone();
            let Some((revision, sdp)) = connection.negotiate(&wanted, now) else {
                continue;
            };
            if connection.ufrag != ufrag {
                // An ICE restart: the client's checks now name the new fragment.
                self.by_ufrag.remove(&ufrag);
                self.by_ufrag.insert(connection.ufrag.clone(), index);
            }
            let sections = connection.sections_for_client();
            let offered: Vec<(Mid, MediaKind, SectionContent)> = connection
                .sections
                .iter()
                .map(|s| (s.mid, s.kind, s.content.clone()))
                .collect();
            let Some(p) = self.participants.get_mut(&participant) else {
                return;
            };
            if kind == ConnectionKind::Call {
                p.outs
                    .retain(|mid, _| offered.iter().any(|(m, _, _)| m == mid));
                for (mid, kind, content) in offered {
                    if matches!(content, SectionContent::Sending { .. }) {
                        continue;
                    }
                    p.outs.entry(mid).or_insert(Out {
                        content,
                        kind,
                        source: None,
                        base: 0,
                        last_seq: None,
                        ts_offset: 0,
                        last_ts: 0,
                        last_at: None,
                        receiving: true,
                        revision,
                    });
                }
            }
            p.tell(Event::Offer {
                participant,
                connection: kind,
                revision,
                sdp,
                sections,
            });
            self.mark(index);
        }
        self.rebuild_routes(participant);
    }

    /// Where every section `receiver` is sent takes its packets from, anew.
    fn rebuild_routes(&mut self, receiver: ParticipantId) {
        let Some(p) = self.participants.get_mut(&receiver) else {
            return;
        };
        for key in std::mem::take(&mut p.routed) {
            if let Some(routes) = self.routes.get_mut(&key) {
                routes.retain(|r| r.receiver != receiver);
            }
        }
        let Some(p) = self.participants.get(&receiver) else {
            return;
        };
        let members = self.calls.get(&p.call);
        let owner = |user: &Uuid| members.and_then(|c| c.by_user.get(user)).copied();
        let mut routed = Vec::new();
        for (mid, out) in &p.outs {
            let source = match &out.content {
                SectionContent::AudioSlot { slot } => p
                    .slots
                    .get(usize::from(*slot))
                    .copied()
                    .flatten()
                    .map(|o| (o, MediaSource::Microphone)),
                SectionContent::Video { user, source } => owner(user).map(|o| (o, *source)),
                SectionContent::ShareAudio { user } => {
                    owner(user).map(|o| (o, MediaSource::ScreenAudio))
                }
                SectionContent::Sending { .. } => None,
            };
            if let Some(key) = source {
                routed.push((key, *mid));
            }
        }
        for (key, mid) in &routed {
            self.routes.entry(*key).or_default().push(Route {
                receiver,
                mid: *mid,
            });
        }
        if let Some(p) = self.participants.get_mut(&receiver) {
            p.routed = routed.into_iter().map(|(key, _)| key).collect();
        }
    }

    // --- Slots and speaking -----------------------------------------------------------------

    /// Fills each participant's audio slots: everyone else's microphone while they fit, else the
    /// most recently loud of those not muted. An occupant stays in its slot while it is among
    /// those chosen, so a slot changes speaker only to make room.
    ///
    /// A participant has as many slots as there are other microphones, up to `audio_slots`, and
    /// never fewer than it had or than one: each section costs its connection on every poll whether or not
    /// it carries anything, so a call of six holds five, and a call only renegotiates while it
    /// grows towards `audio_slots`. A slot is told of (`Event::Slots`) before the offer that adds
    /// its section.
    fn assign_slots(&mut self, call: CallId) {
        let Some(members) = self.calls.get(&call).map(|c| c.participants.clone()) else {
            return;
        };
        let slots = usize::from(self.config.audio_slots);
        let mut ranked: Vec<(usize, ParticipantId, bool, bool, Option<Instant>)> = members
            .iter()
            .enumerate()
            .filter_map(|(order, id)| {
                let mic = self
                    .participants
                    .get(id)?
                    .sending
                    .get(&MediaSource::Microphone)?;
                Some((
                    order,
                    *id,
                    !mic.paused,
                    mic.levels.speaking(),
                    mic.levels.last_spoke,
                ))
            })
            .collect();
        // A muted microphone, which sends nothing, last; then speaking now, the most recently
        // loud, and who joined first.
        ranked.sort_by(|a, b| {
            b.2.cmp(&a.2)
                .then(b.3.cmp(&a.3))
                .then(b.4.cmp(&a.4))
                .then(a.0.cmp(&b.0))
        });
        let users: HashMap<ParticipantId, Uuid> = members
            .iter()
            .filter_map(|id| Some((*id, self.participants.get(id)?.user)))
            .collect();
        for receiver in members {
            let chosen: Vec<ParticipantId> = ranked
                .iter()
                .map(|(_, id, _, _, _)| *id)
                .filter(|id| *id != receiver)
                .take(slots)
                .collect();
            let Some(p) = self.participants.get_mut(&receiver) else {
                continue;
            };
            let mut next = p.slots.clone();
            let grew = next.len() < chosen.len();
            if grew {
                next.resize(chosen.len(), None);
            }
            for occupant in next.iter_mut() {
                if occupant.is_some_and(|o| !chosen.contains(&o)) {
                    *occupant = None;
                }
            }
            for id in &chosen {
                if next.contains(&Some(*id)) {
                    continue;
                }
                if let Some(free) = next.iter_mut().find(|o| o.is_none()) {
                    *free = Some(*id);
                }
            }
            if next == p.slots {
                continue;
            }
            let mut assignments = Vec::new();
            for (slot, new) in next.iter().enumerate() {
                let old = p.slots.get(slot).copied().flatten();
                if old != *new || slot >= p.slots.len() {
                    assignments.push(SlotAssignment {
                        slot: slot as u8,
                        user: new.and_then(|id| users.get(&id).copied()),
                    });
                }
            }
            let Some(p) = self.participants.get_mut(&receiver) else {
                continue;
            };
            p.slots = next;
            for out in p.outs.values_mut() {
                if let SectionContent::AudioSlot { slot } = out.content
                    && assignments.iter().any(|a| a.slot == slot)
                {
                    out.source = None;
                }
            }
            p.tell(Event::Slots {
                participant: receiver,
                assignments,
            });
            if grew {
                // Offers the new slots' sections, and routes to them once answered.
                self.settle(receiver);
            } else {
                self.rebuild_routes(receiver);
            }
        }
    }

    fn tick(&mut self, now: Instant) {
        let mut contested = Vec::new();
        for (id, p) in self.participants.iter_mut() {
            let mut changed = false;
            for sending in p.sending.values_mut() {
                changed |= sending.levels.close_interval(now).is_some();
            }
            let speaking = p.sending.values().any(|s| s.levels.speaking());
            if changed && speaking != p.speaking {
                p.speaking = speaking;
                p.tell(Event::Speaking {
                    participant: *id,
                    speaking,
                });
            }
            if changed && !contested.contains(&p.call) {
                contested.push(p.call);
            }
        }
        let slots = usize::from(self.config.audio_slots);
        for call in contested {
            // While everyone fits, slots move only as people come and go.
            let microphones = self.calls.get(&call).map_or(0, |c| {
                c.participants
                    .iter()
                    .filter(|id| {
                        self.participants
                            .get(id)
                            .is_some_and(|p| p.sending.contains_key(&MediaSource::Microphone))
                    })
                    .count()
            });
            if microphones > slots {
                self.assign_slots(call);
            }
        }
        for index in 0..self.conns.len() {
            let Some(conn) = self.conns[index].as_mut() else {
                continue;
            };
            if conn
                .offered_at
                .is_some_and(|at| now.duration_since(at) > ANSWER_TIMEOUT)
            {
                conn.offered_at = None;
                conn.failed.get_or_insert(Failure::AnswerTimedOut);
                self.report(index);
            } else if !conn.connected && now.duration_since(conn.created_at) > CONNECT_TIMEOUT {
                conn.failed.get_or_insert(Failure::NeverConnected);
                self.report(index);
            }
        }
        let limit = Duration::from_secs(net::TCP_UNIDENTIFIED_SECONDS);
        for index in 0..self.streams.len() {
            if self.streams[index]
                .as_ref()
                .is_some_and(|s| !s.identified && now.duration_since(s.accepted_at) > limit)
            {
                self.drop_stream(index);
            }
        }
    }

    // --- Sockets ----------------------------------------------------------------------------

    fn receive_udp(&mut self, buf: &mut [u8]) {
        for listener in 0..self.listeners.len() {
            loop {
                let (n, peer) = match self.listeners[listener].udp.recv_from(buf) {
                    Ok(read) => read,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        tracing::warn!("media UDP: {e}");
                        break;
                    }
                };
                let local = self.listeners[listener].udp_local;
                self.dispatch(Protocol::Udp, local, peer, &buf[..n]);
            }
        }
    }

    fn accept_tcp(&mut self) {
        for listener in 0..self.listeners.len() {
            loop {
                let (stream, peer) = match self.listeners[listener].tcp.accept() {
                    Ok(accepted) => accepted,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                    Err(e) => {
                        tracing::warn!("media TCP accept: {e}");
                        break;
                    }
                };
                if self.streams.iter().flatten().count() >= net::MAX_TCP_STREAMS {
                    continue;
                }
                let Ok(stream) = Stream::new(stream, peer, listener) else {
                    continue;
                };
                let index = match self.streams.iter().position(Option::is_none) {
                    Some(free) => free,
                    None => {
                        self.streams.push(None);
                        self.streams.len() - 1
                    }
                };
                self.streams[index] = Some(stream);
                self.tcp_by_peer.insert(peer, index);
            }
        }
    }

    fn receive_tcp(&mut self) {
        for index in 0..self.streams.len() {
            let Some(stream) = self.streams[index].as_mut() else {
                continue;
            };
            let mut packets = Vec::new();
            let result = stream.receive(&mut |packet| packets.push(packet.to_vec()));
            let (peer, local) = (stream.peer, self.listeners[stream.listener].tcp_local);
            for packet in packets {
                if self.dispatch(Protocol::Tcp, local, peer, &packet)
                    && let Some(stream) = self.streams[index].as_mut()
                {
                    stream.identified = true;
                }
            }
            if result.is_err() {
                self.drop_stream(index);
            } else if let Some(stream) = self.streams[index].as_mut()
                && stream.flush().is_err()
            {
                self.drop_stream(index);
            }
        }
    }

    fn drop_stream(&mut self, index: usize) {
        if let Some(stream) = self.streams[index].take() {
            self.tcp_by_peer.remove(&stream.peer);
            self.by_addr.remove(&(
                Protocol::Tcp,
                self.listeners[stream.listener].tcp_local,
                stream.peer,
            ));
        }
    }

    /// Gives one packet to the connection it is for; whether one took it.
    fn dispatch(
        &mut self,
        proto: Protocol,
        local: SocketAddr,
        peer: SocketAddr,
        packet: &[u8],
    ) -> bool {
        let now = Instant::now();
        let Ok(contents) = packet.try_into() else {
            return false;
        };
        let input = Input::Receive(
            now,
            Receive {
                proto,
                source: peer,
                destination: local,
                contents,
            },
        );
        let key = (proto, local, peer);
        let index = match self.by_addr.get(&key) {
            Some(&index) => index,
            None => {
                let Some(&index) = net::stun_local_ufrag(packet).and_then(|u| self.by_ufrag.get(u))
                else {
                    return false;
                };
                if !self.conns[index]
                    .as_mut()
                    .is_some_and(|c| c.accepts(&input))
                {
                    return false;
                }
                self.by_addr.insert(key, index);
                index
            }
        };
        let Some(conn) = self.conns[index].as_mut() else {
            self.by_addr.remove(&key);
            return false;
        };
        conn.handle_input(input);
        // str0m is polled after each input: what an input gives (a received packet) is lost if
        // another input comes before the poll.
        self.process(index);
        true
    }

    /// Sends one of str0m's transmissions out of the socket or stream it names.
    fn transmit(
        listeners: &[Listener],
        streams: &mut [Option<Stream>],
        tcp: &HashMap<SocketAddr, usize>,
        t: Transmit,
    ) {
        match t.proto {
            Protocol::Udp => {
                if let Some(listener) = listeners.iter().find(|l| l.udp_local == t.source) {
                    let _ = listener.udp.send_to(&t.contents, t.destination);
                }
            }
            Protocol::Tcp => {
                if let Some(stream) = tcp.get(&t.destination).and_then(|&i| streams[i].as_mut()) {
                    // A stream whose client cannot keep up is closed on its next read.
                    let _ = stream.send(&t.contents);
                }
            }
            Protocol::SslTcp | Protocol::Tls => {}
        }
    }

    fn expire(&mut self, now: Instant) {
        while let Some(&Reverse((at, index))) = self.timers.peek() {
            if at > now {
                break;
            }
            self.timers.pop();
            if self.queued.get(index).copied().flatten() != Some(at) {
                continue;
            }
            self.queued[index] = None;
            if let Some(conn) = self.conns[index].as_mut() {
                conn.handle_input(Input::Timeout(now));
                self.mark(index);
            }
        }
    }

    fn wait(&mut self) {
        let now = Instant::now();
        let next = self
            .timers
            .peek()
            .map(|Reverse((at, _))| *at)
            .unwrap_or(now + MAX_SLEEP)
            .min(self.next_tick)
            .min(now + MAX_SLEEP);
        let mut fds = vec![libc::pollfd {
            fd: self.woken.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        }];
        for listener in &self.listeners {
            for fd in [listener.udp.as_raw_fd(), listener.tcp.as_raw_fd()] {
                fds.push(libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                });
            }
        }
        for stream in self.streams.iter().flatten() {
            let mut events = libc::POLLIN;
            if stream.wants_write() {
                events |= libc::POLLOUT;
            }
            fds.push(libc::pollfd {
                fd: stream.stream.as_raw_fd(),
                events,
                revents: 0,
            });
        }
        sleep_on(&mut fds, next.saturating_duration_since(now));
    }

    // --- Connections ------------------------------------------------------------------------

    /// Polls connection `index` and acts on what it gave.
    fn process(&mut self, index: usize) {
        let events = {
            let Some(conn) = self.conns[index].as_mut() else {
                return;
            };
            let (listeners, streams, tcp) = (&self.listeners, &mut self.streams, &self.tcp_by_peer);
            let events = conn.poll(&mut |t| Self::transmit(listeners, streams, tcp, t));
            if self.queued[index] != Some(conn.timeout) {
                self.queued[index] = Some(conn.timeout);
                self.timers.push(Reverse((conn.timeout, index)));
            }
            events
        };
        for event in events {
            match event {
                RtcEvent::Connected => {
                    if let Some(conn) = self.conns[index].as_ref()
                        && let Some(p) = self.participants.get(&conn.participant)
                    {
                        p.tell(Event::Connected {
                            participant: conn.participant,
                            connection: conn.kind,
                        });
                    }
                }
                RtcEvent::IceConnectionStateChange(IceConnectionState::Disconnected) => {
                    // A client that moved network answers an ICE restart and goes on.
                    let Some(conn) = self.conns[index].as_mut() else {
                        continue;
                    };
                    conn.restart_ice = true;
                    let participant = conn.participant;
                    self.settle(participant);
                }
                RtcEvent::RtpPacket(packet) => self.forward(index, packet),
                RtcEvent::KeyframeRequest(request) => {
                    self.keyframe_requested(index, request.mid, request.kind)
                }
                _ => {}
            }
        }
        self.report(index);
    }

    /// Tells a connection's participant it failed, once.
    fn report(&mut self, index: usize) {
        let Some(conn) = self.conns[index].as_ref() else {
            return;
        };
        let Some(reason) = conn.failed else {
            return;
        };
        if std::mem::replace(&mut self.reported[index], true) {
            return;
        }
        if let Some(p) = self.participants.get(&conn.participant) {
            p.tell(Event::Failed {
                participant: conn.participant,
                connection: conn.kind,
                reason,
            });
        }
    }

    /// Notes that connection `index` has something to send, for `flush_dirty`.
    fn mark(&mut self, index: usize) {
        if self.marked.len() <= index {
            self.marked.resize(index + 1, false);
        }
        if !std::mem::replace(&mut self.marked[index], true) {
            self.dirty.push(index);
        }
    }

    fn flush_dirty(&mut self) {
        while let Some(index) = self.dirty.pop() {
            self.marked[index] = false;
            self.process(index);
        }
    }

    // --- Forwarding -------------------------------------------------------------------------

    fn forward(&mut self, index: usize, packet: RtpPacket) {
        let Some(conn) = self.conns[index].as_mut() else {
            return;
        };
        let participant = conn.participant;
        let ssrc = *packet.header.ssrc;
        let Some(mid) = conn.mid_of(ssrc) else {
            return;
        };
        let Some(SectionContent::Sending { source }) = conn.section(mid).map(|s| s.content.clone())
        else {
            return;
        };
        let codec = conn.codec(*packet.header.payload_type);
        let Some(p) = self.participants.get_mut(&participant) else {
            return;
        };
        let Some(sending) = p.sending.get_mut(&source) else {
            return;
        };
        sending.ssrc = Some(ssrc);
        let became_live = !std::mem::replace(&mut sending.live, true);
        let paused = sending.paused;
        if !paused
            && source.kind() == MediaKind::Audio
            && let Some(level) = packet.header.ext_vals.audio_level
        {
            sending.levels.hear(level);
        }
        if became_live {
            p.tell(Event::Live {
                participant,
                source,
            });
        }
        if paused {
            return;
        }
        let key = (participant, source);
        let keyframe = codec.is_some_and(|c| is_keyframe(c, &packet.payload));
        let sequence: u64 = *packet.seq_no;
        let timestamp = packet.header.timestamp;
        let now = Instant::now();
        let Some(routes) = self.routes.get(&key).cloned() else {
            return;
        };
        let mut want_keyframe = false;
        for route in routes {
            let Some(receiver) = self.participants.get_mut(&route.receiver) else {
                continue;
            };
            let deafened = receiver.deafened;
            let target = receiver.conn;
            let answered = self.conns[target].as_ref().map_or(0, |c| c.answered);
            let Some(out) = receiver.outs.get_mut(&route.mid) else {
                continue;
            };
            let audio = out.kind == MediaKind::Audio;
            if !out.receiving || (deafened && audio) || out.revision > answered {
                continue;
            }
            if out.source != Some(key) {
                if !audio && !keyframe {
                    want_keyframe = true;
                    continue;
                }
                let next = out.last_seq.map_or(1, |last| last + 1);
                out.base = sequence.wrapping_sub(next);
                let clock = if audio { 48_000 } else { 90_000 };
                let elapsed = out.last_at.map_or(0, |at| {
                    (now.duration_since(at).as_micros() * clock / 1_000_000) as u32
                });
                out.ts_offset = out.last_ts.wrapping_add(elapsed).wrapping_sub(timestamp);
                out.source = Some(key);
            }
            let out_sequence = sequence.wrapping_sub(out.base);
            if out.last_seq.is_some_and(|last| out_sequence + 1024 < last) {
                // From before the source began here: a retransmission of what was never sent on.
                continue;
            }
            let out_timestamp = timestamp.wrapping_add(out.ts_offset);
            if out.last_seq.is_none_or(|last| out_sequence > last) {
                out.last_seq = Some(out_sequence);
                out.last_ts = out_timestamp;
                out.last_at = Some(now);
            }
            let slot = matches!(out.content, SectionContent::AudioSlot { .. });
            let Some(target_conn) = self.conns[target].as_mut() else {
                continue;
            };
            let payload_type = codec
                .and_then(|c| target_conn.payload_type(c))
                .unwrap_or(*packet.header.payload_type);
            let mut write = RtpWrite::new(
                payload_type.into(),
                SeqNo::from(out_sequence),
                out_timestamp,
                packet.timestamp,
                packet.payload.clone(),
            )
            .marker(packet.header.marker)
            .ext_vals(ExtensionValues {
                audio_level: packet.header.ext_vals.audio_level,
                voice_activity: packet.header.ext_vals.voice_activity,
                ..Default::default()
            })
            .nackable(!audio);
            if slot {
                // Names the speaker for a client that reads contributing sources.
                write = write.csrc(&[ssrc]);
            }
            if target_conn.write(route.mid, write) {
                self.mark(target);
            }
        }
        if want_keyframe {
            self.request_keyframe(key, KeyframeRequestKind::Pli);
        }
    }

    fn keyframe_requested(&mut self, index: usize, mid: Mid, kind: KeyframeRequestKind) {
        let Some(conn) = self.conns[index].as_ref() else {
            return;
        };
        let source = self
            .participants
            .get(&conn.participant)
            .and_then(|p| p.outs.get(&mid))
            .and_then(|out| out.source);
        if let Some(source) = source {
            self.request_keyframe(source, kind);
        }
    }

    /// Asks a video source for a keyframe, at most once per `KEYFRAME_REQUEST_INTERVAL`.
    fn request_keyframe(&mut self, (owner, source): SourceKey, kind: KeyframeRequestKind) {
        let now = Instant::now();
        let Some(p) = self.participants.get_mut(&owner) else {
            return;
        };
        let conn = if p.sending.get(&source).is_some_and(|s| s.external) {
            p.capture
        } else {
            Some(p.conn)
        };
        let Some(sending) = p.sending.get_mut(&source) else {
            return;
        };
        if sending
            .last_keyframe_request
            .is_some_and(|at| now.duration_since(at) < KEYFRAME_REQUEST_INTERVAL)
        {
            return;
        }
        let Some(ssrc) = sending.ssrc else {
            return;
        };
        sending.last_keyframe_request = Some(now);
        if let Some(index) = conn
            && let Some(conn) = self.conns[index].as_mut()
        {
            conn.request_keyframe(ssrc, kind);
            self.mark(index);
        }
    }
}
