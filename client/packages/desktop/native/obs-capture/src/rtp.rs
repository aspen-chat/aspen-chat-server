//! The SRTP sender: encoded frames in (H.264 NAL units or Opus packets, one sender per stream),
//! SRTP to the voice server's plain transport out, with the RTCP that comes back acted on. The voice server (mediasoup) learns where to send RTCP from
//! the first packet it receives, so RTP and RTCP share this one socket.
//!
//! What the RTCP does: a NACK resends the packets it names from a buffer of recent ones; a
//! receiver estimate (REMB) becomes the encoder's bitrate, within the limits the capture was
//! started with; picture loss and intra requests are counted and otherwise left to the short
//! keyframe interval, since libobs has no way to force a keyframe on demand.

use bytes::Bytes;
use rtcp::packet::Packet as RtcpPacket;
use rtcp::payload_feedbacks::full_intra_request::FullIntraRequest;
use rtcp::payload_feedbacks::picture_loss_indication::PictureLossIndication;
use rtcp::payload_feedbacks::receiver_estimated_maximum_bitrate::ReceiverEstimatedMaximumBitrate;
use rtcp::sender_report::SenderReport;
use rtcp::transport_feedbacks::transport_layer_nack::TransportLayerNack;
use rtp::codecs::h264::H264Payloader;
use rtp::codecs::opus::OpusPayloader;
use rtp::packetizer::{Packetizer, new_packetizer};
use rtp::sequence::new_random_sequencer;
use serde::Deserialize;
use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use webrtc_srtp::context::Context;
use webrtc_srtp::protection_profile::ProtectionProfile;
use webrtc_util::marshal::Marshal;

/// Where and how to send, as the voice server answered `produceRtp`.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RtpTarget {
    pub ip: IpAddr,
    pub port: u16,
    pub ssrc: u32,
    pub payload_type: u8,
    pub srtp_crypto_suite: String,
    pub srtp_key_base64: String,
}

/// Largest RTP payload; well under a typical MTU with the SRTP tag and headers added.
const MTU: usize = 1200;
/// RTP header extension id for `abs-send-time`, which the voice server uses for its receiver
/// bandwidth estimate; it matches what the producer's parameters declare.
const ABS_SEND_TIME_ID: u8 = 4;
/// What a sender carries, which decides its payloader and clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    /// H.264 in Annex B NAL units, a 90 kHz clock.
    Video,
    /// Opus frames, a 48 kHz clock.
    Audio,
}

impl StreamKind {
    fn clock_rate(self) -> u32 {
        match self {
            StreamKind::Video => 90_000,
            StreamKind::Audio => 48_000,
        }
    }
}
/// How many recent packets are kept for retransmission.
const RETRANSMIT_BUFFER: usize = 2048;
const SENDER_REPORT_INTERVAL: Duration = Duration::from_secs(1);
/// How often at most the encoder bitrate follows the receiver's estimate.
const BITRATE_UPDATE_INTERVAL: Duration = Duration::from_millis(1500);

/// A change the RTCP asked for, applied by the capture on its own thread.
pub trait Feedback: Send + Sync {
    fn set_bitrate_kbps(&self, kbps: u32);
}

struct Sending {
    socket: UdpSocket,
    srtp: Context,
    packetizer: Box<dyn Packetizer + Send + Sync>,
    last_pts_us: Option<f64>,
    recent: VecDeque<(u16, Bytes)>,
    last_rtp_timestamp: u32,
}

pub struct RtpSender {
    sending: Mutex<Sending>,
    kind: StreamKind,
    ssrc: u32,
    packets_sent: AtomicU64,
    bytes_sent: AtomicU64,
    pub keyframes_wanted: AtomicU64,
    stopped: Arc<AtomicBool>,
}

