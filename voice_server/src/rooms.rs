//! Calls as this server carries them. A room is one channel's call: a mediasoup router, an
//! audio level observer that drives speaking events, and the participants with their
//! transports, producers, and consumers. Rooms come into being with their first participant
//! and go with their last, and the API server hears about each step through the reporter.
//! Files offered in a call, and the transfers between its participants, are the room's too
//! (`transfers`). Media a client sends or receives as SRTP itself is `plain_rtp`.

mod plain_rtp;
mod transfers;

use crate::media::{media_codecs, media_kind, wire_kind};
use crate::reporter::Reporter;
use crate::transfer::Relay;
use mediasoup::prelude::*;
use mediasoup::types::data_structures::{DtlsState, TransportTuple};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::num::NonZeroU16;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use uuid::Uuid;
use voice_protocol::control::{ParticipantSnapshot, REPORT_PARTITIONS, VoiceReport, partition};
use voice_protocol::signal::{
    KickReason, MediaKind as WireKind, MediaSource, ParticipantInfo, ProducerInfo, ServerMessage,
    TransportDirection,
};
use voice_protocol::token::Grants;

/// What a transport assumes a participant can receive before it has measured, in bits per
/// second: enough for a screen share at full quality from its first seconds. The voice server
/// assumes its network can carry the best picture and lets each receiver's own bandwidth
/// estimate bring it down, rather than starting low (mediasoup's default is 600 kbps) and
/// making every share blurry while the estimate climbs.
const INITIAL_OUTGOING_BITRATE: u64 = 10_000_000;

/// Volumes above this, in dBvo, count as speaking.
const SPEAKING_THRESHOLD_DBVO: i8 = -50;
/// How often the observer reports volumes; speaking flips at most this often.
const SPEAKING_INTERVAL_MS: u16 = 300;

