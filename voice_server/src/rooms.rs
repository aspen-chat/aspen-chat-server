//! Calls as this server carries them. A room is one channel's call: a mediasoup router, an
//! audio level observer that drives speaking events, and the participants with their
//! transports, producers, and consumers. Rooms come into being with their first participant
//! and go with their last, and the API server hears about each step through the reporter.

use crate::reporter::Reporter;
use mediasoup::prelude::*;
use mediasoup::types::data_structures::TransportTuple;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::num::{NonZeroU8, NonZeroU16, NonZeroU32};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;
use voice_protocol::control::VoiceReport;
use voice_protocol::signal::{
    KickReason, MediaKind as WireKind, MediaSource, ParticipantInfo, ProducerInfo, ServerMessage,
    TransportDirection,
};

/// Volumes above this, in dBvo, count as speaking.
const SPEAKING_THRESHOLD_DBVO: i8 = -50;
/// How often the observer reports volumes; speaking flips at most this often.
const SPEAKING_INTERVAL_MS: u16 = 300;

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
    #[error("mediasoup refused: {0}")]
    Media(String),
    #[error("{0}")]
    BadParameters(String),
}

/// Where a participant's frames go: the writer half of their socket.
pub type Outbox = mpsc::UnboundedSender<ServerMessage>;

struct Participant {
    user: Uuid,
    outbox: Outbox,
    rtp_capabilities: Option<RtpCapabilities>,
    send_transport: Option<WebRtcTransport>,
    recv_transport: Option<WebRtcTransport>,
    /// The transports an external sender (the desktop shell's game capture) delivers SRTP to,
    /// by the producer each feeds.
    rtp_transports: HashMap<ProducerId, PlainTransport>,
    producers: HashMap<ProducerId, (Producer, MediaSource)>,
    /// Producers the participant also consumes themself, as a preview of what an external
    /// sender is delivering on their behalf.
    own_preview: HashSet<ProducerId>,
    /// By consumer id: the consumer and whose producer it carries.
    consumers: HashMap<ConsumerId, (Consumer, Uuid)>,
    muted: bool,
    deafened: bool,
    speaking: bool,
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

    fn sharing_screen(&self) -> bool {
        self.producers
            .values()
            .any(|(_, source)| *source == MediaSource::Screen)
    }

