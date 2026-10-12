//! How long the database has taken, kept for the Administration Dashboard's chart.
//!
//! Each server counts every statement's round trip and every wait for a pooled connection
//! (`database::latency`). Each minute, [`spawn_recorder`] adds what its server counted to the
//! rows of `database_latency`, one row for the step that minute falls in of each [`Range`], so
//! a row holds what every server counted in its step and its percentiles are the whole
//! deployment's. [`read`] gives a range as its rows, and the recurring job `sweepExpired`
//! deletes rows older than their range reaches.

use crate::context::GlobalServerContext;
use crate::database::latency;
use chrono::{DateTime, Utc};
use diesel::sql_types::{Array, BigInt, Integer, Timestamptz};
use diesel_async::RunQueryDsl;
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

/// How often each server writes what it counted, and the shortest step.
const MINUTE_SECONDS: i32 = 60;

/// How many minutes a server keeps what it could not write, as while the database is away,
/// to write when it can; past it the oldest are let go.
const MAX_UNWRITTEN: usize = 60;

/// How far back a chart reaches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::VariantArray)]
pub enum Range {
    OneHour,
    OneDay,
    SevenDays,
}

impl Range {
    /// The length of each step of the range, in seconds.
    pub fn step_seconds(self) -> i32 {
        match self {
            Range::OneHour => MINUTE_SECONDS,
            Range::OneDay => 15 * 60,
            Range::SevenDays => 60 * 60,
        }
    }

    /// How many steps the range has.
    fn steps(self) -> i32 {
        match self {
            Range::OneHour => 60,
            Range::OneDay => 96,
            Range::SevenDays => 168,
        }
    }

    /// How old a step of the range's length is when `sweepExpired` deletes it: the range and
    /// two steps more, since a range read late in a step still begins a whole range before the
    /// step it ends with.
    pub(crate) fn kept_seconds(self) -> i32 {
        (self.steps() + 2) * self.step_seconds()
    }
}

/// When the step of `seconds` that `at` falls in began. Steps are counted from the epoch, so
/// every server agrees on them whatever its time zone.
fn step_start(at: DateTime<Utc>, seconds: i32) -> DateTime<Utc> {
    let seconds = i64::from(seconds);
    DateTime::from_timestamp(at.timestamp().div_euclid(seconds) * seconds, 0).unwrap_or(at)
}

/// What this server counted in the minute that began at `at`.
struct Minute {
    at: DateTime<Utc>,
    queries: Vec<i64>,
    pool_waits: Vec<i64>,
}

/// Starts writing what this server counts, at the end of each minute by the clock. A minute
/// that cannot be written is kept and written with the next, in its own place.
pub fn spawn_recorder(state: GlobalServerContext) {
    tokio::spawn(async move {
        let mut unwritten: VecDeque<Minute> = VecDeque::new();
        loop {
            let now = Utc::now();
            let minute = step_start(now, MINUTE_SECONDS);
            let ends = minute + chrono::Duration::seconds(i64::from(MINUTE_SECONDS));
            tokio::time::sleep((ends - now).to_std().unwrap_or_default()).await;
            if unwritten.len() == MAX_UNWRITTEN {
                unwritten.pop_front();
            }
            unwritten.push_back(Minute {
                at: minute,
                queries: latency::QUERIES.take(),
                pool_waits: latency::POOL_WAITS.take(),
            });
            while let Some(minute) = unwritten.front() {
                if let Err(e) = write(&state, minute).await {
                    tracing::warn!(
                        waiting = unwritten.len(),
                        "could not record the database's latency: {e}"
                    );
                    break;
                }
                unwritten.pop_front();
            }
        }
    });
}

/// Adds `minute` to the step it falls in of every range.
async fn write(state: &GlobalServerContext, minute: &Minute) -> crate::Result<()> {
    use strum::VariantArray;
    let steps: Vec<i32> = Range::VARIANTS
        .iter()
        .map(|range| range.step_seconds())
        .collect();
    let starts: Vec<DateTime<Utc>> = steps
        .iter()
        .map(|&seconds| step_start(minute.at, seconds))
        .collect();
    let mut conn = state.connection_pool.get().await?;
    diesel::sql_query(
        "INSERT INTO database_latency (step_seconds, at, queries, pool_waits) \
         SELECT step.seconds, step.at, $3, $4 \
         FROM unnest($1::integer[], $2::timestamptz[]) AS step(seconds, at) \
         ON CONFLICT (step_seconds, at) DO UPDATE SET \
             queries = aspen_add_counts(database_latency.queries, EXCLUDED.queries), \
             pool_waits = aspen_add_counts(database_latency.pool_waits, EXCLUDED.pool_waits)",
    )
    .bind::<Array<Integer>, _>(steps)
    .bind::<Array<Timestamptz>, _>(starts)
    .bind::<Array<BigInt>, _>(&minute.queries)
    .bind::<Array<BigInt>, _>(&minute.pool_waits)
    .execute(conn.as_mut())
    .await?;
    Ok(())
}

