//! Calls as this server carries them. A room is one channel's call: its participants, what each
//! may do and sends, and its media on one media thread (`voice_media`), which holds every
//! participant's peer connection and forwards between them. Rooms decide; the media thread does
//! what they ask and tells them what happened (`media_event`). Rooms come into being with their
//! first participant and go with their last, and the API server hears about each step through
//! the reporter. Files offered in a call, and the transfers between its participants, are the
//! room's too (`transfers`).

mod transfers;

use crate::reporter::Reports;
use crate::transfer::Relay;
use base64::Engine as _;
use rand::RngExt as _;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};
use uuid::Uuid;
use voice_media::{CallId, Command, Event, Failure, Media, ParticipantId};
use voice_protocol::control::{ParticipantSnapshot, REPORT_PARTITIONS, VoiceReport, partition};
use voice_protocol::signal::{
    ConnectionKind, KickReason, MediaSource, ParticipantInfo, RESUME_GRACE_SECONDS, ServerMessage,
    SourceInfo, WatchedVideo,
};
use voice_protocol::token::Grants;

/// How long a participant whose socket closed without `leave` stays in the call, waiting to be
/// resumed (`RESUME_GRACE_SECONDS`).
const RESUME_GRACE: Duration = Duration::from_secs(RESUME_GRACE_SECONDS);
/// Audio sections each participant receives, filled with whoever is loudest: past this many
/// speaking at once nobody follows anyway.
pub const AUDIO_SLOTS: u8 = 16;
/// Videos one participant may watch at once.
pub const MAX_WATCHED_VIDEO: usize = 25;

#[derive(Debug, thiserror::Error)]
pub enum RoomError {
    #[error("the participant is not in the call")]
    NotInCall,
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
    /// A resume whose secret names no participant waiting to be resumed: it was already used,
    /// or the grace ran out. The client joins again.
    #[error("there is nothing to resume; join the call again")]
    NothingToResume,
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

/// The participant one signalling socket made, or took back with `resume`: its call's channel,
/// its user, and its connection. Every frame from the socket acts through it, and is refused
/// (`NotInCall`) once that participant is gone, whether removed, replaced from another socket,
/// resumed from another, or in a call that has since ended and been followed by another, so a
/// socket can never act for someone else's place in a call, nor for its own once it has lost
/// it.
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

/// A source a participant asked to send.
#[derive(Clone, Copy, Debug)]
struct Sending {
    external: bool,
    /// Whether its media has arrived at the server, which is when the others are told of it.
    live: bool,
}

struct Participant {
    user: Uuid,
    connection: Connection,
    /// Its place on the call's media thread.
    media: ParticipantId,
    outbox: Outbox,
    /// The secret its socket may be resumed with, until it is used.
    resume: String,
    /// Whether its socket closed without `leave`, so that it waits to be resumed.
    detached: bool,
    sending: HashMap<MediaSource, Sending>,
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
/// `max_seats_per_user`.
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

/// A secret to resume a socket with: 256 random bits.
fn resume_secret() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 32]>())
}

impl Participant {
    /// Whether their microphone is paused: by their own choice or a moderator's. This is the
    /// mute everyone else, and the API server, is told of.
    fn silenced(&self) -> bool {
        self.muted || self.server_muted
    }

    fn sharing_screen(&self) -> bool {
        self.sending
            .get(&MediaSource::Screen)
            .is_some_and(|sending| sending.live)
    }

    /// What the others are told the participant sends: what has arrived at the server.
    fn sources(&self) -> Vec<SourceInfo> {
        let mut sources: Vec<SourceInfo> = self
            .sending
            .iter()
            .filter(|(_, sending)| sending.live)
            .map(|(source, _)| SourceInfo {
                source: *source,
                paused: *source == MediaSource::Microphone && self.silenced(),
            })
            .collect();
        sources.sort_by_key(|info| info.source as u8);
        sources
    }

    fn info(&self) -> ParticipantInfo {
        ParticipantInfo {
            user: self.user,
            muted: self.silenced(),
            deafened: self.deafened,
            speaking: self.speaking,
            sources: self.sources(),
        }
    }

    fn send(&self, message: ServerMessage) {
        self.outbox.send(&message);
    }
}

pub struct Room {
    pub session: Uuid,
    pub channel: Uuid,
    /// The media thread the call is on.
    thread: usize,
    /// Where the media thread tells of this call's participants (`media_event`).
    events: voice_media::Events,
    participants: Mutex<HashMap<Uuid, Participant>>,
    /// Whose place on the media thread each `ParticipantId` is.
    by_media: Mutex<HashMap<ParticipantId, Uuid>>,
    /// Set, under the participants lock, when the last participant leaves. A closed room
    /// takes no one in: whoever arrives next starts the channel's next call. It keeps the
    /// channel's place until its end is reported, so snapshots list it until then.
    closed: AtomicBool,
    /// Becomes true once the room's end has been reported and it has left the channel's place,
    /// so the next call's start is reported after it.
    ended: watch::Sender<bool>,
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

/// How a join found a room (`Rooms::enter`).
enum Entry {
    /// It has closed, and takes no one in.
    Closed,
    Entered {
        /// The media place of a participant of the same user it replaced from another socket.
        replaced: Option<(ParticipantId, bool)>,
    },
}

/// What a voice server carries at one moment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    pub rooms: usize,
    pub participants: usize,
    pub connections: usize,
    pub sections: usize,
}

