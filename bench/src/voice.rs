//! Simulated call participants.
//!
//! A simulated participant joins a call the way the reference client does (a join offer from the
//! API server, a latency probe of every candidate, the signalling socket to the nearest), but
//! moves its media over plain SRTP rather than WebRTC: it sends its microphone, and its screen
//! when it shares one, with `produceRtp`, and receives everyone else's with `consumeRtp`. That
//! exercises what a voice server spends its capacity on (receiving, SRTP decryption,
//! forwarding to every other participant, encryption, sending) without a browser's ICE and DTLS,
//! whose cost is paid once per call.
//!
//! Microphone packets are Opus-sized every 20 ms at the behaviour's bitrate; a shared screen is
//! 30 frames a second of H.264-shaped NAL units at its bitrate, a keyframe every two seconds and
//! whenever the server asks for one. The payloads are filler: the server forwards media without
//! decoding it.
//!
//! Measured: `voice:join` (join offer to the voice server's `ready`), `voice:probe` (the health
//! probes), `voice:jitter` (RFC 3550 interarrival jitter of each received stream, every second),
//! `voice:rtt` (from the server's receiver reports on what the participant sends), and the counters
//! `voice_packets_expected` and `voice_packets_received`, whose ratio is the loss.

use crate::stats::Recorder;
use base64::Engine;
use bytes::BytesMut;
use futures_util::{SinkExt, StreamExt};
use rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtcp::receiver_report::ReceiverReport;
use rtcp::sender_report::SenderReport;
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use voice_protocol::signal::{ClientMessage, MediaSource, ServerMessage};
use webrtc_srtp::context::Context;
use webrtc_srtp::protection_profile::ProtectionProfile;
use webrtc_util::marshal::{Marshal, Unmarshal};

const AUDIO_FRAME: Duration = Duration::from_millis(20);
const VIDEO_FPS: u32 = 30;
const KEYFRAME_EVERY: Duration = Duration::from_secs(2);
const MAX_PAYLOAD: usize = 1100;
const REPORT_EVERY: Duration = Duration::from_secs(1);

/// One call to make.
pub struct CallPlan {
    pub seconds: f64,
    pub screen: bool,
    pub audio_bitrate: u32,
    pub screen_bitrate: u32,
}

fn srtp(key_base64: &str) -> Result<Context, String> {
    let material = base64::engine::general_purpose::STANDARD
        .decode(key_base64)
        .map_err(|e| format!("SRTP key is not base64: {e}"))?;
    if material.len() != 30 {
        return Err("SRTP keying material must be 30 bytes".into());
    }
    Context::new(
        &material[..16],
        &material[16..],
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .map_err(|e| format!("SRTP context: {e}"))
}

/// RTP and RTCP share a port (RFC 5761): RTCP packet types are 192 to 223, which read as
/// payload types 64 to 95.
fn is_rtcp(packet: &[u8]) -> bool {
    packet.len() > 1 && (64..=95).contains(&(packet[1] & 0x7f))
}

/// The middle 32 bits of the NTP time now, as receiver reports quote it.
fn ntp_now() -> (u64, u32) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let ntp = ((now.as_secs() + 2_208_988_800) << 32)
        | ((u64::from(now.subsec_nanos()) << 32) / 1_000_000_000);
    (ntp, (ntp >> 16) as u32)
}

type Signal = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    WsMessage,
>;

async fn send(sink: &mut Signal, message: &ClientMessage) -> Result<(), String> {
    let text = serde_json::to_string(message).map_err(|e| e.to_string())?;
    sink.send(WsMessage::Text(text.into()))
        .await
        .map_err(|e| e.to_string())
}

/// Picks the candidate that answers its health probe soonest, as the reference client does.
async fn nearest(http: &reqwest::Client, offer: &Value, recorder: &Recorder) -> Option<String> {
    let candidates: Vec<String> = offer["candidates"]
        .as_array()?
        .iter()
        .filter_map(|c| c["url"].as_str().map(str::to_string))
        .collect();
    let probes = candidates.iter().map(|url| async move {
        let started = Instant::now();
        let ok = http
            .get(format!("{}/health", url.trim_end_matches('/')))
            .timeout(Duration::from_secs(3))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        (url.clone(), ok.then(|| started.elapsed()))
    });
    let results = futures_util::future::join_all(probes).await;
    for (_, elapsed) in &results {
        if let Some(elapsed) = elapsed {
            recorder.latency("voice:probe", *elapsed);
        }
    }
    results
        .into_iter()
        .filter_map(|(url, elapsed)| elapsed.map(|e| (url, e)))
        .min_by_key(|(_, e)| *e)
        .map(|(url, _)| url)
}

