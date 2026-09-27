//! A run's results: agents' snapshots merged into figures per phase and over time, the
//! deployment's own metrics, and the verdict against the profile's service levels.

use crate::profile::Profile;
use crate::scrape::{self, Finding, Sample};
use crate::stats::{self, Summary};
use aspen_bench_protocol::coordination::{Phase, Snapshot};
use hdrhistogram::Histogram;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The sign-in request's latency, which `http:*` leaves out.
const SIGN_IN_METRIC: &str = "http:POST /auth/login";

/// Merges snapshots as they arrive.
pub struct Aggregate {
    start_ns: i64,
    interval_ns: i64,
    phases: BTreeMap<Phase, PhaseAccumulator>,
    timeline: BTreeMap<i64, Point>,
}

#[derive(Default)]
struct PhaseAccumulator {
    histograms: BTreeMap<String, Histogram<u64>>,
    counters: BTreeMap<String, u64>,
    first_ns: Option<i64>,
    last_ns: Option<i64>,
}

/// One report interval across every agent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    /// Seconds from the start, at the interval's end.
    pub t: f64,
    pub phase: Option<Phase>,
    pub connected: u64,
    pub metrics: BTreeMap<String, Summary>,
    pub counters: BTreeMap<String, u64>,
    #[serde(skip)]
    histograms: BTreeMap<String, Histogram<u64>>,
    #[serde(skip)]
    connected_by_agent: BTreeMap<u32, u64>,
}

impl Aggregate {
    pub fn new(start_ns: i64, interval_seconds: f64) -> Self {
        Self {
            start_ns,
            interval_ns: (interval_seconds * 1e9) as i64,
            phases: BTreeMap::new(),
            timeline: BTreeMap::new(),
        }
    }

    pub fn add(&mut self, snapshot: &Snapshot) -> Result<(), String> {
        let bucket =
            ((snapshot.ended_ns - self.start_ns) as f64 / self.interval_ns as f64).round() as i64;
        let point = self.timeline.entry(bucket).or_default();
        point.t = bucket as f64 * self.interval_ns as f64 / 1e9;
        point.phase = point.phase.max(snapshot.phase);
        point
            .connected_by_agent
            .insert(snapshot.agent, snapshot.connected);
        point.connected = point.connected_by_agent.values().sum();
        for (name, n) in &snapshot.counters {
            *point.counters.entry(name.clone()).or_default() += n;
        }
        let Some(phase) = snapshot.phase else {
            return Ok(());
        };
        let acc = self.phases.entry(phase).or_default();
        acc.first_ns = Some(
            acc.first_ns
                .map_or(snapshot.ended_ns, |f| f.min(snapshot.ended_ns)),
        );
        acc.last_ns = Some(
            acc.last_ns
                .map_or(snapshot.ended_ns, |l| l.max(snapshot.ended_ns)),
        );
        for (name, n) in &snapshot.counters {
            *acc.counters.entry(name.clone()).or_default() += n;
        }
        for (name, encoded) in &snapshot.histograms {
            let h = stats::decode(encoded)?;
            acc.histograms
                .entry(name.clone())
                .or_insert_with(stats::histogram)
                .add(&h)
                .map_err(|e| format!("{e:?}"))?;
            point
                .histograms
                .entry(name.clone())
                .or_insert_with(stats::histogram)
                .add(&h)
                .map_err(|e| format!("{e:?}"))?;
        }
        Ok(())
    }

