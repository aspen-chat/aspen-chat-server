//! How long the database takes, as this server sees it: each statement's round trip, and each
//! wait for a connection from the pool.
//!
//! Both are counted into a [`Histogram`] of fixed buckets, so that what every server counted in
//! a minute can be added together bucket by bucket and the percentiles read from the sum are
//! those of the whole deployment. The buckets are HDR-shaped: four to each doubling, each a
//! quarter of its doubling wide, from one microsecond to about two minutes, so a percentile read
//! from them is within an eighth of the truth. The layout is what `database_latency`'s rows hold
//! (`app::database_latency`); changing it makes rows that cannot be added to those already kept.

use diesel::connection::{Instrumentation, InstrumentationEvent};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How many buckets a histogram has. The last holds everything from its lower bound up.
pub const BUCKETS: usize = 104;

/// Every statement's round trip on a pooled connection, from when it is handed to the
/// connection until its answer has arrived. The pool's own check of a connection is left out:
/// it is part of the wait for that connection.
pub static QUERIES: Histogram = Histogram::new(aspen_metrics::api::DB_QUERY_DURATION);

/// Every wait for a connection from the pool, refused ones included: the wait for one to be
/// free, opening it when the pool had none, and the check that an idle one still answers.
pub static POOL_WAITS: Histogram = Histogram::new(aspen_metrics::api::DB_POOL_WAIT_DURATION);

/// The bucket a duration of `micros` microseconds is counted in: below four, the microseconds
/// themselves; from there, the doubling it falls in and which quarter of that doubling.
fn bucket_of(micros: u64) -> usize {
    if micros < 4 {
        return micros as usize;
    }
    let doubling = 63 - micros.leading_zeros() as usize;
    let quarter = (micros >> (doubling - 2)) as usize & 3;
    ((doubling - 1) * 4 + quarter).min(BUCKETS - 1)
}

/// The shortest duration counted in `bucket`, in microseconds.
fn lower_bound(bucket: usize) -> u64 {
    if bucket < 4 {
        bucket as u64
    } else {
        (4 + (bucket % 4) as u64) << (bucket / 4 - 1)
    }
}

/// Durations counted by bucket since they were last taken, and observed as the Prometheus
/// histogram `metric`.
pub struct Histogram {
    metric: &'static str,
    buckets: [AtomicU64; BUCKETS],
}

impl Histogram {
    const fn new(metric: &'static str) -> Self {
        Self {
            metric,
            buckets: [const { AtomicU64::new(0) }; BUCKETS],
        }
    }

    pub fn record(&self, elapsed: Duration) {
        let micros = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);
        self.buckets[bucket_of(micros)].fetch_add(1, Ordering::Relaxed);
        metrics::histogram!(self.metric).record(elapsed.as_secs_f64());
    }

    /// The count in each bucket since the last call, which this resets. A duration recorded
    /// meanwhile is in this answer or the next, never both.
    pub fn take(&self) -> Vec<i64> {
        self.buckets
            .iter()
            .map(|bucket| i64::try_from(bucket.swap(0, Ordering::Relaxed)).unwrap_or(i64::MAX))
            .collect()
    }
}

/// How many durations `buckets` counts.
pub fn count(buckets: &[i64]) -> u64 {
    buckets.iter().map(|&n| u64::try_from(n).unwrap_or(0)).sum()
}

/// The duration that `quantile` (0 to 1) of those counted in `buckets` were at or under, placed
/// within its bucket as if the bucket's durations were spread evenly; `None` when none were
/// counted. Buckets past [`BUCKETS`] are read as the last.
pub fn quantile(buckets: &[i64], quantile: f64) -> Option<Duration> {
    let total = count(buckets);
    if total == 0 {
        return None;
    }
    let rank = (quantile * total as f64).ceil().clamp(1.0, total as f64);
    let mut below = 0u64;
    for (bucket, &held) in buckets.iter().enumerate() {
        let held = u64::try_from(held).unwrap_or(0);
        if held > 0 && (below + held) as f64 >= rank {
            let bucket = bucket.min(BUCKETS - 1);
            let lower = lower_bound(bucket) as f64;
            let micros = if bucket == BUCKETS - 1 {
                lower
            } else {
                let width = lower_bound(bucket + 1) as f64 - lower;
                lower + width * (rank - below as f64) / held as f64
            };
            return Some(Duration::from_micros(micros.round() as u64));
        }
        below += held;
    }
    None
}

