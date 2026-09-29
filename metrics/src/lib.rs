//! Prometheus metrics. Both servers export them on a listener of their own (`[metrics]
//! listen_addr`, loopback by default, since they describe the deployment's inside), and the
//! benchmark tool reads them by the names here to say which part of a deployment ran out
//! first. Every metric is named once, below, and used through these constants.
//!
//! Each server also exports its process's CPU time, memory, and open files
//! (`process_cpu_seconds_total`, `process_resident_memory_bytes`, ...).

use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use std::net::SocketAddr;
use std::time::Duration;

/// Logical CPUs of the host a server runs on, so its CPU time can be read as a share.
pub const HOST_CPUS: &str = "aspen_host_cpus";

/// The allocator's own view of a server's memory, exported when its global allocator is
/// jemalloc (the `jemalloc` feature). Together they tell a leak from memory the allocator
/// keeps: a leak raises `allocated`, while memory freed but not returned to the system shows as
/// `resident` well above `allocated`.
pub mod memory {
    /// Bytes the program holds, allocated and not freed.
    pub const ALLOCATED: &str = "aspen_memory_allocated_bytes";
    /// Bytes in pages the allocator has in use for those allocations.
    pub const ACTIVE: &str = "aspen_memory_active_bytes";
    /// Bytes of physical memory the allocator holds, its part of the process's resident size.
    pub const RESIDENT: &str = "aspen_memory_resident_bytes";
    /// Bytes the allocator has mapped from the system.
    pub const MAPPED: &str = "aspen_memory_mapped_bytes";
    /// Bytes of address space kept for reuse rather than returned; not resident.
    pub const RETAINED: &str = "aspen_memory_retained_bytes";
}

/// API server metrics.
pub mod api {
    /// Requests answered, by `route` (method and path template) and `status`.
    pub const HTTP_REQUESTS: &str = "aspen_http_requests_total";
    /// Time to answer, by `route`.
    pub const HTTP_DURATION: &str = "aspen_http_request_duration_seconds";
    /// Database connections, by `state`: `size` (open), `available` (idle), `waiting` (tasks
    /// waiting for one), and `max`.
    pub const DB_POOL: &str = "aspen_db_pool_connections";
    /// Time for JetStream to acknowledge a published event.
    pub const EVENT_PUBLISH_DURATION: &str = "aspen_event_publish_duration_seconds";
    /// Event copies published, one per subject an event goes to.
    pub const EVENTS_PUBLISHED: &str = "aspen_events_published_total";
    /// Open event streams that have identified.
    pub const EVENT_STREAMS: &str = "aspen_event_streams";
    /// Event frames sent to clients.
    pub const EVENTS_DELIVERED: &str = "aspen_events_delivered_total";
    /// Event stream connections, by `outcome`: `resumed`, `replayed`, `rejected`.
    pub const EVENT_STREAM_CONNECTS: &str = "aspen_event_stream_connects_total";
    /// Event streams the server closed, by `reason`: `slow` (the client fell a queue behind)
    /// or `gap` (the server's feed missed events, so every stream must resume).
    pub const EVENT_STREAMS_DROPPED: &str = "aspen_event_streams_dropped_total";
    /// Events the server holds for catch-up, the stream's retention window.
    pub const EVENT_FEED_RETAINED: &str = "aspen_event_feed_retained";
    /// Bytes of event payload the server holds for catch-up.
    pub const EVENT_FEED_RETAINED_BYTES: &str = "aspen_event_feed_retained_bytes";
    /// Time for one routing shard to hand one event to each of its connections that reads it.
    pub const EVENT_ROUTE_DURATION: &str = "aspen_event_route_duration_seconds";
    /// Time for one request's rate limit check against Valkey.
    pub const RATE_LIMIT_CHECK_DURATION: &str = "aspen_rate_limit_check_duration_seconds";
    /// Requests refused for going too fast, by `route`.
    pub const RATE_LIMIT_REFUSALS: &str = "aspen_rate_limit_refusals_total";
    /// 1 while a suspension of the rate limits is in force.
    pub const RATE_LIMITS_SUSPENDED: &str = "aspen_rate_limits_suspended";
}

