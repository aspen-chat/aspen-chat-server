//! Suspending rate limits for a while, as a benchmark needs.
//!
//! An operator suspends limits with the API server's `limits suspend` command, which writes one
//! record to the NATS key-value bucket `BUCKET`: when it started, when it ends, why, and what
//! it covers. Every API and voice server watches the record. By default a suspension exempts a
//! list of client networks (the load generators') from the limits that count by address, and
//! leaves every other limit in force, since those describe what a real user may do and a
//! benchmark should show real traffic stays inside them; `scope = all` lifts every limit.
//!
//! A suspension always ends by itself. Each server compares the record's end with its own
//! clock and, whatever the record says, never honours one for longer than its own
//! `max_suspension_seconds` after it started, so a forgotten or mistaken record lapses. The
//! bucket also discards records older than `BUCKET_MAX_AGE`. While one is in force every server
//! logs a warning each minute.

use async_nats::jetstream::kv::{Config as KvConfig, Operation, Store};
use futures_util::StreamExt;
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The key-value bucket holding the suspension record, and its one key.
pub const BUCKET: &str = "aspen_rate_limits";
pub const KEY: &str = "suspension";
/// Records older than this vanish from the bucket whatever any server's limit.
pub const BUCKET_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How often a server says limits are suspended.
const WARNING_INTERVAL: Duration = Duration::from_secs(60);

/// What a suspension covers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Scope {
    /// Requests from these networks skip the limits that count by address.
    Networks { networks: Vec<String> },
    /// Every limit, for everyone.
    All,
}

/// The record in the bucket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Suspension {
    /// Unix milliseconds.
    pub started_at: u64,
    pub until: u64,
    pub scope: Scope,
    pub reason: String,
    /// Who suspended the limits: the operator's user and host, as the command saw them.
    pub by: String,
}

/// What a suspension does for one request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Exemption {
    None,
    /// Skip the limits that count by address.
    AddressLimits,
    All,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl Suspension {
    /// When this server stops honouring the record: its end, or `max` after it started,
    /// whichever comes first.
    pub fn effective_until(&self, max: Duration) -> u64 {
        let cap = self
            .started_at
            .saturating_add(u64::try_from(max.as_millis()).unwrap_or(u64::MAX));
        self.until.min(cap)
    }

    /// Checks a record before it is written; the error says what is wrong.
    pub fn validate(&self, max: Duration) -> Result<Vec<IpNet>, String> {
        if self.until <= self.started_at {
            return Err("a suspension must end after it starts".into());
        }
        if self.until - self.started_at > u64::try_from(max.as_millis()).unwrap_or(u64::MAX) {
            return Err(format!(
                "a suspension may last at most {} seconds (max_suspension_seconds)",
                max.as_secs()
            ));
        }
        match &self.scope {
            Scope::All => Ok(Vec::new()),
            Scope::Networks { networks } if networks.is_empty() => {
                Err("a suspension for networks must name at least one".into())
            }
            Scope::Networks { networks } => networks
                .iter()
                .map(|network| parse_network(network))
                .collect(),
        }
    }
}

fn parse_network(text: &str) -> Result<IpNet, String> {
    text.parse::<IpNet>()
        .or_else(|_| text.parse::<IpAddr>().map(IpNet::from))
        .map_err(|_| format!("{text:?} is not an address or network"))
}

/// A suspension as a server holds it, with its networks parsed.
#[derive(Clone, Debug)]
struct Active {
    record: Suspension,
    networks: Vec<IpNet>,
}

/// The suspension this server honours, kept current by `watch`.
#[derive(Clone, Debug)]
pub struct SuspensionState {
    active: Arc<RwLock<Option<Active>>>,
    max: Duration,
}

impl SuspensionState {
    /// No suspension, honouring any later one for at most `max`.
    pub fn new(max: Duration) -> Self {
        Self {
            active: Arc::new(RwLock::new(None)),
            max,
        }
    }