/// How long a transport has to connect (its DTLS handshake done, or a plain transport's sender
/// heard from) before it is closed, so a client cannot hold ports from the media range with
/// transports it never uses. ICE and DTLS take a few seconds on the worst networks a call works
/// over.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
pub enum RoomError {
    #[error("no such transport")]
    UnknownTransport,
    #[error("no such producer")]
    UnknownProducer,
    #[error("no such consumer")]
    UnknownConsumer,
    #[error("the participant is not in the call")]
    NotInCall,
    #[error("mediasoup could not create the call's router: {0}")]
    CreateRouter(#[from] mediasoup::worker::CreateRouterError),
    #[error("mediasoup refused: {0}")]
    MediaRequest(#[from] mediasoup::worker::RequestError),
    #[error("mediasoup refused the producer: {0}")]
    Produce(#[from] ProduceError),
    #[error("the plain transport has no SRTP parameters")]
    NoSrtpParameters,
    #[error("{0}")]
    BadParameters(String),
    /// The join token does not grant sending from this source.
    #[error("this call does not let you send {0:?}")]
    NotPermitted(voice_protocol::signal::MediaSource),
    /// The join token does not grant offering files.
    #[error("this call does not let you offer files")]
    TransferNotPermitted,
    #[error("no such offer, or it no longer stands")]
    UnknownOffer,
    #[error("no such transfer")]
    UnknownTransfer,
    /// The user is already in as many calls on this server as they may be.
    #[error("you are already in {0} calls on this voice server; leave one first")]
    TooManySeats(usize),
    /// The call holds as many people as it may.
    #[error("this call is full ({0} people)")]
    CallFull(usize),
    /// A producer's kind does not match its source (`MediaSource::kind`).
    #[error("a {of:?} producer carries {expected:?}, not {kind:?}")]
    WrongKind {
        of: MediaSource,
        kind: WireKind,
        expected: WireKind,
    },
}

/// What a join token admits its holder to, as the participant it makes starts out.
#[derive(Clone, Debug)]
pub struct Admission {
    /// What they may send and offer (`JoinClaims::grants`).
    pub grants: Grants,
    /// The sign-in the token was issued to; none for a bot's.
    pub sign_in: Option<String>,
    /// Whether a moderator's mute of them stands where the call is.
    pub server_muted: bool,
}

impl From<&voice_protocol::token::JoinClaims> for Admission {
    fn from(claims: &voice_protocol::token::JoinClaims) -> Self {
        Self {
            grants: claims.grants(),
            sign_in: claims.sign_in.clone(),
            server_muted: claims.server_muted,
        }
    }
}

/// Where a participant's frames go: their socket's outbox.
pub use crate::outbox::Outbox;

/// One signalling socket's place in a call. A user joining again from another socket replaces
/// their participant, and the socket replaced must not take the new one out when it closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Connection(u64);

/// The participant one signalling socket made: its call's channel, its user, and its
/// connection. Every frame from the socket acts through it, and is refused (`NotInCall`) once
/// that participant is gone, whether removed, replaced from another socket, or in a call that
/// has since ended and been followed by another, so a socket can never act for someone else's
/// place in a call, nor for its own once it has lost it.
#[derive(Clone, Copy, Debug)]
pub struct Seat {
    pub channel: Uuid,
    pub user: Uuid,
    connection: Connection,
}

/// The participant `seat` made, while they are still in the call.
fn seated(
    participants: &HashMap<Uuid, Participant>,
    seat: Seat,
) -> Result<&Participant, RoomError> {
    participants
        .get(&seat.user)
        .filter(|participant| participant.connection == seat.connection)
        .ok_or(RoomError::NotInCall)
}

fn seated_mut(
    participants: &mut HashMap<Uuid, Participant>,
    seat: Seat,
) -> Result<&mut Participant, RoomError> {
    participants
        .get_mut(&seat.user)
        .filter(|participant| participant.connection == seat.connection)
        .ok_or(RoomError::NotInCall)
}

struct Participant {
    user: Uuid,
    connection: Connection,
    outbox: Outbox,
    rtp_capabilities: Option<RtpCapabilities>,
    send_transport: Option<WebRtcTransport>,
    recv_transport: Option<ReceiveTransport>,
    /// The transports an external sender (the desktop shell's game capture) delivers SRTP to,
    /// by the producer each feeds.
    rtp_transports: HashMap<ProducerId, PlainTransport>,
    producers: HashMap<ProducerId, (Producer, MediaSource)>,
    /// Producers the participant also consumes themself, as a preview of what an external
    /// sender is delivering on their behalf.
    own_preview: HashSet<ProducerId>,
    /// By consumer id: the consumer and whose producer it carries.
    consumers: HashMap<ConsumerId, (Consumer, Uuid)>,
    /// Muted as the participant themself asked.
    muted: bool,
    /// Muted by a moderator (`VoiceCommand::Mute`). Only another such command lifts it: while
    /// it stands their microphone stays paused whatever they ask.
    server_muted: bool,
    deafened: bool,
    speaking: bool,
    /// What they may send and offer, from their join token and then the API server.
    grants: Grants,
    /// The sign-in their join token was issued to (`JoinClaims::sign_in`); none for a bot.
    /// They leave when it ends (`VoiceCommand::EndSignIns`).
    sign_in: Option<String>,
    /// Their place among the calls their user is in on this server. Passed on to whoever
    /// replaces them from another socket, and given back as they leave.
    seat: Option<SeatClaim>,
}

/// How many calls each user is in on this server, so none can be in more than
/// `max_seats_per_user` (each takes transports, and so ports from the media range).
#[derive(Default)]
struct SeatCounts(Mutex<HashMap<Uuid, usize>>);

/// One of a user's places in a call, counted in `SeatCounts` until it is dropped.
struct SeatClaim {
    counts: Arc<SeatCounts>,
    user: Uuid,
}

impl SeatCounts {
    /// A place for `user`, unless they already hold `max`.
    fn claim(self: &Arc<Self>, user: Uuid, max: usize) -> Option<SeatClaim> {
        let mut counts = self.0.lock().expect("seat counts lock");
        let held = counts.entry(user).or_default();
        if *held >= max {
            return None;
        }
        *held += 1;
        Some(SeatClaim {
            counts: Arc::clone(self),
            user,
        })
    }
}

impl Drop for SeatClaim {
    fn drop(&mut self) {
        let mut counts = self.counts.0.lock().expect("seat counts lock");
        if let Some(held) = counts.get_mut(&self.user) {
            *held -= 1;
            if *held == 0 {
                counts.remove(&self.user);
            }
        }
    }
}

/// A participant leaving the call, by any path, hangs their socket up: it may send nothing more
/// for them, and their client, told why if there is a reason, is let go.
impl Drop for Participant {
    fn drop(&mut self) {
        self.outbox.hang_up();
    }
}

impl Participant {
    /// A participant sends at most one producer per source (one microphone, one screen, one
    /// screen's sound), so one client cannot multiply what everyone else in the call receives;
    /// replacing one means closing it first.
    fn ensure_source_free(&self, source: MediaSource) -> Result<(), RoomError> {
        if self
            .producers
            .values()
            .any(|(_, existing)| *existing == source)
        {
            return Err(RoomError::BadParameters(format!(
                "already producing {source:?}; close that producer first"
            )));
        }
        Ok(())
    }

    /// Whether their microphone is paused: by their own choice or a moderator's. This is the
    /// mute everyone else, and the API server, is told of.
    fn silenced(&self) -> bool {
        self.muted || self.server_muted
    }

    /// Whether a new producer of `source` may join the participant's, checked under the room's
    /// lock as it is added: making one awaits mediasoup, and meanwhile their grants may have
    /// been taken away or another producer of the source added.
    fn admit(&self, source: MediaSource) -> Result<(), RoomError> {
        if !self.grants.may_produce(source) {
            return Err(RoomError::NotPermitted(source));
        }
        self.ensure_source_free(source)
    }

    fn sharing_screen(&self) -> bool {
        self.producers
            .values()
            .any(|(_, source)| *source == MediaSource::Screen)
    }

    fn info(&self) -> ParticipantInfo {
        ParticipantInfo {
            user: self.user,
            muted: self.silenced(),
            deafened: self.deafened,
            speaking: self.speaking,
            producers: self
                .producers
                .values()
                .map(|(producer, source)| ProducerInfo {
                    id: producer.id().to_string(),
                    kind: wire_kind(producer.kind()),
                    source: *source,
                    paused: producer.paused(),
                })
                .collect(),
        }
    }

    fn send(&self, message: ServerMessage) {
        self.outbox.send(&message);
    }
}

pub struct Room {
    pub session: Uuid,
    pub channel: Uuid,
    router: Router,
    /// Held for its lifetime: dropping it stops the volume events.
    _audio_observer: AudioLevelObserver,
    participants: Mutex<HashMap<Uuid, Participant>>,
    /// Set, under the participants lock, when the last participant leaves. A closed room
    /// takes no one in: whoever arrives next starts the channel's next call. It keeps the
    /// channel's place until its end is reported, so snapshots list it until then.
    closed: AtomicBool,
    /// Becomes true once the room's end has been reported and it has left the channel's place,
    /// so the next call's start is reported after it.
    ended: watch::Sender<bool>,
    /// Which user each audio producer belongs to, for the volume events.
    producer_owner: Mutex<HashMap<ProducerId, Uuid>>,
    /// Files offered in the call that may still be accepted, by id.
    offers: Mutex<HashMap<Uuid, transfers::Offer>>,
    /// Transfers under way, by offer and receiver. They outlive their offers.
    transfers: Mutex<HashMap<(Uuid, Uuid), transfers::Transfer>>,
}

impl Room {
    /// Refuses a frame from a socket whose participant is no longer in this call.
    fn require(&self, seat: Seat) -> Result<(), RoomError> {
        seated(&self.participants.lock().expect("room lock"), seat).map(|_| ())
    }

    fn broadcast(&self, message: &ServerMessage, except: Option<Uuid>) {
        let text = crate::outbox::frame_text(message);
        for participant in self.participants.lock().expect("room lock").values() {
            if Some(participant.user) != except {
                participant.outbox.send_text(text.clone());
            }
        }
    }
}

/// Where a participant's consumers are: a WebRTC transport for a browser, or a plain SRTP one
/// for a client that asked with `consumeRtp`.
#[derive(Clone)]
enum ReceiveTransport {
    WebRtc(WebRtcTransport),
    Plain(PlainTransport),
}

impl ReceiveTransport {
    async fn consume(&self, options: ConsumerOptions) -> Result<Consumer, ConsumeError> {
        match self {
            ReceiveTransport::WebRtc(transport) => transport.consume(options).await,
            ReceiveTransport::Plain(transport) => transport.consume(options).await,
        }
    }
}

/// A transport that must connect within `CONNECT_TIMEOUT`.
#[derive(Clone, Copy)]
enum Unconnected {
    /// A browser's send or receive transport.
    WebRtc(TransportId),
    /// A plain receive transport (`consumeRtp`).
    PlainReceive(TransportId),
    /// The plain transport feeding an external sender's producer (`produceRtp`).
    PlainSend(ProducerId),
}

/// Whether a plain transport has heard from its sender, which is when it learns where to send.
fn heard_from(transport: &PlainTransport) -> bool {
    matches!(transport.tuple(), TransportTuple::WithRemote { .. })
}

/// What a voice server carries at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub rooms: usize,
    pub participants: usize,
    pub producers: usize,
    pub consumers: usize,
    pub transports: usize,
}

/// Every call on this server.
pub struct Rooms {
    server: Uuid,
    workers: Vec<Worker>,
    next_worker: AtomicUsize,
    rtc_ip: IpAddr,
    announced_address: Option<String>,
    reporter: Reporter,
    relay: Arc<Relay>,
    rooms: Mutex<HashMap<Uuid, Arc<Room>>>,
    next_connection: AtomicU64,
    seats: Arc<SeatCounts>,
    max_seats_per_user: usize,
    max_participants_per_call: usize,
}

impl Rooms {
    pub fn new(
        server: Uuid,
        workers: Vec<Worker>,
        rtc_ip: IpAddr,
        announced_address: Option<String>,
        reporter: Reporter,
        relay: Arc<Relay>,
        limits: &crate::config::LimitSettings,
    ) -> Arc<Self> {
        Arc::new(Self {
            server,
            workers,
            next_worker: AtomicUsize::new(0),
            rtc_ip,
            announced_address,
            reporter,
            relay,
            rooms: Mutex::new(HashMap::new()),
            next_connection: AtomicU64::new(0),
            seats: Arc::default(),
            max_seats_per_user: limits.max_seats_per_user,
            max_participants_per_call: limits.max_participants_per_call,
        })
    }

    /// Everyone in every call, for the load report.
    /// How much this server is carrying, for its metrics.
    pub fn census(&self) -> Census {
        let rooms = self.rooms.lock().expect("rooms lock");
        let mut census = Census {
            rooms: rooms.len(),
            ..Census::default()
        };
        for room in rooms.values() {
            let participants = room.participants.lock().expect("room lock");
            census.participants += participants.len();
            for participant in participants.values() {
                census.producers += participant.producers.len();
                census.consumers += participant.consumers.len();
                census.transports += usize::from(participant.send_transport.is_some())
                    + usize::from(participant.recv_transport.is_some())
                    + participant.rtp_transports.len();
            }
        }
        census
    }

    pub fn participant_count(&self) -> u32 {
        self.rooms
            .lock()
            .expect("rooms lock")
            .values()
            .map(|room| room.participants.lock().expect("room lock").len())
            .sum::<usize>()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    fn room(&self, channel: Uuid) -> Result<Arc<Room>, RoomError> {
        self.rooms
            .lock()
            .expect("rooms lock")
            .get(&channel)
            .cloned()
            .ok_or(RoomError::NotInCall)
    }

    /// Takes `room` out of the channel's place, unless another has already taken it.
    fn forget(&self, room: &Arc<Room>) {
        let mut rooms = self.rooms.lock().expect("rooms lock");
        if rooms
            .get(&room.channel)
            .is_some_and(|current| Arc::ptr_eq(current, room))
        {
            rooms.remove(&room.channel);
        }
    }

    /// Every call this server holds and who is in it, ending with the list of them in each lane
    /// (every lane's, even one with no calls). Made inside `Reporter::report_with`, so it
    /// describes the calls as of its place among the reports.
    pub fn snapshot(&self) -> Vec<VoiceReport> {
        let rooms: Vec<Arc<Room>> = self
            .rooms
            .lock()
            .expect("rooms lock")
            .values()
            .cloned()
            .collect();
        let mut reports: Vec<VoiceReport> = rooms
            .iter()
            .map(|room| VoiceReport::SessionSnapshot {
                server: self.server,
                session: room.session,
                channel: room.channel,
                participants: room
                    .participants
                    .lock()
                    .expect("room lock")
                    .values()
                    .map(|participant| ParticipantSnapshot {
                        user: participant.user,
                        muted: participant.silenced(),
                        deafened: participant.deafened,
                        sharing_screen: participant.sharing_screen(),
                    })
                    .collect(),
            })
            .collect();
        reports.extend((0..REPORT_PARTITIONS).map(|lane| {
            VoiceReport::SessionsHeld {
                server: self.server,
                partition: lane,
                sessions: rooms
                    .iter()
                    .filter(|room| partition(room.channel) == lane)
                    .map(|room| room.session)
                    .collect(),
            }
        }));
        reports
    }

    fn room_of_session(&self, session: Uuid) -> Option<Arc<Room>> {
        self.rooms
            .lock()
            .expect("rooms lock")
            .values()
            .find(|room| room.session == session)
            .cloned()
    }

    /// Puts `user` in `channel`'s call, starting the call if it has none, and replies with
    /// `ready`. A user already in the call from another socket is replaced. Returns the seat
    /// the socket's frames act through. Refused when the user is already in
    /// `max_seats_per_user` other calls here, or the call holds `max_participants_per_call`.
    /// Every join is reported, a replacement's too, so the API server checks each against what
    /// its token was issued to.
    pub async fn join(
        self: &Arc<Self>,
        channel: Uuid,
        user: Uuid,
        outbox: Outbox,
        admission: Admission,
    ) -> Result<Seat, RoomError> {
        let connection = Connection(self.next_connection.fetch_add(1, Ordering::Relaxed));
        loop {
            let existing = self
                .rooms
                .lock()
                .expect("rooms lock")
                .get(&channel)
                .cloned();
            let room = match existing {
                Some(room) => room,
                None => self.start_room(channel).await?,
            };
            let entered = match self.enter(&room, user, connection, &outbox, admission.clone()) {
                Ok(entered) => entered,
                Err(e) => {
                    // A room this join started holds no one; it ends as though its last
                    // participant left.
                    self.end_if_empty(&room);
                    return Err(e);
                }
            };
            if entered {
                self.reporter.report(VoiceReport::ParticipantJoined {
                    session: room.session,
                    channel,
                    user,
                    sign_in: admission.sign_in,
                });
                // A participant joining muted by a moderator is recorded so: their joining is
                // recorded unmuted, and the recheck that follows finds the mute already
                // standing and changes nothing.
                if admission.server_muted {
                    let report = room
                        .participants
                        .lock()
                        .expect("room lock")
                        .get(&user)
                        .filter(|participant| participant.connection == connection)
                        .map(|participant| Self::state_report(&room, participant));
                    if let Some(report) = report {
                        self.reporter.report(report);
                    }
                }
                info!(
                    channel = channel.to_string(),
                    user = user.to_string(),
                    "joined a call"
                );
                return Ok(Seat {
                    channel,
                    user,
                    connection,
                });
            }
            // The room's last participant left as this one arrived. Once its end is reported
            // and it has gone from the channel's place, this one starts the channel's next call.
            let _ = room.ended.subscribe().wait_for(|ended| *ended).await;
        }
    }

    /// Adds the participant to `room`, replacing any of the same user from another socket, and
    /// sends them `ready`, returning false when the room has closed.
    fn enter(
        &self,
        room: &Room,
        user: Uuid,
        connection: Connection,
        outbox: &Outbox,
        admission: Admission,
    ) -> Result<bool, RoomError> {
        let Admission {
            grants,
            sign_in,
            server_muted,
        } = admission;
        let mut participants = room.participants.lock().expect("room lock");
        if room.closed.load(Ordering::Relaxed) {
            return Ok(false);
        }
        let seat = match participants.get_mut(&user) {
            Some(old) => old.seat.take(),
            None => {
                if participants.len() >= self.max_participants_per_call {
                    return Err(RoomError::CallFull(self.max_participants_per_call));
                }
                Some(
                    self.seats
                        .claim(user, self.max_seats_per_user)
                        .ok_or(RoomError::TooManySeats(self.max_seats_per_user))?,
                )
            }
        };
        let replaced = participants.remove(&user);
        if let Some(old) = &replaced {
            old.send(ServerMessage::Kicked {
                reason: KickReason::Replaced,
            });
        }
        let others: Vec<ParticipantInfo> = participants.values().map(Participant::info).collect();
        let joined = ServerMessage::ParticipantJoined {
            user,
            muted: server_muted,
            deafened: false,
        };
        for other in participants.values() {
            other.send(joined.clone());
        }
        let participant = Participant {
            user,
            connection,
            outbox: outbox.clone(),
            rtp_capabilities: None,
            send_transport: None,
            recv_transport: None,
            rtp_transports: HashMap::new(),
            producers: HashMap::new(),
            own_preview: HashSet::new(),
            consumers: HashMap::new(),
            muted: false,
            server_muted,
            deafened: false,
            speaking: false,
            grants,
            sign_in,
            seat,
        };
        participant.send(ServerMessage::Ready {
            session: room.session,
            user,
            router_rtp_capabilities: serde_json::to_value(room.router.rtp_capabilities())
                .expect("capabilities serialize"),
            participants: others,
            offers: room.standing_offers(),
            links: room.links(),
            transfers: self.relay.policy(),
        });
        participants.insert(user, participant);
        Ok(true)
    }

    async fn start_room(self: &Arc<Self>, channel: Uuid) -> Result<Arc<Room>, RoomError> {
        let index = self.next_worker.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        let worker = &self.workers[index];
        let router = worker
            .create_router(RouterOptions::new(media_codecs()))
            .await?;
        let audio_observer = router
            .create_audio_level_observer({
                let mut options = AudioLevelObserverOptions::default();
                options.max_entries = NonZeroU16::new(32).expect("non-zero");
                options.threshold = SPEAKING_THRESHOLD_DBVO;
                options.interval = SPEAKING_INTERVAL_MS;
                options
            })
            .await?;
        let session = Uuid::now_v7();
        let room = Arc::new(Room {
            session,
            channel,
            router,
            _audio_observer: audio_observer.clone(),
            participants: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            ended: watch::Sender::new(false),
            producer_owner: Mutex::new(HashMap::new()),
            offers: Mutex::new(HashMap::new()),
            transfers: Mutex::new(HashMap::new()),
        });
        {
            let mut rooms = self.rooms.lock().expect("rooms lock");
            // Another join started the channel's call while this router was being made: that
            // one is the call, and this room is dropped unused.
            if let Some(other) = rooms.get(&channel) {
                return Ok(Arc::clone(other));
            }
            rooms.insert(channel, Arc::clone(&room));
        }
        // Volume events arrive on mediasoup's own threads; they are handed to a task that
        // owns the room and reports, so the callbacks stay quick and never block.
        let (speaking_tx, mut speaking_rx) = mpsc::unbounded_channel::<HashSet<Uuid>>();
        {
            let room = Arc::downgrade(&room);
            let tx = speaking_tx.clone();
            audio_observer
                .on_volumes(move |volumes| {
                    let Some(room) = room.upgrade() else {
                        return;
                    };
                    let owners = room.producer_owner.lock().expect("owner lock");
                    let loud: HashSet<Uuid> = volumes
                        .iter()
                        .filter_map(|v| owners.get(&v.producer.id()).copied())
                        .collect();
                    let _ = tx.send(loud);
                })
                .detach();
            let tx = speaking_tx;
            audio_observer
                .on_silence(move || {
                    let _ = tx.send(HashSet::new());
                })
                .detach();
        }
        {
            let rooms = Arc::clone(self);
            let room = Arc::downgrade(&room);
            tokio::spawn(async move {
                while let Some(loud) = speaking_rx.recv().await {
                    let Some(room) = room.upgrade() else {
                        break;
                    };
                    rooms.apply_speaking(&room, &loud).await;
                }
            });
        }
        self.reporter.report(VoiceReport::SessionStarted {
            server: self.server,
            session,
            channel,
        });
        info!(
            channel = channel.to_string(),
            session = session.to_string(),
            "call started"
        );
        Ok(room)
    }

    async fn apply_speaking(&self, room: &Room, loud: &HashSet<Uuid>) {
        let changes: Vec<(Uuid, bool)> = {
            let mut participants = room.participants.lock().expect("room lock");
            let mut changes = Vec::new();
            for participant in participants.values_mut() {
                let speaking = loud.contains(&participant.user) && !participant.silenced();
                if speaking != participant.speaking {
                    participant.speaking = speaking;
                    changes.push((participant.user, speaking));
                }
            }
            changes
        };
        for (user, speaking) in changes {
            room.broadcast(&ServerMessage::Speaking { user, speaking }, None);
            self.reporter.report(VoiceReport::Speaking {
                session: room.session,
                channel: room.channel,
                user,
                speaking,
            });
        }
    }

    pub async fn set_capabilities(&self, seat: Seat, capabilities: Value) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        let capabilities: RtpCapabilities = serde_json::from_value(capabilities)
            .map_err(|e| RoomError::BadParameters(format!("rtpCapabilities: {e}")))?;
        {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = seated_mut(&mut participants, seat)?;
            participant.rtp_capabilities = Some(capabilities);
        }
        self.ensure_consumers(&room, seat.user).await;
        Ok(())
    }

    pub async fn create_transport(
        self: &Arc<Self>,
        seat: Seat,
        direction: TransportDirection,
    ) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        if direction == TransportDirection::Send {
            // One send transport per participant: a second would hold more ports while the
            // first lives on in its producers. One closed for not connecting may be replaced.
            let participants = room.participants.lock().expect("room lock");
            if seated(&participants, seat)?.send_transport.is_some() {
                return Err(RoomError::BadParameters(
                    "the participant already has a send transport".to_string(),
                ));
            }
        }
        room.require(seat)?;
        let mut listen = ListenInfo {
            protocol: Protocol::Udp,
            ip: self.rtc_ip,
            announced_address: self.announced_address.clone(),
            expose_internal_ip: false,
            port: None,
            port_range: None,
            flags: None,
            send_buffer_size: None,
            recv_buffer_size: None,
        };
        let udp = listen.clone();
        listen.protocol = Protocol::Tcp;
        let mut options =
            WebRtcTransportOptions::new(WebRtcTransportListenInfos::new(udp).insert(listen));
        options.enable_udp = true;
        options.enable_tcp = true;
        options.prefer_udp = true;
        options.initial_available_outgoing_bitrate = INITIAL_OUTGOING_BITRATE;
        let transport = room.router.create_webrtc_transport(options).await?;
        self.expire_unconnected(&room, seat, Unconnected::WebRtc(transport.id()));
        let message = ServerMessage::TransportCreated {
            direction,
            id: transport.id().to_string(),
            ice_parameters: serde_json::to_value(transport.ice_parameters()).expect("serializes"),
            ice_candidates: serde_json::to_value(transport.ice_candidates()).expect("serializes"),
            dtls_parameters: serde_json::to_value(transport.dtls_parameters()).expect("serializes"),
        };
        {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = seated_mut(&mut participants, seat)?;
            match direction {
                TransportDirection::Send if participant.send_transport.is_some() => {
                    return Err(RoomError::BadParameters(
                        "the participant already has a send transport".to_string(),
                    ));
                }
                TransportDirection::Send => participant.send_transport = Some(transport),
                TransportDirection::Recv => {
                    participant.recv_transport = Some(ReceiveTransport::WebRtc(transport));
                }
            }
            participant.send(message);
        }
        if direction == TransportDirection::Recv {
            self.ensure_consumers(&room, seat.user).await;
        }
        Ok(())
    }

    /// Closes the transport `pending` names, after `CONNECT_TIMEOUT`, if it has not connected
    /// by then and is still the participant's.
    fn expire_unconnected(self: &Arc<Self>, room: &Arc<Room>, seat: Seat, pending: Unconnected) {
        let rooms = Arc::clone(self);
        let room = Arc::downgrade(room);
        tokio::spawn(async move {
            tokio::time::sleep(CONNECT_TIMEOUT).await;
            if let Some(room) = room.upgrade() {
                rooms.close_unconnected(&room, seat, pending).await;
            }
        });
    }

    /// Closes a transport of the participant `seat` made that has not connected, with what it
    /// carries: a send transport's producers, a receive transport's consumers (a new receive
    /// transport gets them again), or an external sender's producer. The client is told.
    async fn close_unconnected(&self, room: &Room, seat: Seat, pending: Unconnected) {
        let (closing, state) = {
            let mut participants = room.participants.lock().expect("room lock");
            let Ok(participant) = seated_mut(&mut participants, seat) else {
                return;
            };
            let mut closing: Vec<ProducerId> = Vec::new();
            match pending {
                Unconnected::WebRtc(id) => {
                    let unconnected = |t: &WebRtcTransport| {
                        t.id() == id && t.dtls_state() != DtlsState::Connected
                    };
                    if participant.send_transport.as_ref().is_some_and(unconnected) {
                        participant.send_transport = None;
                        // Every producer not fed over a plain transport is on the send one.
                        closing = participant
                            .producers
                            .keys()
                            .filter(|producer| !participant.rtp_transports.contains_key(producer))
                            .copied()
                            .collect();
                    } else if matches!(
                        &participant.recv_transport,
                        Some(ReceiveTransport::WebRtc(t)) if unconnected(t)
                    ) {
                        participant.recv_transport = None;
                        participant.consumers.clear();
                    } else {
                        return;
                    }
                }
                Unconnected::PlainReceive(id) => {
                    if matches!(
                        &participant.recv_transport,
                        Some(ReceiveTransport::Plain(t)) if t.id() == id && !heard_from(t)
                    ) {
                        participant.recv_transport = None;
                        participant.consumers.clear();
                    } else {
                        return;
                    }
                }
                Unconnected::PlainSend(producer) => {
                    if participant
                        .rtp_transports
                        .get(&producer)
                        .is_some_and(|t| !heard_from(t))
                    {
                        participant.rtp_transports.remove(&producer);
                        participant.own_preview.remove(&producer);
                        closing.push(producer);
                    } else {
                        return;
                    }
                }
            }
            let mut screen = false;
            let closing: Vec<Producer> = closing
                .into_iter()
                .filter_map(|id| participant.producers.remove(&id))
                .map(|(producer, source)| {
                    screen |= source == MediaSource::Screen;
                    producer
                })
                .collect();
            participant.send(ServerMessage::Error {
                detail: format!(
                    "a transport did not connect within {} seconds and was closed",
                    CONNECT_TIMEOUT.as_secs()
                ),
                fatal: false,
                retry_after_seconds: None,
                refused: None,
            });
            (
                closing,
                screen.then(|| Self::state_report(room, participant)),
            )
        };
        for producer in closing {
            self.drop_producer(room, producer).await;
        }
        if let Some(report) = state {
            self.reporter.report(report);
        }
    }

    fn transport(
        participant: &Participant,
        transport_id: &str,
    ) -> Result<WebRtcTransport, RoomError> {
        let receive = match &participant.recv_transport {
            Some(ReceiveTransport::WebRtc(transport)) => Some(transport),
            _ => None,
        };
        [participant.send_transport.as_ref(), receive]
            .into_iter()
            .flatten()
            .find(|t| t.id().to_string() == transport_id)
            .cloned()
            .ok_or(RoomError::UnknownTransport)
    }

    pub async fn connect_transport(
        &self,
        seat: Seat,
        transport_id: &str,
        dtls_parameters: Value,
    ) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        let dtls_parameters: DtlsParameters = serde_json::from_value(dtls_parameters)
            .map_err(|e| RoomError::BadParameters(format!("dtlsParameters: {e}")))?;
        let transport = {
            let participants = room.participants.lock().expect("room lock");
            Self::transport(seated(&participants, seat)?, transport_id)?
        };
        transport
            .connect(WebRtcTransportRemoteParameters { dtls_parameters })
            .await?;
        let participants = room.participants.lock().expect("room lock");
        if let Ok(participant) = seated(&participants, seat) {
            participant.send(ServerMessage::TransportConnected {
                transport_id: transport_id.to_string(),
            });
        }
        Ok(())
    }

