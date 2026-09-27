//! What an agent measures. Every virtual user reports samples to one `Recorder`; a single task
//! keeps the histograms (HdrHistogram, microseconds, three significant figures) and counters,
//! and hands them over as a `Snapshot` every report interval and at every change of phase.
//!
//! Metric names:
//! - `http:<route>`: a request, from when it was due to start to its answer (see
//!   `crate::user`), by route (`POST /channels/{channel}/messages`);
//! - `delivery`, `delivery:dm`: a message from its sender to each receiver;
//! - `connect`: signing in, starting up, and the event stream becoming ready;
//! - `ready`: the event stream, from opening it to `ready`, reconnections included;
//! - `lag`: how late the generator started actions it had planned;
//! - `recovery`: from the server dropping an event stream to its resumption;
//! - `voice:*`: see `crate::voice`.
//!
//! Counters: `requests`, `errors`, `status:<route>:<code>`, `events`, `messages_sent`,
//! `rate_limited`, `disconnects`, and others named where they are counted.

use crate::clock::Clock;
use aspen_bench_protocol::coordination::{Phase, Snapshot};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hdrhistogram::Histogram;
use hdrhistogram::serialization::{Deserializer, Serializer, V2Serializer};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::mpsc;

/// The longest latency recorded exactly: an hour.
const MAX_MICROS: u64 = 3_600_000_000;

pub fn histogram() -> Histogram<u64> {
    Histogram::new_with_bounds(1, MAX_MICROS, 3).expect("valid histogram bounds")
}

enum Sample {
    Latency(Arc<str>, u64),
    Count(Arc<str>, u64),
    Phase(Phase),
}

/// Where virtual users report; cheap to clone.
#[derive(Clone)]
pub struct Recorder {
    tx: mpsc::UnboundedSender<Sample>,
    connected: Arc<AtomicU64>,
}

impl Recorder {
    pub fn latency(&self, name: impl Into<Arc<str>>, elapsed: Duration) {
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        let _ = self.tx.send(Sample::Latency(name.into(), micros));
    }

    pub fn latency_micros(&self, name: impl Into<Arc<str>>, micros: u64) {
        let _ = self.tx.send(Sample::Latency(name.into(), micros));
    }

    pub fn count(&self, name: impl Into<Arc<str>>, n: u64) {
        let _ = self.tx.send(Sample::Count(name.into(), n));
    }

    /// Starts a new phase; measurements from here on belong to it.
    pub fn phase(&self, phase: Phase) {
        let _ = self.tx.send(Sample::Phase(phase));
    }

    pub fn connected(&self) -> &AtomicU64 {
        &self.connected
    }
}

/// Starts the aggregating task. Snapshots arrive on the returned receiver every `interval`,
/// at each phase change, and once more when every `Recorder` is dropped.
pub fn start(
    agent: u32,
    interval: Duration,
    clock: Clock,
) -> (Recorder, mpsc::UnboundedReceiver<Snapshot>) {
    let (tx, mut rx) = mpsc::unbounded_channel::<Sample>();
    let (out, snapshots) = mpsc::unbounded_channel();
    let connected = Arc::new(AtomicU64::new(0));
    let recorder = Recorder {
        tx,
        connected: Arc::clone(&connected),
    };
    tokio::spawn(async move {
        let mut histograms: HashMap<Arc<str>, Histogram<u64>> = HashMap::new();
        let mut counters: BTreeMap<String, u64> = BTreeMap::new();
        let mut phase: Option<Phase> = None;
        let mut index = 0u64;
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await;
        let flush = |histograms: &mut HashMap<Arc<str>, Histogram<u64>>,
                     counters: &mut BTreeMap<String, u64>,
                     phase: Option<Phase>,
                     index: &mut u64| {
            let snapshot = Snapshot {
                agent,
                interval: *index,
                phase,
                histograms: histograms
                    .drain()
                    .map(|(name, h)| (name.to_string(), encode(&h)))
                    .collect(),
                counters: std::mem::take(counters),
                connected: connected.load(Ordering::Relaxed),
                ended_ns: clock.now_ns(),
            };
            *index += 1;
            let _ = out.send(snapshot);
        };
        loop {
            tokio::select! {
                sample = rx.recv() => match sample {
                    Some(Sample::Latency(name, micros)) => {
                        let _ = histograms
                            .entry(name)
                            .or_insert_with(histogram)
                            .record(micros.clamp(1, MAX_MICROS));
                    }
                    Some(Sample::Count(name, n)) => {
                        *counters.entry(name.to_string()).or_default() += n;
                    }
                    Some(Sample::Phase(next)) => {
                        flush(&mut histograms, &mut counters, phase, &mut index);
                        phase = Some(next);
                        ticker.reset();
                    }
                    None => {
                        flush(&mut histograms, &mut counters, phase, &mut index);
                        return;
                    }
                },
                _ = ticker.tick() => flush(&mut histograms, &mut counters, phase, &mut index),
            }
        }
    });
    (recorder, snapshots)
}