/// Runs one call from a join offer until it has lasted `plan.seconds` or `stop` is set.
pub async fn run_call(
    http: reqwest::Client,
    offer: Value,
    plan: CallPlan,
    recorder: Recorder,
    mut stop: watch::Receiver<bool>,
    started: Instant,
) -> Result<(), String> {
    let server = nearest(&http, &offer, &recorder)
        .await
        .ok_or("no voice server answered its probe")?;
    let token = offer["token"]
        .as_str()
        .ok_or("the offer has no token")?
        .to_string();
    let url = format!(
        "{}/ws",
        server
            .trim_end_matches('/')
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1)
    );
    let (socket, _) = tokio_tungstenite::connect_async(url.as_str())
        .await
        .map_err(|e| e.to_string())?;
    let (mut sink, mut source) = socket.split();
    let (frames_tx, mut frames) = mpsc::unbounded_channel::<ServerMessage>();
    let reader = tokio::spawn(async move {
        while let Some(Ok(frame)) = source.next().await {
            if let WsMessage::Text(text) = frame
                && let Ok(message) = serde_json::from_str::<ServerMessage>(&text)
                && frames_tx.send(message).is_err()
            {
                break;
            }
        }
    });
    let expect = async |frames: &mut mpsc::UnboundedReceiver<ServerMessage>,
                        what: &str,
                        pick: &(dyn Fn(&ServerMessage) -> bool + Sync)|
           -> Result<ServerMessage, String> {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let message = tokio::time::timeout_at(deadline, frames.recv())
                .await
                .map_err(|_| format!("no {what} from the voice server"))?
                .ok_or("the voice server closed the socket")?;
            if let ServerMessage::Error {
                detail,
                fatal: true,
            } = &message
            {
                return Err(detail.clone());
            }
            if pick(&message) {
                return Ok(message);
            }
        }
    };
    send(&mut sink, &ClientMessage::Identify { token }).await?;
    let ServerMessage::Ready {
        router_rtp_capabilities,
        ..
    } = expect(&mut frames, "ready", &|m| {
        matches!(m, ServerMessage::Ready { .. })
    })
    .await?
    else {
        unreachable!()
    };
    recorder.latency("voice:join", started.elapsed());
    recorder.count("voice_calls", 1);
    send(
        &mut sink,
        &ClientMessage::SetCapabilities {
            rtp_capabilities: router_rtp_capabilities,
        },
    )
    .await?;

    // Receiving.
    send(&mut sink, &ClientMessage::ConsumeRtp).await?;
    let ServerMessage::RtpConsuming {
        ip,
        port,
        srtp_key_base64,
        ..
    } = expect(&mut frames, "rtpConsuming", &|m| {
        matches!(m, ServerMessage::RtpConsuming { .. })
    })
    .await?
    else {
        unreachable!()
    };
    let receive = Arc::new(bind(&ip, port).await?);
    let clocks: Arc<std::sync::Mutex<HashMap<u32, u32>>> = Arc::default();
    let receiving = tokio::spawn(receive_media(
        Arc::clone(&receive),
        srtp(&srtp_key_base64)?,
        srtp(&srtp_key_base64)?,
        Arc::clone(&clocks),
        recorder.clone(),
    ));

    // Sending: the microphone, and a screen.
    let mut senders = Vec::new();
    for (source, bitrate) in [
        (MediaSource::Microphone, plan.audio_bitrate),
        (MediaSource::Screen, plan.screen_bitrate),
    ] {
        if source == MediaSource::Screen && !plan.screen {
            continue;
        }
        send(&mut sink, &ClientMessage::ProduceRtp { source }).await?;
        let ServerMessage::RtpProduced {
            ip,
            port,
            ssrc,
            payload_type,
            srtp_key_base64,
            ..
        } = expect(
            &mut frames,
            "rtpProduced",
            &|m| matches!(m, ServerMessage::RtpProduced { source: s, .. } if *s == source),
        )
        .await?
        else {
            unreachable!()
        };
        let socket = Arc::new(bind(&ip, port).await?);
        senders.push(tokio::spawn(send_media(
            socket,
            srtp(&srtp_key_base64)?,
            srtp(&srtp_key_base64)?,
            ssrc,
            payload_type,
            source,
            bitrate,
            recorder.clone(),
        )));
    }

    // Keep the call going: resume consumers as they are announced, until the time is up.
    let end = Instant::now() + Duration::from_secs_f64(plan.seconds);
    loop {
        tokio::select! {
            message = frames.recv() => match message {
                Some(ServerMessage::NewConsumer { consumer_id, kind, rtp_parameters, .. }) => {
                    if let Some(ssrc) = rtp_parameters["encodings"][0]["ssrc"].as_u64() {
                        let rate = if kind == voice_protocol::signal::MediaKind::Audio { 48_000 } else { 90_000 };
                        clocks.lock().expect("clocks lock").insert(ssrc as u32, rate);
                    }
                    send(&mut sink, &ClientMessage::ResumeConsumer { consumer_id }).await?;
                }
                Some(ServerMessage::Error { detail, fatal: true }) => return Err(detail),
                Some(ServerMessage::Kicked { .. }) | None => break,
                Some(_) => {}
            },
            _ = tokio::time::sleep_until(end) => break,
            _ = stop.changed() => break,
        }
    }
    let _ = send(&mut sink, &ClientMessage::Leave).await;
    let _ = sink.close().await;
    for sender in senders {
        sender.abort();
    }
    receiving.abort();
    reader.abort();
    Ok(())
}

