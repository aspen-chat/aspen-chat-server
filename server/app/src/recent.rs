//! Values worked out at most once per `FRESH_FOR` per key on each server: anything asked for
//! often whose answer may be a few seconds old, such as who is online. Requests that arrive while
//! a value is being worked out wait for that one answer.

use lru::LruCache;
use std::hash::Hash;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

/// How long one value is reused before it is worked out again.
const FRESH_FOR: Duration = Duration::from_secs(10);
/// How many keys' values one server keeps.
const REMEMBERED: NonZeroUsize = NonZeroUsize::new(10_000).unwrap();

/// One key's value: when it was started, and the value once worked out.
type Entry<V> = (Instant, Arc<OnceCell<V>>);

/// Each key's latest value, shared by every request on this server.
pub struct Recent<K: Hash + Eq, V> {
    values: Mutex<LruCache<K, Entry<V>>>,
}

impl<K: Hash + Eq, V> Default for Recent<K, V> {
    fn default() -> Self {
        Self {
            values: Mutex::new(LruCache::new(REMEMBERED)),
        }
    }
}

impl<K: Hash + Eq, V: Clone> Recent<K, V> {
    /// The value for `key`: one started within `FRESH_FOR`, or else a new one from `work`. A
    /// value whose work fails is not kept, so the next request tries again.
    pub async fn get_or_work<F, Fut>(&self, key: K, work: F) -> crate::Result<V>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = crate::Result<V>>,
    {
        let cell = {
            let mut values = self.values.lock().unwrap_or_else(|e| e.into_inner());
            match values.get(&key) {
                Some((started, cell)) if started.elapsed() < FRESH_FOR => cell.clone(),
                _ => {
                    let cell = Arc::new(OnceCell::new());
                    values.put(key, (Instant::now(), cell.clone()));
                    cell
                }
            }
        };
        cell.get_or_try_init(work).await.cloned()
    }
}