    /// Summaries per phase, with the steady phase's histograms kept for comparison.
    pub fn finish(self, steady_seconds: f64) -> (BTreeMap<Phase, PhaseStats>, Vec<Point>) {
        let phases = self
            .phases
            .into_iter()
            .map(|(phase, acc)| {
                let seconds = if phase == Phase::Steady {
                    steady_seconds
                } else {
                    acc.first_ns
                        .zip(acc.last_ns)
                        .map_or(0.0, |(f, l)| ((l - f) as f64 / 1e9).max(1.0))
                };
                let mut all = stats::histogram();
                for (name, h) in &acc.histograms {
                    // Signing in is slow on purpose (password hashing) and has its own level,
                    // `connect`; `http:*` is every other request.
                    if name.starts_with("http:") && name != SIGN_IN_METRIC {
                        let _ = all.add(h);
                    }
                }
                let mut metrics: BTreeMap<String, Summary> = acc
                    .histograms
                    .iter()
                    .map(|(name, h)| (name.clone(), Summary::of(h)))
                    .collect();
                if !all.is_empty() {
                    metrics.insert("http:*".into(), Summary::of(&all));
                }
                let encoded = acc
                    .histograms
                    .iter()
                    .map(|(name, h)| (name.clone(), stats::encode(h)))
                    .collect();
                (
                    phase,
                    PhaseStats {
                        seconds,
                        metrics,
                        counters: acc.counters,
                        histograms: encoded,
                    },
                )
            })
            .collect();
        let timeline = self
            .timeline
            .into_values()
            .map(|mut point| {
                point.metrics = point
                    .histograms
                    .iter()
                    .map(|(name, h)| (name.clone(), Summary::of(h)))
                    .collect();
                point
            })
            .collect();
        (phases, timeline)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhaseStats {
    pub seconds: f64,
    pub metrics: BTreeMap<String, Summary>,
    pub counters: BTreeMap<String, u64>,
    /// Base64 HdrHistogram V2, for comparing runs exactly.
    pub histograms: BTreeMap<String, String>,
}

impl PhaseStats {
    pub fn counter(&self, name: &str) -> u64 {
        self.counters.get(name).copied().unwrap_or(0)
    }

    pub fn per_second(&self, name: &str) -> f64 {
        if self.seconds > 0.0 {
            self.counter(name) as f64 / self.seconds
        } else {
            0.0
        }
    }
}

/// One service level, judged.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    pub target: f64,
    pub actual: Option<f64>,
    pub pass: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Verdict {
    /// Every service level met, with a trustworthy generator.
    pub pass: bool,
    pub checks: Vec<Check>,
    /// Whether the load generator kept up with its own schedule. When it did not, the run
    /// measured the generator, and says so.
    pub generator_kept_up: bool,
    pub generator_lag_p99_ms: f64,
}

/// The figures of `name` over several phases together.
fn across(phases: &[Option<&PhaseStats>], name: &str) -> Option<Summary> {
    let mut merged = stats::histogram();
    for phase in phases.iter().flatten() {
        if let Some(h) = phase
            .histograms
            .get(name)
            .and_then(|e| stats::decode(e).ok())
        {
            let _ = merged.add(&h);
        }
    }
    (!merged.is_empty()).then(|| Summary::of(&merged))
}

/// How many users were connected through the steady phase: the median over its intervals, so
/// neither a late straggler nor the first to leave at the end moves it.
pub fn connected_in_steady(timeline: &[Point]) -> Option<u64> {
    let mut connected: Vec<u64> = timeline
        .iter()
        .filter(|p| p.phase == Some(Phase::Steady))
        .map(|p| p.connected)
        .collect();
    connected.sort_unstable();
    connected.get(connected.len() / 2).copied()
}

/// Judges a run against the profile's service levels. `online` is how many users were meant
/// to be online and `timeline` how many were.
pub fn judge(
    profile: &Profile,
    phases: &BTreeMap<Phase, PhaseStats>,
    online: usize,
    timeline: &[Point],
) -> Verdict {
    let slo = &profile.slo;
    let steady = phases.get(&Phase::Steady);
    let metric = |name: &str| steady.and_then(|s| s.metrics.get(name));
    let mut checks = Vec::new();
    let mut at_most = |name: &str, target: Option<f64>, actual: Option<f64>| {
        if let Some(target) = target {
            checks.push(Check {
                name: name.to_string(),
                target,
                actual,
                pass: actual.is_some_and(|a| a <= target),
            });
        }
    };
    at_most(
        "delivery p99 (ms)",
        slo.delivery_p99_ms,
        metric("delivery").map(|m| m.p99_ms),
    );
    at_most(
        "delivery p50 (ms)",
        slo.delivery_p50_ms,
        metric("delivery").map(|m| m.p50_ms),
    );
    at_most(
        "request p99 (ms)",
        slo.request_p99_ms,
        metric("http:*").map(|m| m.p99_ms),
    );
    for (route, target) in &slo.route_p99_ms {
        at_most(
            &format!("{route} p99 (ms)"),
            Some(*target),
            metric(&format!("http:{route}")).map(|m| m.p99_ms),
        );
    }
    // Connections are mostly made in the ramp, so it counts too.
    let connect = across(&[phases.get(&Phase::Ramp), steady], "connect");
    at_most(
        "connect p99 (ms)",
        slo.connect_p99_ms,
        connect.map(|m| m.p99_ms),
    );
    at_most(
        "recovery p99 (ms)",
        slo.recovery_p99_ms,
        metric("recovery").map(|m| m.p99_ms),
    );
    let requests = steady.map_or(0, |s| s.counter("requests"));
    let errors = steady.map_or(0, |s| s.counter("errors"));
    let error_rate = if requests > 0 {
        Some(errors as f64 / requests as f64)
    } else {
        None
    };
    at_most("error rate", Some(slo.error_rate), error_rate.or(Some(0.0)));
    let voice_loss = steady.and_then(|s| {
        let expected = s.counter("voice_packets_expected");
        (expected > 0).then(|| 1.0 - s.counter("voice_packets_received") as f64 / expected as f64)
    });
    at_most("voice loss", slo.voice_loss, voice_loss.map(|l| l.max(0.0)));
    at_most(
        "voice jitter p99 (ms)",
        slo.voice_jitter_ms,
        metric("voice:jitter").map(|m| m.p99_ms),
    );
    if online > 0 {
        let share = connected_in_steady(timeline).map(|c| c as f64 / online as f64);
        checks.push(Check {
            name: "connected share".to_string(),
            target: slo.connected_share,
            actual: share,
            pass: share.is_some_and(|s| s >= slo.connected_share),
        });
    }
    let lag = metric("lag").map_or(0.0, |m| m.p99_ms);
    let generator_kept_up = lag <= profile.load.generator_lag_limit_ms;
    Verdict {
        pass: generator_kept_up && checks.iter().all(|c| c.pass),
        checks,
        generator_kept_up,
        generator_lag_p99_ms: lag,
    }
}

/// The shortest steady phase memory growth is read from: ten minutes.
pub const MIN_GROWTH_WINDOW: f64 = 600.0;

/// Everything a run produced.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub profile: String,
    pub description: String,
    pub run: String,
    /// Unix seconds.
    pub started_at: i64,
    pub online_users: usize,
    pub agents: u32,
    pub phases: BTreeMap<Phase, PhaseStats>,
    pub timeline: Vec<Point>,
    pub verdict: Verdict,
    pub server_samples: Vec<Sample>,
    /// Resources that ran short, in the order they did; the first is the likeliest bottleneck.
    pub findings: Vec<Finding>,
    /// Resident memory growth per hour, by metrics endpoint.
    pub memory_growth_per_hour: BTreeMap<String, f64>,
    /// Growth per hour of what the program holds allocated (`aspen_memory_allocated_bytes`),
    /// by metrics endpoint, for servers that export it. Resident memory that grows while this
    /// does not is the allocator keeping freed memory; this growing is a leak.
    #[serde(default)]
    pub heap_growth_per_hour: BTreeMap<String, f64>,
    /// Capacity mode: each step's online users and verdict.
    #[serde(default)]
    pub capacity: Vec<CapacityStep>,
    /// Commands run at set times, and how they ended.
    #[serde(default)]
    pub commands: Vec<CommandRun>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapacityStep {
    /// Users meant to be online.
    pub online_users: usize,
    /// Users connected through the steady phase.
    #[serde(default)]
    pub connected: u64,
    pub pass: bool,
    pub checks: Vec<Check>,
    pub findings: Vec<Finding>,
    pub steady: Option<PhaseStats>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandRun {
    pub t: f64,
    pub command: String,
    pub status: Option<i32>,
    pub output: String,
}

impl Report {
    pub fn steady(&self) -> Option<&PhaseStats> {
        self.phases.get(&Phase::Steady)
    }

    /// Reads the deployment's samples. Memory growth is fitted over the steady phase alone,
    /// from `steady_from` seconds for `steady_seconds`, and only when that is at least
    /// `MIN_GROWTH_WINDOW` long: memory rises while users connect, and a short window says
    /// nothing about leaks.
    pub fn analyse_server(&mut self, steady_from: f64, steady_seconds: f64) {
        self.findings = scrape::findings(&self.server_samples);
        if steady_seconds < MIN_GROWTH_WINDOW {
            return;
        }
        let steady: Vec<Sample> = self
            .server_samples
            .iter()
            .filter(|s| s.t >= steady_from && s.t <= steady_from + steady_seconds)
            .cloned()
            .collect();
        let endpoints: std::collections::BTreeSet<String> =
            steady.iter().map(|s| s.endpoint.clone()).collect();
        let growth = |series: &str| -> BTreeMap<String, f64> {
            endpoints
                .iter()
                .filter_map(|e| scrape::growth_per_hour(&steady, e, series).map(|g| (e.clone(), g)))
                .collect()
        };
        self.memory_growth_per_hour = growth("process_resident_memory_bytes");
        self.heap_growth_per_hour = growth(aspen_metrics::memory::ALLOCATED);
    }
}

/// Differences between two reports' steady phases, for regression checks: each shared latency
/// metric's p99, old and new, and whether the new one is worse by more than `tolerance` (a share).
pub fn compare(old: &Report, new: &Report, tolerance: f64) -> Vec<(String, f64, f64, bool)> {
    let (Some(a), Some(b)) = (old.steady(), new.steady()) else {
        return Vec::new();
    };
    a.metrics
        .iter()
        .filter_map(|(name, before)| {
            let after = b.metrics.get(name)?;
            if before.count < 10 || after.count < 10 {
                return None;
            }
            let worse = after.p99_ms > before.p99_ms * (1.0 + tolerance)
                && after.p99_ms - before.p99_ms > 1.0;
            Some((name.clone(), before.p99_ms, after.p99_ms, worse))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn snapshot(
        agent: u32,
        ended_s: f64,
        phase: Phase,
        latencies: &[u64],
        counters: &[(&str, u64)],
    ) -> Snapshot {
        let mut h = stats::histogram();
        for l in latencies {
            h.record(*l).unwrap();
        }
        Snapshot {
            agent,
            interval: 0,
            phase: Some(phase),
            histograms: BTreeMap::from([("delivery".to_string(), stats::encode(&h))]),
            counters: counters.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            connected: 10,
            ended_ns: (ended_s * 1e9) as i64,
        }
    }

    fn profile() -> Profile {
        Profile::from_toml(
            "name = \"t\"\n[target]\napi = \"x\"\n[population]\nusers = 10\ncommunities = 1\nlargest_community = 10\nsmallest_community = 1\n[load]\nonline = 1\nramp_up_seconds = 0\nduration_seconds = 10\n[behaviours.a]\nshare = 1\n[slo]\ndelivery_p99_ms = 50\nerror_rate = 0.01\n",
        )
        .unwrap()
    }

    #[test]
    fn agents_merge_by_phase_and_by_time() {
        let mut aggregate = Aggregate::new(0, 5.0);
        aggregate
            .add(&snapshot(
                0,
                5.0,
                Phase::Steady,
                &[10_000, 20_000],
                &[("requests", 100)],
            ))
            .unwrap();
        aggregate
            .add(&snapshot(
                1,
                5.1,
                Phase::Steady,
                &[30_000],
                &[("requests", 50), ("errors", 1)],
            ))
            .unwrap();
        let (phases, timeline) = aggregate.finish(10.0);
        let steady = &phases[&Phase::Steady];
        assert_eq!(steady.metrics["delivery"].count, 3);
        assert_eq!(steady.counter("requests"), 150);
        assert_eq!(steady.per_second("requests"), 15.0);
        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].connected, 20);
        let verdict = judge(&profile(), &phases, 20, &timeline);
        assert!(verdict.pass, "{verdict:?}");
        let verdict = judge(&profile(), &phases, 40, &timeline);
        assert!(!verdict.pass, "half the users never connected");
    }

    #[test]
    fn a_missed_level_fails_the_verdict() {
        let mut aggregate = Aggregate::new(0, 5.0);
        aggregate
            .add(&snapshot(
                0,
                5.0,
                Phase::Steady,
                &[90_000; 5],
                &[("requests", 10), ("errors", 1)],
            ))
            .unwrap();
        let (phases, timeline) = aggregate.finish(5.0);
        let verdict = judge(&profile(), &phases, 0, &timeline);
        assert!(!verdict.pass);
        assert_eq!(verdict.checks.iter().filter(|c| !c.pass).count(), 2);
    }
}
