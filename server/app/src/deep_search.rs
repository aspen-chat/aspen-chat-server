//! How many searches go deep at once (`[search] deep_concurrency`).
//!
//! A search goes deep when what it reads for free finds nothing (`app::search`): it then reads
//! on through history until it finds something, a step at a time, each step a full core of the
//! database for a moment. So the deployment runs `deep_concurrency` steps at once, whoever they
//! are for ([`DeepSearches::place`], then [`take_slot`]). A place is held for one step, not for
//! a search, so searches waiting on each other all advance.
//!
//! The count is the database's, as advisory locks a step's transaction holds and gives back
//! when it ends, so no server's stopping can strand one. Each server also lets only that many
//! of its own searches try at once, in the order they asked, so the rest wait holding no
//! connection. Between servers a place goes to whichever asks first once it is free.

use diesel::sql_types::Integer;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::time::Instant;

/// How long a step waits before asking again for a place another server's step holds.
const SLOT_RETRY: Duration = Duration::from_millis(25);

/// The first key of the advisory locks that are the deployment's places; the second is the
/// place's number. No other advisory lock in the server takes two keys.
const SLOT_LOCK_CLASS: i32 = 0x5345_4152;

/// This server's side of the deployment's places; cheap to clone.
#[derive(Clone)]
pub struct DeepSearches {
    trying: Arc<Semaphore>,
    slots: i32,
}

impl DeepSearches {
    pub fn new(concurrency: u32) -> Self {
        let slots = i32::try_from(concurrency.max(1)).unwrap_or(i32::MAX);
        Self {
            trying: Arc::new(Semaphore::new(
                usize::try_from(slots)
                    .unwrap_or(1)
                    .min(Semaphore::MAX_PERMITS),
            )),
            slots,
        }
    }

    /// How many steps the deployment runs at once.
    pub fn slots(&self) -> i32 {
        self.slots
    }

    /// This server's leave to try for one of the deployment's places, in the order asked, or
    /// `None` when none came by `deadline`.
    pub async fn place(&self, deadline: Instant) -> Option<OwnedSemaphorePermit> {
        tokio::time::timeout_at(deadline, self.trying.clone().acquire_owned())
            .await
            .ok()?
            .ok()
    }
}

#[derive(diesel::QueryableByName)]
struct Slot {
    #[diesel(sql_type = Integer)]
    #[allow(dead_code)]
    slot: i32,
}

/// Takes one of the deployment's `slots` places for the transaction `conn` is in, which gives it
/// back when it ends. `false` when every place is held, and the caller ends the transaction and
/// tries again after [`SLOT_RETRY`].
pub async fn take_slot(conn: &mut AsyncPgConnection, slots: i32) -> diesel::QueryResult<bool> {
    // The places are tried from a different one each time, one after another, and the first
    // free one is taken: `LIMIT 1` ends the scan there, so no second lock is taken.
    let first = rand::random_range(0..slots);
    let taken: Vec<Slot> = diesel::sql_query(
        "SELECT (g + $2) % $3 AS slot FROM generate_series(0, $3 - 1) AS g \
         WHERE pg_try_advisory_xact_lock($1, (g + $2) % $3) LIMIT 1",
    )
    .bind::<Integer, _>(SLOT_LOCK_CLASS)
    .bind::<Integer, _>(first)
    .bind::<Integer, _>(slots)
    .load(conn)
    .await?;
    Ok(!taken.is_empty())
}

/// Waits before a step asks again for a place, or says `false` when `deadline` comes first.
pub async fn wait_for_slot(deadline: Instant) -> bool {
    let until = Instant::now() + SLOT_RETRY;
    if until >= deadline {
        return false;
    }
    tokio::time::sleep_until(until).await;
    true
}
