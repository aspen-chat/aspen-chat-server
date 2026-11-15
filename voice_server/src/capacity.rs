//! `voice_server estimate-capacity`: a conservative estimate of how many people this machine
//! can hold in calls, for the `capacity` it is registered with (`voice-servers add`).
//!
//! It reads the hardware (logical cores and any container CPU quota, memory and any container
//! limit, and the bandwidth out: `--link-mbps`, else `[rtc] upload_mbps`, else the link speed of
//! the interface that holds the default route) and the media settings
//! (`media_threads`, each of which uses at most one core). Then it measures what one forwarded
//! stream costs on this CPU: a media thread of its own on loopback carries calls of synthetic
//! participants (`voice_media::peer`), each a full WebRTC connection (ICE, DTLS, SRTP both
//! ways, RTCP), for a few seconds, and the thread's CPU time and what it allocated are divided
//! by the streams it forwarded. The participants run in this process too; only the media
//! thread is counted.
//!
//! From a call shape (the size of a call, the share of people sharing a screen, the bitrates)
//! follows what one participant costs in CPU, memory, and bandwidth out; each resource, less a
//! safety margin, then holds some number of participants, and the estimate is the smallest.
//! The calibration counts a received packet as costing as much as a sent one, which leans
//! towards too little rather than too much.

use crate::config::MediaConfig;
use crate::rooms::{AUDIO_SLOTS, MAX_WATCHED_VIDEO};
use anyhow::{Context, bail};
use serde::Serialize;
use std::collections::HashSet;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use uuid::Uuid;
use voice_media::peer::{Counters, Peer, PeerOptions, Synthetic};
use voice_media::{CallId, Command, Event, Media, MediaAddress, ParticipantId};
use voice_protocol::signal::{MediaSource, SectionContent};

/// What a call is assumed to look like.
#[derive(clap::Args, Debug, Clone, Serialize)]
pub struct CallShape {
    /// People in a typical call.
    #[arg(long, default_value_t = 6)]
    pub call_size: u32,
    /// Share of the people in a call sharing a screen (or a game), each with its sound.
    #[arg(long, default_value_t = 0.2)]
    pub screen_share: f64,
    /// Bitrate of a microphone or a shared screen's sound, in kbit/s.
    #[arg(long, default_value_t = 64.0)]
    pub audio_kbps: f64,
    /// Bitrate of a shared screen's picture, in kbit/s.
    #[arg(long, default_value_t = 2500.0)]
    pub screen_kbps: f64,
}

#[derive(clap::Args, Debug)]
pub struct EstimateArgs {
    #[command(flatten)]
    pub shape: CallShape,
    /// Share of each resource held back, for load this does not model (signalling, bursts,
    /// retransmissions, the rest of the machine).
    #[arg(long, default_value_t = 0.3)]
    pub margin: f64,
    /// The machine's usable bandwidth out, in Mbit/s, over `[rtc] upload_mbps` and the
    /// interface's speed: what the host provides (a cloud instance's network allowance, say).
    #[arg(long)]
    pub link_mbps: Option<f64>,
    /// How long each kind of stream is measured for.
    #[arg(long, default_value_t = 5)]
    pub calibration_seconds: u64,
    /// Print the estimate as JSON.
    #[arg(long)]
    pub json: bool,
}

/// Bytes a media packet carries besides its payload: IPv4 (20), UDP (8), RTP (12), and the
/// SRTP authentication tag (16, AES-GCM's).
const PACKET_OVERHEAD: f64 = 56.0;
const MAX_PAYLOAD: usize = voice_media::peer::MAX_PAYLOAD;
/// Audio frames a second: Opus in 20 ms frames.
const AUDIO_FPS: f64 = 50.0;
/// Frames a second of a shared screen.
const VIDEO_FPS: f64 = 30.0;
/// People in each of the calibration's calls of one kind of stream: each one's stream is
/// forwarded to the five others.
const CALIBRATION_CALL: usize = 6;
/// Calls the per-stream calibrations hold: enough streams to measure well, few enough not to
/// fill a core.
const CALIBRATION_CALLS: usize = 10;
/// Memory the process needs before any call: its binary, threads, and buffers.
const BASE_MEMORY: u64 = 512 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct Hardware {
    /// Logical cores the process may use, after any CPU quota.
    pub cores: f64,
    pub media_threads: usize,
    /// Memory the process may use, after any container limit.
    pub memory_bytes: u64,
    pub interface: Option<String>,
    /// Bandwidth out, from `--link-mbps`, `[rtc] upload_mbps`, or the interface's speed.
    pub link_mbps: Option<f64>,
}