/// What one kind of duration came to over a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Figures {
    /// How many were counted.
    pub count: u64,
    pub median: Duration,
    pub p95: Duration,
    pub p99: Duration,
}

impl Figures {
    /// `None` when `buckets` counted nothing.
    fn of(buckets: &[i64]) -> Option<Self> {
        Some(Self {
            count: latency::count(buckets),
            median: latency::quantile(buckets, 0.5)?,
            p95: latency::quantile(buckets, 0.95)?,
            p99: latency::quantile(buckets, 0.99)?,
        })
    }
}

/// One step of a range; each of its figures is `None` when no server recorded any in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Point {
    /// When the step began.
    pub at: DateTime<Utc>,
    pub queries: Option<Figures>,
    pub pool_waits: Option<Figures>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Series {
    pub step_seconds: i32,
    /// Every step of the range, oldest first, ending with the step the last whole minute fell
    /// in, the latest any server has written.
    pub points: Vec<Point>,
}

#[derive(diesel::QueryableByName)]
struct Row {
    #[diesel(sql_type = Timestamptz)]
    at: DateTime<Utc>,
    #[diesel(sql_type = Array<BigInt>)]
    queries: Vec<i64>,
    #[diesel(sql_type = Array<BigInt>)]
    pool_waits: Vec<i64>,
}

/// The database's latency over `range`: at most the range's steps of rows, through the table's
/// key.
pub async fn read(state: &GlobalServerContext, range: Range) -> crate::Result<Series> {
    let step_seconds = range.step_seconds();
    let step = chrono::Duration::seconds(i64::from(step_seconds));
    let last_minute = step_start(Utc::now(), MINUTE_SECONDS)
        - chrono::Duration::seconds(i64::from(MINUTE_SECONDS));
    let last = step_start(last_minute, step_seconds);
    let first = last - step * (range.steps() - 1);
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<Row> = diesel::sql_query(
        "SELECT at, queries, pool_waits FROM database_latency \
         WHERE step_seconds = $1 AND at >= $2 AND at <= $3",
    )
    .bind::<Integer, _>(step_seconds)
    .bind::<Timestamptz, _>(first)
    .bind::<Timestamptz, _>(last)
    .load(conn.as_mut())
    .await?;
    let rows: HashMap<DateTime<Utc>, Row> = rows.into_iter().map(|row| (row.at, row)).collect();
    let points = (0..range.steps())
        .map(|i| {
            let at = first + step * i;
            let row = rows.get(&at);
            Point {
                at,
                queries: row.and_then(|row| Figures::of(&row.queries)),
                pool_waits: row.and_then(|row| Figures::of(&row.pool_waits)),
            }
        })
        .collect();
    Ok(Series {
        step_seconds,
        points,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_begin_on_multiples_of_their_length() {
        let at = DateTime::parse_from_rfc3339("2026-10-12T16:47:31Z")
            .unwrap()
            .to_utc();
        let begins = |seconds| step_start(at, seconds).to_rfc3339();
        assert_eq!(begins(60), "2026-10-12T16:47:00+00:00");
        assert_eq!(begins(900), "2026-10-12T16:45:00+00:00");
        assert_eq!(begins(3600), "2026-10-12T16:00:00+00:00");
    }

    #[test]
    fn figures_are_read_only_from_what_was_counted() {
        assert_eq!(Figures::of(&[0; latency::BUCKETS]), None);
        let mut buckets = vec![0; latency::BUCKETS];
        buckets[40] = 10;
        let figures = Figures::of(&buckets).unwrap();
        assert_eq!(figures.count, 10);
        assert!(figures.median <= figures.p95 && figures.p95 <= figures.p99);
    }
}
