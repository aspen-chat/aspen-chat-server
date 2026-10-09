//! Whether this API server takes on another event stream now, and how many it identifies at once
//! (`[limits] max_identifying_event_streams`).
//!
//! A stream is cheap to keep but dear to start: identifying it reads the database, and its
//! client, unless it resumes, reads its whole state again over REST right after. When a crowd
//! connects at once (a deployment of millions coming back from an outage of a large ISP, or a
//! server restarting), taking every stream as it comes would let their reads queue for the
//! database pool ahead of the requests of everyone already connected, each wait ending in
//! `serverBusy`, and the refused clients trying again. So a server admits streams at the pace it
//! can serve them, and tells the rest when to come back:
//!
//! - at most `max_identifying_event_streams` identify at once (half the pool by default), so
//!   identifying holds at most that share of the pool;
//! - a stream that cannot start identifying within [`ADMISSION_WAIT`] is refused;
//! - while requests already queue for the pool, a full pool's worth of them, new streams are
//!   refused without waiting, so the clients those requests come from are served first.
//!
//! A refused stream is closed with `serverBusy`, naming [`BUSY_RETRY_AFTER`]; its client waits at
//! least that long and spreads its return over as long again.

use diesel_async::AsyncPgConnection;
use diesel_async::pooled_connection::deadpool::Pool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// How long a stream waits for a place to identify in before it is refused.
pub const ADMISSION_WAIT: Duration = Duration::from_secs(2);

/// How long a client refused a stream waits before trying again, at the least.
pub const BUSY_RETRY_AFTER: Duration = Duration::from_secs(10);

/// The places to identify streams in; cheap to clone.
#[derive(Clone)]
pub struct StreamAdmission {
    identifying: Arc<Semaphore>,
    pool: Pool<AsyncPgConnection>,
}

/// One stream's place while it identifies, given back when dropped.
pub struct Identifying {
    _place: OwnedSemaphorePermit,
}

impl StreamAdmission {
    /// `limit` places, or half of `pool`'s connections when `None`.
    pub fn new(limit: Option<usize>, pool: Pool<AsyncPgConnection>) -> Self {
        let places = limit.unwrap_or(pool.status().max_size / 2).max(1);
        Self {
            identifying: Arc::new(Semaphore::new(places.min(Semaphore::MAX_PERMITS))),
            pool,
        }
    }

    /// A place to identify one stream in, or `None` when the server is too busy to take it on.
    pub async fn admit(&self) -> Option<Identifying> {
        if self.pool_queued() {
            return None;
        }
        let place = tokio::time::timeout(ADMISSION_WAIT, self.identifying.clone().acquire_owned())
            .await
            .ok()?
            .ok()?;
        Some(Identifying { _place: place })
    }

    /// Whether as many requests wait for a database connection as the pool holds.
    fn pool_queued(&self) -> bool {
        let status = self.pool.status();
        status.waiting >= status.max_size.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pool of `size` that never connects: admission reads only its status.
    fn pool(size: usize) -> Pool<AsyncPgConnection> {
        let database: crate::database::Database = "postgres://127.0.0.1:1/none".parse().unwrap();
        Pool::builder(database.manager())
            .max_size(size)
            .build()
            .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn streams_identify_a_few_at_once_and_the_rest_are_turned_away() {
        let admission = StreamAdmission::new(Some(2), pool(10));
        let first = admission.admit().await.unwrap();
        let _second = admission.admit().await.unwrap();
        let started = tokio::time::Instant::now();
        assert!(admission.admit().await.is_none());
        assert_eq!(started.elapsed(), ADMISSION_WAIT);
        drop(first);
        assert!(admission.admit().await.is_some());
    }

    #[test]
    fn half_the_pool_identifies_when_not_configured() {
        let admission = StreamAdmission::new(None, pool(10));
        assert_eq!(admission.identifying.available_permits(), 5);
        let admission = StreamAdmission::new(None, pool(1));
        assert_eq!(admission.identifying.available_permits(), 1);
    }
}