    fn info(&self) -> ParticipantInfo {
        ParticipantInfo {
            user: self.user,
            muted: self.muted,
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
        // A closed outbox means the socket is gone; its participant is removed on that path.
        let _ = self.outbox.send(message);
    }
}

pub struct Room {
    pub session: Uuid,
    pub channel: Uuid,
    router: Router,
    /// Held for its lifetime: dropping it stops the volume events.
    _audio_observer: AudioLevelObserver,
    participants: Mutex<HashMap<Uuid, Participant>>,
    /// Which user each audio producer belongs to, for the volume events.
    producer_owner: Mutex<HashMap<ProducerId, Uuid>>,
}

impl Room {
    fn broadcast(&self, message: &ServerMessage, except: Option<Uuid>) {
        for participant in self.participants.lock().expect("room lock").values() {
            if Some(participant.user) != except {
                participant.send(message.clone());
            }
        }
    }
}

/// Every call on this server.
pub struct Rooms {
    server: Uuid,
    workers: Vec<Worker>,
    next_worker: AtomicUsize,
    rtc_ip: IpAddr,
    announced_address: Option<String>,
    reporter: Reporter,
    rooms: Mutex<HashMap<Uuid, Arc<Room>>>,
}

fn wire_kind(kind: MediaKind) -> WireKind {
    match kind {
        MediaKind::Audio => WireKind::Audio,
        MediaKind::Video => WireKind::Video,
    }
}

fn media_kind(kind: WireKind) -> MediaKind {
    match kind {
        WireKind::Audio => MediaKind::Audio,
        WireKind::Video => MediaKind::Video,
    }
}

/// The codecs every router offers: Opus for voice and VP8 for shared screens and games, the
/// pair every browser and mediasoup client support.
fn h264_parameters() -> RtpCodecParametersParameters {
    let mut parameters = RtpCodecParametersParameters::default();
    parameters.insert("packetization-mode", 1u32);
    parameters.insert("profile-level-id", "42e01f");
    parameters.insert("level-asymmetry-allowed", 1u32);
    parameters
}

fn media_codecs() -> Vec<RtpCodecCapability> {
    vec![
        RtpCodecCapability::Audio {
            mime_type: MimeTypeAudio::Opus,
            preferred_payload_type: None,
            clock_rate: NonZeroU32::new(48_000).expect("non-zero"),
            channels: NonZeroU8::new(2).expect("non-zero"),
            parameters: RtpCodecParametersParameters::from([
                ("useinbandfec", 1_u32.into()),
                ("usedtx", 1_u32.into()),
            ]),
            rtcp_feedback: vec![RtcpFeedback::TransportCc],
        },
        // Browsers send their camera and screen video as VP8; see H.264 below for the order.
        RtpCodecCapability::Video {
            mime_type: MimeTypeVideo::Vp8,
            preferred_payload_type: None,
            clock_rate: NonZeroU32::new(90_000).expect("non-zero"),
            parameters: RtpCodecParametersParameters::default(),
            rtcp_feedback: vec![
                RtcpFeedback::Nack,
                RtcpFeedback::NackPli,
                RtcpFeedback::CcmFir,
                RtcpFeedback::GoogRemb,
                RtcpFeedback::TransportCc,
            ],
        },
        // H.264 constrained baseline is what the desktop shell's game capture sends; every
        // browser decodes it, Firefox through OpenH264. The level is ignored on matching. It
        // comes after VP8 because a browser producing video takes the router's first codec it
        // can send, and Chromium's H.264 encoder is the software OpenH264, which drops frames
        // on screen content where its VP8 encoder keeps up.
        RtpCodecCapability::Video {
            mime_type: MimeTypeVideo::H264,
            preferred_payload_type: None,
            clock_rate: NonZeroU32::new(90_000).expect("clock rate"),
            parameters: h264_parameters(),
            rtcp_feedback: vec![
                RtcpFeedback::Nack,
                RtcpFeedback::NackPli,
                RtcpFeedback::CcmFir,
                RtcpFeedback::GoogRemb,
                RtcpFeedback::TransportCc,
            ],
        },
    ]
}

impl Rooms {
    pub fn new(
        server: Uuid,
        workers: Vec<Worker>,
        rtc_ip: IpAddr,
        announced_address: Option<String>,
        reporter: Reporter,
    ) -> Arc<Self> {
        Arc::new(Self {
            server,
            workers,
            next_worker: AtomicUsize::new(0),
            rtc_ip,
            announced_address,
            reporter,
            rooms: Mutex::new(HashMap::new()),
        })
    }

    /// Everyone in every call, for the load report.
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

    fn room_of_session(&self, session: Uuid) -> Option<Arc<Room>> {
        self.rooms
            .lock()
            .expect("rooms lock")
            .values()
            .find(|room| room.session == session)
            .cloned()
    }

    /// Puts `user` in `channel`'s call, starting the call if it has none, and replies with
    /// `ready`. A user already in the call from another socket is replaced.
    pub async fn join(
        self: &Arc<Self>,
        channel: Uuid,
        user: Uuid,
        outbox: Outbox,
    ) -> Result<(), RoomError> {
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
        let replaced = {
            let mut participants = room.participants.lock().expect("room lock");
            let replaced = participants.remove(&user);
            if let Some(old) = &replaced {
                old.send(ServerMessage::Kicked {
                    reason: KickReason::Replaced,
                });
            }
            let others: Vec<ParticipantInfo> =
                participants.values().map(Participant::info).collect();
            let joined = ServerMessage::ParticipantJoined {
                user,
                muted: false,
                deafened: false,
            };
            for other in participants.values() {
                other.send(joined.clone());
            }
            let participant = Participant {
                user,
                outbox,
                rtp_capabilities: None,
                send_transport: None,
                recv_transport: None,
                rtp_transports: HashMap::new(),
                producers: HashMap::new(),
                own_preview: HashSet::new(),
                consumers: HashMap::new(),
                muted: false,
                deafened: false,
                speaking: false,
            };
            participant.send(ServerMessage::Ready {
                session: room.session,
                user,
                router_rtp_capabilities: serde_json::to_value(room.router.rtp_capabilities())
                    .expect("capabilities serialize"),
                participants: others,
            });
            participants.insert(user, participant);
            replaced.is_some()
        };
        if !replaced {
            self.reporter
                .report(VoiceReport::ParticipantJoined {
                    session: room.session,
                    user,
                })
                .await;
        }
        info!(
            channel = channel.to_string(),
            user = user.to_string(),
            "joined a call"
        );
        Ok(())
    }