fn srtp_context(target: &RtpTarget) -> Result<Context, String> {
    if target.srtp_crypto_suite != "AES_CM_128_HMAC_SHA1_80" {
        return Err(format!(
            "unsupported SRTP crypto suite {}",
            target.srtp_crypto_suite
        ));
    }
    use base64::Engine;
    let material = base64::engine::general_purpose::STANDARD
        .decode(&target.srtp_key_base64)
        .map_err(|e| format!("SRTP key is not base64: {e}"))?;
    if material.len() != 30 {
        return Err("SRTP keying material must be 30 bytes for AES_CM_128_HMAC_SHA1_80".into());
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

impl RtpSender {
    /// Opens the socket and starts the RTCP listener; returns the sender to hand packets to.
    pub fn start(
        target: &RtpTarget,
        kind: StreamKind,
        feedback: Arc<dyn Feedback>,
        min_kbps: u32,
        max_kbps: u32,
    ) -> Result<Arc<Self>, String> {
        let bind: SocketAddr = if target.ip.is_ipv4() {
            "0.0.0.0:0".parse().expect("address")
        } else {
            "[::]:0".parse().expect("address")
        };
        let socket = UdpSocket::bind(bind).map_err(|e| format!("UDP socket: {e}"))?;
        socket
            .connect((target.ip, target.port))
            .map_err(|e| format!("UDP connect to {}:{}: {e}", target.ip, target.port))?;
        let receiver = socket.try_clone().map_err(|e| format!("UDP socket: {e}"))?;
        let payloader: Box<dyn rtp::packetizer::Payloader + Send + Sync> = match kind {
            StreamKind::Video => Box::new(H264Payloader::default()),
            StreamKind::Audio => Box::new(OpusPayloader),
        };
        let mut packetizer = new_packetizer(
            MTU,
            target.payload_type,
            target.ssrc,
            payloader,
            Box::new(new_random_sequencer()),
            kind.clock_rate(),
        );
        if kind == StreamKind::Video {
            packetizer.enable_abs_send_time(ABS_SEND_TIME_ID);
        }
        let sender = Arc::new(Self {
            sending: Mutex::new(Sending {
                socket,
                srtp: srtp_context(target)?,
                packetizer: Box::new(packetizer),
                last_pts_us: None,
                recent: VecDeque::with_capacity(RETRANSMIT_BUFFER),
                last_rtp_timestamp: 0,
            }),
            kind,
            ssrc: target.ssrc,
            packets_sent: AtomicU64::new(0),
            bytes_sent: AtomicU64::new(0),
            keyframes_wanted: AtomicU64::new(0),
            stopped: Arc::new(AtomicBool::new(false)),
        });
        let mut rtcp_srtp = srtp_context(target)?;
        let listener = Arc::clone(&sender);
        std::thread::Builder::new()
            .name("aspen-rtcp".into())
            .spawn(move || {
                let mut buffer = vec![0u8; 2048];
                let mut last_report = Instant::now();
                let mut last_bitrate_update = Instant::now() - BITRATE_UPDATE_INTERVAL;
                receiver
                    .set_read_timeout(Some(Duration::from_millis(250)))
                    .expect("socket timeout");
                while !listener.stopped.load(Ordering::SeqCst) {
                    if last_report.elapsed() >= SENDER_REPORT_INTERVAL {
                        listener.send_sender_report(&mut rtcp_srtp);
                        last_report = Instant::now();
                    }
                    let length = match receiver.recv(&mut buffer) {
                        Ok(length) => length,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                || e.kind() == std::io::ErrorKind::TimedOut =>
                        {
                            continue;
                        }
                        Err(_) => break,
                    };
                    let Ok(plain) = rtcp_srtp.decrypt_rtcp(&buffer[..length]) else {
                        continue;
                    };
                    let mut raw = plain;
                    let Ok(packets) = rtcp::packet::unmarshal(&mut raw) else {
                        continue;
                    };
                    for packet in packets {
                        listener.handle_rtcp(
                            packet.as_ref(),
                            &feedback,
                            min_kbps,
                            max_kbps,
                            &mut last_bitrate_update,
                        );
                    }
                }
            })
            .map_err(|e| format!("RTCP thread: {e}"))?;
        Ok(sender)
    }

    /// Sends one encoded frame: Annex B NAL units, with the parameter sets in front of
    /// keyframes, at the presentation time libobs stamped it with.
    pub fn send_frame(&self, data: &[u8], pts_us: f64) {
        let mut sending = self.sending.lock().expect("sending lock");
        let samples = match sending.last_pts_us {
            None => 0,
            Some(previous) => {
                ((pts_us - previous).max(0.0) * f64::from(self.kind.clock_rate()) / 1e6) as u32
            }
        };
        sending.last_pts_us = Some(pts_us);
        let payload = Bytes::copy_from_slice(data);
        let packets = match sending.packetizer.packetize(&payload, samples) {
            Ok(packets) => packets,
            Err(_) => return,
        };
        for packet in packets {
            sending.last_rtp_timestamp = packet.header.timestamp;
            let Ok(raw) = packet.marshal() else { continue };
            let Ok(encrypted) = sending.srtp.encrypt_rtp(&raw) else {
                continue;
            };
            if sending.socket.send(&encrypted).is_ok() {
                self.packets_sent.fetch_add(1, Ordering::Relaxed);
                self.bytes_sent
                    .fetch_add(raw.len() as u64, Ordering::Relaxed);
            }
            if sending.recent.len() == RETRANSMIT_BUFFER {
                sending.recent.pop_front();
            }
            sending
                .recent
                .push_back((packet.header.sequence_number, encrypted));
        }
    }

    fn handle_rtcp(
        &self,
        packet: &(dyn RtcpPacket + Send + Sync),
        feedback: &Arc<dyn Feedback>,
        min_kbps: u32,
        max_kbps: u32,
        last_bitrate_update: &mut Instant,
    ) {
        let any = packet.as_any();
        if let Some(nack) = any.downcast_ref::<TransportLayerNack>() {
            if nack.media_ssrc != self.ssrc {
                return;
            }
            let wanted: Vec<u16> = nack
                .nacks
                .iter()
                .flat_map(|pair| pair.packet_list())
                .collect();
            let sending = self.sending.lock().expect("sending lock");
            for (sequence, encrypted) in &sending.recent {
                if wanted.contains(sequence) {
                    let _ = sending.socket.send(encrypted);
                }
            }
        } else if let Some(remb) = any.downcast_ref::<ReceiverEstimatedMaximumBitrate>() {
            // The estimate steers the video encoder; audio runs at its set rate.
            if self.kind == StreamKind::Video
                && last_bitrate_update.elapsed() >= BITRATE_UPDATE_INTERVAL
            {
                let kbps = ((remb.bitrate / 1000.0) as u32).clamp(min_kbps, max_kbps);
                feedback.set_bitrate_kbps(kbps);
                *last_bitrate_update = Instant::now();
            }
        } else if any.downcast_ref::<PictureLossIndication>().is_some()
            || any.downcast_ref::<FullIntraRequest>().is_some()
        {
            self.keyframes_wanted.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn send_sender_report(&self, srtp: &mut Context) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        // NTP: seconds since 1900 in the high word, fraction in the low word.
        let ntp_time = ((now.as_secs() + 2_208_988_800) << 32)
            | ((u64::from(now.subsec_nanos()) << 32) / 1_000_000_000);
        let (rtp_time, socket) = {
            let sending = self.sending.lock().expect("sending lock");
            (sending.last_rtp_timestamp, sending.socket.try_clone().ok())
        };
        let report = SenderReport {
            ssrc: self.ssrc,
            ntp_time,
            rtp_time,
            packet_count: self.packets_sent.load(Ordering::Relaxed) as u32,
            octet_count: self.bytes_sent.load(Ordering::Relaxed) as u32,
            ..Default::default()
        };
        let (Ok(raw), Some(socket)) = (report.marshal(), socket) else {
            return;
        };
        if let Ok(encrypted) = srtp.encrypt_rtcp(&raw) {
            let _ = socket.send(&encrypted);
        }
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}