    pub async fn produce(
        &self,
        seat: Seat,
        transport_id: &str,
        kind: WireKind,
        source: MediaSource,
        rtp_parameters: Value,
    ) -> Result<(), RoomError> {
        let user = seat.user;
        if kind != source.kind() {
            return Err(RoomError::WrongKind {
                of: source,
                kind,
                expected: source.kind(),
            });
        }
        let room = self.room(seat.channel)?;
        let rtp_parameters: RtpParameters = serde_json::from_value(rtp_parameters)
            .map_err(|e| RoomError::BadParameters(format!("rtpParameters: {e}")))?;
        let (transport, muted) = {
            let participants = room.participants.lock().expect("room lock");
            let participant = seated(&participants, seat)?;
            participant.ensure_source_free(source)?;
            (
                Self::transport(participant, transport_id)?,
                participant.silenced(),
            )
        };
        let mut options = ProducerOptions::new(media_kind(kind), rtp_parameters);
        let paused = muted && source == MediaSource::Microphone;
        options.paused = paused;
        let producer = transport.produce(options).await?;
        Self::observe_audio(&room, &producer, user).await;
        let producer_id = producer.id();
        let state = {
            let mut participants = room.participants.lock().expect("room lock");
            match seated_mut(&mut participants, seat).and_then(|participant| {
                participant.admit(source)?;
                Ok(participant)
            }) {
                Ok(participant) => {
                    participant
                        .producers
                        .insert(producer_id, (producer.clone(), source));
                    participant.send(ServerMessage::Produced {
                        producer_id: producer_id.to_string(),
                        source,
                    });
                    (source == MediaSource::Screen).then(|| Self::state_report(&room, participant))
                }
                Err(e) => {
                    drop(participants);
                    Self::refuse_producer(&room, producer);
                    return Err(e);
                }
            }
        };
        if source == MediaSource::Microphone {
            Self::settle_microphone(&room, seat, &producer, paused).await;
        }
        drop(producer);
        if let Some(report) = state {
            self.reporter.report(report);
        }
        let others: Vec<Uuid> = room
            .participants
            .lock()
            .expect("room lock")
            .keys()
            .copied()
            .filter(|other| *other != user)
            .collect();
        for other in others {
            self.ensure_consumers(&room, other).await;
        }
        Ok(())
    }