/// Every call on this server.
pub struct Rooms {
    server: Uuid,
    media: Media,
    reporter: Reports,
    relay: Arc<Relay>,
    rooms: Mutex<HashMap<Uuid, Arc<Room>>>,
    /// Which participant each resume secret takes back: its channel and user.
    resumes: Mutex<HashMap<String, (Uuid, Uuid)>>,
    next_connection: AtomicU64,
    next_media: AtomicU64,
    seats: Arc<SeatCounts>,
    max_seats_per_user: usize,
    max_participants_per_call: usize,
    audio_slots: u8,
}

impl Rooms {
    pub fn new(
        server: Uuid,
        media: Media,
        reporter: Reports,
        relay: Arc<Relay>,
        limits: &crate::config::LimitSettings,
        audio_slots: u8,
    ) -> Arc<Self> {
        Arc::new(Self {
            server,
            media,
            reporter,
            relay,
            rooms: Mutex::new(HashMap::new()),
            resumes: Mutex::new(HashMap::new()),
            next_connection: AtomicU64::new(0),
            next_media: AtomicU64::new(1),
            seats: Arc::default(),
            max_seats_per_user: limits.max_seats_per_user,
            max_participants_per_call: limits.max_participants_per_call,
            audio_slots,
        })
    }

    /// Resolves once a media thread has stopped, which stops the server.
    pub async fn media_stopped(&self) {
        self.media.stopped().await;
    }

