//! `voice_server estimate-capacity`: a conservative estimate of how many people this machine
//! can hold in calls, for the `capacity` of its `[[voice.servers]]` entry.
//!
//! It reads the hardware (logical cores and any container CPU quota, memory and any container
//! limit, the link speed of the interface that holds the default route) and the media settings
//! (`workers`, each of which uses at most one core, and the RTC port range). Then it measures what
//! one forwarded stream costs on this CPU: a mediasoup worker on loopback forwards synthetic
//! Opus and H.264 streams, each to five SRTP consumers as in a call of six, for a few seconds,
//! and the worker thread's CPU time and the allocator's growth are divided by the streams.
//! Every six consumers share a receiving transport, as the streams one participant receives do.
//!
//! From a call shape (the size of a call, the share of people sharing a screen, the bitrates)
//! follows what one participant costs in CPU, memory, bandwidth out, and ports; each resource,
//! less a safety margin, then holds some number of participants, and the estimate is the
//! smallest. The calibration counts a received packet as costing as much as a sent one, which
//! leans towards too little rather than too much.

use crate::config::MediaConfig;
use crate::media::{h264_parameters, local_tuple, media_codecs};
use anyhow::{Context, bail};
use mediasoup::prelude::*;
use mediasoup::worker::WorkerLogLevel;
use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr};
use std::num::{NonZeroU8, NonZeroU32};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;

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
    /// The machine's usable bandwidth out, in Mbit/s, where the interface's speed is unknown or
    /// is not what the host provides (a cloud instance's network allowance, say).
    #[arg(long)]
    pub link_mbps: Option<f64>,
    /// How long each kind of stream is measured for.
    #[arg(long, default_value_t = 5)]
    pub calibration_seconds: u64,
    /// Print the estimate as JSON.
    #[arg(long)]
    pub json: bool,
}

/// Loopback ports the calibration's transports take: its own range, not the server's, so it runs
/// beside a running server and needs no more of the RTC range than that has.
const CALIBRATION_PORTS: std::ops::RangeInclusive<u16> = 20_000..=29_999;

/// Bytes a media packet carries besides its payload: IPv4 (20), UDP (8), RTP (12), and the
/// SRTP authentication tag (10).
const PACKET_OVERHEAD: f64 = 50.0;
/// The largest RTP payload the calibration sends, below a typical path MTU.
const MAX_PAYLOAD: usize = 1100;
/// Audio frames a second: Opus in 20 ms frames.
const AUDIO_FPS: f64 = 50.0;
/// Frames a second of a shared screen.
const VIDEO_FPS: f64 = 30.0;
/// Consumers of each calibration producer: the others in a call of six.
const FAN_OUT: usize = 5;
/// Consumers sharing one receiving transport, as the streams a participant in a call of six
/// receives share theirs.
const CONSUMERS_PER_RECEIVER: usize = 6;
/// Memory the process needs before any call: its binary, threads, and workers.
const BASE_MEMORY: u64 = 512 * 1024 * 1024;
/// Ports a participant holds from the RTC range: a transport to send and one to receive, and
/// for a game capture two more of its own, counted for everyone who shares.
const PORTS_PER_PARTICIPANT: f64 = 2.0;
const PORTS_PER_SHARER: f64 = 2.0;

