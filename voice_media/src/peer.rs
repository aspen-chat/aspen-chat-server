//! A synthetic client: a str0m peer connection on a socket of its own, answering the server's
//! offers as a browser does, sending Opus- and H.264-shaped media on the sections offered for
//! it, and counting what arrives. The media threads' tests, the voice server's capacity
//! estimate, and the bench's simulated participants all run on it. It carries no real sound or
//! picture: an Opus frame is its size in filler, and a video frame its packets, an SPS and a
//! PPS leading each second's keyframe, as an encoder's do.
//!
//! How offers reach it and its answers go back is its user's: a test hands them over in
//! process, the bench over the voice server's signalling socket.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use str0m::change::SdpOffer;
use str0m::format::Codec;
use str0m::media::Mid;
use str0m::net::{Protocol, Receive, TcpType};
use str0m::rtp::{ExtensionValues, RtpWrite, SeqNo, Ssrc};
use str0m::{Candidate, Event as RtcEvent, Input, Output, Rtc};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, oneshot};

/// The largest RTP payload sent, below a typical path MTU.
pub const MAX_PAYLOAD: usize = 1100;

/// What a peer sends on one section.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Synthetic {
    /// Opus in 20 ms frames at `kbps`, each carrying `level` (RFC 6464 dBov, `None` sends none).
    Audio { kbps: f64, level: Option<i8> },
    /// H.264 at `kbps` and `fps`, a keyframe each second.
    Video { kbps: f64, fps: f64 },
}

impl Synthetic {
    /// A microphone at 64 kbit/s, loud enough to count as speaking.
    pub const VOICE: Synthetic = Synthetic::Audio {
        kbps: 64.0,
        level: Some(-20),
    };
    /// A shared screen at 2.5 Mbit/s and 30 frames a second.
    pub const SCREEN: Synthetic = Synthetic::Video {
        kbps: 2500.0,
        fps: 30.0,
    };

    /// Bytes a frame carries, and frames a second.
    pub fn frame(self) -> (usize, f64) {
        let (kbps, fps) = match self {
            Synthetic::Audio { kbps, .. } => (kbps, 50.0),
            Synthetic::Video { kbps, fps } => (kbps, fps),
        };
        (((kbps * 1000.0 / 8.0 / fps) as usize).max(1), fps)
    }

    /// Media packets a second: a keyframe adds an SPS and a PPS.
    pub fn packets_per_second(self) -> f64 {
        let (bytes, fps) = self.frame();
        let extra = if matches!(self, Synthetic::Video { .. }) {
            2.0
        } else {
            0.0
        };
        bytes.div_ceil(MAX_PAYLOAD) as f64 * fps + extra
    }
}

/// One packet a peer received, for a test that looks at each (`PeerOptions::detail`).
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

/// What a peer has done, counted as it goes.
#[derive(Debug, Default)]
pub struct Counters {
    pub connected: AtomicBool,
    pub received: AtomicU64,
    /// Media bytes written, RTP payloads alone.
    pub sent_bytes: AtomicU64,
    /// The latest round-trip time the server's receiver reports gave, in microseconds; zero
    /// before one has, or without `PeerOptions::stats`.
    pub rtt_micros: AtomicU64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PeerOptions {
    /// Reach the server over TCP alone, as a client whose network blocks UDP.
    pub tcp: bool,
    /// Hand every packet received to `Peer::received`, not only count it.
    pub detail: bool,
    /// Read str0m's statistics each second, for the round-trip time (`Counters::rtt_micros`).
    pub stats: bool,
}

enum ToPeer {
    Offer {
        sdp: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    Send {
        mid: String,
        media: Synthetic,
    },
    Stop {
        mid: String,
    },
    Ssrc {
        mid: String,
        reply: oneshot::Sender<Option<u32>>,
    },
}

/// A running peer; dropping it stops it.
pub struct Peer {
    commands: mpsc::UnboundedSender<ToPeer>,
    pub counters: Arc<Counters>,
    /// Every packet received, when the peer was made with `detail`.
    pub received: mpsc::UnboundedReceiver<Received>,
}

impl Peer {
    /// Starts a peer on loopback. It must run inside a Tokio runtime.
    pub fn start(options: PeerOptions) -> Self {
        let (commands, received_commands) = mpsc::unbounded_channel();
        let (received_tx, received) = mpsc::unbounded_channel();
        let counters = Arc::new(Counters::default());
        tokio::spawn(run(
            received_commands,
            options,
            Arc::clone(&counters),
            options.detail.then_some(received_tx),
        ));
        Self {
            commands,
            counters,
            received,
        }
    }