pub fn encode(histogram: &Histogram<u64>) -> String {
    let mut bytes = Vec::new();
    V2Serializer::new()
        .serialize(histogram, &mut bytes)
        .expect("histograms serialize to memory");
    STANDARD.encode(bytes)
}

pub fn decode(text: &str) -> Result<Histogram<u64>, String> {
    let bytes = STANDARD.decode(text).map_err(|e| e.to_string())?;
    Deserializer::new()
        .deserialize(&mut bytes.as_slice())
        .map_err(|e| format!("{e:?}"))
}

/// A histogram's figures, in milliseconds.
#[derive(Clone, Copy, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub count: u64,
    pub p50_ms: f64,
    pub p90_ms: f64,
    pub p99_ms: f64,
    pub p999_ms: f64,
    pub max_ms: f64,
    pub mean_ms: f64,
}

impl Summary {
    pub fn of(h: &Histogram<u64>) -> Self {
        let ms = |micros: u64| micros as f64 / 1000.0;
        Self {
            count: h.len(),
            p50_ms: ms(h.value_at_quantile(0.5)),
            p90_ms: ms(h.value_at_quantile(0.9)),
            p99_ms: ms(h.value_at_quantile(0.99)),
            p999_ms: ms(h.value_at_quantile(0.999)),
            max_ms: ms(h.max()),
            mean_ms: h.mean() / 1000.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn samples_come_back_in_snapshots_by_phase() {
        let (recorder, mut snapshots) = start(3, Duration::from_secs(3600), Clock::default());
        recorder.phase(Phase::Ramp);
        recorder.latency("http:GET /x", Duration::from_millis(5));
        recorder.count("requests", 2);
        recorder.phase(Phase::Steady);
        recorder.latency("http:GET /x", Duration::from_millis(7));
        drop(recorder);
        let mut all = Vec::new();
        while let Some(s) = snapshots.recv().await {
            all.push(s);
        }
        let ramp = all.iter().find(|s| s.phase == Some(Phase::Ramp)).unwrap();
        assert_eq!(ramp.counters["requests"], 2);
        assert_eq!(ramp.agent, 3);
        let h = decode(&ramp.histograms["http:GET /x"]).unwrap();
        assert_eq!(h.len(), 1);
        assert!((4900..=5100).contains(&h.max()));
        let steady = all.iter().find(|s| s.phase == Some(Phase::Steady)).unwrap();
        assert_eq!(decode(&steady.histograms["http:GET /x"]).unwrap().len(), 1);
        assert!(steady.interval > ramp.interval);
    }

    #[test]
    fn histograms_round_trip() {
        let mut h = histogram();
        for v in [1, 10, 100, 1000] {
            h.record(v).unwrap();
        }
        let back = decode(&encode(&h)).unwrap();
        assert_eq!(back.len(), 4);
        assert_eq!(Summary::of(&back).count, 4);
    }
}