/// Aborts a task when dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn bind(ip: &str, port: u16) -> Result<UdpSocket, String> {
    let remote: SocketAddr = format!("{ip}:{port}")
        .parse()
        .or_else(|_| format!("[{ip}]:{port}").parse())
        .map_err(|_| format!("{ip}:{port} is not an address"))?;
    let local = if remote.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(local).await.map_err(|e| e.to_string())?;
    socket.connect(remote).await.map_err(|e| e.to_string())?;
    Ok(socket)
}

/// Per received stream: RFC 3550 sequence and jitter bookkeeping.
#[derive(Default)]
struct StreamStats {
    base: Option<u32>,
    highest: u32,
    cycles: u32,
    received: u64,
    /// Expected and received at the last report, for per-interval counts.
    reported_expected: u64,
    reported_received: u64,
    transit: Option<u32>,
    jitter: f64,
}

impl StreamStats {
    fn packet(&mut self, sequence: u16, timestamp: u32, arrival_units: i64) {
        let sequence = u32::from(sequence);
        match self.base {
            None => {
                self.base = Some(sequence);
                self.highest = sequence;
            }
            Some(_) => {
                let highest16 = self.highest & 0xffff;
                if sequence < highest16 && highest16 - sequence > 0x8000 {
                    self.cycles += 1 << 16;
                    self.highest = sequence;
                } else if sequence > highest16 && sequence - highest16 < 0x8000 {
                    self.highest = sequence;
                }
            }
        }
        self.received += 1;
        // Timestamps are 32-bit and wrap (a forwarding server may start them anywhere), so
        // transit and its change are taken modulo 2^32, as RFC 3550 does.
        let transit = (arrival_units as u32).wrapping_sub(timestamp);
        if let Some(previous) = self.transit {
            let d = f64::from((transit.wrapping_sub(previous) as i32).unsigned_abs());
            self.jitter += (d - self.jitter) / 16.0;
        }
        self.transit = Some(transit);
    }

    fn expected(&self) -> u64 {
        self.base.map_or(0, |base| {
            u64::from(self.cycles + self.highest) - u64::from(base) + 1
        })
    }
}

