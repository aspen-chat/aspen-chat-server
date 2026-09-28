//! The health of the deployment's servers, for the Administration Dashboard.
//!
//! Each API server writes a heartbeat every `HEARTBEAT_INTERVAL` to the NATS key-value bucket
//! `aspen_fleet`, under a key of its own: how long it has run, its open event streams, its
//! request and server error rates, its memory, and its database connections. Entries expire
//! after `BUCKET_MAX_AGE`, so a server that stops simply drops out. The figures are counted
//! where the Prometheus metrics of the same names are recorded (`aspen_metrics::api`); the
//! metrics endpoints stay on each server's loopback interface, and the heartbeat is how the
//! dashboard sees every server without reaching them. Voice servers already report their load
//! to the API servers (`app::voice`), and their health is read from those reports.

use crate::api::GlobalServerContext;
use crate::app::{self, VoiceServerId};
use crate::database::schema::voice_server;
use async_nats::jetstream::kv::{Config as KvConfig, Store};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

const BUCKET: &str = "aspen_fleet";
/// How often an API server writes its heartbeat.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
/// How long a heartbeat lasts without a newer one: a few missed beats.
const BUCKET_MAX_AGE: Duration = Duration::from_secs(30);

static REQUESTS: AtomicU64 = AtomicU64::new(0);
static SERVER_ERRORS: AtomicU64 = AtomicU64::new(0);
static EVENT_STREAMS: AtomicI64 = AtomicI64::new(0);

/// The requests answered and server errors among them, since the server started.
fn request_counts() -> (u64, u64) {
    (
        AtomicU64::load(&REQUESTS, Ordering::Relaxed),
        AtomicU64::load(&SERVER_ERRORS, Ordering::Relaxed),
    )
}

/// Counts a request this server answered, with its status.
pub fn note_request(status: u16) {
    REQUESTS.fetch_add(1, Ordering::Relaxed);
    if status >= 500 {
        SERVER_ERRORS.fetch_add(1, Ordering::Relaxed);
    }
}

/// Counts an event stream connection opening (`1`) or closing (`-1`).
pub fn note_event_stream(change: i64) {
    EVENT_STREAMS.fetch_add(change, Ordering::Relaxed);
}

/// What one API server says about itself.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApiServerHeartbeat {
    /// Chosen when the server starts, so a restarted server is a new entry.
    pub instance: String,
    pub host: String,
    pub version: String,
    pub started_at: DateTime<Utc>,
    pub reported_at: DateTime<Utc>,
    pub event_streams: i64,
    pub requests_per_minute: f64,
    pub server_errors_per_minute: f64,
    /// Bytes the allocator holds from the system; `None` where it cannot say.
    pub resident_bytes: Option<u64>,
    pub db_connections: u32,
    pub db_connections_idle: u32,
}

/// Opens the bucket, creating it on first use.
async fn bucket(context: &async_nats::jetstream::Context) -> app::Result<Store> {
    if let Ok(store) = context.get_key_value(BUCKET).await {
        return Ok(store);
    }
    context
        .create_key_value(KvConfig {
            bucket: BUCKET.to_string(),
            description: "API server heartbeats (app::fleet)".to_string(),
            history: 1,
            max_age: BUCKET_MAX_AGE,
            ..Default::default()
        })
        .await
        .map_err(|e| app::Error::NatsSubscribe(format!("could not open {BUCKET}: {e}")))
}

