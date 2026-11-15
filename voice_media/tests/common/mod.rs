//! Clients for the media threads' tests: each a str0m peer on a loopback socket, answering the
//! server's offers as a browser would, sending synthetic media, and reporting what it receives.

#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use str0m::change::SdpOffer;
use str0m::format::Codec;
use str0m::media::Mid;
use str0m::net::{Protocol, Receive};
use str0m::rtp::{ExtensionValues, RtpWrite, SeqNo, Ssrc};
use str0m::{Event as RtcEvent, Input, Output, Rtc};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
use voice_media::{CallId, Command, Event, Media, MediaAddress, MediaConfig, ParticipantId};
use voice_protocol::signal::{
    ConnectionKind, MediaSource, Section, SectionContent, SlotAssignment,
};

pub fn media(audio_slots: u8) -> Media {
    Media::start(MediaConfig {
        addresses: vec![MediaAddress {
            ip: "127.0.0.1".parse().unwrap(),
            announced: None,
        }],
        first_port: 0,
        threads: 1,
        audio_slots,
        max_watched_video: 25,
    })
    .expect("media threads start")
}

/// One packet a client received.
#[derive(Debug, Clone)]
pub struct Received {
    pub mid: String,
    pub ssrc: u32,
    pub seq: u64,
    pub timestamp: u32,
    pub csrc: Vec<u32>,
    pub audio_level: Option<i8>,
    pub first_byte: u8,
    pub at: Instant,
}

enum ToClient {
    Offer {
        sdp: String,
        reply: oneshot::Sender<String>,
    },
    /// Sends Opus-shaped packets at this level on the section `mid`, every 20 ms, until stopped.
    Audio {
        mid: String,
        level: Option<i8>,
    },
    /// Sends H.264-shaped frames on `mid` at 30 a second, a keyframe each second.
    Video {
        mid: String,
    },
    Stop {
        mid: String,
    },
    /// The SSRC the client sends `mid` with.
    Ssrc {
        mid: String,
        reply: oneshot::Sender<Option<u32>>,
    },
}

/// A participant in a test: its id on the media thread, its events, and its client.
pub struct Peer {
    pub id: ParticipantId,
    pub user: Uuid,
    pub events: mpsc::UnboundedReceiver<Event>,
    client: mpsc::UnboundedSender<ToClient>,
    pub received: mpsc::UnboundedReceiver<Received>,
    /// The sections of the latest offer.
    pub sections: Vec<Section>,
    pub slots: HashMap<u8, Option<Uuid>>,
    pub log: Vec<Event>,
}

impl Peer {
    pub fn join(media: &Media, call: CallId, id: u64) -> Self {
        let (events_tx, events) = mpsc::unbounded_channel();
        let (client, commands) = mpsc::unbounded_channel();
        let (received_tx, received) = mpsc::unbounded_channel();
        let user = Uuid::now_v7();
        tokio::spawn(run_client(commands, received_tx));
        assert!(media.send(
            0,
            Command::Join {
                call,
                participant: ParticipantId(id),
                user,
                events: events_tx,
            },
        ));
        Self {
            id: ParticipantId(id),
            user,
            events,
            client,
            received,
            sections: Vec::new(),
            slots: HashMap::new(),
            log: Vec::new(),
        }
    }