    async fn start_room(self: &Arc<Self>, channel: Uuid) -> Result<Arc<Room>, RoomError> {
        let index = self.next_worker.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        let worker = &self.workers[index];
        let router = worker
            .create_router(RouterOptions::new(media_codecs()))
            .await
            .map_err(|e| RoomError::Media(e.to_string()))?;
        let audio_observer = router
            .create_audio_level_observer({
                let mut options = AudioLevelObserverOptions::default();
                options.max_entries = NonZeroU16::new(32).expect("non-zero");
                options.threshold = SPEAKING_THRESHOLD_DBVO;
                options.interval = SPEAKING_INTERVAL_MS;
                options
            })
            .await
            .map_err(|e| RoomError::Media(e.to_string()))?;
        let session = Uuid::now_v7();
        let room = Arc::new(Room {
            session,
            channel,
            router,
            _audio_observer: audio_observer.clone(),
            participants: Mutex::new(HashMap::new()),
            producer_owner: Mutex::new(HashMap::new()),
        });
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
        self.rooms
            .lock()
            .expect("rooms lock")
            .insert(channel, Arc::clone(&room));
        self.reporter
            .report(VoiceReport::SessionStarted {
                server: self.server,
                session,
                channel,
            })
            .await;
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
                let speaking = loud.contains(&participant.user) && !participant.muted;
                if speaking != participant.speaking {
                    participant.speaking = speaking;
                    changes.push((participant.user, speaking));
                }
            }
            changes
        };
        for (user, speaking) in changes {
            room.broadcast(&ServerMessage::Speaking { user, speaking }, None);
            self.reporter
                .report(VoiceReport::Speaking {
                    session: room.session,
                    user,
                    speaking,
                })
                .await;
        }
    }

    pub async fn set_capabilities(
        &self,
        channel: Uuid,
        user: Uuid,
        capabilities: Value,
    ) -> Result<(), RoomError> {
        let room = self.room(channel)?;
        let capabilities: RtpCapabilities = serde_json::from_value(capabilities)
            .map_err(|e| RoomError::BadParameters(format!("rtpCapabilities: {e}")))?;
        {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = participants.get_mut(&user).ok_or(RoomError::NotInCall)?;
            participant.rtp_capabilities = Some(capabilities);
        }
        self.ensure_consumers(&room, user).await;
        Ok(())
    }

    pub async fn create_transport(
        &self,
        channel: Uuid,
        user: Uuid,
        direction: TransportDirection,
    ) -> Result<(), RoomError> {
        let room = self.room(channel)?;
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
        let transport = room
            .router
            .create_webrtc_transport(options)
            .await
            .map_err(|e| RoomError::Media(e.to_string()))?;
        let message = ServerMessage::TransportCreated {
            direction,
            id: transport.id().to_string(),
            ice_parameters: serde_json::to_value(transport.ice_parameters()).expect("serializes"),
            ice_candidates: serde_json::to_value(transport.ice_candidates()).expect("serializes"),
            dtls_parameters: serde_json::to_value(transport.dtls_parameters()).expect("serializes"),
        };
        {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = participants.get_mut(&user).ok_or(RoomError::NotInCall)?;
            match direction {
                TransportDirection::Send => participant.send_transport = Some(transport),
                TransportDirection::Recv => participant.recv_transport = Some(transport),
            }
            participant.send(message);
        }
        if direction == TransportDirection::Recv {
            self.ensure_consumers(&room, user).await;
        }
        Ok(())
    }

    fn transport(
        participant: &Participant,
        transport_id: &str,
    ) -> Result<WebRtcTransport, RoomError> {
        [&participant.send_transport, &participant.recv_transport]
            .into_iter()
            .flatten()
            .find(|t| t.id().to_string() == transport_id)
            .cloned()
            .ok_or(RoomError::UnknownTransport)
    }

    pub async fn connect_transport(
        &self,
        channel: Uuid,
        user: Uuid,
        transport_id: &str,
        dtls_parameters: Value,
    ) -> Result<(), RoomError> {
        let room = self.room(channel)?;
        let dtls_parameters: DtlsParameters = serde_json::from_value(dtls_parameters)
            .map_err(|e| RoomError::BadParameters(format!("dtlsParameters: {e}")))?;
        let transport = {
            let participants = room.participants.lock().expect("room lock");
            let participant = participants.get(&user).ok_or(RoomError::NotInCall)?;
            Self::transport(participant, transport_id)?
        };
        transport
            .connect(WebRtcTransportRemoteParameters { dtls_parameters })
            .await
            .map_err(|e| RoomError::Media(e.to_string()))?;
        let participants = room.participants.lock().expect("room lock");
        if let Some(participant) = participants.get(&user) {
            participant.send(ServerMessage::TransportConnected {
                transport_id: transport_id.to_string(),
            });
        }
        Ok(())
    }

    pub async fn produce(
        &self,
        channel: Uuid,
        user: Uuid,
        transport_id: &str,
        kind: WireKind,
        source: MediaSource,
        rtp_parameters: Value,
    ) -> Result<(), RoomError> {
        let room = self.room(channel)?;
        let rtp_parameters: RtpParameters = serde_json::from_value(rtp_parameters)
            .map_err(|e| RoomError::BadParameters(format!("rtpParameters: {e}")))?;
        let (transport, muted) = {
            let participants = room.participants.lock().expect("room lock");
            let participant = participants.get(&user).ok_or(RoomError::NotInCall)?;
            participant.ensure_source_free(source)?;
            (
                Self::transport(participant, transport_id)?,
                participant.muted,
            )
        };
        let mut options = ProducerOptions::new(media_kind(kind), rtp_parameters);
        options.paused = muted && source == MediaSource::Microphone;
        let producer = transport
            .produce(options)
            .await
            .map_err(|e| RoomError::Media(e.to_string()))?;
        if producer.kind() == MediaKind::Audio {
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
        let producer_id = producer.id();
        let state = {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = participants.get_mut(&user).ok_or(RoomError::NotInCall)?;
            participant
                .producers
                .insert(producer_id, (producer, source));
            participant.send(ServerMessage::Produced {
                producer_id: producer_id.to_string(),
                source,
            });
            (source == MediaSource::Screen).then(|| Self::state_report(&room, participant))
        };
        if let Some(report) = state {
            self.reporter.report(report).await;
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

    pub async fn resume_consumer(
        &self,
        channel: Uuid,
        user: Uuid,
        consumer_id: &str,
    ) -> Result<(), RoomError> {
        let room = self.room(channel)?;
        let consumer = {
            let participants = room.participants.lock().expect("room lock");
            let participant = participants.get(&user).ok_or(RoomError::NotInCall)?;
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
        consumer
            .resume()
            .await
            .map_err(|e| RoomError::Media(e.to_string()))
    }

    /// Makes a producer fed by SRTP the client sends itself, on a plain transport that learns
    /// the sender's address from its first packet, and tells the client where to send. The
    /// client consumes the producer too, as its own preview.
    pub async fn produce_rtp(
        &self,
        channel: Uuid,
        user: Uuid,
        source: MediaSource,
    ) -> Result<(), RoomError> {
        let room = self.room(channel)?;
        {
            let participants = room.participants.lock().expect("room lock");
            participants
                .get(&user)
                .ok_or(RoomError::NotInCall)?
                .ensure_source_free(source)?;
        }
        let listen = ListenInfo {
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
        let mut options = PlainTransportOptions::new(listen);
        options.rtcp_mux = true;
        options.comedia = true;
        options.enable_srtp = true;
        options.srtp_crypto_suite = SrtpCryptoSuite::AesCm128HmacSha180;
        let transport = room
            .router
            .create_plain_transport(options)
            .await
            .map_err(|e| RoomError::Media(e.to_string()))?;
        let srtp = transport
            .srtp_parameters()
            .ok_or_else(|| RoomError::Media("no SRTP parameters".into()))?;
        // One key both ways: the sender encrypts with it, and decrypts the RTCP it gets back.
        transport
            .connect(PlainTransportRemoteParameters {
                ip: None,
                port: None,
                rtcp_port: None,
                srtp_parameters: Some(srtp.clone()),
            })
            .await
            .map_err(|e| RoomError::Media(e.to_string()))?;
        let ssrc = (Uuid::now_v7().as_u128() as u32) | 1;
        // Video is H.264 (the helper's x264), audio Opus (the helper's ffmpeg encoder); the
        // payload types are the producer's own and need only be distinct from each other.
        let (kind, payload_type, codec) = match source {
            MediaSource::ScreenAudio => (
                MediaKind::Audio,
                100,
                RtpCodecParameters::Audio {
                    mime_type: MimeTypeAudio::Opus,
                    payload_type: 100,
                    clock_rate: NonZeroU32::new(48_000).expect("clock rate"),
                    channels: NonZeroU8::new(2).expect("channels"),
                    // Stereo: consumers take their codec parameters from the producer, and
                    // a browser decodes Opus as mono unless they carry `sprop-stereo`.
                    parameters: RtpCodecParametersParameters::from([(
                        "sprop-stereo",
                        1_u32.into(),
                    )]),
                    rtcp_feedback: vec![],
                },
            ),
            _ => (
                MediaKind::Video,
                96,
                RtpCodecParameters::Video {
                    mime_type: MimeTypeVideo::H264,
                    payload_type: 96,
                    clock_rate: NonZeroU32::new(90_000).expect("clock rate"),
                    parameters: h264_parameters(),
                    rtcp_feedback: vec![
                        RtcpFeedback::Nack,
                        RtcpFeedback::NackPli,
                        RtcpFeedback::CcmFir,
                        RtcpFeedback::GoogRemb,
                    ],
                },
            ),
        };
        let rtp_parameters = RtpParameters {
            mid: None,
            msid: None,
            codecs: vec![codec],
            header_extensions: if kind == MediaKind::Video {
                vec![RtpHeaderExtensionParameters {
                    uri: RtpHeaderExtensionUri::AbsSendTime,
                    id: 4,
                    encrypt: false,
                }]
            } else {
                vec![]
            },
            encodings: vec![RtpEncodingParameters {
                ssrc: Some(ssrc),
                ..RtpEncodingParameters::default()
            }],
            rtcp: RtcpParameters {
                cname: Some(format!("aspen-{user}")),
                reduced_size: true,
            },
        };
        let producer = transport
            .produce(ProducerOptions::new(kind, rtp_parameters))
            .await
            .map_err(|e| RoomError::Media(e.to_string()))?;
        let (local_address, local_port) = match transport.tuple() {
            TransportTuple::WithRemote {
                local_address,
                local_port,
                ..
            }
            | TransportTuple::LocalOnly {
                local_address,
                local_port,
                ..
            } => (local_address, local_port),
        };
        let producer_id = producer.id();
        let state = {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = participants.get_mut(&user).ok_or(RoomError::NotInCall)?;
            participant
                .producers
                .insert(producer_id, (producer, source));
            // Only video is previewed back to the sender; their own audio would be an echo.
            if kind == MediaKind::Video {
                participant.own_preview.insert(producer_id);
            }
            participant.rtp_transports.insert(producer_id, transport);
            participant.send(ServerMessage::RtpProduced {
                producer_id: producer_id.to_string(),
                source,
                ip: local_address.to_string(),
                port: local_port,
                ssrc,
                payload_type,
                srtp_crypto_suite: "AES_CM_128_HMAC_SHA1_80".to_string(),
                srtp_key_base64: srtp.key_base64.clone(),
            });
            (source == MediaSource::Screen).then(|| Self::state_report(&room, participant))
        };
        if let Some(report) = state {
            self.reporter.report(report).await;
        }
        let everyone: Vec<Uuid> = room
            .participants
            .lock()
            .expect("room lock")
            .keys()
            .copied()
            .collect();
        for member in everyone {
            self.ensure_consumers(&room, member).await;
        }
        Ok(())
    }

    pub async fn close_producer(
        &self,
        channel: Uuid,
        user: Uuid,
        producer_id: &str,
    ) -> Result<(), RoomError> {
        let room = self.room(channel)?;
        let (removed, state) = {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = participants.get_mut(&user).ok_or(RoomError::NotInCall)?;
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
            self.reporter.report(report).await;
        }
        Ok(())
    }

    /// The participant's state as the API server records it.
    fn state_report(room: &Room, participant: &Participant) -> VoiceReport {
        VoiceReport::ParticipantState {
            session: room.session,
            user: participant.user,
            muted: participant.muted,
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

    /// Applies a mute or deafen, whether the participant asked or a moderator did.
    pub async fn set_state(
        &self,
        channel: Uuid,
        user: Uuid,
        muted: bool,
        deafened: bool,
    ) -> Result<(), RoomError> {
        let room = self.room(channel)?;
        let (microphones, consumers, report) = {
            let mut participants = room.participants.lock().expect("room lock");
            let participant = participants.get_mut(&user).ok_or(RoomError::NotInCall)?;
            participant.muted = muted;
            participant.deafened = deafened;
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
                microphones,
                consumers,
                Self::state_report(&room, participant),
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
        self.reporter.report(report).await;
        if muted {
            self.apply_speaking(&room, &HashSet::new()).await;
        }
        Ok(())
    }

    /// Takes `user` out of the call, telling them why if there is a reason, and ends the call
    /// when they were the last one in it.
    pub async fn leave(&self, channel: Uuid, user: Uuid, reason: Option<KickReason>) {
        let Ok(room) = self.room(channel) else {
            return;
        };
        let (removed, producers, empty) = {
            let mut participants = room.participants.lock().expect("room lock");
            let Some(participant) = participants.remove(&user) else {
                return;
            };
            if let Some(reason) = reason {
                participant.send(ServerMessage::Kicked { reason });
            }
            let producers: Vec<Producer> = participant
                .producers
                .into_values()
                .map(|(producer, _)| producer)
                .collect();
            (true, producers, participants.is_empty())
        };
        if !removed {
            return;
        }
        for producer in producers {
            self.drop_producer(&room, producer).await;
        }
        room.broadcast(&ServerMessage::ParticipantLeft { user }, None);
        self.reporter
            .report(VoiceReport::ParticipantLeft {
                session: room.session,
                user,
            })
            .await;
        info!(
            channel = channel.to_string(),
            user = user.to_string(),
            "left the call"
        );
        if empty {
            self.rooms.lock().expect("rooms lock").remove(&channel);
            self.reporter
                .report(VoiceReport::SessionEnded {
                    session: room.session,
                })
                .await;
            info!(
                channel = channel.to_string(),
                session = room.session.to_string(),
                "call ended"
            );
        }
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
                if let Some(room) = self.room_of_session(session) {
                    let deafened = room
                        .participants
                        .lock()
                        .expect("room lock")
                        .get(&user)
                        .map(|p| p.deafened);
                    if let Some(deafened) = deafened
                        && let Err(e) = self.set_state(room.channel, user, muted, deafened).await
                    {
                        warn!(error = e.to_string(), "mute command not applied");
                    }
                }
            }
            VoiceCommand::Kick { session, user } => {
                if let Some(room) = self.room_of_session(session) {
                    self.leave(room.channel, user, Some(KickReason::Kicked))
                        .await;
                }
            }
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
            let users: Vec<Uuid> = room
                .participants
                .lock()
                .expect("room lock")
                .keys()
                .copied()
                .collect();
            for user in users {
                self.leave(room.channel, user, Some(KickReason::ServerStopping))
                    .await;
            }
        }
    }
}