    /// Gives `user` a consumer for every producer of everyone else they do not consume yet,
    /// once they have a receive transport and have said what they can play.
    async fn ensure_consumers(&self, room: &Room, user: Uuid) {
        let (transport, capabilities, wanted) = {
            let participants = room.participants.lock().expect("room lock");
            let Some(participant) = participants.get(&user) else {
                return;
            };
            let (Some(transport), Some(capabilities)) =
                (&participant.recv_transport, &participant.rtp_capabilities)
            else {
                return;
            };
            let consumed: HashSet<ProducerId> = participant
                .consumers
                .values()
                .map(|(consumer, _)| consumer.producer_id())
                .collect();
            let wanted: Vec<(Uuid, Producer, MediaSource)> = participants
                .values()
                .flat_map(|other| {
                    other
                        .producers
                        .values()
                        .filter(|(producer, _)| {
                            (other.user != user || other.own_preview.contains(&producer.id()))
                                && !consumed.contains(&producer.id())
                        })
                        .map(|(producer, source)| (other.user, producer.clone(), *source))
                })
                .collect();
            (transport.clone(), capabilities.clone(), wanted)
        };
        for (owner, producer, source) in wanted {
            if !room.router.can_consume(&producer.id(), &capabilities) {
                continue;
            }
            let mut options = ConsumerOptions::new(producer.id(), capabilities.clone());
            options.paused = true;
            let consumer = match transport.consume(options).await {
                Ok(consumer) => consumer,
                Err(e) => {
                    warn!(error = e.to_string(), "consumer not created");
                    continue;
                }
            };
            let message = ServerMessage::NewConsumer {
                consumer_id: consumer.id().to_string(),
                producer_id: producer.id().to_string(),
                user: owner,
                kind: wire_kind(consumer.kind()),
                source,
                rtp_parameters: serde_json::to_value(consumer.rtp_parameters())
                    .expect("serializes"),
                producer_paused: consumer.producer_paused(),
            };
            let mut participants = room.participants.lock().expect("room lock");
            if let Some(participant) = participants.get_mut(&user) {
                participant
                    .consumers
                    .insert(consumer.id(), (consumer, owner));
                participant.send(message);
            }
        }
    }

