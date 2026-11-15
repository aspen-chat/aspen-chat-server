//! Simulated call participants.
//!
//! A simulated participant joins a call the way the reference client does: a join offer from the
//! API server, a latency probe of every candidate, the signalling socket to the nearest, and a
//! WebRTC connection the server offers and it answers. That connection is a synthetic peer
//! (`voice_media::peer`), str0m, with the ICE, DTLS, SRTP, and RTCP a browser has, so a voice
//! server spends on it what it spends on a person. It sends its microphone, and its screen when
//! it shares one, and watches every screen shared in the call, as the client shows them.
//!
//! Microphone packets are Opus-sized every 20 ms at the behaviour's bitrate; a shared screen is
//! 30 frames a second of H.264-shaped NAL units at its bitrate, a keyframe every second. The
//! payloads are filler: the server forwards media without decoding it.
//!
//! Measured: `voice:join` (join offer to the voice server's `ready`), `voice:probe` (the health
//! probes), `voice:jitter` (RFC 3550 interarrival jitter of each received stream, every second),
//! `voice:rtt` (from the server's receiver reports on what the participant sends), and the counters
//! `voice_packets_expected` and `voice_packets_received`, whose ratio is the loss, and
//! `voice_bytes_sent`.

use crate::stats::Recorder;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use voice_media::peer::{Peer, PeerOptions, Received, Synthetic};
use voice_protocol::signal::{
    ClientMessage, MediaKind, MediaSource, SectionContent, ServerMessage, WatchedVideo,
};

const REPORT_EVERY: Duration = Duration::from_secs(1);
/// The most shares one participant watches, as the voice server allows.
const MAX_WATCHED: usize = 25;

/// One call to make.
pub struct CallPlan {
    pub seconds: f64,
    pub screen: bool,
    pub audio_bitrate: u32,
    pub screen_bitrate: u32,
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
    send(&mut sink, &ClientMessage::Identify { token }).await?;
    let ready = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match frames.recv().await {
                Some(ServerMessage::Ready { participants, .. }) => return Ok(participants),
                Some(ServerMessage::Error {
                    detail,
                    fatal: true,
                    ..
                }) => return Err(detail),
                Some(_) => {}
                None => return Err("the voice server closed the socket".to_string()),
            }
        }
    })
    .await
    .map_err(|_| "no ready from the voice server".to_string())??;
    recorder.latency("voice:join", started.elapsed());
    recorder.count("voice_calls", 1);

    let mut peer = Peer::start(PeerOptions {
        detail: true,
        stats: true,
        ..PeerOptions::default()
    });
    let microphone = Synthetic::Audio {
        kbps: f64::from(plan.audio_bitrate) / 1000.0,
        level: Some(-30),
    };
    let screen = Synthetic::Video {
        kbps: f64::from(plan.screen_bitrate) / 1000.0,
        fps: 30.0,
    };
    send(
        &mut sink,
        &ClientMessage::Send {
            source: MediaSource::Microphone,
            external: false,
        },
    )
    .await?;
    if plan.screen {
        send(
            &mut sink,
            &ClientMessage::Send {
                source: MediaSource::Screen,
                external: false,
            },
        )
        .await?;
    }

    // Who shares a screen, which this participant watches.
    let mut sharing: Vec<uuid::Uuid> = ready
        .iter()
        .filter(|p| p.sources.iter().any(|s| s.source == MediaSource::Screen))
        .map(|p| p.user)
        .collect();
    let watch = |sharing: &[uuid::Uuid]| ClientMessage::Watch {
        video: sharing
            .iter()
            .take(MAX_WATCHED)
            .map(|user| WatchedVideo {
                user: *user,
                source: MediaSource::Screen,
            })
            .collect(),
    };
    if !sharing.is_empty() {
        send(&mut sink, &watch(&sharing)).await?;
    }
    // What each section received carries, for its clock rate.
    let mut kinds: HashMap<String, MediaKind> = HashMap::new();
    let mut started_sending: Vec<String> = Vec::new();
    let mut streams: HashMap<u32, StreamStats> = HashMap::new();
    let mut report = tokio::time::interval(REPORT_EVERY);
    let epoch = Instant::now();
    let end = Instant::now() + Duration::from_secs_f64(plan.seconds);
    loop {
        tokio::select! {
            message = frames.recv() => match message {
                Some(ServerMessage::Offer { connection, revision, sdp, sections }) => {
                    let answer = peer.answer(sdp).await?;
                    send(&mut sink, &ClientMessage::Answer { connection, revision, sdp: answer }).await?;
                    for section in sections {
                        kinds.insert(section.mid.clone(), section.kind);
                        if let SectionContent::Sending { source } = section.content
                            && !started_sending.contains(&section.mid)
                        {
                            started_sending.push(section.mid.clone());
                            peer.send(&section.mid, match source {
                                MediaSource::Screen | MediaSource::Camera => screen,
                                MediaSource::Microphone | MediaSource::ScreenAudio => microphone,
                            });
                        }
                    }
                }
                Some(ServerMessage::Sources { user, sources }) => {
                    let shares = sources.iter().any(|s| s.source == MediaSource::Screen);
                    let known = sharing.contains(&user);
                    if shares != known {
                        if shares {
                            sharing.push(user);
                        } else {
                            sharing.retain(|u| *u != user);
                        }
                        send(&mut sink, &watch(&sharing)).await?;
                    }
                }
                Some(ServerMessage::Error { detail, fatal: true, .. }) => return Err(detail),
                Some(ServerMessage::Kicked { .. }) | None => break,
                Some(_) => {}
            },
            _ = report.tick() => {
                for Received { mid, ssrc, seq, timestamp, at, .. } in peer.drain() {
                    let rate = match kinds.get(&mid) {
                        Some(MediaKind::Video) => 90_000,
                        _ => 48_000,
                    };
                    let arrival = (at.duration_since(epoch.into_std()).as_secs_f64() * f64::from(rate)) as i64;
                    streams.entry(ssrc).or_insert_with(|| StreamStats::new(rate)).packet(seq, timestamp, arrival);
                }
                for stats in streams.values_mut() {
                    stats.report(&recorder);
                }
                let rtt = peer.counters.rtt_micros.load(Ordering::Relaxed);
                if rtt > 0 {
                    recorder.latency_micros("voice:rtt", rtt);
                }
                recorder.count("voice_bytes_sent", peer.counters.sent_bytes.swap(0, Ordering::Relaxed));
            }
            _ = tokio::time::sleep_until(end) => break,
            _ = stop.changed() => break,
        }
    }
    let _ = send(&mut sink, &ClientMessage::Leave).await;
    let _ = sink.close().await;
    reader.abort();
    Ok(())
}