/// What one forwarded stream, or one participant in a call, costs here.
#[derive(Debug, Serialize)]
pub struct Cost {
    /// Share of one core.
    pub cpu: f64,
    pub memory_bytes: f64,
    /// Share of the expected packets that arrived during the measurement.
    pub delivered: f64,
}

#[derive(Debug, Serialize)]
pub struct Limit {
    pub resource: &'static str,
    pub participants: u64,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Estimate {
    pub hardware: Hardware,
    pub shape: CallShape,
    pub margin: f64,
    pub audio_stream: Cost,
    pub video_stream: Cost,
    /// Measured in whole calls of the shape, which counts each call's router and each
    /// participant's transports as well as the streams.
    pub participant_in_calls: Cost,
    /// Per participant: share of a core, bytes, Mbit/s out.
    pub participant_cpu: f64,
    pub participant_memory_bytes: f64,
    pub participant_mbps_out: f64,
    pub limits: Vec<Limit>,
    /// The largest call one media thread holds within the margin: a call lives on one thread.
    pub largest_call: u64,
    pub capacity: u64,
    pub warnings: Vec<String>,
}

pub async fn run(args: EstimateArgs, media: MediaConfig) -> anyhow::Result<()> {
    if !(2..=1000).contains(&args.shape.call_size) {
        bail!("--call-size must be between 2 and 1000");
    }
    if !(0.0..=1.0).contains(&args.shape.screen_share) || !(0.0..1.0).contains(&args.margin) {
        bail!("--screen-share must be between 0 and 1, and --margin at least 0 and below 1");
    }
    let mut warnings = Vec::new();
    let hardware = inspect(&media, args.link_mbps, &mut warnings);
    if !args.json {
        eprintln!(
            "calibrating: forwarding synthetic streams for {} s per kind…",
            args.calibration_seconds
        );
    }
    let seconds = Duration::from_secs(args.calibration_seconds.max(1));
    let audio = calibrate(&Load::audio(&args.shape), seconds).await?;
    let video = calibrate(&Load::video(&args.shape), seconds).await?;
    let calls = calibrate(&Load::calls(&args.shape), seconds).await?;
    for (name, cost) in [("audio", &audio), ("video", &video), ("call", &calls)] {
        if cost.delivered < 0.95 {
            warnings.push(format!(
                "only {:.0}% of the calibration's {name} packets arrived; the media thread may have been short of CPU, so the estimate may be high",
                cost.delivered * 100.0
            ));
        }
    }
    let estimate = estimate(
        hardware,
        args.shape,
        args.margin,
        audio,
        video,
        calls,
        warnings,
    );
    if args.json {
        println!("{}", serde_json::to_string_pretty(&estimate)?);
    } else {
        print!("{}", describe(&estimate));
    }
    Ok(())
}

/// Streams one participant receives in a call of `n` with `share` of them sharing: every
/// other's microphone up to the audio slots, and each sharer's picture and sound.
fn streams_received(shape: &CallShape) -> (f64, f64) {
    let others = f64::from(shape.call_size - 1);
    let sharers = others * shape.screen_share;
    (others.min(f64::from(AUDIO_SLOTS)) + sharers, sharers)
}

/// Bits a second on the wire for one stream of `kbps` sent in `fps` frames.
fn wire_mbps(kbps: f64, fps: f64) -> f64 {
    let frame_bytes = kbps * 1000.0 / 8.0 / fps;
    let packets = (frame_bytes / MAX_PAYLOAD as f64).ceil().max(1.0) * fps;
    (kbps * 1000.0 + packets * PACKET_OVERHEAD * 8.0) / 1_000_000.0
}

fn estimate(
    hardware: Hardware,
    shape: CallShape,
    margin: f64,
    audio: Cost,
    video: Cost,
    calls: Cost,
    mut warnings: Vec<String>,
) -> Estimate {
    let keep = 1.0 - margin;
    let (audio_streams, video_streams) = streams_received(&shape);
    // A participant's own streams in are counted as costing what one sent does.
    let (own_audio, own_video) = (1.0 + shape.screen_share, shape.screen_share);
    // The larger of what the streams add up to and what whole calls measured, for each.
    let participant_cpu = ((audio_streams + own_audio) * audio.cpu
        + (video_streams + own_video) * video.cpu)
        .max(calls.cpu);
    let participant_memory_bytes = (audio_streams * audio.memory_bytes
        + video_streams * video.memory_bytes)
        .max(calls.memory_bytes);
    let participant_mbps_out = audio_streams * wire_mbps(shape.audio_kbps, AUDIO_FPS)
        + video_streams * wire_mbps(shape.screen_kbps, VIDEO_FPS);

    let usable_cores = hardware.cores.min(hardware.media_threads as f64);
    if (hardware.media_threads as f64) < hardware.cores.floor() {
        warnings.push(format!(
            "`media_threads` is {} on a machine with {:.0} cores; each thread uses at most one core, so raising it to {:.0} would hold more",
            hardware.media_threads, hardware.cores, hardware.cores.floor()
        ));
    }
    let per = |budget: f64, cost: f64| -> u64 {
        if cost > 0.0 {
            (budget / cost).floor().max(0.0) as u64
        } else {
            u64::MAX
        }
    };
    let mut limits = vec![
        Limit {
            resource: "CPU",
            participants: per(usable_cores * keep, participant_cpu),
            detail: format!(
                "{usable_cores:.1} cores for media threads × {:.0}%, {:.3}% of a core each",
                keep * 100.0,
                participant_cpu * 100.0
            ),
        },
        Limit {
            resource: "memory",
            participants: per(
                (hardware.memory_bytes.saturating_sub(BASE_MEMORY)) as f64 * keep,
                participant_memory_bytes,
            ),
            detail: format!(
                "{} less {} for the process, × {:.0}%, {} each",
                bytes(hardware.memory_bytes as f64),
                bytes(BASE_MEMORY as f64),
                keep * 100.0,
                bytes(participant_memory_bytes)
            ),
        },
    ];
    match hardware.link_mbps {
        Some(link) => limits.push(Limit {
            resource: "bandwidth out",
            participants: per(link * keep, participant_mbps_out),
            detail: format!(
                "{link:.0} Mbit/s × {:.0}%, {participant_mbps_out:.2} Mbit/s each",
                keep * 100.0
            ),
        }),
        None => warnings.push(
            "the link speed is unknown, so bandwidth is not counted; set [rtc] upload_mbps to the upload a speed test measures".to_string(),
        ),
    }
    let capacity = limits.iter().map(|l| l.participants).min().unwrap_or(0);
    // Everyone in a call adds to the cost of everyone else, so the largest call one media
    // thread holds is where n people each forwarding to n - 1 others fill a core.
    let largest_call = (2..=10_000u32)
        .take_while(|&n| {
            let shape = CallShape {
                call_size: n,
                ..shape.clone()
            };
            let (a, v) = streams_received(&shape);
            let each =
                (a + 1.0 + shape.screen_share) * audio.cpu + (v + shape.screen_share) * video.cpu;
            f64::from(n) * each <= keep
        })
        .last()
        .map_or(0, u64::from);
    Estimate {
        hardware,
        shape,
        margin,
        audio_stream: audio,
        video_stream: video,
        participant_in_calls: calls,
        participant_cpu,
        participant_memory_bytes,
        participant_mbps_out,
        limits,
        largest_call,
        capacity,
        warnings,
    }
}

fn bytes(n: f64) -> String {
    if n >= 1024.0 * 1024.0 * 1024.0 {
        format!("{:.1} GiB", n / (1024.0 * 1024.0 * 1024.0))
    } else if n >= 1024.0 * 1024.0 {
        format!("{:.1} MiB", n / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KiB", n / 1024.0)
    }
}

fn describe(e: &Estimate) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let h = &e.hardware;
    let _ = writeln!(out, "Hardware");
    let _ = writeln!(
        out,
        "  CPU        {:.1} cores, {} media threads (each uses at most one core)",
        h.cores, h.media_threads
    );
    let _ = writeln!(out, "  memory     {}", bytes(h.memory_bytes as f64));
    let _ = writeln!(
        out,
        "  network    {}, {}",
        h.interface.as_deref().unwrap_or("interface unknown"),
        h.link_mbps
            .map_or("speed unknown".to_string(), |m| format!("{m:.0} Mbit/s"))
    );
    let _ = writeln!(out, "Measured here, per forwarded stream");
    for (name, cost) in [("audio", &e.audio_stream), ("video", &e.video_stream)] {
        let _ = writeln!(
            out,
            "  {name:<10} {:.3}% of a core, {}",
            cost.cpu * 100.0,
            bytes(cost.memory_bytes)
        );
    }
    let s = &e.shape;
    let (audio, video) = streams_received(s);
    let _ = writeln!(
        out,
        "Per participant (calls of {}, {:.0}% sharing a screen, audio {:.0} kbit/s, screens {:.0} kbit/s)",
        s.call_size,
        s.screen_share * 100.0,
        s.audio_kbps,
        s.screen_kbps
    );
    let _ = writeln!(
        out,
        "  measured in calls   {:.3}% of a core, {}",
        e.participant_in_calls.cpu * 100.0,
        bytes(e.participant_in_calls.memory_bytes)
    );
    let _ = writeln!(
        out,
        "  receives {audio:.1} audio and {video:.1} video streams; counted as {:.3}% of a core, {}, {:.2} Mbit/s out",
        e.participant_cpu * 100.0,
        bytes(e.participant_memory_bytes),
        e.participant_mbps_out
    );
    let _ = writeln!(out, "Limits, holding back {:.0}% of each", e.margin * 100.0);
    for limit in &e.limits {
        let binds = if limit.participants == e.capacity {
            "  ← binds"
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "  {:<14} {:>8} participants  ({}){binds}",
            limit.resource, limit.participants, limit.detail
        );
    }
    let _ = writeln!(
        out,
        "  one call fits on one media thread up to about {} people",
        e.largest_call
    );
    for warning in &e.warnings {
        let _ = writeln!(out, "note: {warning}");
    }
    let _ = writeln!(
        out,
        "\nRegister this server with it, from the dashboard or where an API server runs:\n\
         aspen-chat-server voice-servers add <name> --url <url> --capacity {}",
        e.capacity
    );
    out
}

fn inspect(media: &MediaConfig, link_mbps: Option<f64>, warnings: &mut Vec<String>) -> Hardware {
    let logical = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
    let cores = match cgroup_cpu_quota() {
        Some(quota) if quota < logical => {
            warnings.push(format!(
                "a CPU quota of {quota:.1} cores applies, below the {logical:.0} the machine has"
            ));
            quota
        }
        _ => logical,
    };
    let mut memory_bytes = meminfo_total().unwrap_or(0);
    if let Some(limit) = cgroup_memory_limit()
        && (memory_bytes == 0 || limit < memory_bytes)
    {
        memory_bytes = limit;
    }
    let interface = default_route_interface();
    let link_mbps = match link_mbps.or(media.rtc.upload_mbps) {
        Some(given) => Some(given),
        None => {
            let card = interface.as_deref().and_then(interface_speed_mbps);
            if card.is_some() {
                warnings.push(
                    "bandwidth out is the network card's speed; an internet connection slower than \
                     it, as most at home are, carries fewer people: measure its upload with a speed \
                     test, and set [rtc] upload_mbps"
                        .to_string(),
                );
            }
            card
        }
    };
    Hardware {
        cores,
        media_threads: media.media_threads.max(1),
        memory_bytes,
        interface,
        link_mbps,
    }
}

/// Cores allowed by a cgroup v2 CPU quota (`cpu.max`), if one is set.
fn cgroup_cpu_quota() -> Option<f64> {
    let text = std::fs::read_to_string("/sys/fs/cgroup/cpu.max").ok()?;
    let mut fields = text.split_whitespace();
    let quota: f64 = fields.next()?.parse().ok()?;
    let period: f64 = fields.next()?.parse().ok()?;
    (period > 0.0).then_some(quota / period)
}

/// A cgroup v2 memory limit (`memory.max`), if one is set.
fn cgroup_memory_limit() -> Option<u64> {
    std::fs::read_to_string("/sys/fs/cgroup/memory.max")
        .ok()?
        .trim()
        .parse()
        .ok()
}

fn meminfo_total() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

/// The interface of the IPv4 default route, from `/proc/net/route`.
fn default_route_interface() -> Option<String> {
    let text = std::fs::read_to_string("/proc/net/route").ok()?;
    text.lines().skip(1).find_map(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        (fields.get(1) == Some(&"00000000")).then(|| fields[0].to_string())
    })
}