    pub async fn resume_consumer(&self, seat: Seat, consumer_id: &str) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        let consumer = {
            let participants = room.participants.lock().expect("room lock");
            let participant = seated(&participants, seat)?;
            let consumer = participant
                .consumers
                .values()
                .find(|(consumer, _)| consumer.id().to_string() == consumer_id)
                .map(|(consumer, _)| consumer.clone())
                .ok_or(RoomError::UnknownConsumer)?;
            if participant.deafened && consumer.kind() == MediaKind::Audio {
                return Ok(());
            }
            consumer
        };
        consumer.resume().await.map_err(RoomError::from)
    }

    /// Closes a producer made for a participant who, by the time it was ready, was gone or no
    /// longer allowed it. No one consumes it yet.
    fn refuse_producer(room: &Room, producer: Producer) {
        room.producer_owner
            .lock()
            .expect("owner lock")
            .remove(&producer.id());
        drop(producer);
    }

    /// Brings a new microphone, made `paused` or not by the participant's mute as it stood
    /// before mediasoup made it, in line with their mute as it stands now: a mute, a
    /// moderator's above all, that arrived meanwhile found no microphone to pause. It checks
    /// again after each change, since another may have arrived during it.
    async fn settle_microphone(room: &Room, seat: Seat, producer: &Producer, mut paused: bool) {
        loop {
            let silenced = match seated(&room.participants.lock().expect("room lock"), seat) {
                Ok(participant) => participant.silenced(),
                Err(_) => return,
            };
            if silenced == paused {
                return;
            }
            let result = if silenced {
                producer.pause().await
            } else {
                producer.resume().await
            };
            if let Err(e) = result {
                warn!(error = e.to_string(), "microphone pause state not applied");
                return;
            }
            paused = silenced;
        }
    }

    /// Lets the room's audio level observer hear an audio producer, so its owner is reported
    /// speaking.
    async fn observe_audio(room: &Room, producer: &Producer, user: Uuid) {
        if producer.kind() != MediaKind::Audio {
            return;
        }
        room.producer_owner
            .lock()
            .expect("owner lock")
            .insert(producer.id(), user);
        if let Err(e) = room
            ._audio_observer
            .add_producer(RtpObserverAddProducerOptions::new(producer.id()))
            .await
        {
            warn!(
                error = e.to_string(),
                "audio producer not observed for speaking"
            );
        }
    }

    pub async fn close_producer(&self, seat: Seat, producer_id: &str) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        let (removed, state) = {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = seated_mut(&mut participants, seat)?;
            let id = participant
                .producers
                .keys()
                .find(|id| id.to_string() == producer_id)
                .copied()
                .ok_or(RoomError::UnknownProducer)?;
            let removed = participant.producers.remove(&id);
            if participant.own_preview.remove(&id) {
                participant.rtp_transports.remove(&id);
            }
            let state = removed
                .as_ref()
                .is_some_and(|(_, source)| *source == MediaSource::Screen)
                .then(|| Self::state_report(&room, participant));
            (removed.map(|(producer, _)| producer), state)
        };
        if let Some(producer) = removed {
            self.drop_producer(&room, producer).await;
        }
        if let Some(report) = state {
            self.reporter.report(report);
        }
        Ok(())
    }

    /// What the participant `seat` made may do in their call; nothing once they are gone.
    pub fn grants(&self, seat: Seat) -> Grants {
        self.room(seat.channel)
            .ok()
            .and_then(|room| {
                seated(&room.participants.lock().expect("room lock"), seat)
                    .ok()
                    .map(|p| p.grants)
            })
            .unwrap_or_default()
    }

    /// Whether a `setState` of `muted` and `deafened` from the participant `seat` made only
    /// silences them further: it mutes or deafens them and lifts neither. Such a frame is never
    /// rate limited, so nothing, a flood of others' frames included, can keep someone's
    /// microphone open after they asked to close it. Each one changes something and undoing it
    /// is limited, so they cannot come faster than the limits allow either.
    pub fn quietens(&self, seat: Seat, muted: bool, deafened: bool) -> bool {
        let Ok(room) = self.room(seat.channel) else {
            return false;
        };
        let participants = room.participants.lock().expect("room lock");
        seated(&participants, seat).is_ok_and(|p| {
            muted >= p.muted && deafened >= p.deafened && (muted, deafened) != (p.muted, p.deafened)
        })
    }

    /// Sets what `user` may do in `room`'s call, as the API server says: their producers of a
    /// source no longer allowed close, their offers are withdrawn if they may no longer offer
    /// files, and they are told. Nothing happens when nothing changed.
    async fn set_grants(&self, room: &Arc<Room>, user: Uuid, grants: Grants) {
        let (closing, state) = {
            let mut participants = room.participants.lock().expect("room lock");
            let Some(participant) = participants.get_mut(&user) else {
                return;
            };
            if participant.grants == grants {
                return;
            }
            participant.grants = grants;
            let refused: Vec<ProducerId> = participant
                .producers
                .iter()
                .filter(|(_, (_, source))| !grants.may_produce(*source))
                .map(|(id, _)| *id)
                .collect();
            let mut closing = Vec::new();
            let mut screen = false;
            for id in refused {
                if let Some((producer, source)) = participant.producers.remove(&id) {
                    if participant.own_preview.remove(&id) {
                        participant.rtp_transports.remove(&id);
                    }
                    screen |= source == MediaSource::Screen;
                    closing.push(producer);
                }
            }
            participant.send(ServerMessage::GrantsChanged { grants });
            (
                closing,
                screen.then(|| Self::state_report(room, participant)),
            )
        };
        for producer in closing {
            self.drop_producer(room, producer).await;
        }
        if let Some(report) = state {
            self.reporter.report(report);
        }
        if !grants.transfer_files {
            self.withdraw_offers_of(room.channel, user);
            self.end_transfers_sent_by(room, user).await;
        }
    }

    /// The participant's state as the API server records it.
    fn state_report(room: &Room, participant: &Participant) -> VoiceReport {
        VoiceReport::ParticipantState {
            session: room.session,
            channel: room.channel,
            user: participant.user,
            muted: participant.silenced(),
            deafened: participant.deafened,
            sharing_screen: participant.sharing_screen(),
        }
    }

    /// Closes a producer and every consumer of it, telling their owners.
    async fn drop_producer(&self, room: &Room, producer: Producer) {
        let id = producer.id();
        room.producer_owner.lock().expect("owner lock").remove(&id);
        let mut participants = room.participants.lock().expect("room lock");
        for participant in participants.values_mut() {
            let gone: Vec<ConsumerId> = participant
                .consumers
                .iter()
                .filter(|(_, (consumer, _))| consumer.producer_id() == id)
                .map(|(consumer_id, _)| *consumer_id)
                .collect();
            for consumer_id in gone {
                participant.consumers.remove(&consumer_id);
                participant.send(ServerMessage::ConsumerClosed {
                    consumer_id: consumer_id.to_string(),
                });
            }
        }
        drop(producer);
    }

    /// Applies the mute and deafen the participant asked for. A moderator's mute stands over
    /// their own: while it does, unmuting themself leaves their microphone paused.
    pub async fn set_state(
        &self,
        seat: Seat,
        muted: bool,
        deafened: bool,
    ) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        self.apply_state(&room, seat.user, Some(seat.connection), |participant| {
            participant.muted = muted;
            participant.deafened = deafened;
        })
        .await
    }

    /// Makes `change` to `user`'s mute or deafen and carries the result out: their microphones
    /// paused while they are silenced (`Participant::silenced`), their audio consumers while
    /// deafened, and everyone, the API server included, told. With a `connection`, only the
    /// participant that connection made is changed.
    async fn apply_state(
        &self,
        room: &Room,
        user: Uuid,
        connection: Option<Connection>,
        change: impl FnOnce(&mut Participant),
    ) -> Result<(), RoomError> {
        let (muted, deafened, microphones, consumers, report) = {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = participants
                .get_mut(&user)
                .filter(|p| connection.is_none_or(|c| p.connection == c))
                .ok_or(RoomError::NotInCall)?;
            change(participant);
            let (muted, deafened) = (participant.silenced(), participant.deafened);
            let microphones: Vec<Producer> = participant
                .producers
                .values()
                .filter(|(_, source)| *source == MediaSource::Microphone)
                .map(|(producer, _)| producer.clone())
                .collect();
            // Deafening silences what is heard; shared screens stay visible.
            let consumers: Vec<Consumer> = participant
                .consumers
                .values()
                .filter(|(consumer, _)| consumer.kind() == MediaKind::Audio)
                .map(|(consumer, _)| consumer.clone())
                .collect();
            (
                muted,
                deafened,
                microphones,
                consumers,
                Self::state_report(room, participant),
            )
        };
        for producer in &microphones {
            let result = if muted {
                producer.pause().await
            } else {
                producer.resume().await
            };
            if let Err(e) = result {
                warn!(error = e.to_string(), "microphone pause state not applied");
            }
            room.broadcast(
                &ServerMessage::ProducerPaused {
                    producer_id: producer.id().to_string(),
                    paused: muted,
                },
                Some(user),
            );
        }
        for consumer in &consumers {
            let result = if deafened {
                consumer.pause().await
            } else {
                consumer.resume().await
            };
            if let Err(e) = result {
                warn!(error = e.to_string(), "consumer pause state not applied");
            }
        }
        room.broadcast(
            &ServerMessage::ParticipantState {
                user,
                muted,
                deafened,
            },
            None,
        );
        self.reporter.report(report);
        if muted {
            self.apply_speaking(room, &HashSet::new()).await;
        }
        Ok(())
    }

    /// Takes the participant `seat` made out of their call, as their socket closes.
    pub async fn leave_seat(&self, seat: Seat) {
        self.leave(seat.channel, seat.user, Some(seat.connection), None)
            .await;
    }

    /// Takes `user` out of the call, telling them why if there is a reason, and ends the call
    /// when they were the last one in it. With a `connection`, only the participant that
    /// connection made is taken out, not one that has since replaced it.
    pub async fn leave(
        &self,
        channel: Uuid,
        user: Uuid,
        connection: Option<Connection>,
        reason: Option<KickReason>,
    ) {
        let Ok(room) = self.room(channel) else {
            return;
        };
        let (producers, empty) = {
            let mut participants = room.participants.lock().expect("room lock");
            if !participants
                .get(&user)
                .is_some_and(|p| connection.is_none_or(|c| p.connection == c))
            {
                return;
            }
            let Some(mut participant) = participants.remove(&user) else {
                return;
            };
            if let Some(reason) = reason {
                participant.send(ServerMessage::Kicked { reason });
            }
            let producers: Vec<Producer> = std::mem::take(&mut participant.producers)
                .into_values()
                .map(|(producer, _)| producer)
                .collect();
            let empty = participants.is_empty();
            if empty {
                room.closed.store(true, Ordering::Relaxed);
            }
            (producers, empty)
        };
        for producer in producers {
            self.drop_producer(&room, producer).await;
        }
        self.end_everything_of(&room, user).await;
        room.broadcast(&ServerMessage::ParticipantLeft { user }, None);
        self.reporter.report(VoiceReport::ParticipantLeft {
            session: room.session,
            channel,
            user,
        });
        info!(
            channel = channel.to_string(),
            user = user.to_string(),
            "left the call"
        );
        if empty {
            self.end_room(&room);
        }
    }

    /// Ends `room` if it holds no one and has not closed: one a refused join started.
    fn end_if_empty(&self, room: &Arc<Room>) {
        {
            let participants = room.participants.lock().expect("room lock");
            if !participants.is_empty() || room.closed.swap(true, Ordering::Relaxed) {
                return;
            }
        }
        self.end_room(room);
    }

    /// Reports the end of `room`, closed and empty, and lets whoever waits on it start the
    /// channel's next call.
    fn end_room(&self, room: &Arc<Room>) {
        // The room leaves the channel's place as its end is reported, so every snapshot either
        // lists it before its end or follows the end without it.
        self.reporter.report_with(|| {
            self.forget(room);
            vec![VoiceReport::SessionEnded {
                session: room.session,
                channel: room.channel,
            }]
        });
        room.ended.send_replace(true);
        info!(
            channel = room.channel.to_string(),
            session = room.session.to_string(),
            "call ended"
        );
    }

    /// A command from the API server, addressed by session.
    pub async fn command(&self, command: voice_protocol::control::VoiceCommand) {
        use voice_protocol::control::VoiceCommand;
        match command {
            VoiceCommand::Mute {
                session,
                user,
                muted,
            } => {
                // Every recheck sends the mute as it stands; one that changes nothing is let be.
                let Some(room) = self.room_of_session(session) else {
                    return;
                };
                let unchanged = room
                    .participants
                    .lock()
                    .expect("room lock")
                    .get(&user)
                    .is_none_or(|participant| participant.server_muted == muted);
                if !unchanged
                    && let Err(e) = self
                        .apply_state(&room, user, None, |participant| {
                            participant.server_muted = muted;
                        })
                        .await
                {
                    warn!(error = e.to_string(), "mute command not applied");
                }
            }
            VoiceCommand::Kick {
                session,
                user,
                reason,
            } => {
                if let Some(room) = self.room_of_session(session) {
                    let reason = reason.unwrap_or(KickReason::Kicked);
                    self.leave(room.channel, user, None, Some(reason)).await;
                }
            }
            VoiceCommand::Grant {
                session,
                user,
                grants,
            } => {
                if let Some(room) = self.room_of_session(session) {
                    self.set_grants(&room, user, grants).await;
                }
            }
            VoiceCommand::EndSignIns {
                session,
                user,
                ended,
                kept,
            } => {
                let Some(room) = self.room_of_session(session) else {
                    return;
                };
                let connection = room
                    .participants
                    .lock()
                    .expect("room lock")
                    .get(&user)
                    .filter(|participant| {
                        VoiceCommand::ends_sign_in(
                            ended.as_deref(),
                            kept.as_deref(),
                            participant.sign_in.as_deref(),
                        )
                    })
                    .map(|participant| participant.connection);
                if let Some(connection) = connection {
                    self.leave(
                        room.channel,
                        user,
                        Some(connection),
                        Some(KickReason::SignedOut),
                    )
                    .await;
                }
            }
            VoiceCommand::Close { session } => {
                if let Some(room) = self.room_of_session(session) {
                    info!(
                        channel = room.channel.to_string(),
                        session = session.to_string(),
                        "closing a call the API server records on another server"
                    );
                    self.close_room(&room, KickReason::ServerStopping).await;
                }
            }
        }
    }

    /// Takes everyone out of `room`, telling them why, which ends it.
    async fn close_room(&self, room: &Room, reason: KickReason) {
        let users: Vec<Uuid> = room
            .participants
            .lock()
            .expect("room lock")
            .keys()
            .copied()
            .collect();
        for user in users {
            self.leave(room.channel, user, None, Some(reason)).await;
        }
    }

    /// Ends every call, telling everyone the server is stopping.
    pub async fn shutdown(&self) {
        let rooms: Vec<Arc<Room>> = self
            .rooms
            .lock()
            .expect("rooms lock")
            .values()
            .cloned()
            .collect();
        for room in rooms {
            self.close_room(&room, KickReason::ServerStopping).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_user_holds_at_most_their_seats_and_gets_each_back_on_leaving() {
        let counts = Arc::new(SeatCounts::default());
        let (alice, bob) = (Uuid::now_v7(), Uuid::now_v7());
        let first = counts.claim(alice, 2).unwrap();
        let second = counts.claim(alice, 2).unwrap();
        assert!(counts.claim(alice, 2).is_none());
        assert!(counts.claim(bob, 2).is_some());
        drop(first);
        let third = counts.claim(alice, 2).unwrap();
        drop((second, third));
        assert!(counts.0.lock().unwrap().get(&alice).is_none());
    }
}