/// Per received stream: RFC 3550 jitter, and loss from its sequence numbers, which str0m
/// gives extended, so they never wrap.
struct StreamStats {
    rate: u32,
    base: Option<u64>,
    highest: u64,
    received: u64,
    /// Expected and received at the last report, for per-interval counts.
    reported_expected: u64,
    reported_received: u64,
    transit: Option<u32>,
    jitter: f64,
}

impl StreamStats {
    fn new(rate: u32) -> Self {
        Self {
            rate,
            base: None,
            highest: 0,
            received: 0,
            reported_expected: 0,
            reported_received: 0,
            transit: None,
            jitter: 0.0,
        }
    }

    fn packet(&mut self, sequence: u64, timestamp: u32, arrival_units: i64) {
        let base = *self.base.get_or_insert(sequence);
        self.highest = self.highest.max(sequence).max(base);
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
        self.base.map_or(0, |base| self.highest - base + 1)
    }

    fn report(&mut self, recorder: &Recorder) {
        if self.received > 1 {
            recorder.latency_micros(
                "voice:jitter",
                (self.jitter / f64::from(self.rate) * 1e6) as u64,
            );
        }
        let expected = self.expected();
        recorder.count(
            "voice_packets_expected",
            expected.saturating_sub(self.reported_expected),
        );
        recorder.count(
            "voice_packets_received",
            self.received.saturating_sub(self.reported_received),
        );
        self.reported_expected = expected;
        self.reported_received = self.received;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loss_counts_the_sequence_numbers_that_never_came() {
        let mut stats = StreamStats::new(48_000);
        for sequence in [10, 11, 13, 14] {
            stats.packet(sequence, sequence as u32 * 960, sequence as i64 * 960);
        }
        assert_eq!((stats.expected(), stats.received), (5, 4));
        // Even arrival keeps jitter at nothing.
        assert_eq!(stats.jitter, 0.0);
    }
}