    /// The answer to `sdp`, an offer from the server, or why it could not be answered.
    pub async fn answer(&self, sdp: String) -> Result<String, String> {
        let (reply, answer) = oneshot::channel();
        self.commands
            .send(ToPeer::Offer { sdp, reply })
            .map_err(|_| "the peer has stopped".to_string())?;
        answer
            .await
            .map_err(|_| "the peer has stopped".to_string())?
    }

    /// Sends `media` on the section `mid` until stopped.
    pub fn send(&self, mid: &str, media: Synthetic) {
        let _ = self.commands.send(ToPeer::Send {
            mid: mid.into(),
            media,
        });
    }

    pub fn stop(&self, mid: &str) {
        let _ = self.commands.send(ToPeer::Stop { mid: mid.into() });
    }

    /// The SSRC the peer sends `mid` with.
    pub async fn ssrc(&self, mid: &str) -> Option<u32> {
        let (reply, ssrc) = oneshot::channel();
        self.commands
            .send(ToPeer::Ssrc {
                mid: mid.into(),
                reply,
            })
            .ok()?;
        ssrc.await.ok().flatten()
    }

    /// Every packet received since the last call, for a peer made with `detail`.
    pub fn drain(&mut self) -> Vec<Received> {
        let mut all = Vec::new();
        while let Ok(packet) = self.received.try_recv() {
            all.push(packet);
        }
        all
    }
}

struct Sending {
    mid: Mid,
    media: Synthetic,
    seq: u64,
    timestamp: u32,
    frame: u64,
    due: Instant,
}

/// A TCP stream to the server, and what has arrived on it not yet framed.
struct Framed {
    stream: tokio::net::TcpStream,
    peer: SocketAddr,
    read: Vec<u8>,
}

/// The whole RFC 4571 packets that arrive next; a closed stream gives none, for ever.
async fn read_framed(stream: &mut Option<Framed>) -> Vec<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let Some(framed) = stream.as_mut() else {
        return std::future::pending().await;
    };
    let mut buf = [0u8; 4096];
    loop {
        let mut packets = Vec::new();
        while framed.read.len() >= 2 {
            let size = u16::from_be_bytes([framed.read[0], framed.read[1]]) as usize;
            if framed.read.len() < 2 + size {
                break;
            }
            packets.push(framed.read[2..2 + size].to_vec());
            framed.read.drain(..2 + size);
        }
        if !packets.is_empty() {
            return packets;
        }
        match framed.stream.read(&mut buf).await {
            Ok(0) | Err(_) => return std::future::pending().await,
            Ok(n) => framed.read.extend_from_slice(&buf[..n]),
        }
    }
}

async fn run(
    mut commands: mpsc::UnboundedReceiver<ToPeer>,
    options: PeerOptions,
    counters: Arc<Counters>,
    received: Option<mpsc::UnboundedSender<Received>>,
) {
    let Ok(socket) = UdpSocket::bind("127.0.0.1:0").await else {
        return;
    };
    let Ok(local) = socket.local_addr() else {
        return;
    };
    // Over TCP: one stream to the server, opened with the first packet for it, and an active
    // candidate (port 9, as RFC 6544 has it) in place of the UDP one. Packets arrive at the
    // candidate str0m knows, whatever port the stream took.
    let tcp_local: SocketAddr = "127.0.0.1:9".parse().expect("an address");
    let mut stream: Option<Framed> = None;
    let mut rtc = Rtc::builder()
        .set_rtp_mode(true)
        .set_stats_interval(options.stats.then_some(Duration::from_secs(1)))
        .build(Instant::now());
    let candidate = if options.tcp {
        Candidate::builder()
            .tcp()
            .host(tcp_local)
            .tcptype(TcpType::Active)
            .build()
    } else {
        Candidate::host(local, "udp")
    };
    match candidate {
        Ok(candidate) => {
            rtc.add_local_candidate(candidate);
        }
        Err(e) => {
            tracing::warn!("peer candidate: {e}");
            return;
        }
    }
    let mut sending: Vec<Sending> = Vec::new();
    let filler: Arc<[u8]> = vec![0x5a_u8; MAX_PAYLOAD].into();
    let mut buf = vec![0u8; 2048];
    loop {
        let timeout = loop {
            match rtc.poll_output() {
                Ok(Output::Transmit(t)) => {
                    if t.proto == Protocol::Tcp {
                        if stream.is_none() {
                            let Ok(connected) = tokio::net::TcpStream::connect(t.destination).await
                            else {
                                continue;
                            };
                            stream = Some(Framed {
                                stream: connected,
                                peer: t.destination,
                                read: Vec::new(),
                            });
                        }
                        if let Some(framed) = stream.as_mut() {
                            use tokio::io::AsyncWriteExt;
                            let mut packet = (t.contents.len() as u16).to_be_bytes().to_vec();
                            packet.extend_from_slice(&t.contents);
                            let _ = framed.stream.write_all(&packet).await;
                        }
                    } else {
                        let _ = socket.send_to(&t.contents, t.destination).await;
                    }
                }
                Ok(Output::Timeout(at)) => break at,
                Ok(Output::Event(RtcEvent::Connected)) => {
                    counters.connected.store(true, Ordering::Relaxed);
                }
                Ok(Output::Event(RtcEvent::MediaEgressStats(stats))) => {
                    if let Some(rtt) = stats.rtt {
                        counters
                            .rtt_micros
                            .store(rtt.as_micros() as u64, Ordering::Relaxed);
                    }
                }
                Ok(Output::Event(RtcEvent::RtpPacket(packet))) => {
                    counters.received.fetch_add(1, Ordering::Relaxed);
                    if let Some(received) = &received {
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
                }
                Ok(Output::Event(_)) => {}
                Err(e) => {
                    tracing::warn!("peer: {e}");
                    return;
                }
            }
        };
        let due = sending.iter().map(|s| s.due).min();
        let wake = due.map_or(timeout, |due| due.min(timeout));
        tokio::select! {
            packets = read_framed(&mut stream), if stream.is_some() => {
                let Some(peer) = stream.as_ref().map(|s| s.peer) else { continue };
                for packet in packets {
                    let Ok(contents) = packet.as_slice().try_into() else { continue };
                    let input = Input::Receive(Instant::now(), Receive {
                        proto: Protocol::Tcp, source: peer, destination: tcp_local, contents,
                    });
                    if rtc.handle_input(input).is_err() {
                        return;
                    }
                }
            }
            read = socket.recv_from(&mut buf) => {
                let Ok((n, source)) = read else { continue };
                let Ok(contents) = buf[..n].try_into() else { continue };
                let input = Input::Receive(Instant::now(), Receive {
                    proto: Protocol::Udp, source, destination: local, contents,
                });
                if rtc.handle_input(input).is_err() {
                    return;
                }
            }
            command = commands.recv() => {
                let Some(command) = command else { return };
                match command {
                    ToPeer::Offer { sdp, reply } => {
                        let answer = SdpOffer::from_sdp_string(&sdp)
                            .map_err(|e| e.to_string())
                            .and_then(|offer| {
                                rtc.sdp_api().accept_offer(offer).map_err(|e| e.to_string())
                            })
                            .map(|answer| answer.to_sdp_string());
                        let _ = reply.send(answer);
                    }
                    ToPeer::Send { mid, media } => {
                        sending.retain(|s| s.mid.to_string() != mid);
                        sending.push(Sending {
                            mid: mid.as_str().into(),
                            media,
                            seq: 1,
                            timestamp: 0,
                            frame: 0,
                            due: Instant::now(),
                        });
                    }
                    ToPeer::Stop { mid } => sending.retain(|s| s.mid.to_string() != mid),
                    ToPeer::Ssrc { mid, reply } => {
                        let ssrc = rtc
                            .direct_api()
                            .stream_tx_by_mid(mid.as_str().into(), None)
                            .map(|s| *s.ssrc());
                        let _ = reply.send(ssrc);
                    }
                }
                // str0m schedules from its last input: a change (an answer given, say) is
                // acted on once it is given the time.
                if rtc.handle_input(Input::Timeout(Instant::now())).is_err() {
                    return;
                }
            }
            _ = tokio::time::sleep_until(wake.into()) => {
                let now = Instant::now();
                for s in &mut sending {
                    if s.due <= now {
                        let bytes = write_frame(&mut rtc, s, now, &filler);
                        counters.sent_bytes.fetch_add(bytes as u64, Ordering::Relaxed);
                    }
                }
                if rtc.handle_input(Input::Timeout(now)).is_err() {
                    return;
                }
            }
        }
    }
}

/// Writes the frame `s` is due to send; the payload bytes written.
fn write_frame(rtc: &mut Rtc, s: &mut Sending, now: Instant, filler: &Arc<[u8]>) -> usize {
    let (codec, clock) = match s.media {
        Synthetic::Audio { .. } => (Codec::Opus, 48_000),
        Synthetic::Video { .. } => (Codec::H264, 90_000),
    };
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
        return 0;
    };
    let (bytes, fps) = s.media.frame();
    let mut api = rtc.direct_api();
    let Some(tx) = api.stream_tx_by_mid(s.mid, None) else {
        return 0;
    };
    let (video, level) = match s.media {
        Synthetic::Audio { level, .. } => (false, level),
        Synthetic::Video { .. } => (true, None),
    };
    let keyframe = video && s.frame.is_multiple_of(fps as u64);
    let mut packet = |seq: &mut u64, first: u8, size: usize, marker: bool| {
        let mut payload = Vec::with_capacity(size);
        payload.push(first);
        payload.extend_from_slice(&filler[..size.max(1) - 1]);
        tx.write_rtp(
            RtpWrite::new(pt, SeqNo::from(*seq), s.timestamp, now, payload)
                .marker(marker)
                .ext_vals(ExtensionValues {
                    audio_level: level,
                    // str0m writes a level only with its voice activity flag, as browsers send.
                    voice_activity: level.map(|dbov| dbov > -50),
                    ..Default::default()
                })
                .nackable(video),
        );
        *seq += 1;
    };
    if keyframe {
        packet(&mut s.seq, 0x67, 16, false);
        packet(&mut s.seq, 0x68, 4, false);
    }
    let mut left = bytes;
    while left > 0 {
        let size = left.min(MAX_PAYLOAD);
        left -= size;
        let first = match (video, keyframe) {
            (false, _) => 0x78,
            (true, true) => 0x65,
            (true, false) => 0x41,
        };
        packet(&mut s.seq, first, size, video && left == 0);
    }
    s.timestamp = s.timestamp.wrapping_add(clock / fps as u32);
    s.frame += 1;
    s.due += Duration::from_secs_f64(1.0 / fps);
    // A peer that fell behind sends from now rather than in a burst.
    if s.due < now {
        s.due = now;
    }
    bytes
}