#[derive(Debug, Serialize)]
pub struct Hardware {
    /// Logical cores the process may use, after any CPU quota.
    pub cores: f64,
    pub workers: usize,
    /// Memory the process may use, after any container limit.
    pub memory_bytes: u64,
    pub interface: Option<String>,
    /// Bandwidth out, from `--link-mbps` or the interface's speed.
    pub link_mbps: Option<f64>,
    pub ports: u32,
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
    /// The largest call one worker holds within the margin: a call lives on one worker.
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
    let worker = WorkerManager::new()
        .create_worker({
            let mut settings = WorkerSettings::default();
            settings.log_level = WorkerLogLevel::Error;
            settings.rtc_port_range = CALIBRATION_PORTS;
            settings
        })
        .await
        .context("could not start a mediasoup worker")?;
    let audio = calibrate(&worker, Kind::Audio, &args.shape, seconds).await?;
    let video = calibrate(&worker, Kind::Video, &args.shape, seconds).await?;
    let calls = calibrate_calls(&worker, &args.shape, seconds).await?;
    for (name, cost) in [("audio", &audio), ("video", &video), ("call", &calls)] {
        if cost.delivered < 0.95 {
            warnings.push(format!(
                "only {:.0}% of the calibration's {name} packets arrived; the worker may have been short of CPU, so the estimate may be high",
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
/// other's microphone, and each sharer's picture and sound.
fn streams_received(shape: &CallShape) -> (f64, f64) {
    let others = f64::from(shape.call_size - 1);
    let sharers = others * shape.screen_share;
    (others + sharers, sharers)
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

    let usable_cores = hardware.cores.min(hardware.workers as f64);
    if (hardware.workers as f64) < hardware.cores.floor() {
        warnings.push(format!(
            "`workers` is {} on a machine with {:.0} cores; each worker uses at most one core, so raising it to {:.0} would hold more",
            hardware.workers, hardware.cores, hardware.cores.floor()
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
                "{usable_cores:.1} cores for workers × {:.0}%, {:.3}% of a core each",
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
            "the link speed is unknown, so bandwidth is not counted; pass --link-mbps with what the host provides".to_string(),
        ),
    }
    let ports_each = PORTS_PER_PARTICIPANT + PORTS_PER_SHARER * shape.screen_share;
    limits.push(Limit {
        resource: "ports",
        participants: per(f64::from(hardware.ports), ports_each),
        detail: format!("{} UDP ports, {ports_each:.1} each", hardware.ports),
    });
    let capacity = limits.iter().map(|l| l.participants).min().unwrap_or(0);
    // Everyone in a call adds to the cost of everyone else, so the largest call one worker
    // holds is where n people each forwarding to n - 1 others fill a core.
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
        "  CPU        {:.1} cores, {} workers (each uses at most one core)",
        h.cores, h.workers
    );
    let _ = writeln!(out, "  memory     {}", bytes(h.memory_bytes as f64));
    let _ = writeln!(
        out,
        "  network    {}, {}",
        h.interface.as_deref().unwrap_or("interface unknown"),
        h.link_mbps
            .map_or("speed unknown".to_string(), |m| format!("{m:.0} Mbit/s"))
    );
    let _ = writeln!(out, "  ports      {} UDP", h.ports);
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
        "  one call fits on one worker up to about {} people",
        e.largest_call
    );
    for warning in &e.warnings {
        let _ = writeln!(out, "note: {warning}");
    }
    let _ = writeln!(
        out,
        "\nFor this server's [[voice.servers]] entry in aspen.toml:\ncapacity = {}",
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
    let link_mbps = link_mbps.or_else(|| interface.as_deref().and_then(interface_speed_mbps));
    Hardware {
        cores,
        workers: media.workers.max(1),
        memory_bytes,
        interface,
        link_mbps,
        ports: u32::from(media.rtc.max_port.saturating_sub(media.rtc.min_port)) + 1,
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

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Audio,
    Video,
}

/// Where the calibration's consumers send, and how many media packets have arrived there.
struct Sink {
    port: u16,
    received: Arc<AtomicU64>,
    drain: tokio::task::JoinHandle<()>,
}

impl Sink {
    async fn open() -> anyhow::Result<Self> {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let port = socket.local_addr()?.port();
        let received = Arc::new(AtomicU64::new(0));
        let drain = tokio::spawn({
            let received = Arc::clone(&received);
            async move {
                let mut buffer = vec![0u8; 2048];
                while let Ok(n) = socket.recv(&mut buffer).await {
                    // RTCP (payload types 64..=95 once masked) is not media.
                    if n > 1 && !(64..=95).contains(&(buffer[1] & 0x7f)) {
                        received.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        });
        Ok(Self {
            port,
            received,
            drain,
        })
    }

    fn received(&self) -> u64 {
        self.received.load(Ordering::Relaxed)
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        self.drain.abort();
    }
}

/// The router's capabilities as a receiver states them, as a client's device does.
fn receiver_capabilities(router: &Router) -> anyhow::Result<RtpCapabilities> {
    Ok(serde_json::from_value(serde_json::to_value(
        router.rtp_capabilities(),
    )?)?)
}

/// A transport that receives, sending over SRTP to the sink, as a participant's does.
async fn receiving_transport(router: &Router, sink: &Sink) -> anyhow::Result<PlainTransport> {
    let transport = router
        .create_plain_transport({
            let mut options = PlainTransportOptions::new(loopback());
            options.rtcp_mux = true;
            options.comedia = false;
            options.enable_srtp = true;
            options.srtp_crypto_suite = SrtpCryptoSuite::AesCm128HmacSha180;
            options
        })
        .await?;
    let srtp = transport.srtp_parameters();
    transport
        .connect(PlainTransportRemoteParameters {
            ip: Some(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            port: Some(sink.port),
            rtcp_port: None,
            srtp_parameters: srtp,
        })
        .await?;
    Ok(transport)
}

/// A producer of `kind` on a transport of its own, fed by a synthetic stream until `stop`.
struct Source {
    _transport: PlainTransport,
    producer: Producer,
    sender: tokio::task::JoinHandle<()>,
}

impl Source {
    async fn start(
        router: &Router,
        kind: Kind,
        ssrc: u32,
        shape: &CallShape,
        stop: &Arc<AtomicBool>,
    ) -> anyhow::Result<Self> {
        let transport = router
            .create_plain_transport({
                let mut options = PlainTransportOptions::new(loopback());
                options.rtcp_mux = true;
                options.comedia = true;
                options
            })
            .await?;
        let media = if kind == Kind::Audio {
            MediaKind::Audio
        } else {
            MediaKind::Video
        };
        let producer = transport
            .produce(ProducerOptions::new(media, producer_parameters(kind, ssrc)))
            .await?;
        let (_, port) = local_tuple(&transport);
        let (frame_bytes, fps) = frame_bytes(kind, shape);
        let sender = tokio::spawn(send_stream(
            kind,
            ssrc,
            port,
            frame_bytes,
            fps,
            Arc::clone(stop),
        ));
        Ok(Self {
            _transport: transport,
            producer,
            sender,
        })
    }
}

/// Bytes a frame of `kind` carries, and frames a second.
fn frame_bytes(kind: Kind, shape: &CallShape) -> (usize, f64) {
    let (kbps, fps) = match kind {
        Kind::Audio => (shape.audio_kbps, AUDIO_FPS),
        Kind::Video => (shape.screen_kbps, VIDEO_FPS),
    };
    (((kbps * 1000.0 / 8.0 / fps) as usize).max(1), fps)
}

/// Media packets a second one stream of `kind` carries.
fn packets_per_second(kind: Kind, shape: &CallShape) -> f64 {
    let (bytes, fps) = frame_bytes(kind, shape);
    let keyframe_extra = if kind == Kind::Video { 2.0 } else { 0.0 };
    bytes.div_ceil(MAX_PAYLOAD) as f64 * fps + keyframe_extra
}

/// Lets every consumer start (a video consumer waits for a keyframe), then measures the
/// workers' CPU, the packets that arrived, and the allocator's growth since `allocated_before`
/// over `seconds`.
async fn measure(sink: &Sink, allocated_before: u64, seconds: Duration) -> (f64, f64, f64) {
    tokio::time::sleep(Duration::from_secs(2)).await;
    let cpu_before = worker_cpu();
    let received_before = sink.received();
    let started = Instant::now();
    tokio::time::sleep(seconds).await;
    let elapsed = started.elapsed().as_secs_f64();
    let cpu = (worker_cpu() - cpu_before) / elapsed;
    let arrived = (sink.received() - received_before) as f64 / elapsed;
    let allocated = aspen_metrics::allocated_bytes().unwrap_or(0);
    (
        cpu,
        arrived,
        allocated.saturating_sub(allocated_before) as f64,
    )
}

/// Forwards synthetic streams of `kind` through a router on `worker` for `seconds` and measures
/// what one forwarded stream costs.
async fn calibrate(
    worker: &Worker,
    kind: Kind,
    shape: &CallShape,
    seconds: Duration,
) -> anyhow::Result<Cost> {
    // Enough streams for the worker's time to be measured well, few enough not to fill it.
    let producers = if kind == Kind::Audio { 60 } else { 10 };
    let router = worker
        .create_router(RouterOptions::new(media_codecs()))
        .await?;
    let capabilities = receiver_capabilities(&router)?;
    let sink = Sink::open().await?;
    let stop = Arc::new(AtomicBool::new(false));
    let allocated_before = aspen_metrics::allocated_bytes().unwrap_or(0);
    let mut receivers: Vec<PlainTransport> = Vec::new();
    let mut consumers = Vec::new();
    let mut sources = Vec::new();
    for i in 0..producers {
        let source = Source::start(&router, kind, 0x4000_0000 | i as u32, shape, &stop).await?;
        for _ in 0..FAN_OUT {
            if consumers.len() % CONSUMERS_PER_RECEIVER == 0 {
                receivers.push(receiving_transport(&router, &sink).await?);
            }
            let receiver = receivers.last().expect("a receiver was just made");
            consumers.push(
                receiver
                    .consume(ConsumerOptions::new(
                        source.producer.id(),
                        capabilities.clone(),
                    ))
                    .await?,
            );
        }
        sources.push(source);
    }
    let (cpu, arrived, allocated) = measure(&sink, allocated_before, seconds).await;
    stop.store(true, Ordering::Relaxed);
    for source in &mut sources {
        let _ = (&mut source.sender).await;
    }
    let streams = (producers * FAN_OUT) as f64;
    let expected = streams * packets_per_second(kind, shape);
    Ok(Cost {
        cpu: cpu / streams,
        memory_bytes: allocated / streams,
        delivered: (arrived / expected).min(1.0),
    })
}

/// What one participant costs in whole calls of the given shape: every call its own router,
/// every participant a microphone and a receiving transport that consumes everyone else, and
/// the call's share of them a screen with its sound as well. This counts what the per-stream
/// figures leave out: each call's router and each participant's transports.
async fn calibrate_calls(
    worker: &Worker,
    shape: &CallShape,
    seconds: Duration,
) -> anyhow::Result<Cost> {
    let size = shape.call_size as usize;
    // Enough people to measure, however large a call.
    let calls = (60 / size).max(1);
    let sink = Sink::open().await?;
    let stop = Arc::new(AtomicBool::new(false));
    let allocated_before = aspen_metrics::allocated_bytes().unwrap_or(0);
    let mut kept = Vec::new();
    let mut ssrc = 0x5000_0000u32;
    let mut expected = 0.0;
    for call in 0..calls {
        let router = worker
            .create_router(RouterOptions::new(media_codecs()))
            .await?;
        let capabilities = receiver_capabilities(&router)?;
        // Spread the sharers over the calls so their share of everyone is the shape's.
        let sharers_before = (shape.screen_share * (call * size) as f64).floor() as usize;
        let sharers =
            (shape.screen_share * ((call + 1) * size) as f64).floor() as usize - sharers_before;
        let mut sources: Vec<(usize, Source)> = Vec::new();
        for person in 0..size {
            let mut kinds = vec![Kind::Audio];
            if person < sharers {
                kinds.extend([Kind::Video, Kind::Audio]);
            }
            for kind in kinds {
                ssrc += 1;
                sources.push((
                    person,
                    Source::start(&router, kind, ssrc, shape, &stop).await?,
                ));
            }
        }
        let mut consumers = Vec::new();
        let mut receivers = Vec::new();
        for person in 0..size {
            let receiver = receiving_transport(&router, &sink).await?;
            for (owner, source) in &sources {
                if *owner == person {
                    continue;
                }
                let kind = if source.producer.kind() == MediaKind::Audio {
                    Kind::Audio
                } else {
                    Kind::Video
                };
                expected += packets_per_second(kind, shape);
                consumers.push(
                    receiver
                        .consume(ConsumerOptions::new(
                            source.producer.id(),
                            capabilities.clone(),
                        ))
                        .await?,
                );
            }
            receivers.push(receiver);
        }
        kept.push((router, sources, receivers, consumers));
    }
    let (cpu, arrived, allocated) = measure(&sink, allocated_before, seconds).await;
    stop.store(true, Ordering::Relaxed);
    for (_, sources, _, _) in &mut kept {
        for (_, source) in sources {
            let _ = (&mut source.sender).await;
        }
    }
    let participants = (calls * size) as f64;
    Ok(Cost {
        cpu: cpu / participants,
        memory_bytes: allocated / participants,
        delivered: if expected > 0.0 {
            (arrived / expected).min(1.0)
        } else {
            1.0
        },
    })
}

fn loopback() -> ListenInfo {
    ListenInfo {
        protocol: Protocol::Udp,
        ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        announced_address: None,
        expose_internal_ip: false,
        port: None,
        port_range: None,
        flags: None,
        send_buffer_size: None,
        recv_buffer_size: None,
    }
}

/// A producer of `kind` as the voice server makes them for clients that send RTP themselves.
fn producer_parameters(kind: Kind, ssrc: u32) -> RtpParameters {
    let codec = match kind {
        Kind::Audio => RtpCodecParameters::Audio {
            mime_type: MimeTypeAudio::Opus,
            payload_type: 100,
            clock_rate: NonZeroU32::new(48_000).expect("clock rate"),
            channels: NonZeroU8::new(2).expect("channels"),
            parameters: RtpCodecParametersParameters::default(),
            rtcp_feedback: vec![],
        },
        Kind::Video => RtpCodecParameters::Video {
            mime_type: MimeTypeVideo::H264,
            payload_type: 96,
            clock_rate: NonZeroU32::new(90_000).expect("clock rate"),
            parameters: h264_parameters(),
            rtcp_feedback: vec![RtcpFeedback::Nack, RtcpFeedback::NackPli],
        },
    };
    RtpParameters {
        mid: None,
        msid: None,
        codecs: vec![codec],
        header_extensions: vec![],
        encodings: vec![RtpEncodingParameters {
            ssrc: Some(ssrc),
            ..RtpEncodingParameters::default()
        }],
        rtcp: RtcpParameters {
            cname: Some(format!("calibration-{ssrc}")),
            reduced_size: true,
        },
    }
}

/// Sends one synthetic stream to a producer's transport until `stop`: Opus-sized frames, or
/// H.264-shaped ones split into packets, with a keyframe every second so every consumer starts.
async fn send_stream(
    kind: Kind,
    ssrc: u32,
    port: u16,
    frame_bytes: usize,
    fps: f64,
    stop: Arc<AtomicBool>,
) {
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await else {
        return;
    };
    if socket.connect((Ipv4Addr::LOCALHOST, port)).await.is_err() {
        return;
    }
    let (payload_type, clock_rate) = match kind {
        Kind::Audio => (100u8, 48_000.0),
        Kind::Video => (96u8, 90_000.0),
    };
    let step = (clock_rate / fps) as u32;
    let mut ticker = tokio::time::interval(Duration::from_secs_f64(1.0 / fps));
    let (mut sequence, mut timestamp, mut frame) = (0u16, 0u32, 0u64);
    let filler = vec![0x5a_u8; MAX_PAYLOAD];
    while !stop.load(Ordering::Relaxed) {
        ticker.tick().await;
        let keyframe = kind == Kind::Video && frame.is_multiple_of(fps as u64);
        if keyframe {
            // A sequence and a picture parameter set lead every keyframe, as an encoder's do;
            // the sequence parameter set is what mediasoup recognises a keyframe by.
            for (nal, size) in [(0x67u8, 16usize), (0x68, 4)] {
                let mut packet = rtp_header(payload_type, false, sequence, timestamp, ssrc);
                packet.push(nal);
                packet.extend_from_slice(&filler[..size - 1]);
                let _ = socket.send(&packet).await;
                sequence = sequence.wrapping_add(1);
            }
        }
        let mut left = frame_bytes;
        while left > 0 {
            let size = left.min(MAX_PAYLOAD);
            left -= size;
            let mut packet = rtp_header(
                payload_type,
                kind == Kind::Video && left == 0,
                sequence,
                timestamp,
                ssrc,
            );
            packet.push(match kind {
                Kind::Audio => 0x78,
                Kind::Video if keyframe => 0x65,
                Kind::Video => 0x41,
            });
            packet.extend_from_slice(&filler[..size - 1]);
            let _ = socket.send(&packet).await;
            sequence = sequence.wrapping_add(1);
        }
        timestamp = timestamp.wrapping_add(step);
        frame += 1;
    }
}

/// The twelve bytes of an RTP header with no CSRCs or extensions.
fn rtp_header(payload_type: u8, marker: bool, sequence: u16, timestamp: u32, ssrc: u32) -> Vec<u8> {
    let mut header = Vec::with_capacity(12 + MAX_PAYLOAD);
    header.push(0x80);
    header.push(payload_type | if marker { 0x80 } else { 0 });
    header.extend_from_slice(&sequence.to_be_bytes());
    header.extend_from_slice(&timestamp.to_be_bytes());
    header.extend_from_slice(&ssrc.to_be_bytes());
    header
}

/// CPU seconds all mediasoup worker threads of this process have used.
fn worker_cpu() -> f64 {
    crate::metrics::worker_cpu_seconds()
        .into_iter()
        .map(|(_, seconds)| seconds)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hardware(cores: f64, workers: usize, link: Option<f64>) -> Hardware {
        Hardware {
            cores,
            workers,
            memory_bytes: 16 * 1024 * 1024 * 1024,
            interface: Some("eth0".into()),
            link_mbps: link,
            ports: 10_000,
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
    fn workers_bound_the_cores_used_and_an_unknown_link_is_noted() {
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
        // Eight times the workers, eight times the cores: each limit is rounded down.
        assert!((cpu(&few) * 8..(cpu(&few) + 1) * 8).contains(&cpu(&many)));
        assert!(few.warnings.iter().any(|w| w.contains("workers")));
        assert!(few.warnings.iter().any(|w| w.contains("--link-mbps")));
        assert!(few.limits.iter().all(|l| l.resource != "bandwidth out"));
    }
}