    fn set(&self, record: Option<Suspension>) {
        let active = record.and_then(|record| match &record.scope {
            Scope::All => Some(Active {
                record,
                networks: Vec::new(),
            }),
            Scope::Networks { networks } => {
                match networks.iter().map(|n| parse_network(n)).collect() {
                    Ok(networks) => Some(Active { record, networks }),
                    Err(e) => {
                        tracing::error!(error = e, "ignoring a malformed rate limit suspension");
                        None
                    }
                }
            }
        });
        *self.active.write().expect("suspension lock") = active;
    }

    /// The record in force now, if any.
    pub fn current(&self) -> Option<Suspension> {
        let now = now_ms();
        self.active
            .read()
            .expect("suspension lock")
            .as_ref()
            .filter(|active| now < active.record.effective_until(self.max))
            .map(|active| active.record.clone())
    }

    /// What the suspension in force does for a request from `ip`.
    pub fn exemption(&self, ip: Option<IpAddr>) -> Exemption {
        let guard = self.active.read().expect("suspension lock");
        let Some(active) = guard.as_ref() else {
            return Exemption::None;
        };
        if now_ms() >= active.record.effective_until(self.max) {
            return Exemption::None;
        }
        match active.record.scope {
            Scope::All => Exemption::All,
            Scope::Networks { .. } => match ip {
                Some(ip) => {
                    let ip = crate::canonical(ip);
                    if active.networks.iter().any(|net| net.contains(&ip)) {
                        Exemption::AddressLimits
                    } else {
                        Exemption::None
                    }
                }
                None => Exemption::None,
            },
        }
    }
}

/// Opens the bucket, creating it on first use.
pub async fn bucket(client: async_nats::Client) -> Result<Store, String> {
    let context = async_nats::jetstream::new(client);
    if let Ok(store) = context.get_key_value(BUCKET).await {
        return Ok(store);
    }
    context
        .create_key_value(KvConfig {
            bucket: BUCKET.to_string(),
            description: "Rate limit suspensions (aspen_limits::suspension)".to_string(),
            history: 1,
            max_age: BUCKET_MAX_AGE,
            ..Default::default()
        })
        .await
        .map_err(|e| format!("could not create the {BUCKET} bucket: {e}"))
}

/// The record in the bucket, whether or not it is still in force.
pub async fn read(store: &Store) -> Result<Option<Suspension>, String> {
    let value = store
        .get(KEY)
        .await
        .map_err(|e| format!("could not read the suspension: {e}"))?;
    value
        .map(|bytes| {
            serde_json::from_slice(&bytes).map_err(|e| format!("malformed suspension: {e}"))
        })
        .transpose()
}

pub async fn write(store: &Store, record: &Suspension) -> Result<(), String> {
    let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    store
        .put(KEY, bytes.into())
        .await
        .map(|_| ())
        .map_err(|e| format!("could not write the suspension: {e}"))
}

pub async fn clear(store: &Store) -> Result<(), String> {
    store
        .purge(KEY)
        .await
        .map_err(|e| format!("could not remove the suspension: {e}"))
}