    /// Reads events, answering every offer as a browser would, until `want` matches one or
    /// `within` passes.
    pub async fn until(
        &mut self,
        media: &Media,
        within: Duration,
        want: impl Fn(&Event) -> bool,
    ) -> Option<Event> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            let event = tokio::time::timeout_at(deadline, self.events.recv())
                .await
                .ok()??;
            self.log.push(event.clone());
            match &event {
                Event::Offer {
                    connection,
                    revision,
                    sdp,
                    sections,
                    ..
                } => {
                    assert_eq!(*connection, ConnectionKind::Call);
                    self.sections = sections.clone();
                    let (reply, answer) = oneshot::channel();
                    self.client
                        .send(ToClient::Offer {
                            sdp: sdp.clone(),
                            reply,
                        })
                        .unwrap();
                    let answer = answer.await.expect("the client answers");
                    assert!(media.send(
                        0,
                        Command::Answer {
                            participant: self.id,
                            connection: *connection,
                            revision: *revision,
                            sdp: answer,
                        },
                    ));
                }
                Event::Slots { assignments, .. } => {
                    for SlotAssignment { slot, user } in assignments {
                        self.slots.insert(*slot, *user);
                    }
                }
                _ => {}
            }
            if want(&event) {
                return Some(event);
            }
        }
    }

    /// Settles whatever offers are outstanding.
    pub async fn settle(&mut self, media: &Media) {
        self.until(media, Duration::from_millis(400), |_| false)
            .await;
    }

    pub fn mid_of(&self, content: &SectionContent) -> Option<String> {
        self.sections
            .iter()
            .find(|s| &s.content == content)
            .map(|s| s.mid.clone())
    }

    pub fn send_audio(&self, mid: &str, level: Option<i8>) {
        self.client
            .send(ToClient::Audio {
                mid: mid.into(),
                level,
            })
            .unwrap();
    }

    pub fn send_video(&self, mid: &str) {
        self.client
            .send(ToClient::Video { mid: mid.into() })
            .unwrap();
    }

    pub fn stop(&self, mid: &str) {
        self.client
            .send(ToClient::Stop { mid: mid.into() })
            .unwrap();
    }

    pub async fn ssrc(&self, mid: &str) -> Option<u32> {
        let (reply, ssrc) = oneshot::channel();
        self.client
            .send(ToClient::Ssrc {
                mid: mid.into(),
                reply,
            })
            .unwrap();
        ssrc.await.ok().flatten()
    }

    /// Every packet received so far.
    pub fn drain(&mut self) -> Vec<Received> {
        let mut all = Vec::new();
        while let Ok(packet) = self.received.try_recv() {
            all.push(packet);
        }
        all
    }
}

pub fn send(media: &Media, command: Command) {
    assert!(media.send(0, command));
}

struct Sending {
    mid: Mid,
    video: bool,
    level: Option<i8>,
    seq: u64,
    timestamp: u32,
    frame: u64,
    due: Instant,
}

async fn run_client(
    mut commands: mpsc::UnboundedReceiver<ToClient>,
    received: mpsc::UnboundedSender<Received>,
) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let local: SocketAddr = socket.local_addr().unwrap();
    let mut rtc = Rtc::builder()
        .set_rtp_mode(true)
        .set_stats_interval(None)
        .build(Instant::now());
    rtc.add_local_candidate(str0m::Candidate::host(local, "udp").unwrap());
    let mut sending: Vec<Sending> = Vec::new();
    let filler: Arc<[u8]> = vec![0x5a_u8; 1100].into();
    let mut buf = vec![0u8; 2048];
    loop {
        let timeout = loop {
            match rtc.poll_output() {
                Ok(Output::Transmit(t)) => {
                    let _ = socket.send_to(&t.contents, t.destination).await;
                }
                Ok(Output::Timeout(at)) => break at,
                Ok(Output::Event(RtcEvent::RtpPacket(packet))) => {
                    let ssrc = *packet.header.ssrc;
                    let mid = rtc
                        .direct_api()
                        .stream_rx(&Ssrc::from(ssrc))
                        .map(|s| s.mid().to_string())
                        .unwrap_or_default();
                    let _ = received.send(Received {
                        mid,
                        ssrc,
                        seq: *packet.seq_no,
                        timestamp: packet.header.timestamp,
                        csrc: packet.header.csrc[..packet.header.csrc_count].to_vec(),
                        audio_level: packet.header.ext_vals.audio_level,
                        first_byte: packet.payload.first().copied().unwrap_or_default(),
                        at: Instant::now(),
                    });
                }
                Ok(Output::Event(_)) => {}
                Err(e) => panic!("client: {e}"),
            }
        };
        let due = sending.iter().map(|s| s.due).min();
        let wake = due.map_or(timeout, |d| d.min(timeout));
        tokio::select! {
            read = socket.recv_from(&mut buf) => {
                let Ok((n, source)) = read else { continue };
                let input = Input::Receive(Instant::now(), Receive {
                    proto: Protocol::Udp, source, destination: local,
                    contents: buf[..n].try_into().unwrap(),
                });
                rtc.handle_input(input).unwrap();
            }
            command = commands.recv() => {
                let Some(command) = command else { return };
                match command {
                    ToClient::Offer { sdp, reply } => {
                        let offer = SdpOffer::from_sdp_string(&sdp).expect("an offer str0m reads");
                        let answer = rtc.sdp_api().accept_offer(offer).expect("an offer str0m accepts");
                        let _ = reply.send(answer.to_sdp_string());
                    }
                    ToClient::Audio { mid, level } => {
                        sending.retain(|s| s.mid.to_string() != mid);
                        sending.push(Sending { mid: mid.as_str().into(), video: false, level, seq: 1, timestamp: 0, frame: 0, due: Instant::now() });
                    }
                    ToClient::Video { mid } => {
                        sending.retain(|s| s.mid.to_string() != mid);
                        sending.push(Sending { mid: mid.as_str().into(), video: true, level: None, seq: 1, timestamp: 0, frame: 0, due: Instant::now() });
                    }
                    ToClient::Stop { mid } => sending.retain(|s| s.mid.to_string() != mid),
                    ToClient::Ssrc { mid, reply } => {
                        let ssrc = rtc.direct_api().stream_tx_by_mid(mid.as_str().into(), None).map(|s| *s.ssrc());
                        let _ = reply.send(ssrc);
                    }
                }
            }
            _ = tokio::time::sleep_until(wake.into()) => {
                let now = Instant::now();
                for s in &mut sending {
                    if s.due > now { continue }
                    write_frame(&mut rtc, s, now, &filler);
                }
                rtc.handle_input(Input::Timeout(now)).unwrap();
            }
        }
    }
}