fn host_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| std::fs::read_to_string("/proc/sys/kernel/hostname").ok())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// Starts writing this server's heartbeat. A failed write is logged and tried again next beat.
pub fn spawn_heartbeat(state: GlobalServerContext) {
    let instance = uuid::Uuid::now_v7().to_string();
    let host = host_name();
    let started_at = Utc::now();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(HEARTBEAT_INTERVAL);
        let mut last = request_counts();
        let mut last_at = std::time::Instant::now();
        let mut store: Option<Store> = None;
        loop {
            interval.tick().await;
            let now = request_counts();
            let minutes = last_at.elapsed().as_secs_f64() / 60.0;
            let per_minute = |now: u64, then: u64| {
                if minutes > 0.0 {
                    now.saturating_sub(then) as f64 / minutes
                } else {
                    0.0
                }
            };
            let pool = state.connection_pool.status();
            let heartbeat = ApiServerHeartbeat {
                instance: instance.clone(),
                host: host.clone(),
                version: env!("CARGO_PKG_VERSION").to_string(),
                started_at,
                reported_at: Utc::now(),
                event_streams: AtomicI64::load(&EVENT_STREAMS, Ordering::Relaxed).max(0),
                requests_per_minute: per_minute(now.0, last.0),
                server_errors_per_minute: per_minute(now.1, last.1),
                resident_bytes: aspen_metrics::resident_bytes(),
                db_connections: u32::try_from(pool.size).unwrap_or(u32::MAX),
                db_connections_idle: u32::try_from(pool.available).unwrap_or(u32::MAX),
            };
            last = now;
            last_at = std::time::Instant::now();
            if store.is_none() {
                match bucket(&state.nats_context).await {
                    Ok(opened) => store = Some(opened),
                    Err(e) => {
                        tracing::warn!("could not open the fleet bucket: {e}");
                        continue;
                    }
                }
            }
            let Some(store) = &store else { continue };
            let bytes = match serde_json::to_vec(&heartbeat) {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::warn!("could not encode the heartbeat: {e}");
                    continue;
                }
            };
            if let Err(e) = store.put(format!("api.{instance}"), bytes.into()).await {
                tracing::warn!("could not write the heartbeat: {e}");
            }
        }
    });
}

/// Every API server that has written a heartbeat recently, by host.
pub async fn read_api_servers(state: &GlobalServerContext) -> app::Result<Vec<ApiServerHeartbeat>> {
    let store = bucket(&state.nats_context).await?;
    let mut keys = store
        .keys()
        .await
        .map_err(|e| app::Error::NatsSubscribe(format!("could not list {BUCKET}: {e}")))?;
    let mut servers = Vec::new();
    while let Some(key) = keys.next().await {
        let Ok(key) = key else { continue };
        if let Ok(Some(bytes)) = store.get(&key).await
            && let Ok(heartbeat) = serde_json::from_slice::<ApiServerHeartbeat>(&bytes)
        {
            servers.push(heartbeat);
        }
    }
    servers.sort_by(|a, b| a.host.cmp(&b.host).then(a.started_at.cmp(&b.started_at)));
    Ok(servers)
}

/// A voice server's standing, from the registry and its last report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceServerHealth {
    pub id: VoiceServerId,
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub capacity: i32,
    pub participants: i32,
    pub last_report_at: Option<DateTime<Utc>>,
    /// Reporting within `[voice] offer_silence_seconds`, so offered to new callers.
    pub reporting: bool,
}

/// Every registered voice server.
pub async fn read_voice_servers(
    state: &GlobalServerContext,
) -> app::Result<Vec<VoiceServerHealth>> {
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<VoiceServerRow> = voice_server::table
        .select(VoiceServerRow::as_select())
        .order(voice_server::name)
        .load(conn.as_mut())
        .await?;
    let silence = chrono::Duration::seconds(
        i64::try_from(state.config.voice.offer_silence_seconds).unwrap_or(i64::MAX),
    );
    let now = Utc::now();
    Ok(rows
        .into_iter()
        .map(|row| VoiceServerHealth {
            reporting: row.last_report_at.is_some_and(|at| now - at <= silence),
            id: row.id,
            name: row.name,
            url: row.url,
            enabled: row.enabled,
            capacity: row.capacity,
            participants: row.reported_participants,
            last_report_at: row.last_report_at,
        })
        .collect())
}

#[derive(Queryable, Selectable)]
#[diesel(table_name = voice_server)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct VoiceServerRow {
    id: VoiceServerId,
    name: String,
    url: String,
    enabled: bool,
    capacity: i32,
    reported_participants: i32,
    last_report_at: Option<DateTime<Utc>>,
}