/// Keeps `state` current from the bucket for as long as the process runs, reconnecting after
/// failures, and logs while a suspension is in force and when it ends. `server` names this
/// process in the log.
pub fn watch(client: async_nats::Client, state: SuspensionState, server: &'static str) {
    {
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = follow(client.clone(), &state).await {
                    tracing::warn!(error = e, "rate limit suspension watch failed; retrying");
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }
    tokio::spawn(async move {
        let mut was_active = false;
        let mut interval = tokio::time::interval(WARNING_INTERVAL);
        loop {
            interval.tick().await;
            match state.current() {
                Some(record) => {
                    let remaining =
                        record.effective_until(state.max).saturating_sub(now_ms()) / 1000;
                    tracing::warn!(
                        server,
                        reason = record.reason,
                        by = record.by,
                        scope = ?record.scope,
                        remaining_seconds = remaining,
                        "RATE LIMITS ARE SUSPENDED"
                    );
                    was_active = true;
                }
                None if was_active => {
                    tracing::info!(server, "rate limits are in force again");
                    was_active = false;
                }
                None => {}
            }
        }
    });
}

async fn follow(client: async_nats::Client, state: &SuspensionState) -> Result<(), String> {
    let store = bucket(client).await?;
    state.set(read(&store).await?);
    let mut updates = store
        .watch(KEY)
        .await
        .map_err(|e| format!("could not watch the suspension: {e}"))?;
    while let Some(update) = updates.next().await {
        let entry = update.map_err(|e| e.to_string())?;
        match entry.operation {
            Operation::Put => match serde_json::from_slice::<Suspension>(&entry.value) {
                Ok(record) => {
                    tracing::warn!(
                        reason = record.reason,
                        by = record.by,
                        "rate limits suspended"
                    );
                    state.set(Some(record));
                }
                Err(e) => tracing::error!(error = %e, "ignoring a malformed suspension"),
            },
            Operation::Delete | Operation::Purge => {
                if state.current().is_some() {
                    tracing::info!("rate limit suspension withdrawn");
                }
                state.set(None);
            }
        }
    }
    Err("the suspension watch ended".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(scope: Scope, from_now_ms: i64) -> Suspension {
        let now = now_ms();
        Suspension {
            started_at: now - 1000,
            until: now.saturating_add_signed(from_now_ms),
            scope,
            reason: "test".into(),
            by: "tester".into(),
        }
    }

    #[test]
    fn networks_exempt_their_addresses_from_address_limits_only() {
        let state = SuspensionState::new(Duration::from_secs(3600));
        let ip = |text: &str| Some(text.parse::<IpAddr>().unwrap());
        assert_eq!(state.exemption(ip("10.1.2.3")), Exemption::None);
        state.set(Some(record(
            Scope::Networks {
                networks: vec!["10.0.0.0/8".into(), "192.0.2.7".into()],
            },
            60_000,
        )));
        assert_eq!(state.exemption(ip("10.1.2.3")), Exemption::AddressLimits);
        assert_eq!(
            state.exemption(ip("::ffff:192.0.2.7")),
            Exemption::AddressLimits
        );
        assert_eq!(state.exemption(ip("198.51.100.1")), Exemption::None);
        assert_eq!(state.exemption(None), Exemption::None);
    }

    #[test]
    fn all_exempts_everyone() {
        let state = SuspensionState::new(Duration::from_secs(3600));
        state.set(Some(record(Scope::All, 60_000)));
        assert_eq!(state.exemption(None), Exemption::All);
        assert!(state.current().is_some());
    }

    #[test]
    fn a_suspension_ends_by_itself() {
        let state = SuspensionState::new(Duration::from_secs(3600));
        state.set(Some(record(Scope::All, -1)));
        assert_eq!(state.exemption(None), Exemption::None);
        assert!(state.current().is_none());
    }

    #[test]
    fn a_server_honours_a_record_no_longer_than_its_own_limit() {
        // The record claims a day; this server allows half a second.
        let state = SuspensionState::new(Duration::from_millis(500));
        state.set(Some(record(Scope::All, 86_400_000)));
        assert_eq!(state.exemption(None), Exemption::None);
    }

    #[test]
    fn records_are_checked_before_they_are_written() {
        let max = Duration::from_secs(3600);
        assert!(record(Scope::All, 60_000).validate(max).is_ok());
        assert!(
            record(Scope::All, 7_200_000)
                .validate(max)
                .unwrap_err()
                .contains("at most")
        );
        assert!(
            record(Scope::Networks { networks: vec![] }, 60_000)
                .validate(max)
                .is_err()
        );
        assert!(
            record(
                Scope::Networks {
                    networks: vec!["nowhere".into()]
                },
                60_000
            )
            .validate(max)
            .is_err()
        );
        let mut backwards = record(Scope::All, 60_000);
        backwards.until = backwards.started_at;
        assert!(backwards.validate(max).is_err());
    }

    #[test]
    fn the_record_round_trips_as_json() {
        let original = record(
            Scope::Networks {
                networks: vec!["10.0.0.0/8".into()],
            },
            1000,
        );
        let text = serde_json::to_string(&original).unwrap();
        assert!(text.contains("\"kind\":\"networks\""), "{text}");
        assert_eq!(serde_json::from_str::<Suspension>(&text).unwrap(), original);
    }
}