fn write_frame(rtc: &mut Rtc, s: &mut Sending, now: Instant, filler: &Arc<[u8]>) {
    let codec = if s.video { Codec::H264 } else { Codec::Opus };
    // A payload type the server offered for this section, as a browser chooses one.
    let offered: Vec<_> = rtc
        .media(s.mid)
        .map(|m| m.remote_pts().to_vec())
        .unwrap_or_default();
    let Some(pt) = rtc
        .codec_config()
        .params()
        .iter()
        .find(|p| p.spec().codec == codec && offered.contains(&p.pt()))
        .map(|p| p.pt())
    else {
        return;
    };
    let mut api = rtc.direct_api();
    let Some(tx) = api.stream_tx_by_mid(s.mid, None) else {
        return;
    };
    let mut packet = |seq: &mut u64, first: u8, size: usize, marker: bool, level: Option<i8>| {
        let mut payload = vec![first];
        payload.extend_from_slice(&filler[..size - 1]);
        tx.write_rtp(
            RtpWrite::new(pt, SeqNo::from(*seq), s.timestamp, now, payload)
                .marker(marker)
                .ext_vals(ExtensionValues {
                    audio_level: level,
                    // str0m writes a level only with its voice activity flag, as browsers send.
                    voice_activity: level.map(|dbov| dbov > -50),
                    ..Default::default()
                })
                .nackable(s.video),
        );
        *seq += 1;
    };
    if s.video {
        let keyframe = s.frame.is_multiple_of(30);
        if keyframe {
            packet(&mut s.seq, 0x67, 16, false, None);
            packet(&mut s.seq, 0x68, 4, false, None);
        }
        for i in 0..3 {
            packet(
                &mut s.seq,
                if keyframe { 0x65 } else { 0x41 },
                1000,
                i == 2,
                None,
            );
        }
        s.timestamp = s.timestamp.wrapping_add(3000);
        s.due += Duration::from_millis(33);
    } else {
        packet(&mut s.seq, 0x78, 160, false, s.level);
        s.timestamp = s.timestamp.wrapping_add(960);
        s.due += Duration::from_millis(20);
    }
    s.frame += 1;
    if s.due < now {
        s.due = now;
    }
}

/// The sending section of `source` in an offer's sections.
pub fn sending(source: MediaSource) -> SectionContent {
    SectionContent::Sending { source }
}