/// Voice server metrics.
pub mod voice {
    pub const ROOMS: &str = "aspen_voice_rooms";
    pub const PARTICIPANTS: &str = "aspen_voice_participants";
    pub const PRODUCERS: &str = "aspen_voice_producers";
    pub const CONSUMERS: &str = "aspen_voice_consumers";
    /// WebRTC and plain transports, each holding ports from the media range.
    pub const TRANSPORTS: &str = "aspen_voice_transports";
    /// Signalling frames handled, by `kind`.
    pub const FRAMES: &str = "aspen_voice_frames_total";
    /// Signalling frames refused for going too fast, by `kind`.
    pub const FRAMES_REFUSED: &str = "aspen_voice_frames_refused_total";
    /// HTTP requests refused for going too fast, by `route`.
    pub const HTTP_REFUSED: &str = "aspen_voice_http_refused_total";
    /// CPU seconds each mediasoup worker process has used, by `worker`.
    pub const WORKER_CPU: &str = "aspen_voice_worker_cpu_seconds";
    /// 1 while a suspension of the rate limits is in force.
    pub const RATE_LIMITS_SUSPENDED: &str = "aspen_voice_rate_limits_suspended";
    /// File transfers under way, by `mode` as the receiver chose it.
    pub const TRANSFERS: &str = "aspen_voice_transfers";
    /// Bytes the TURN relay forwarded between two transfers' allocations.
    pub const RELAYED_BYTES: &str = "aspen_voice_relayed_bytes_total";
    /// Bytes the TURN relay dropped for exceeding `[transfer] relay_mbps`.
    pub const RELAY_DROPPED_BYTES: &str = "aspen_voice_relay_dropped_bytes_total";
}

/// Latency buckets, in seconds, for every `_duration_seconds` histogram: fine below 10 ms, where
/// healthy requests land, and out to 10 s, where an overloaded deployment's do.
const DURATION_BUCKETS: [f64; 16] = [
    0.0005, 0.001, 0.0025, 0.005, 0.0075, 0.01, 0.025, 0.05, 0.075, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0,
    10.0,
];

/// How often gauges that are sampled rather than kept current are refreshed.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

/// Starts exporting on `listen_addr` (`GET /metrics`), including the process's own figures.
/// Must be called inside a Tokio runtime.
pub fn install(listen_addr: SocketAddr) -> Result<(), String> {
    PrometheusBuilder::new()
        .with_http_listener(listen_addr)
        .set_buckets_for_metric(
            Matcher::Suffix("_duration_seconds".to_string()),
            &DURATION_BUCKETS,
        )
        .map_err(|e| e.to_string())?
        .install()
        .map_err(|e| format!("could not export metrics on {listen_addr}: {e}"))?;
    metrics::gauge!(HOST_CPUS)
        .set(std::thread::available_parallelism().map_or(1, |n| n.get()) as f64);
    let process = metrics_process::Collector::default();
    process.describe();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(SAMPLE_INTERVAL);
        loop {
            interval.tick().await;
            process.collect();
            #[cfg(feature = "jemalloc")]
            sample_jemalloc();
        }
    });
    tracing::info!(%listen_addr, "exporting metrics");
    Ok(())
}

/// Bytes the program holds allocated, by jemalloc's count, refreshed as it is read; `None` if the
/// statistics cannot be read.
#[cfg(feature = "jemalloc")]
pub fn allocated_bytes() -> Option<u64> {
    tikv_jemalloc_ctl::epoch::advance().ok()?;
    tikv_jemalloc_ctl::stats::allocated::read()
        .ok()
        .map(|bytes| bytes as u64)
}

/// Bytes the allocator holds in physical memory, by jemalloc's count, refreshed as it is read;
/// `None` if the statistics cannot be read, or the program does not use jemalloc.
pub fn resident_bytes() -> Option<u64> {
    #[cfg(feature = "jemalloc")]
    {
        tikv_jemalloc_ctl::epoch::advance().ok()?;
        tikv_jemalloc_ctl::stats::resident::read()
            .ok()
            .map(|bytes| bytes as u64)
    }
    #[cfg(not(feature = "jemalloc"))]
    None
}

/// Reads jemalloc's statistics into the `memory` gauges. They are refreshed only when the
/// epoch advances, which is done here.
#[cfg(feature = "jemalloc")]
fn sample_jemalloc() {
    use tikv_jemalloc_ctl::{epoch, stats};
    if let Err(e) = epoch::advance() {
        tracing::warn!("could not refresh the allocator's statistics: {e}");
        return;
    }
    let figures = [
        (memory::ALLOCATED, stats::allocated::read()),
        (memory::ACTIVE, stats::active::read()),
        (memory::RESIDENT, stats::resident::read()),
        (memory::MAPPED, stats::mapped::read()),
        (memory::RETAINED, stats::retained::read()),
    ];
    for (name, value) in figures {
        if let Ok(bytes) = value {
            metrics::gauge!(name).set(bytes as f64);
        }
    }
}