async fn receive_media(
    socket: Arc<UdpSocket>,
    mut rtp_srtp: Context,
    mut rtcp_srtp: Context,
    clocks: Arc<std::sync::Mutex<HashMap<u32, u32>>>,
    recorder: Recorder,
) {
    // The server learns where to send from this socket's first packet.
    let mut hello = tokio::time::interval(REPORT_EVERY);
    let epoch = Instant::now();
    let mut streams: HashMap<u32, StreamStats> = HashMap::new();
    let mut buffer = vec![0u8; 2048];
    loop {
        tokio::select! {
            _ = hello.tick() => {
                let report = ReceiverReport { ssrc: 0x5a5a_0001, ..Default::default() };
                if let Ok(raw) = report.marshal()
                    && let Ok(encrypted) = rtcp_srtp.encrypt_rtcp(&raw)
                {
                    let _ = socket.send(&encrypted).await;
                }
                for (ssrc, stats) in &mut streams {
                    let rate = clocks.lock().expect("clocks lock").get(ssrc).copied().unwrap_or(48_000);
                    if stats.received > 1 {
                        recorder.latency_micros("voice:jitter", (stats.jitter / f64::from(rate) * 1e6) as u64);
                    }
                    let expected = stats.expected();
                    recorder.count("voice_packets_expected", expected.saturating_sub(stats.reported_expected));
                    recorder.count("voice_packets_received", stats.received.saturating_sub(stats.reported_received));
                    stats.reported_expected = expected;
                    stats.reported_received = stats.received;
                }
            }
            received = socket.recv(&mut buffer) => {
                let Ok(length) = received else { return };
                let packet = &buffer[..length];
                if is_rtcp(packet) {
                    let _ = rtcp_srtp.decrypt_rtcp(packet);
                    continue;
                }
                let Ok(plain) = rtp_srtp.decrypt_rtp(packet) else {
                    recorder.count("voice_decrypt_failures", 1);
                    continue;
                };
                let Ok(parsed) = rtp::packet::Packet::unmarshal(&mut plain.clone()) else { continue };
                let ssrc = parsed.header.ssrc;
                // Only consumers announced with `newConsumer` carry media; the server's
                // bandwidth probes (SSRC 1234) and retransmissions do not count.
                let Some(rate) = clocks.lock().expect("clocks lock").get(&ssrc).copied() else {
                    continue;
                };
                let arrival = (epoch.elapsed().as_secs_f64() * f64::from(rate)) as i64;
                streams.entry(ssrc).or_default().packet(parsed.header.sequence_number, parsed.header.timestamp, arrival);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn send_media(
    socket: Arc<UdpSocket>,
    mut rtp_srtp: Context,
    rtcp_srtp: Context,
    ssrc: u32,
    payload_type: u8,
    source: MediaSource,
    bitrate: u32,
    recorder: Recorder,
) {
    let video = source == MediaSource::Screen;
    let (period, clock_step) = if video {
        (Duration::from_secs(1) / VIDEO_FPS, 90_000 / VIDEO_FPS)
    } else {
        (AUDIO_FRAME, 960)
    };
    let frame_bytes =
        (u64::from(bitrate) / 8 * period.as_micros() as u64 / 1_000_000).max(20) as usize;
    let keyframe_wanted = Arc::new(AtomicBool::new(true));
    let sent_packets = Arc::new(AtomicU32::new(0));
    // Feedback from the server: receiver reports for the round trip, keyframe requests. The
    // guard ends it when this sender is aborted, so nothing outlives the call.
    let _feedback = AbortOnDrop({
        let socket = Arc::clone(&socket);
        let keyframe_wanted = Arc::clone(&keyframe_wanted);
        let recorder = recorder.clone();
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 2048];
            let mut feedback = rtcp_srtp;
            while let Ok(length) = socket.recv(&mut buffer).await {
                let Ok(plain) = feedback.decrypt_rtcp(&buffer[..length]) else {
                    continue;
                };
                let Ok(packets) = rtcp::packet::unmarshal(&mut plain.clone()) else {
                    continue;
                };
                for packet in packets {
                    let any = packet.as_any();
                    if let Some(rr) = any.downcast_ref::<ReceiverReport>() {
                        for report in &rr.reports {
                            if report.last_sender_report == 0 {
                                continue;
                            }
                            let (_, now) = ntp_now();
                            let rtt = now
                                .wrapping_sub(report.last_sender_report)
                                .wrapping_sub(report.delay);
                            // Middle-32 NTP units are 1/65536 s.
                            let micros = u64::from(rtt) * 1_000_000 / 65_536;
                            if micros < 10_000_000 {
                                recorder.latency_micros("voice:rtt", micros.max(1));
                            }
                        }
                    }
                    if any.downcast_ref::<PictureLossIndication>().is_some()
                        || any.downcast_ref::<FullIntraRequest>().is_some()
                    {
                        keyframe_wanted.store(true, Ordering::Relaxed);
                    }
                }
            }
        })
    });
    let mut sequence: u16 = 1;
    let mut timestamp: u32 = 0;
    let mut ticker = tokio::time::interval(period);
    let mut last_report = Instant::now();
    let mut last_keyframe = Instant::now();
    let mut octets: u32 = 0;
    let mut frame_filler = BytesMut::zeroed(frame_bytes);
    frame_filler.fill(0x5a);
    loop {
        ticker.tick().await;
        let chunks: Vec<(u8, usize)> = if video {
            let keyframe = keyframe_wanted.swap(false, Ordering::Relaxed)
                || last_keyframe.elapsed() >= KEYFRAME_EVERY;
            if keyframe {
                last_keyframe = Instant::now();
            }
            // One NAL unit per packet: an IDR slice (type 5) for a keyframe, else a non-IDR
            // slice (type 1). A keyframe is led by a sequence and a picture parameter set, as
            // an encoder's is; the sequence parameter set is what mediasoup recognises a
            // keyframe by, and its consumers forward nothing until they have seen one.
            let nal = if keyframe { 0x65 } else { 0x41 };
            let mut left = frame_bytes;
            let mut chunks = if keyframe {
                vec![(0x67, 16), (0x68, 4)]
            } else {
                Vec::new()
            };
            while left > 0 {
                let size = left.min(MAX_PAYLOAD);
                chunks.push((nal, size));
                left -= size;
            }
            chunks
        } else {
            vec![(0x78, frame_bytes)]
        };
        let last = chunks.len() - 1;
        for (i, (first_byte, size)) in chunks.into_iter().enumerate() {
            let mut payload = BytesMut::with_capacity(size);
            payload.extend_from_slice(&[first_byte]);
            payload.extend_from_slice(&frame_filler[..size.saturating_sub(1)]);
            let packet = rtp::packet::Packet {
                header: rtp::header::Header {
                    version: 2,
                    marker: video && i == last,
                    payload_type,
                    sequence_number: sequence,
                    timestamp,
                    ssrc,
                    ..Default::default()
                },
                payload: payload.freeze(),
            };
            sequence = sequence.wrapping_add(1);
            let Ok(raw) = packet.marshal() else { continue };
            if let Ok(encrypted) = rtp_srtp.encrypt_rtp(&raw)
                && socket.send(&encrypted).await.is_ok()
            {
                sent_packets.fetch_add(1, Ordering::Relaxed);
                octets = octets.wrapping_add(size as u32);
                recorder.count("voice_bytes_sent", encrypted.len() as u64);
            }
        }
        timestamp = timestamp.wrapping_add(clock_step);
        if last_report.elapsed() >= REPORT_EVERY {
            last_report = Instant::now();
            let (ntp_time, _) = ntp_now();
            let report = SenderReport {
                ssrc,
                ntp_time,
                rtp_time: timestamp,
                packet_count: sent_packets.load(Ordering::Relaxed),
                octet_count: octets,
                ..Default::default()
            };
            // The sending context keeps its own RTCP index, so it encrypts reports too.
            if let Ok(raw) = report.marshal()
                && let Ok(encrypted) = rtp_srtp.encrypt_rtcp(&raw)
            {
                let _ = socket.send(&encrypted).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtcp_is_told_from_rtp_by_payload_type() {
        assert!(is_rtcp(&[0x80, 200]));
        assert!(is_rtcp(&[0x80, 201]));
        assert!(!is_rtcp(&[0x80, 100]));
        assert!(!is_rtcp(&[0x80, 96 | 0x80]));
    }

    #[test]
    fn loss_and_jitter_follow_rfc_3550() {
        let mut stats = StreamStats::default();
        // Packets 10..=19 with 13 and 17 lost, each 960 units apart and arriving on time.
        for sequence in (10u16..20).filter(|s| *s != 13 && *s != 17) {
            let ts = u32::from(sequence) * 960;
            stats.packet(sequence, ts, i64::from(ts) + 5000);
        }
        assert_eq!(stats.expected(), 10);
        assert_eq!(stats.received, 8);
        assert!(stats.jitter < 1.0);
        // A timestamp wrapping round 2^32 is not jitter.
        let mut wrap = StreamStats::default();
        for (i, ts) in [u32::MAX - 959, u32::MAX, 959, 1919]
            .into_iter()
            .enumerate()
        {
            wrap.packet(i as u16, ts, i as i64 * 960);
        }
        assert!(wrap.jitter < 1.0, "{}", wrap.jitter);
        // Wrapping round the sequence space counts a cycle.
        let mut wrapping = StreamStats::default();
        for sequence in [65534u16, 65535, 0, 1] {
            wrapping.packet(sequence, 0, 0);
        }
        assert_eq!(wrapping.expected(), 4);
    }
}