/// The negotiated speed of an interface, in Mbit/s; unknown for most virtual and wireless ones.
fn interface_speed_mbps(interface: &str) -> Option<f64> {
    let speed: i64 = std::fs::read_to_string(format!("/sys/class/net/{interface}/speed"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    (speed > 0).then_some(speed as f64)
}

/// One participant of a calibration's calls: what it sends, and which others of its call it
/// watches.
struct Planned {
    sends: Vec<(MediaSource, Synthetic)>,
    watches: Vec<usize>,
}

/// A calibration: calls of planned participants, and what a result is divided by.
struct Load {
    calls: Vec<Vec<Planned>>,
    /// Forwarded streams, or participants for whole calls.
    units: usize,
}

impl Load {
    fn voice(shape: &CallShape) -> Synthetic {
        Synthetic::Audio {
            kbps: shape.audio_kbps,
            // Loud, so the levels are read as a call's are.
            level: Some(-30),
        }
    }

    fn screen(shape: &CallShape) -> Synthetic {
        Synthetic::Video {
            kbps: shape.screen_kbps,
            fps: VIDEO_FPS,
        }
    }

    /// Calls of six with everyone's microphone, each forwarded to the five others.
    fn audio(shape: &CallShape) -> Self {
        let calls: Vec<Vec<Planned>> = (0..CALIBRATION_CALLS)
            .map(|_| {
                (0..CALIBRATION_CALL)
                    .map(|_| Planned {
                        sends: vec![(MediaSource::Microphone, Self::voice(shape))],
                        watches: Vec::new(),
                    })
                    .collect()
            })
            .collect();
        Self {
            calls,
            units: CALIBRATION_CALLS * CALIBRATION_CALL * (CALIBRATION_CALL - 1),
        }
    }

    /// Calls of six where one shares a screen, picture alone, and the five others watch it.
    fn video(shape: &CallShape) -> Self {
        let calls: Vec<Vec<Planned>> = (0..CALIBRATION_CALLS)
            .map(|_| {
                (0..CALIBRATION_CALL)
                    .map(|index| match index {
                        0 => Planned {
                            sends: vec![(MediaSource::Screen, Self::screen(shape))],
                            watches: Vec::new(),
                        },
                        _ => Planned {
                            sends: Vec::new(),
                            watches: vec![0],
                        },
                    })
                    .collect()
            })
            .collect();
        Self {
            calls,
            units: CALIBRATION_CALLS * (CALIBRATION_CALL - 1),
        }
    }

    /// Whole calls of the shape: everyone a microphone, the call's share of them a screen and
    /// its sound as well, which everyone else watches.
    fn calls(shape: &CallShape) -> Self {
        let size = shape.call_size as usize;
        // Enough people to measure, however large a call.
        let count = (60 / size).max(1);
        let calls: Vec<Vec<Planned>> = (0..count)
            .map(|call| {
                // Spread the sharers over the calls so their share of everyone is the shape's.
                let before = (shape.screen_share * (call * size) as f64).floor() as usize;
                let sharers = ((shape.screen_share * ((call + 1) * size) as f64).floor() as usize
                    - before)
                    .min(MAX_WATCHED_VIDEO);
                (0..size)
                    .map(|person| {
                        let mut sends = vec![(MediaSource::Microphone, Self::voice(shape))];
                        if person < sharers {
                            sends.push((MediaSource::Screen, Self::screen(shape)));
                            sends.push((MediaSource::ScreenAudio, Self::voice(shape)));
                        }
                        Planned {
                            sends,
                            watches: (0..sharers).filter(|s| *s != person).collect(),
                        }
                    })
                    .collect()
            })
            .collect();
        Self {
            units: count * size,
            calls,
        }
    }

    /// Media packets a second the participants should receive, all told: each their audio
    /// slots' worth of others' microphones, and the picture and sound of each share watched.
    fn expected_packets_per_second(&self) -> f64 {
        self.calls
            .iter()
            .map(|call| {
                call.iter()
                    .enumerate()
                    .map(|(index, planned)| {
                        let microphones = call
                            .iter()
                            .enumerate()
                            .filter(|(other, p)| {
                                *other != index
                                    && p.sends.iter().any(|(s, _)| *s == MediaSource::Microphone)
                            })
                            .map(|(_, p)| {
                                p.sends
                                    .iter()
                                    .find(|(s, _)| *s == MediaSource::Microphone)
                                    .map_or(0.0, |(_, m)| m.packets_per_second())
                            })
                            .take(usize::from(AUDIO_SLOTS))
                            .sum::<f64>();
                        let watched: f64 = planned
                            .watches
                            .iter()
                            .flat_map(|sharer| &call[*sharer].sends)
                            .filter(|(s, _)| *s != MediaSource::Microphone)
                            .map(|(_, m)| m.packets_per_second())
                            .sum();
                        microphones + watched
                    })
                    .sum::<f64>()
            })
            .sum()
    }
}

/// Answers one participant's offers through its peer, and starts sending each source once its
/// section is offered, until the media thread stops.
async fn drive(
    media: Arc<Media>,
    participant: ParticipantId,
    mut events: tokio::sync::mpsc::UnboundedReceiver<Event>,
    peer: Peer,
    sends: Vec<(MediaSource, Synthetic)>,
) {
    let mut started = HashSet::new();
    while let Some(event) = events.recv().await {
        let Event::Offer {
            connection,
            revision,
            sdp,
            sections,
            ..
        } = event
        else {
            continue;
        };
        let Ok(answer) = peer.answer(sdp).await else {
            return;
        };
        media.send(
            0,
            Command::Answer {
                participant,
                connection,
                revision,
                sdp: answer,
            },
        );
        for section in sections {
            if let SectionContent::Sending { source } = section.content
                && let Some((_, synthetic)) = sends.iter().find(|(s, _)| *s == source)
                && started.insert(section.mid.clone())
            {
                peer.send(&section.mid, *synthetic);
            }
        }
    }
}

/// What the calibration's media thread holds, by its own count.
async fn thread_allocated(media: &Media) -> u64 {
    let (reply, census) = tokio::sync::oneshot::channel();
    if !media.send(0, Command::Census { reply }) {
        return 0;
    }
    census
        .await
        .ok()
        .and_then(|census| census.allocated_bytes)
        .unwrap_or(0)
}

/// Carries `load` on a media thread of its own for `seconds`, after letting every connection
/// open and every video start (it waits for a keyframe), and measures what one unit cost.
async fn calibrate(load: &Load, seconds: Duration) -> anyhow::Result<Cost> {
    let media = Arc::new(
        Media::start(voice_media::MediaConfig {
            addresses: vec![MediaAddress {
                ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
                announced: None,
            }],
            first_port: 0,
            threads: 1,
            audio_slots: AUDIO_SLOTS,
            max_watched_video: MAX_WATCHED_VIDEO,
        })
        .context("could not start a media thread for the calibration")?,
    );
    let allocated_before = thread_allocated(&media).await;
    let mut counters: Vec<Arc<Counters>> = Vec::new();
    let mut next = 1u64;
    for call in &load.calls {
        let id = CallId(Uuid::now_v7());
        let users: Vec<Uuid> = call.iter().map(|_| Uuid::now_v7()).collect();
        for (index, planned) in call.iter().enumerate() {
            let participant = ParticipantId(next);
            next += 1;
            let (events, received) = tokio::sync::mpsc::unbounded_channel();
            let peer = Peer::start(PeerOptions::default());
            counters.push(Arc::clone(&peer.counters));
            media.send(
                0,
                Command::Join {
                    call: id,
                    participant,
                    user: users[index],
                    events,
                },
            );
            for (source, _) in &planned.sends {
                media.send(
                    0,
                    Command::Send {
                        participant,
                        source: *source,
                        external: false,
                    },
                );
            }
            if !planned.watches.is_empty() {
                media.send(
                    0,
                    Command::Watch {
                        participant,
                        video: planned
                            .watches
                            .iter()
                            .map(|sharer| (users[*sharer], MediaSource::Screen))
                            .collect(),
                    },
                );
            }
            tokio::spawn(drive(
                Arc::clone(&media),
                participant,
                received,
                peer,
                planned.sends.clone(),
            ));
        }
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while counters
        .iter()
        .any(|c| !c.connected.load(Ordering::Relaxed))
    {
        if Instant::now() > deadline {
            bail!("the calibration's connections did not all connect within 30 seconds");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Every video starts on its source's next keyframe, at most a second away.
    tokio::time::sleep(Duration::from_secs(2)).await;
    let received = || {
        counters
            .iter()
            .map(|c| c.received.load(Ordering::Relaxed))
            .sum::<u64>()
    };
    let cpu_before = media_thread_cpu();
    let received_before = received();
    let started = Instant::now();
    tokio::time::sleep(seconds).await;
    let elapsed = started.elapsed().as_secs_f64();
    let cpu = cores_used(cpu_before, media_thread_cpu(), elapsed)?;
    let arrived = (received() - received_before) as f64 / elapsed;
    let allocated = thread_allocated(&media)
        .await
        .saturating_sub(allocated_before) as f64;
    let units = load.units.max(1) as f64;
    Ok(Cost {
        cpu: cpu / units,
        memory_bytes: allocated / units,
        delivered: (arrived / load.expected_packets_per_second().max(1.0)).min(1.0),
    })
}

/// The cores the media thread used between two readings of `media_thread_cpu`, `elapsed`
/// seconds apart. Forwarding the calibration's streams always costs the thread time, so a
/// reading that did not rise means its thread was not found, and an estimate made from it
/// would count CPU as free.
fn cores_used(before: f64, after: f64, elapsed: f64) -> anyhow::Result<f64> {
    anyhow::ensure!(
        after > before,
        "the media thread's CPU time could not be measured (no thread of this process named \
         `{}…` was found under /proc/self/task, or it used no time), so no capacity can be \
         estimated",
        crate::metrics::MEDIA_THREAD_NAME
    );
    Ok((after - before) / elapsed)
}

/// CPU seconds the media threads of this process have used.
fn media_thread_cpu() -> f64 {
    crate::metrics::media_thread_cpu_seconds()
        .into_iter()
        .map(|(_, seconds)| seconds)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hardware(cores: f64, media_threads: usize, link: Option<f64>) -> Hardware {
        Hardware {
            cores,
            media_threads,
            memory_bytes: 16 * 1024 * 1024 * 1024,
            interface: Some("eth0".into()),
            link_mbps: link,
        }
    }

    fn cost(cpu: f64, memory_bytes: f64) -> Cost {
        Cost {
            cpu,
            memory_bytes,
            delivered: 1.0,
        }
    }

    fn shape() -> CallShape {
        CallShape {
            call_size: 6,
            screen_share: 0.2,
            audio_kbps: 64.0,
            screen_kbps: 2500.0,
        }
    }

    #[test]
    fn cpu_that_was_not_measured_stops_the_estimate() {
        assert_eq!(cores_used(1.0, 2.0, 4.0).unwrap(), 0.25);
        assert!(cores_used(0.0, 0.0, 4.0).is_err());
        assert!(cores_used(3.0, 3.0, 4.0).is_err());
    }

    #[test]
    fn a_participant_receives_no_more_microphones_than_it_has_slots() {
        let big = CallShape {
            call_size: 100,
            screen_share: 0.0,
            ..shape()
        };
        let (audio, _) = streams_received(&big);
        assert_eq!(audio, f64::from(AUDIO_SLOTS));
        let calls = Load::calls(&big);
        let per_person = calls.expected_packets_per_second() / calls.units as f64;
        assert!((per_person - f64::from(AUDIO_SLOTS) * 50.0).abs() < 1e-9);
    }

    #[test]
    fn a_participant_receives_everyone_elses_streams() {
        let (audio, video) = streams_received(&shape());
        assert!(
            (audio - 6.0).abs() < 1e-9,
            "five microphones and one screen's sound"
        );
        assert!((video - 1.0).abs() < 1e-9, "one screen");
    }

    #[test]
    fn the_scarcest_resource_sets_the_capacity() {
        let e = estimate(
            hardware(8.0, 8, Some(1000.0)),
            shape(),
            0.3,
            cost(0.0001, 50_000.0),
            cost(0.002, 300_000.0),
            cost(0.0, 0.0),
            Vec::new(),
        );
        let smallest = e.limits.iter().map(|l| l.participants).min().unwrap();
        assert_eq!(e.capacity, smallest);
        let bandwidth = e
            .limits
            .iter()
            .find(|l| l.resource == "bandwidth out")
            .unwrap();
        // 1000 Mbit/s × 70% over about 3.1 Mbit/s each: six audio streams and one screen, with
        // their packet overhead.
        assert!(
            (215..=235).contains(&bandwidth.participants),
            "{bandwidth:?}"
        );
        assert_eq!(e.capacity, bandwidth.participants);
        assert!(e.largest_call >= 6);
    }

    #[test]
    fn whole_calls_count_when_they_cost_more_than_their_streams() {
        let streams_only = estimate(
            hardware(8.0, 8, None),
            shape(),
            0.3,
            cost(0.0001, 50_000.0),
            cost(0.002, 300_000.0),
            cost(0.0, 0.0),
            Vec::new(),
        );
        let with_calls = estimate(
            hardware(8.0, 8, None),
            shape(),
            0.3,
            cost(0.0001, 50_000.0),
            cost(0.002, 300_000.0),
            cost(0.0, 3_000_000.0),
            Vec::new(),
        );
        assert!((streams_only.participant_memory_bytes - 600_000.0).abs() < 1.0);
        assert_eq!(with_calls.participant_memory_bytes, 3_000_000.0);
        assert_eq!(with_calls.participant_cpu, streams_only.participant_cpu);
    }

    #[test]
    fn media_threads_bound_the_cores_used_and_an_unknown_link_is_noted() {
        let many = estimate(
            hardware(32.0, 32, None),
            shape(),
            0.3,
            cost(0.001, 1.0),
            cost(0.01, 1.0),
            cost(0.0, 0.0),
            Vec::new(),
        );
        let few = estimate(
            hardware(32.0, 4, None),
            shape(),
            0.3,
            cost(0.001, 1.0),
            cost(0.01, 1.0),
            cost(0.0, 0.0),
            Vec::new(),
        );
        let cpu = |e: &Estimate| {
            e.limits
                .iter()
                .find(|l| l.resource == "CPU")
                .unwrap()
                .participants
        };
        // Eight times the threads, eight times the cores: each limit is rounded down.
        assert!((cpu(&few) * 8..(cpu(&few) + 1) * 8).contains(&cpu(&many)));
        assert!(few.warnings.iter().any(|w| w.contains("media_threads")));
        assert!(few.warnings.iter().any(|w| w.contains("upload_mbps")));
        assert!(few.limits.iter().all(|l| l.resource != "bandwidth out"));
    }
}