    /// How much this server is carrying, for its metrics: the rooms here, the rest from each
    /// media thread.
    pub async fn census(&self) -> Census {
        let mut census = Census {
            rooms: self.rooms.lock().expect("rooms lock").len(),
            ..Census::default()
        };
        for thread in 0..self.media.threads() {
            let (reply, answer) = tokio::sync::oneshot::channel();
            if !self.media.send(thread, Command::Census { reply }) {
                continue;
            }
            if let Ok(counted) = answer.await {
                census.participants += counted.participants;
                census.connections += counted.connections;
                census.sections += counted.sections;
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

    fn media(&self, room: &Room, command: Command) {
        if !self.media.send(room.thread, command) {
            warn!("a command for a media thread that has stopped");
        }
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
    /// (every lane's, even one with no calls). Made inside `Reports::report_with`, so it
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
                        sign_in: participant.sign_in.clone(),
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
    /// `ready`; the media thread then offers their connection. A user already in the call from
    /// another socket is replaced. Returns the seat the socket's frames act through. Refused
    /// when the user is already in `max_seats_per_user` other calls here, or the call holds
    /// `max_participants_per_call`. Every join is reported, a replacement's too, so the API
    /// server checks each against what its token was issued to.
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
                None => self.start_room(channel)?,
            };
            let replaced = match self.enter(&room, user, connection, &outbox, admission.clone()) {
                Ok(Entry::Entered { replaced }) => replaced,
                Ok(Entry::Closed) => {
                    // The room's last participant left as this one arrived. Once its end is
                    // reported and it has gone from the channel's place, this one starts the
                    // channel's next call.
                    let _ = room.ended.subscribe().wait_for(|ended| *ended).await;
                    continue;
                }
                Err(e) => {
                    // A room this join started holds no one; it ends as though its last
                    // participant left.
                    self.end_if_empty(&room);
                    return Err(e);
                }
            };
            // What the socket replaced had in the call goes as it does when a participant
            // leaves. The new socket's frames are read only once this returns, so nothing of
            // its own is caught up in it.
            let was_replaced = replaced.is_some();
            let mut stopped_speaking = false;
            if let Some((media, speaking)) = replaced {
                self.media(&room, Command::Leave { participant: media });
                room.by_media.lock().expect("media lock").remove(&media);
                room.broadcast(
                    &ServerMessage::Sources {
                        user,
                        sources: Vec::new(),
                    },
                    Some(user),
                );
                self.end_everything_of(&room, user).await;
                stopped_speaking = speaking;
            }
            self.reporter.report(VoiceReport::ParticipantJoined {
                session: room.session,
                channel,
                user,
                sign_in: admission.sign_in,
            });
            if stopped_speaking {
                self.tell_speaking(&room, user, false);
            }
            // The API server records a join unmuted, undeafened, and not sharing, and leaves
            // the record of someone already in the call as it is. So the state is reported
            // where either falls short: for a participant joining muted by a moderator (the
            // recheck that follows finds the mute already standing and changes nothing), and
            // for one who replaced another socket, whose record still describes that one.
            if was_replaced || admission.server_muted {
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
    }

    /// Adds the participant to `room`, replacing any of the same user from another socket,
    /// sends them `ready`, and seats them on the media thread. The one replaced is told and hung
    /// up here; its place on the media thread is handed back for `join` to end.
    fn enter(
        &self,
        room: &Room,
        user: Uuid,
        connection: Connection,
        outbox: &Outbox,
        admission: Admission,
    ) -> Result<Entry, RoomError> {
        let Admission {
            grants,
            sign_in,
            server_muted,
        } = admission;
        let mut participants = room.participants.lock().expect("room lock");
        if room.closed.load(Ordering::Relaxed) {
            return Ok(Entry::Closed);
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
        let replaced = participants.remove(&user).map(|old| {
            old.send(ServerMessage::Kicked {
                reason: KickReason::Replaced,
            });
            self.resumes
                .lock()
                .expect("resumes lock")
                .remove(&old.resume);
            (old.media, old.speaking)
        });
        let others: Vec<ParticipantInfo> = participants.values().map(Participant::info).collect();
        let joined = ServerMessage::ParticipantJoined {
            user,
            muted: server_muted,
            deafened: false,
        };
        for other in participants.values() {
            other.send(joined.clone());
        }
        let media = ParticipantId(self.next_media.fetch_add(1, Ordering::Relaxed));
        let resume = resume_secret();
        self.resumes
            .lock()
            .expect("resumes lock")
            .insert(resume.clone(), (room.channel, user));
        let participant = Participant {
            user,
            connection,
            media,
            outbox: outbox.clone(),
            resume: resume.clone(),
            detached: false,
            sending: HashMap::new(),
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
            participants: others,
            audio_slots: self.audio_slots,
            max_watched_video: MAX_WATCHED_VIDEO as u32,
            resume,
            offers: room.standing_offers(),
            links: room.links(),
            transfers: self.relay.policy(),
        });
        participants.insert(user, participant);
        room.by_media
            .lock()
            .expect("media lock")
            .insert(media, user);
        // Under the room's lock, so the offer the media thread makes follows `ready`.
        self.media(
            room,
            Command::Join {
                call: CallId(room.session),
                participant: media,
                user,
                events: room.events.clone(),
            },
        );
        Ok(Entry::Entered { replaced })
    }

    /// Takes back the participant `secret` was given to, for a socket that closed without
    /// `leave` within `RESUME_GRACE`: the new socket's outbox takes the old one's place, the
    /// participant is told the call as `ready` tells it, with a new secret, and each of its
    /// connections is restated (`Command::Restate`).
    pub fn resume(&self, secret: &str, outbox: Outbox) -> Result<Seat, RoomError> {
        let (channel, user) = self
            .resumes
            .lock()
            .expect("resumes lock")
            .remove(secret)
            .ok_or(RoomError::NothingToResume)?;
        let room = self.room(channel).map_err(|_| RoomError::NothingToResume)?;
        let connection = Connection(self.next_connection.fetch_add(1, Ordering::Relaxed));
        let mut participants = room.participants.lock().expect("room lock");
        let others: Vec<ParticipantInfo> = participants
            .values()
            .filter(|p| p.user != user)
            .map(Participant::info)
            .collect();
        let participant = participants
            .get_mut(&user)
            .filter(|p| p.resume == secret)
            .ok_or(RoomError::NothingToResume)?;
        // A socket still open for it (one its client gave up on) is let go.
        let old = std::mem::replace(&mut participant.outbox, outbox);
        old.hang_up();
        participant.connection = connection;
        participant.detached = false;
        participant.resume = resume_secret();
        self.resumes
            .lock()
            .expect("resumes lock")
            .insert(participant.resume.clone(), (channel, user));
        participant.send(ServerMessage::Ready {
            session: room.session,
            user,
            participants: others,
            audio_slots: self.audio_slots,
            max_watched_video: MAX_WATCHED_VIDEO as u32,
            resume: participant.resume.clone(),
            offers: room.standing_offers(),
            links: room.links(),
            transfers: self.relay.policy(),
        });
        self.media(
            &room,
            Command::Restate {
                participant: participant.media,
            },
        );
        info!(
            channel = channel.to_string(),
            user = user.to_string(),
            "resumed"
        );
        Ok(Seat {
            channel,
            user,
            connection,
        })
    }

    fn start_room(self: &Arc<Self>, channel: Uuid) -> Result<Arc<Room>, RoomError> {
        let session = Uuid::now_v7();
        let (events, received) = mpsc::unbounded_channel();
        let room = Arc::new(Room {
            session,
            channel,
            thread: self.media.assign(),
            events,
            participants: Mutex::new(HashMap::new()),
            by_media: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
            ended: watch::Sender::new(false),
            offers: Mutex::new(HashMap::new()),
            transfers: Mutex::new(HashMap::new()),
        });
        {
            let mut rooms = self.rooms.lock().expect("rooms lock");
            if let Some(other) = rooms.get(&channel) {
                return Ok(Arc::clone(other));
            }
            rooms.insert(channel, Arc::clone(&room));
        }
        tokio::spawn(Self::media_events(
            Arc::downgrade(self),
            Arc::downgrade(&room),
            received,
        ));
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

    /// What the media thread tells of a room's participants, in order, until the room goes.
    async fn media_events(
        rooms: Weak<Rooms>,
        room: Weak<Room>,
        mut received: mpsc::UnboundedReceiver<Event>,
    ) {
        while let Some(event) = received.recv().await {
            let (Some(rooms), Some(room)) = (rooms.upgrade(), room.upgrade()) else {
                return;
            };
            rooms.media_event(&room, event).await;
        }
    }

    async fn media_event(&self, room: &Room, event: Event) {
        let participant = match &event {
            Event::Offer { participant, .. }
            | Event::Sections { participant, .. }
            | Event::Connected { participant, .. }
            | Event::Failed { participant, .. }
            | Event::Live { participant, .. }
            | Event::Speaking { participant, .. }
            | Event::Slots { participant, .. } => *participant,
        };
        // Events of a participant that has since left or been replaced are let go.
        let Some(user) = room
            .by_media
            .lock()
            .expect("media lock")
            .get(&participant)
            .copied()
        else {
            return;
        };
        match event {
            Event::Offer {
                connection,
                revision,
                sdp,
                sections,
                ..
            } => self.send_to(
                room,
                user,
                ServerMessage::Offer {
                    connection,
                    revision,
                    sdp,
                    sections,
                },
            ),
            Event::Sections {
                connection,
                sections,
                ..
            } => self.send_to(
                room,
                user,
                ServerMessage::Sections {
                    connection,
                    sections,
                },
            ),
            Event::Slots { assignments, .. } => {
                self.send_to(room, user, ServerMessage::Slots { assignments })
            }
            Event::Connected { connection, .. } => {
                info!(user = user.to_string(), ?connection, "media connected");
            }
            Event::Failed {
                connection, reason, ..
            } => self.connection_failed(room, user, connection, reason).await,
            Event::Live { source, .. } => self.source_live(room, user, source),
            Event::Speaking { speaking, .. } => {
                let changed = {
                    let mut participants = room.participants.lock().expect("room lock");
                    let Some(p) = participants.get_mut(&user) else {
                        return;
                    };
                    let speaking = speaking && !p.silenced();
                    (p.speaking != speaking).then(|| {
                        p.speaking = speaking;
                        speaking
                    })
                };
                if let Some(speaking) = changed {
                    self.tell_speaking(room, user, speaking);
                }
            }
        }
    }

    fn send_to(&self, room: &Room, user: Uuid, message: ServerMessage) {
        if let Some(participant) = room.participants.lock().expect("room lock").get(&user) {
            participant.send(message);
        }
    }

    /// A source's first media arrived: it is sent on, and the others told, if its sender may
    /// still send it; otherwise it is stopped, the grant having gone while it was set up.
    fn source_live(&self, room: &Room, user: Uuid, source: MediaSource) {
        let (sources, report, refused) = {
            let mut participants = room.participants.lock().expect("room lock");
            let Some(p) = participants.get_mut(&user) else {
                return;
            };
            if !p.grants.may_send(source) {
                p.sending.remove(&source);
                (None, None, Some(p.media))
            } else {
                let Some(sending) = p.sending.get_mut(&source) else {
                    return;
                };
                sending.live = true;
                p.send(ServerMessage::Sending { source });
                (
                    Some(p.sources()),
                    (source == MediaSource::Screen).then(|| Self::state_report(room, p)),
                    None,
                )
            }
        };
        if let Some(media) = refused {
            self.media(
                room,
                Command::StopSending {
                    participant: media,
                    source,
                },
            );
            return;
        }
        if let Some(sources) = sources {
            room.broadcast(&ServerMessage::Sources { user, sources }, None);
        }
        if let Some(report) = report {
            self.reporter.report(report);
        }
    }

    /// A participant's connection failed at the media thread. Their call connection failing
    /// takes them out of the call, and their client rejoins; their capture connection failing
    /// stops what the game capture helper sends, and they are told.
    async fn connection_failed(
        &self,
        room: &Room,
        user: Uuid,
        connection: ConnectionKind,
        reason: Failure,
    ) {
        warn!(
            user = user.to_string(),
            ?connection,
            ?reason,
            "a peer connection failed"
        );
        match connection {
            ConnectionKind::Call => {
                self.leave(room.channel, user, None, Some(KickReason::ConnectionFailed))
                    .await;
            }
            ConnectionKind::Capture => {
                let external: Vec<MediaSource> = {
                    let participants = room.participants.lock().expect("room lock");
                    let Some(p) = participants.get(&user) else {
                        return;
                    };
                    p.send(ServerMessage::Error {
                        detail: "the game capture's connection to the voice server failed"
                            .to_string(),
                        fatal: false,
                        retry_after_seconds: None,
                        refused: None,
                    });
                    p.sending
                        .iter()
                        .filter(|(_, s)| s.external)
                        .map(|(source, _)| *source)
                        .collect()
                };
                for source in external {
                    self.stop_source(room, user, source);
                }
            }
        }
    }

    /// Tells everyone in the call, and the API server, that `user` started or stopped speaking.
    fn tell_speaking(&self, room: &Room, user: Uuid, speaking: bool) {
        room.broadcast(&ServerMessage::Speaking { user, speaking }, None);
        self.reporter.report(VoiceReport::Speaking {
            session: room.session,
            channel: room.channel,
            user,
            speaking,
        });
    }

    /// Asks for a section to send `source` on, which the media thread offers. A participant
    /// sends at most one of each source, so one client cannot multiply what everyone else in
    /// the call receives; a microphone starts paused while they are muted.
    pub fn send(&self, seat: Seat, source: MediaSource, external: bool) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        let mut participants = room.participants.lock().expect("room lock");
        let p = seated_mut(&mut participants, seat)?;
        if !p.grants.may_send(source) {
            return Err(RoomError::NotPermitted(source));
        }
        if p.sending.contains_key(&source) {
            return Err(RoomError::BadParameters(format!(
                "already sending {source:?}; stop it first"
            )));
        }
        p.sending.insert(
            source,
            Sending {
                external,
                live: false,
            },
        );
        let media = p.media;
        let paused = source == MediaSource::Microphone && p.silenced();
        self.media(
            &room,
            Command::Send {
                participant: media,
                source,
                external,
            },
        );
        if paused {
            self.media(
                &room,
                Command::SetSourcePaused {
                    participant: media,
                    source,
                    paused: true,
                },
            );
        }
        Ok(())
    }

    pub fn stop_sending(&self, seat: Seat, source: MediaSource) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        room.require(seat)?;
        self.stop_source(&room, seat.user, source);
        Ok(())
    }

    /// Stops what `user` sends of `source`, telling everyone when it was live.
    fn stop_source(&self, room: &Room, user: Uuid, source: MediaSource) {
        let (media, live, sources, report) = {
            let mut participants = room.participants.lock().expect("room lock");
            let Some(p) = participants.get_mut(&user) else {
                return;
            };
            let Some(sending) = p.sending.remove(&source) else {
                return;
            };
            (
                p.media,
                sending.live,
                p.sources(),
                (sending.live && source == MediaSource::Screen)
                    .then(|| Self::state_report(room, p)),
            )
        };
        self.media(
            room,
            Command::StopSending {
                participant: media,
                source,
            },
        );
        if live {
            room.broadcast(&ServerMessage::Sources { user, sources }, None);
        }
        if let Some(report) = report {
            self.reporter.report(report);
        }
    }

    /// Hands the participant's answer to the media thread.
    pub fn answer(
        &self,
        seat: Seat,
        connection: ConnectionKind,
        revision: u64,
        sdp: String,
    ) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        let media = seated(&room.participants.lock().expect("room lock"), seat)?.media;
        self.media(
            &room,
            Command::Answer {
                participant: media,
                connection,
                revision,
                sdp,
            },
        );
        Ok(())
    }

    pub fn watch(&self, seat: Seat, video: Vec<WatchedVideo>) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        let media = seated(&room.participants.lock().expect("room lock"), seat)?.media;
        self.media(
            &room,
            Command::Watch {
                participant: media,
                video: video.into_iter().map(|v| (v.user, v.source)).collect(),
            },
        );
        Ok(())
    }

