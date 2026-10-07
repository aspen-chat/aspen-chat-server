//! Pushes waiting to be sent. The dispatcher decides whom an event wakes and queues a push for
//! each of their phones here; it does not wait for push services to answer, so a slow or
//! unreachable one delays only its own pushes. Each push is its own task, waiting for a slot of
//! its endpoint's origin and then one of the server's, and given [`SEND_TIMEOUT`] to be answered.
//! A subscription whose push service has not answered [`SUSPEND_AFTER`] times in a row is
//! skipped until [`SUSPENSION`] has passed since the last of them.

use super::{Delivery, Pointer, PushSubscription, SendError, send};
use crate::context::GlobalServerContext;
use aspen_schema::push_subscription;
use diesel::QueryDsl as _;
use diesel_async::RunQueryDsl;
use fred::prelude::KeysInterface as _;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// How long a push service has to answer one push.
pub(super) const SEND_TIMEOUT: Duration = Duration::from_secs(3);
/// How many pushes one server sends at once.
const SENDING: usize = 1024;
/// How many of those may be to one origin, so that one slow push service holds only its share.
const SENDING_PER_ORIGIN: usize = 256;
/// How many pushes may wait on one server. Queueing more waits for room, which holds up the
/// dispatcher, so an event's pushes are delayed rather than lost however many there are.
const WAITING: usize = 200_000;
/// How many of those may be to one origin. Pushes beyond it are dropped: an origin that far
/// behind is not answering, and waiting for it would hold up everyone else's.
const WAITING_PER_ORIGIN: usize = 50_000;
/// How many times in a row a subscription's push service may fail to answer before it is
/// suspended.
const SUSPEND_AFTER: i64 = 5;
/// How long a suspension lasts, counted from the last push that went unanswered.
const SUSPENSION: Duration = Duration::from_secs(60 * 60);

struct Queue {
    /// Room for pushes to wait in.
    room: Arc<Semaphore>,
    /// Slots for pushes being sent.
    sending: Semaphore,
    /// Each origin with pushes waiting or being sent.
    origins: Mutex<HashMap<String, Origin>>,
}

struct Origin {
    slots: Arc<Semaphore>,
    waiting: usize,
}

static QUEUE: LazyLock<Queue> = LazyLock::new(|| Queue {
    room: Arc::new(Semaphore::new(WAITING)),
    sending: Semaphore::new(SENDING),
    origins: Mutex::new(HashMap::new()),
});

/// An origin's share of the queue, given back when the push is done or dropped.
struct Held {
    origin: String,
    _room: OwnedSemaphorePermit,
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut origins = QUEUE.origins.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = origins.get_mut(&self.origin) {
            entry.waiting -= 1;
            if entry.waiting == 0 {
                origins.remove(&self.origin);
            }
        }
        metrics::gauge!(aspen_metrics::api::PUSHES_WAITING).decrement(1.0);
    }
}

/// Queues `pointer` for `subscription`, waiting for room when the server's queue is full.
pub(super) async fn enqueue(
    state: &GlobalServerContext,
    subscription: PushSubscription,
    pointer: Pointer,
    badge: Option<i64>,
) {
    let room = QUEUE
        .room
        .clone()
        .acquire_owned()
        .await
        .expect("the push queue's room is never closed");
    let origin = reqwest::Url::parse(&subscription.endpoint)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| subscription.endpoint.clone());
    let slots = {
        let mut origins = QUEUE.origins.lock().unwrap_or_else(|e| e.into_inner());
        let entry = origins.entry(origin.clone()).or_insert_with(|| Origin {
            slots: Arc::new(Semaphore::new(SENDING_PER_ORIGIN)),
            waiting: 0,
        });
        if entry.waiting >= WAITING_PER_ORIGIN {
            drop(origins);
            outcome("dropped");
            tracing::debug!(origin, "dropped a push: its origin has too many waiting");
            return;
        }
        entry.waiting += 1;
        entry.slots.clone()
    };
    metrics::gauge!(aspen_metrics::api::PUSHES_WAITING).increment(1.0);
    let held = Held {
        origin,
        _room: room,
    };
    let state = state.clone();
    tokio::spawn(async move {
        let _held = held;
        let Ok(_slot) = slots.acquire().await else {
            return;
        };
        let Ok(_sending) = QUEUE.sending.acquire().await else {
            return;
        };
        deliver(&state, &subscription, pointer, badge).await;
    });
}

fn outcome(outcome: &'static str) {
    metrics::counter!(aspen_metrics::api::PUSHES, "outcome" => outcome).increment(1);
}

fn failing_key(subscription: &PushSubscription) -> String {
    format!("push:failing:{}", subscription.id.0)
}

/// Sends one push, dropping the subscription when its push service says it is gone, and
/// counting toward its suspension when the push service does not answer.
async fn deliver(
    state: &GlobalServerContext,
    subscription: &PushSubscription,
    pointer: Pointer,
    badge: Option<i64>,
) {
    let key = failing_key(subscription);
    // Valkey being out of reach suspends no one.
    let failing: Option<i64> = state.valkey.get(&key).await.unwrap_or(None);
    if failing.is_some_and(|failing| failing >= SUSPEND_AFTER) {
        outcome("suspended");
        return;
    }
    match send(state, subscription, pointer, badge).await {
        Ok(Delivery::Accepted) => {
            outcome("accepted");
            if failing.is_some() {
                let _: Result<(), _> = state.valkey.del(&key).await;
            }
        }
        Ok(Delivery::Gone) => {
            outcome("gone");
            tracing::debug!(subscription = %subscription.id.0, "push subscription is gone");
            let _ = async {
                let mut conn = state.connection_pool.get().await?;
                diesel::delete(push_subscription::table.find(subscription.id))
                    .execute(conn.as_mut())
                    .await?;
                Ok::<_, crate::Error>(())
            }
            .await;
        }
        Err(SendError::Http(e)) if e.is_timeout() || e.is_connect() => {
            outcome("unanswered");
            tracing::warn!(
                endpoint = subscription.endpoint,
                "a push went unanswered: {e}"
            );
            let counted = async {
                let _: i64 = state.valkey.incr(&key).await?;
                let _: () = state
                    .valkey
                    .expire(&key, SUSPENSION.as_secs() as i64, None)
                    .await?;
                Ok::<_, fred::error::Error>(())
            }
            .await;
            if let Err(e) = counted {
                tracing::warn!("counting an unanswered push failed: {e}");
            }
        }
        Err(e) => {
            outcome("failed");
            tracing::warn!(endpoint = subscription.endpoint, "a push failed: {e}");
        }
    }
}