/// How many statements one connection may have under way before the timer takes them for
/// abandoned ones and starts over.
const MAX_UNDER_WAY: usize = 64;

/// Times the statements of one connection into a histogram, for as long as one caller holds it
/// (`Pool::get` installs a new one each time). Statements sent together on a connection are
/// answered in the order they were sent, so each answer ends the oldest one under way. A
/// statement whose future is dropped never reports its end, and would leave every later one
/// measured from too early a start; a new timer for each holder keeps that to the holder that
/// dropped it.
pub(crate) struct QueryTimer {
    into: &'static Histogram,
    under_way: VecDeque<Instant>,
}

impl QueryTimer {
    pub(crate) fn new(into: &'static Histogram) -> Self {
        Self {
            into,
            under_way: VecDeque::new(),
        }
    }
}

impl Instrumentation for QueryTimer {
    fn on_connection_event(&mut self, event: InstrumentationEvent<'_>) {
        match event {
            InstrumentationEvent::StartQuery { .. } => {
                if self.under_way.len() == MAX_UNDER_WAY {
                    self.under_way.clear();
                }
                self.under_way.push_back(Instant::now());
            }
            InstrumentationEvent::FinishQuery { .. } => {
                if let Some(started) = self.under_way.pop_front() {
                    self.into.record(started.elapsed());
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_duration_falls_in_the_bucket_whose_bounds_hold_it() {
        for micros in (0..5000).chain([65_535, 65_536, 1_000_000, 117_440_511]) {
            let bucket = bucket_of(micros);
            assert!(lower_bound(bucket) <= micros, "{micros} in {bucket}");
            assert!(micros < lower_bound(bucket + 1), "{micros} in {bucket}");
        }
        assert_eq!(bucket_of(lower_bound(BUCKETS - 1)), BUCKETS - 1);
        assert_eq!(bucket_of(u64::MAX), BUCKETS - 1);
    }

    #[test]
    fn buckets_are_at_most_a_quarter_of_their_lower_bound_wide() {
        for bucket in 4..BUCKETS - 1 {
            let lower = lower_bound(bucket);
            assert!(lower_bound(bucket + 1) - lower <= lower / 4, "{bucket}");
        }
    }

    #[test]
    fn a_quantile_is_read_from_the_bucket_its_rank_falls_in() {
        let histogram = Histogram::new("test_duration_seconds");
        for millis in 1..=100 {
            histogram.record(Duration::from_millis(millis));
        }
        let buckets = histogram.take();
        assert_eq!(count(&buckets), 100);
        for (q, millis) in [(0.5, 50.0), (0.95, 95.0), (0.99, 99.0)] {
            let read = quantile(&buckets, q).unwrap().as_secs_f64() * 1000.0;
            assert!((read - millis).abs() <= millis / 8.0, "{q}: {read}");
        }
        assert_eq!(count(&histogram.take()), 0);
    }

    #[test]
    fn nothing_counted_has_no_quantile() {
        assert_eq!(quantile(&[0; BUCKETS], 0.5), None);
        assert_eq!(quantile(&[], 0.99), None);
    }

    #[test]
    fn the_last_bucket_reads_as_its_lower_bound() {
        let mut buckets = vec![0; BUCKETS];
        buckets[BUCKETS - 1] = 3;
        assert_eq!(
            quantile(&buckets, 0.5),
            Some(Duration::from_micros(lower_bound(BUCKETS - 1)))
        );
    }

    #[test]
    fn answers_end_the_oldest_statement_under_way() {
        static COUNTED: Histogram = Histogram::new("test_duration_seconds");
        let mut timer = QueryTimer::new(&COUNTED);
        let query = diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT 1");
        let query = diesel::debug_query::<diesel::pg::Pg, _>(&query);
        timer.on_connection_event(InstrumentationEvent::start_query(&query));
        timer.on_connection_event(InstrumentationEvent::start_query(&query));
        timer.on_connection_event(InstrumentationEvent::finish_query(&query, None));
        assert_eq!(timer.under_way.len(), 1);
        timer.on_connection_event(InstrumentationEvent::finish_query(&query, None));
        timer.on_connection_event(InstrumentationEvent::finish_query(&query, None));
        assert!(timer.under_way.is_empty());
        assert_eq!(count(&COUNTED.take()), 2);
    }
}