    pub fn set_receiving(&self, seat: Seat, mid: String, receiving: bool) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        let media = seated(&room.participants.lock().expect("room lock"), seat)?.media;
        self.media(
            &room,
            Command::SetReceiving {
                participant: media,
                mid,
                receiving,
            },
        );
        Ok(())
    }

    pub fn restart_ice(&self, seat: Seat, connection: ConnectionKind) -> Result<(), RoomError> {
        let room = self.room(seat.channel)?;
        let media = seated(&room.participants.lock().expect("room lock"), seat)?.media;
        self.media(
            &room,
            Command::RestartIce {
                participant: media,
                connection,
            },
        );
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

    /// Sets what `user` may do in `room`'s call, as the API server says: their sources no
    /// longer allowed stop, their offers are withdrawn if they may no longer offer files, and
    /// they are told. Nothing happens when nothing changed.
    async fn set_grants(&self, room: &Arc<Room>, user: Uuid, grants: Grants) {
        let refused: Vec<MediaSource> = {
            let mut participants = room.participants.lock().expect("room lock");
            let Some(participant) = participants.get_mut(&user) else {
                return;
            };
            if participant.grants == grants {
                return;
            }
            participant.grants = grants;
            participant
                .sending
                .keys()
                .filter(|source| !grants.may_send(**source))
                .copied()
                .collect()
        };
        for source in refused {
            self.stop_source(room, user, source);
        }
        self.send_to(room, user, ServerMessage::GrantsChanged { grants });
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
    }

    /// Makes `change` to `user`'s mute or deafen and carries the result out: their microphone
    /// paused at the media thread while they are silenced (`Participant::silenced`), their
    /// audio stopped while deafened, and everyone, the API server included, told. With a
    /// `connection`, only the participant that connection made is changed.
    fn apply_state(
        &self,
        room: &Room,
        user: Uuid,
        connection: Option<Connection>,
        change: impl FnOnce(&mut Participant),
    ) -> Result<(), RoomError> {
        let (media, muted, deafened, quieted, sources, report) = {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = participants
                .get_mut(&user)
                .filter(|p| connection.is_none_or(|c| p.connection == c))
                .ok_or(RoomError::NotInCall)?;
            change(participant);
            let (muted, deafened) = (participant.silenced(), participant.deafened);
            // Someone silenced mid-word stops speaking now, not at the next interval.
            let quieted = muted && std::mem::take(&mut participant.speaking);
            (
                participant.media,
                muted,
                deafened,
                quieted,
                participant.sources(),
                Self::state_report(room, participant),
            )
        };
        self.media(
            room,
            Command::SetSourcePaused {
                participant: media,
                source: MediaSource::Microphone,
                paused: muted,
            },
        );
        self.media(
            room,
            Command::SetDeafened {
                participant: media,
                deafened,
            },
        );
        room.broadcast(
            &ServerMessage::ParticipantState {
                user,
                muted,
                deafened,
            },
            None,
        );
        room.broadcast(&ServerMessage::Sources { user, sources }, Some(user));
        self.reporter.report(report);
        if quieted {
            self.tell_speaking(room, user, false);
        }
        Ok(())
    }

    /// The socket of the participant `seat` made closed. With `leave` they go now; without,
    /// they stay in the call for `RESUME_GRACE`, media flowing, for the socket to be resumed,
    /// and go once it has passed.
    pub async fn socket_closed(self: &Arc<Self>, seat: Seat, left: bool) {
        if left {
            self.leave(seat.channel, seat.user, Some(seat.connection), None)
                .await;
            return;
        }
        let Ok(room) = self.room(seat.channel) else {
            return;
        };
        if let Ok(p) = seated_mut(&mut room.participants.lock().expect("room lock"), seat) {
            p.detached = true;
        } else {
            return;
        }
        let rooms = Arc::clone(self);
        tokio::spawn(async move {
            tokio::time::sleep(RESUME_GRACE).await;
            let still_detached = room
                .participants
                .lock()
                .expect("room lock")
                .get(&seat.user)
                .is_some_and(|p| p.connection == seat.connection && p.detached);
            if still_detached {
                rooms
                    .leave(seat.channel, seat.user, Some(seat.connection), None)
                    .await;
            }
        });
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
        let (media, empty) = {
            let mut participants = room.participants.lock().expect("room lock");
            if !participants
                .get(&user)
                .is_some_and(|p| connection.is_none_or(|c| p.connection == c))
            {
                return;
            }
            let Some(participant) = participants.remove(&user) else {
                return;
            };
            if let Some(reason) = reason {
                participant.send(ServerMessage::Kicked { reason });
            }
            self.resumes
                .lock()
                .expect("resumes lock")
                .remove(&participant.resume);
            let empty = participants.is_empty();
            if empty {
                room.closed.store(true, Ordering::Relaxed);
            }
            (participant.media, empty)
        };
        room.by_media.lock().expect("media lock").remove(&media);
        self.media(&room, Command::Leave { participant: media });
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
                    && let Err(e) = self.apply_state(&room, user, None, |participant| {
                        participant.server_muted = muted;
                    })
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
    use crate::config::{LimitSettings, TransferConfig};
    use crate::outbox::{OUTBOX_BYTES, OutboxReader};
    use futures_util::FutureExt;
    use std::net::{IpAddr, Ipv4Addr};
    use voice_media::peer::{Peer, PeerOptions, Synthetic};
    use voice_protocol::signal::SectionContent;

    const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);
    const EVERYTHING: Grants = Grants {
        speak: true,
        share_screen: true,
        camera: true,
        transfer_files: true,
    };
    const WAIT: Duration = Duration::from_secs(10);

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

    /// Rooms on a media thread of their own, media and STUN on loopback, and the reports they
    /// make, which go nowhere else.
    async fn rooms() -> (Arc<Rooms>, mpsc::UnboundedReceiver<VoiceReport>) {
        let media = Media::start(voice_media::MediaConfig {
            addresses: vec![voice_media::MediaAddress {
                ip: LOOPBACK,
                announced: None,
            }],
            first_port: 0,
            threads: 1,
            audio_slots: AUDIO_SLOTS,
            max_watched_video: MAX_WATCHED_VIDEO,
        })
        .expect("media threads start");
        let addresses = vec![crate::config::MediaAddress {
            ip: LOOPBACK,
            announced: None,
        }];
        let transfer = TransferConfig {
            relay_mbps: 0,
            port: 0,
            ..TransferConfig::default()
        };
        let relay = Relay::start(&transfer, &addresses).await.expect("relay");
        let (reports, made) = Reports::new();
        let rooms = Rooms::new(
            Uuid::now_v7(),
            media,
            reports,
            Arc::new(relay),
            &LimitSettings::built_in().expect("built-in limits"),
            AUDIO_SLOTS,
        );
        (rooms, made)
    }

    /// One socket's place in a call, what the server has sent it, and a peer that answers its
    /// offers.
    struct Client {
        seat: Seat,
        outbox: Outbox,
        frames: OutboxReader,
        peer: Peer,
        log: Vec<ServerMessage>,
    }

    impl Client {
        async fn join(rooms: &Arc<Rooms>, channel: Uuid, user: Uuid) -> Self {
            let (outbox, frames) = Outbox::new(OUTBOX_BYTES);
            let admission = Admission {
                grants: EVERYTHING,
                sign_in: None,
                server_muted: false,
            };
            let seat = rooms
                .join(channel, user, outbox.clone(), admission)
                .await
                .expect("joins");
            Self {
                seat,
                outbox,
                frames,
                peer: Peer::start(PeerOptions::default()),
                log: Vec::new(),
            }
        }

        /// Every frame sent to the socket since the last call, without waiting.
        fn sent(&mut self) -> Vec<ServerMessage> {
            let frames: Vec<ServerMessage> =
                std::iter::from_fn(|| self.frames.recv().now_or_never().flatten())
                    .map(|text| serde_json::from_str(&text).expect("a frame"))
                    .collect();
            self.log.extend(frames.iter().cloned());
            frames
        }

        /// Reads frames, answering each offer through the peer and sending `send` on the
        /// sections offered for it, until `want` matches one or `WAIT` passes.
        async fn until(
            &mut self,
            rooms: &Rooms,
            send: Option<(MediaSource, Synthetic)>,
            want: impl Fn(&ServerMessage) -> bool,
        ) -> Option<ServerMessage> {
            let deadline = tokio::time::Instant::now() + WAIT;
            loop {
                let text = tokio::time::timeout_at(deadline, self.frames.recv())
                    .await
                    .ok()??;
                let frame: ServerMessage = serde_json::from_str(&text).expect("a frame");
                self.log.push(frame.clone());
                if let ServerMessage::Offer {
                    connection,
                    revision,
                    sdp,
                    sections,
                } = &frame
                {
                    let answer = self.peer.answer(sdp.clone()).await.expect("an answer");
                    rooms
                        .answer(self.seat, *connection, *revision, answer)
                        .expect("answered");
                    for section in sections {
                        if let (SectionContent::Sending { source }, Some((wanted, media))) =
                            (&section.content, send)
                            && *source == wanted
                        {
                            self.peer.send(&section.mid, media);
                        }
                    }
                }
                if want(&frame) {
                    return Some(frame);
                }
            }
        }
    }

    fn made(reports: &mut mpsc::UnboundedReceiver<VoiceReport>) -> Vec<VoiceReport> {
        std::iter::from_fn(|| reports.try_recv().ok()).collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_source_is_told_of_once_its_media_arrives_and_stops_with_its_grant() {
        let (rooms, mut reports) = rooms().await;
        let channel = Uuid::now_v7();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let mut alice = Client::join(&rooms, channel, a).await;
        let mut bob = Client::join(&rooms, channel, b).await;
        rooms
            .send(alice.seat, MediaSource::Screen, false)
            .expect("may share");
        alice
            .until(
                &rooms,
                Some((MediaSource::Screen, Synthetic::SCREEN)),
                |f| {
                    matches!(
                        f,
                        ServerMessage::Sending {
                            source: MediaSource::Screen
                        }
                    )
                },
            )
            .await
            .expect("Alice's screen arrives at the server");
        bob.until(&rooms, None, |f| {
            matches!(f, ServerMessage::Sources { user, sources } if *user == a && !sources.is_empty())
        })
        .await
        .expect("Bob is told Alice shares her screen");
        let sharing = |reports: &[VoiceReport], sharing: bool| {
            reports.iter().any(|r| {
                matches!(r, VoiceReport::ParticipantState { user, sharing_screen, .. }
                    if *user == a && *sharing_screen == sharing)
            })
        };
        assert!(sharing(&made(&mut reports), true));

        // The grant taken away, the screen stops, and everyone hears so.
        let room = rooms.room(channel).unwrap();
        let grants = Grants {
            share_screen: false,
            ..EVERYTHING
        };
        rooms.set_grants(&room, a, grants).await;
        bob.until(&rooms, None, |f| {
            matches!(f, ServerMessage::Sources { user, sources } if *user == a && sources.is_empty())
        })
        .await
        .expect("Bob is told the share stopped");
        assert!(sharing(&made(&mut reports), false));
        assert!(matches!(
            rooms.send(alice.seat, MediaSource::Screen, false),
            Err(RoomError::NotPermitted(MediaSource::Screen))
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn muting_one_participant_leaves_the_others_speaking() {
        let (rooms, mut reports) = rooms().await;
        let channel = Uuid::now_v7();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let mut alice = Client::join(&rooms, channel, a).await;
        let bob = Client::join(&rooms, channel, b).await;
        let room = rooms.room(channel).unwrap();
        let media_of = |user: Uuid| room.participants.lock().unwrap()[&user].media;
        for (user, media) in [(a, media_of(a)), (b, media_of(b))] {
            rooms
                .media_event(
                    &room,
                    Event::Speaking {
                        participant: media,
                        speaking: true,
                    },
                )
                .await;
            assert!(room.participants.lock().unwrap()[&user].speaking);
        }
        alice.sent();
        made(&mut reports);

        rooms.set_state(bob.seat, true, false).await.unwrap();
        let speaking: Vec<(Uuid, bool)> = alice
            .sent()
            .into_iter()
            .filter_map(|frame| match frame {
                ServerMessage::Speaking { user, speaking } => Some((user, speaking)),
                _ => None,
            })
            .collect();
        assert_eq!(speaking, [(b, false)]);
        let reported: Vec<(Uuid, bool)> = made(&mut reports)
            .into_iter()
            .filter_map(|report| match report {
                VoiceReport::Speaking { user, speaking, .. } => Some((user, speaking)),
                _ => None,
            })
            .collect();
        assert_eq!(reported, [(b, false)]);
        // A muted participant the media thread hears is not speaking.
        rooms
            .media_event(
                &room,
                Event::Speaking {
                    participant: media_of(b),
                    speaking: true,
                },
            )
            .await;
        assert!(room.participants.lock().unwrap()[&a].speaking);
        assert!(!room.participants.lock().unwrap()[&b].speaking);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_participant_replaced_from_another_socket_leaves_nothing_behind() {
        let (rooms, mut reports) = rooms().await;
        let channel = Uuid::now_v7();
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        let mut desktop = Client::join(&rooms, channel, a).await;
        let mut bob = Client::join(&rooms, channel, b).await;
        rooms
            .send(desktop.seat, MediaSource::Microphone, false)
            .unwrap();
        desktop
            .until(
                &rooms,
                Some((MediaSource::Microphone, Synthetic::VOICE)),
                |f| matches!(f, ServerMessage::Sending { .. }),
            )
            .await
            .expect("the desktop's microphone is live");
        let offer = Uuid::now_v7();
        rooms
            .offer_file(desktop.seat, offer, "notes.txt".to_string(), 1, true, 60)
            .await
            .unwrap();
        bob.sent();
        made(&mut reports);

        let mut phone = Client::join(&rooms, channel, a).await;

        // The desktop is told and hung up, and its socket acts for no one.
        assert!(desktop.sent().contains(&ServerMessage::Kicked {
            reason: KickReason::Replaced
        }));
        assert!(desktop.outbox.hung_up().now_or_never().is_some());
        assert!(matches!(
            rooms.set_state(desktop.seat, false, false).await,
            Err(RoomError::NotInCall)
        ));
        rooms.socket_closed(desktop.seat, true).await;

        // Bob is told the desktop's sources and offer went with it.
        let told = bob.sent();
        assert!(told.contains(&ServerMessage::Sources {
            user: a,
            sources: Vec::new()
        }));
        assert!(told.contains(&ServerMessage::FileWithdrawn {
            offer,
            reason: voice_protocol::signal::OfferEnd::Left,
        }));
        // The phone is a bare place in the call, a connection of its own on the media thread.
        assert!(matches!(
            phone.sent().first(),
            Some(ServerMessage::Ready { user, .. }) if *user == a
        ));
        let room = rooms.room(channel).unwrap();
        assert!(room.offers.lock().unwrap().is_empty());
        assert_eq!(room.by_media.lock().unwrap().len(), 2);
        let census = rooms.census().await;
        assert_eq!((census.rooms, census.participants), (1, 2));
        let reported = made(&mut reports);
        let joined = reported
            .iter()
            .position(|r| matches!(r, VoiceReport::ParticipantJoined { user, .. } if *user == a))
            .expect("the join is reported");
        assert!(reported[joined..].iter().any(|r| matches!(r,
            VoiceReport::ParticipantState { user, muted: false, sharing_screen: false, .. } if *user == a)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_socket_that_closes_without_leaving_is_resumed_once_with_its_secret() {
        let (rooms, mut reports) = rooms().await;
        let channel = Uuid::now_v7();
        let a = Uuid::now_v7();
        let mut alice = Client::join(&rooms, channel, a).await;
        alice
            .until(&rooms, None, |f| matches!(f, ServerMessage::Offer { .. }))
            .await
            .expect("Alice is offered her connection");
        let Some(ServerMessage::Ready { resume, .. }) = alice.log.first().cloned() else {
            panic!("ready first");
        };
        made(&mut reports);

        // The socket closes without a word: Alice stays in the call, waiting.
        rooms.socket_closed(alice.seat, false).await;
        assert_eq!(rooms.participant_count(), 1);
        let (outbox, mut frames) = Outbox::new(OUTBOX_BYTES);
        let seat = rooms.resume(&resume, outbox).expect("resumes");
        assert!(matches!(
            rooms.set_state(alice.seat, true, false).await,
            Err(RoomError::NotInCall)
        ));
        rooms
            .set_state(seat, true, false)
            .await
            .expect("the new socket acts for her");
        let first: ServerMessage = serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
        let ServerMessage::Ready { resume: again, .. } = first else {
            panic!("ready on resuming, not {first:?}");
        };
        assert_ne!(again, resume, "a new secret each time");
        // Her connection is restated, from the media thread: its sections, its offer having
        // been answered.
        let restated = tokio::time::timeout(WAIT, async {
            loop {
                let frame: ServerMessage =
                    serde_json::from_str(&frames.recv().await.unwrap()).unwrap();
                if !matches!(frame, ServerMessage::ParticipantState { .. }) {
                    return frame;
                }
            }
        })
        .await
        .expect("a restatement");
        assert!(
            matches!(restated, ServerMessage::Sections { .. }),
            "{restated:?}"
        );
        // A secret is good once; nothing about her was reported as leaving or joining.
        assert!(matches!(
            rooms.resume(&resume, Outbox::new(OUTBOX_BYTES).0),
            Err(RoomError::NothingToResume)
        ));
        assert!(!made(&mut reports).iter().any(|r| matches!(
            r,
            VoiceReport::ParticipantLeft { .. } | VoiceReport::ParticipantJoined { .. }
        )));
    }
}
