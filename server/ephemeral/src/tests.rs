//! What each operation does: the behaviour every store behind [`Ephemeral`] must match. Each
//! behaviour is one body run against both backends, the Valkey of `docker-compose.yaml` or
//! `VALKEY_URL` (skipped, saying so, when it cannot be reached) and [`Memory`]. What only
//! [`Memory`] can show, exact times on a clock the test sets and its bound on bytes, follows.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use fred::interfaces::{HashesInterface, SortedSetsInterface};

use crate::*;

async fn valkey() -> Option<Ephemeral> {
    let url = std::env::var("VALKEY_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    match Ephemeral::valkey_at(&url).await {
        Ok(store) => Some(store),
        Err(e) => {
            eprintln!("skipping: no Valkey at {url}: {e}");
            None
        }
    }
}

fn memory() -> Ephemeral {
    Ephemeral::Memory(Memory::new(64 << 20))
}

/// Runs each named `async fn(&Ephemeral)` as two tests, `<name>::valkey` and `<name>::memory`.
macro_rules! both {
    ($($name:ident),* $(,)?) => {$(
        mod $name {
            #[tokio::test]
            async fn valkey() {
                if let Some(store) = super::valkey().await {
                    super::$name(&store).await;
                }
            }

            #[tokio::test]
            async fn memory() {
                super::$name(&super::memory()).await;
            }
        }
    )*};
}

both!(
    set_writes_as_its_condition_allows_and_answers_the_previous_value,
    set_refuses_an_expiry_not_after_zero_and_writes_one_already_past,
    keys_expire_as_written_and_say_how_long_they_have,
    expiring_at_or_before_now_removes_the_key,
    del_counts_the_keys_it_found_once_each,
    counters_start_from_zero_and_refuse_what_is_not_an_integer,
    a_key_read_as_another_kind_is_refused,
    sets_and_values_are_read_back_in_bulk,
    a_pipeline_answers_each_command_where_it_was_queued,
    a_pipeline_runs_every_command_and_fails_when_one_does,
    a_gcra_bucket_allows_its_burst_then_waits_and_gives_back_what_is_refunded,
    a_gcra_bucket_refills_by_expiring,
    a_gcra_bucket_holding_other_text,
    concurrent_takes_get_exactly_the_burst,
    attempts_count_within_a_window_started_by_the_first,
    attention_lasts_until_its_time_and_is_asked_in_order,
    a_badge_grows_only_while_kept,
    a_pool_starts_full_spends_as_asked_and_is_forgotten_when_full,
    a_spend_under_one_mana_names_its_sender_at_zero,
    credit_held_counts_toward_the_pool_and_caps_it,
    credit_that_is_not_a_number,
    the_first_three_waiting_are_credited_a_third_each_and_paid_from_it,
    two_waiting_take_two_thirds_and_one,
    a_row_larger_than_a_share_of_the_pool_posts_into_debt_once_the_share_is_held,
    paying_with_no_one_waiting_is_refused,
    the_top_spenders_are_summed_over_minutes_and_kept_a_while,
    the_top_of_no_minutes_is_refused_and_a_count_of_zero_is_everyone,
);

/// A key no other test or run uses.
fn key(name: &str) -> String {
    format!("aspen_ephemeral_test:{}:{name}", uuid::Uuid::now_v7())
}

fn now_ms() -> i64 {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap()
}

// What the tests read and write beside the operations, as each backend holds it.

async fn hget(store: &Ephemeral, key: &str, field: &str) -> Option<String> {
    match store {
        Ephemeral::Valkey(client) => client.hget(key, field).await.unwrap(),
        Ephemeral::Memory(memory) => memory.hget(key, field).unwrap(),
    }
}

async fn hset(store: &Ephemeral, key: &str, field: &str, value: &str) {
    match store {
        Ephemeral::Valkey(client) => client.hset::<(), _, _>(key, (field, value)).await.unwrap(),
        Ephemeral::Memory(memory) => memory.hset(key, field, value).unwrap(),
    }
}

async fn zincrby(store: &Ephemeral, key: &str, by: f64, member: &str) {
    match store {
        Ephemeral::Valkey(client) => {
            let _: f64 = client.zincrby(key, by, member).await.unwrap();
        }
        Ephemeral::Memory(memory) => {
            memory.zincrby(key, by, member).unwrap();
        }
    }
}

/// Every member of the sorted set `key` with its score, lowest first.
async fn zrange(store: &Ephemeral, key: &str) -> Vec<(String, f64)> {
    match store {
        Ephemeral::Valkey(client) => client
            .zrange(key, 0, -1, None, false, None, true)
            .await
            .unwrap(),
        Ephemeral::Memory(memory) => memory.zrange(key).unwrap(),
    }
}

async fn set_writes_as_its_condition_allows_and_answers_the_previous_value(store: &Ephemeral) {
    let k = key("set");
    assert!(
        !store
            .set(&k, "a", None, Some(Condition::Present))
            .await
            .unwrap()
    );
    assert_eq!(store.get::<String>(&k).await.unwrap(), None);
    assert!(
        store
            .set(&k, "a", Some(Expiry::Seconds(60)), Some(Condition::Absent))
            .await
            .unwrap()
    );
    assert!(
        !store
            .set(&k, "b", None, Some(Condition::Absent))
            .await
            .unwrap()
    );
    assert_eq!(store.get::<String>(&k).await.unwrap().as_deref(), Some("a"));
    // XX keeps the key's place but not its expiry, unless one is given.
    assert!(
        store
            .set(&k, "c", Some(Expiry::Seconds(60)), Some(Condition::Present))
            .await
            .unwrap()
    );
    assert!(
        matches!(store.ttl(&k).await.unwrap(), Ttl::Left(left) if left > Duration::from_secs(58))
    );
    let previous: Option<String> = store
        .set_previous(&k, 7, None, Some(Condition::Present))
        .await
        .unwrap();
    assert_eq!(previous.as_deref(), Some("c"));
    // Written with no expiry, it lost the one it had; an integer reads as either.
    assert_eq!(store.ttl(&k).await.unwrap(), Ttl::NoExpiry);
    assert_eq!(store.get::<i64>(&k).await.unwrap(), Some(7));
    assert_eq!(store.get::<u64>(&k).await.unwrap(), Some(7));
    assert_eq!(store.get::<String>(&k).await.unwrap().as_deref(), Some("7"));
    // The previous value is answered whether or not the condition let it write.
    let previous: Option<i64> = store
        .set_previous(&k, 8, None, Some(Condition::Absent))
        .await
        .unwrap();
    assert_eq!(previous, Some(7));
    assert_eq!(store.get::<i64>(&k).await.unwrap(), Some(7));
    let fresh = key("fresh");
    let previous: Option<i64> = store
        .set_previous(&fresh, 1, None, Some(Condition::Present))
        .await
        .unwrap();
    assert_eq!(previous, None);
    assert!(!store.exists(&fresh).await.unwrap());
    let previous: Option<i64> = store.set_previous(&fresh, 1, None, None).await.unwrap();
    assert_eq!(previous, None);
    assert_eq!(store.get::<i64>(&fresh).await.unwrap(), Some(1));
    // Text that is not an integer reads only as text.
    store.set(&fresh, "words", None, None).await.unwrap();
    assert!(store.get::<i64>(&fresh).await.is_err());
    store.set(&fresh, -3, None, None).await.unwrap();
    assert!(store.get::<u64>(&fresh).await.is_err());
    assert_eq!(store.getdel::<i64>(&k).await.unwrap(), Some(7));
    assert_eq!(store.getdel::<i64>(&k).await.unwrap(), None);
    assert!(!store.exists(&k).await.unwrap());
    store.del(fresh).await.unwrap();
}

async fn set_refuses_an_expiry_not_after_zero_and_writes_one_already_past(store: &Ephemeral) {
    let k = key("bad-expiry");
    for expiry in [
        Expiry::Seconds(0),
        Expiry::Seconds(-1),
        Expiry::Millis(0),
        Expiry::AtMillis(0),
        Expiry::Seconds(i64::MAX),
    ] {
        assert!(
            store.set(&k, "a", Some(expiry), None).await.is_err(),
            "{expiry:?}"
        );
        assert!(!store.exists(&k).await.unwrap(), "{expiry:?}");
    }
    // A time already past is written, and the key is gone.
    assert!(
        store
            .set(&k, "a", Some(Expiry::AtMillis(now_ms() - 1_000)), None)
            .await
            .unwrap()
    );
    assert!(!store.exists(&k).await.unwrap());
    assert_eq!(store.ttl(&k).await.unwrap(), Ttl::NoKey);
}

async fn keys_expire_as_written_and_say_how_long_they_have(store: &Ephemeral) {
    let (seconds, millis, at, plain) = (key("ex"), key("px"), key("pxat"), key("plain"));
    store
        .set(&seconds, 1, Some(Expiry::Seconds(100)), None)
        .await
        .unwrap();
    store
        .set(&millis, 1, Some(Expiry::Millis(200)), None)
        .await
        .unwrap();
    store
        .set(&at, 1, Some(Expiry::AtMillis(now_ms() + 200)), None)
        .await
        .unwrap();
    store.set(&plain, 1, None, None).await.unwrap();
    assert_eq!(
        store.ttl(&seconds).await.unwrap(),
        Ttl::Left(Duration::from_secs(100))
    );
    assert!(
        matches!(store.pttl(&seconds).await.unwrap(), Ttl::Left(left) if left > Duration::from_secs(99) && left <= Duration::from_secs(100))
    );
    assert!(
        matches!(store.pttl(&millis).await.unwrap(), Ttl::Left(left) if left <= Duration::from_millis(200))
    );
    assert!(
        matches!(store.pttl(&at).await.unwrap(), Ttl::Left(left) if left <= Duration::from_millis(200))
    );
    assert_eq!(store.ttl(&plain).await.unwrap(), Ttl::NoExpiry);
    assert_eq!(store.pttl(&plain).await.unwrap(), Ttl::NoExpiry);
    let absent = key("absent");
    assert_eq!(store.ttl(&absent).await.unwrap(), Ttl::NoKey);
    assert_eq!(store.pttl(&absent).await.unwrap(), Ttl::NoKey);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!store.exists(&millis).await.unwrap());
    assert!(!store.exists(&at).await.unwrap());
    assert_eq!(store.get::<i64>(&millis).await.unwrap(), None);
    assert_eq!(store.ttl(&millis).await.unwrap(), Ttl::NoKey);
    assert!(store.expire(&seconds, 5).await.unwrap());
    assert_eq!(
        store.ttl(&seconds).await.unwrap(),
        Ttl::Left(Duration::from_secs(5))
    );
    assert!(!store.expire(&millis, 5).await.unwrap());
    assert!(!store.exists(&millis).await.unwrap());
    assert!(store.expire(&plain, 60).await.unwrap());
    assert!(matches!(store.ttl(&plain).await.unwrap(), Ttl::Left(_)));
    assert_eq!(store.del(vec![seconds, millis, plain]).await.unwrap(), 2);
}

async fn expiring_at_or_before_now_removes_the_key(store: &Ephemeral) {
    let k = key("expire-now");
    for seconds in [0, -5] {
        store.set(&k, 1, None, None).await.unwrap();
        assert!(store.expire(&k, seconds).await.unwrap());
        assert!(!store.exists(&k).await.unwrap());
    }
    store.set(&k, 1, None, None).await.unwrap();
    assert!(store.expire(&k, i64::MAX).await.is_err());
    assert_eq!(store.ttl(&k).await.unwrap(), Ttl::NoExpiry);
    store.del(k).await.unwrap();
}

async fn del_counts_the_keys_it_found_once_each(store: &Ephemeral) {
    let (a, b) = (key("del-a"), key("del-b"));
    store.set(&a, 1, None, None).await.unwrap();
    store.sadd(&b, vec!["x".into()]).await.unwrap();
    assert_eq!(
        store
            .del(vec![a.clone(), a.clone(), b.clone(), key("none")])
            .await
            .unwrap(),
        2
    );
    assert!(!store.exists(&a).await.unwrap());
    assert!(!store.exists(&b).await.unwrap());
    assert!(store.del(Vec::<String>::new()).await.is_err());
}

async fn counters_start_from_zero_and_refuse_what_is_not_an_integer(store: &Ephemeral) {
    let k = key("count");
    assert_eq!(store.incr::<i64>(&k).await.unwrap(), 1);
    assert_eq!(store.incr::<u64>(&k).await.unwrap(), 2);
    assert_eq!(store.incr::<String>(&k).await.unwrap(), "3");
    assert_eq!(store.incr_by(&k, 5).await.unwrap(), 8);
    assert_eq!(store.incr_by(&k, -10).await.unwrap(), -2);
    assert!(store.incr::<u64>(&k).await.is_err(), "-1 is no u64");
    assert_eq!(store.get::<i64>(&k).await.unwrap(), Some(-1));
    // A counter made by counting has no expiry until it is given one, which counting keeps.
    assert_eq!(store.ttl(&k).await.unwrap(), Ttl::NoExpiry);
    store.expire(&k, 100).await.unwrap();
    store.incr::<i64>(&k).await.unwrap();
    assert_eq!(
        store.ttl(&k).await.unwrap(),
        Ttl::Left(Duration::from_secs(100))
    );
    // Only an integer's own text counts.
    let text = key("text");
    for held in [
        "words",
        "1.5",
        "01",
        "+1",
        " 1",
        "",
        "-0",
        "9223372036854775808",
    ] {
        store.set(&text, held, None, None).await.unwrap();
        assert!(store.incr::<i64>(&text).await.is_err(), "{held:?}");
        assert_eq!(
            store.get::<String>(&text).await.unwrap().as_deref(),
            Some(held)
        );
    }
    store.set(&text, i64::MAX, None, None).await.unwrap();
    assert!(store.incr::<i64>(&text).await.is_err(), "overflow");
    assert_eq!(store.get::<i64>(&text).await.unwrap(), Some(i64::MAX));
    store
        .set(&text, "-9223372036854775808", None, None)
        .await
        .unwrap();
    assert!(store.incr_by(&text, -1).await.is_err(), "underflow");
    assert_eq!(store.incr::<i64>(&text).await.unwrap(), i64::MIN + 1);
    store.del(vec![k, text]).await.unwrap();
}

async fn a_key_read_as_another_kind_is_refused(store: &Ephemeral) {
    let (set, text) = (key("kind-set"), key("kind-text"));
    store.sadd(&set, vec!["a".into()]).await.unwrap();
    store.set(&text, "words", None, None).await.unwrap();
    assert!(store.get::<String>(&set).await.is_err());
    assert!(store.getdel::<String>(&set).await.is_err());
    assert!(store.incr::<i64>(&set).await.is_err());
    assert!(
        store
            .set_previous::<String>(&set, "x", None, None)
            .await
            .is_err()
    );
    assert_eq!(
        store.smembers(&set).await.unwrap(),
        ["a"],
        "refused, it wrote nothing"
    );
    assert!(store.sadd(&text, vec!["a".into()]).await.is_err());
    assert!(store.smembers(&text).await.is_err());
    assert!(store.zrem(&text, "a").await.is_err());
    assert!(store.badge_add(&set).await.is_err());
    // Any kind counts as set for a condition, and a plain write replaces it.
    assert!(
        !store
            .set(&set, "x", None, Some(Condition::Absent))
            .await
            .unwrap()
    );
    assert!(store.set(&set, "x", None, None).await.unwrap());
    assert_eq!(
        store.get::<String>(&set).await.unwrap().as_deref(),
        Some("x")
    );
    store.del(vec![set, text]).await.unwrap();
}

async fn sets_and_values_are_read_back_in_bulk(store: &Ephemeral) {
    let set = key("set");
    assert_eq!(
        store
            .sadd(&set, vec!["a".into(), "b".into(), "a".into()])
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        store
            .sadd(&set, vec!["b".into(), "c".into()])
            .await
            .unwrap(),
        1
    );
    let mut members = store.smembers(&set).await.unwrap();
    members.sort();
    assert_eq!(members, ["a", "b", "c"]);
    assert!(store.smembers(&key("none")).await.unwrap().is_empty());
    assert!(store.sadd(&set, Vec::new()).await.is_err());
    let (one, two) = (key("one"), key("two"));
    store.set(&one, "1", None, None).await.unwrap();
    // What is not text reads as nothing.
    assert_eq!(
        store
            .mget(vec![one.clone(), two.clone(), set.clone(), one.clone()])
            .await
            .unwrap(),
        [Some("1".to_string()), None, None, Some("1".to_string())]
    );
    assert!(store.mget(Vec::new()).await.is_err());
    store.del(vec![set, one]).await.unwrap();
}

async fn a_pipeline_answers_each_command_where_it_was_queued(store: &Ephemeral) {
    let (sorted, plain) = (key("sorted"), key("plain"));
    let mut pipeline = Pipeline::new();
    pipeline.zadd(
        &sorted,
        vec![
            (1.0, "old".into()),
            (2.0, "edge".into()),
            (5.0, "new".into()),
            (9.0, "top".into()),
        ],
    );
    pipeline.expire(&sorted, 60);
    pipeline.set(&plain, 1, Some(Expiry::Seconds(60)), None);
    let previous = pipeline.set_previous::<i64>(&plain, 2, None, Some(Condition::Present));
    let refused = pipeline.set_previous::<String>(key("absent"), 2, None, Some(Condition::Present));
    let left = pipeline.pttl(&sorted);
    let none = pipeline.pttl(key("absent"));
    let forever = pipeline.pttl(&plain);
    let values = pipeline.mget(vec![plain.clone(), key("absent")]);
    // Both ends are included.
    pipeline.zremrangebyscore(&sorted, f64::NEG_INFINITY, 2.0);
    let ranged = pipeline.zrangebyscore(&sorted, 5.0, 9.0);
    let above = pipeline.zrangebyscore(&sorted, 9.5, f64::INFINITY);
    let inverted = pipeline.zrangebyscore(&sorted, 9.0, 5.0);
    let missing = pipeline.zrangebyscore(key("absent"), 0.0, 1.0);
    let mut replies = store.run(pipeline).await.unwrap();
    assert_eq!(replies.take(previous).unwrap(), Some(1));
    assert_eq!(replies.take(refused).unwrap(), None);
    assert!(
        matches!(replies.take(left).unwrap(), Ttl::Left(left) if left > Duration::from_secs(59))
    );
    assert_eq!(replies.take(none).unwrap(), Ttl::NoKey);
    assert_eq!(replies.take(forever).unwrap(), Ttl::NoExpiry);
    assert_eq!(replies.take(values).unwrap(), [Some("2".to_string()), None]);
    assert_eq!(replies.take(ranged).unwrap(), ["new", "top"]);
    assert!(replies.take(above).unwrap().is_empty());
    assert!(replies.take(inverted).unwrap().is_empty());
    assert!(replies.take(missing).unwrap().is_empty());
    assert_eq!(
        zrange(store, &sorted).await,
        [("new".to_string(), 5.0), ("top".to_string(), 9.0)]
    );
    // An empty pipeline sends nothing and answers nothing.
    store.run(Pipeline::new()).await.unwrap();
    assert_eq!(store.zrem(&sorted, "new").await.unwrap(), 1);
    assert_eq!(store.zrem(&sorted, "new").await.unwrap(), 0);
    // Its last member gone, the sorted set is.
    assert_eq!(store.zrem(&sorted, "top").await.unwrap(), 1);
    assert!(!store.exists(&sorted).await.unwrap());
    assert_eq!(store.zrem(&sorted, "top").await.unwrap(), 0);
    store.del(plain).await.unwrap();
}

async fn a_pipeline_runs_every_command_and_fails_when_one_does(store: &Ephemeral) {
    let (text, after) = (key("pipeline-text"), key("pipeline-after"));
    store.set(&text, "words", None, None).await.unwrap();
    let mut pipeline = Pipeline::new();
    pipeline.zadd(&text, vec![(1.0, "a".into())]);
    pipeline.set(&after, 1, None, None);
    assert!(store.run(pipeline).await.is_err());
    assert_eq!(store.get::<i64>(&after).await.unwrap(), Some(1));
    store.del(vec![text, after]).await.unwrap();
}

async fn a_gcra_bucket_allows_its_burst_then_waits_and_gives_back_what_is_refunded(
    store: &Ephemeral,
) {
    let k = key("gcra");
    // Three at once, then one every 10 seconds.
    let rate = GcraRate {
        emission_ms: 10_000,
        tolerance_ms: 20_000,
    };
    // A bucket never used has its whole burst, and peeking spends none of it.
    assert_eq!(store.gcra_peek(&k, rate).await.unwrap(), None);
    assert!(!store.exists(&k).await.unwrap());
    for _ in 0..3 {
        assert_eq!(store.gcra_take(&k, rate).await.unwrap(), None);
    }
    let wait = store.gcra_take(&k, rate).await.unwrap().expect("limited");
    assert!(wait > Duration::from_millis(9_000) && wait <= Duration::from_millis(10_000));
    let peeked = store.gcra_peek(&k, rate).await.unwrap().expect("limited");
    assert!(peeked <= wait);
    // A refused request counts nothing: the bucket expires when its burst is whole again.
    assert!(matches!(
        store.pttl(&k).await.unwrap(),
        Ttl::Left(left) if left > Duration::from_millis(29_000) && left <= Duration::from_millis(30_000)
    ));
    store.gcra_refund(&k, rate).await.unwrap();
    assert_eq!(store.gcra_take(&k, rate).await.unwrap(), None);
    // Refunded down to now, the bucket is gone.
    for _ in 0..3 {
        store.gcra_refund(&k, rate).await.unwrap();
    }
    assert!(!store.exists(&k).await.unwrap());
    // Refunding a bucket not set leaves it so.
    store.gcra_refund(&k, rate).await.unwrap();
    assert!(!store.exists(&k).await.unwrap());
}

async fn a_gcra_bucket_refills_by_expiring(store: &Ephemeral) {
    let k = key("gcra-refill");
    let rate = GcraRate {
        emission_ms: 200,
        tolerance_ms: 0,
    };
    assert_eq!(store.gcra_take(&k, rate).await.unwrap(), None);
    assert!(store.gcra_take(&k, rate).await.unwrap().is_some());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!store.exists(&k).await.unwrap());
    assert_eq!(store.gcra_take(&k, rate).await.unwrap(), None);
    // A bucket that takes no time per request would expire as it is written, which `SET`
    // refuses.
    let free = key("gcra-free");
    let rate = GcraRate {
        emission_ms: 0,
        tolerance_ms: 0,
    };
    assert!(store.gcra_take(&free, rate).await.is_err());
    assert!(!store.exists(&free).await.unwrap());
}

async fn a_gcra_bucket_holding_other_text(store: &Ephemeral) {
    let k = key("gcra-text");
    let rate = GcraRate {
        emission_ms: 1_000,
        tolerance_ms: 0,
    };
    store.set(&k, "words", None, None).await.unwrap();
    assert!(store.gcra_take(&k, rate).await.is_err());
    assert!(store.gcra_peek(&k, rate).await.is_err());
    // Refunding reads it as no bucket.
    store.gcra_refund(&k, rate).await.unwrap();
    assert_eq!(
        store.get::<String>(&k).await.unwrap().as_deref(),
        Some("words")
    );
    // A time in the future, written as a number Lua reads, is the bucket's.
    store
        .set(&k, format!(" {}e0 ", now_ms() + 60_000), None, None)
        .await
        .unwrap();
    let wait = store.gcra_peek(&k, rate).await.unwrap().expect("limited");
    assert!(wait > Duration::from_secs(59));
    store.del(k).await.unwrap();
}

async fn concurrent_takes_get_exactly_the_burst(store: &Ephemeral) {
    let k = key("gcra-race");
    let rate = GcraRate {
        emission_ms: 60_000,
        tolerance_ms: 60_000 * 9,
    };
    let takes: Vec<_> = (0..50)
        .map(|_| {
            let (store, k) = (store.clone(), k.clone());
            tokio::spawn(async move { store.gcra_take(&k, rate).await.unwrap() })
        })
        .collect();
    let mut allowed = 0;
    for take in takes {
        allowed += usize::from(take.await.unwrap().is_none());
    }
    assert_eq!(allowed, 10);
    let attempts = key("attempts-race");
    let counts: Vec<_> = (0..50)
        .map(|_| {
            let (store, attempts) = (store.clone(), attempts.clone());
            tokio::spawn(async move { store.count_attempt(&attempts, 60).await.unwrap() })
        })
        .collect();
    let mut seen = Vec::new();
    for count in counts {
        seen.push(count.await.unwrap());
    }
    seen.sort();
    assert_eq!(seen, (1..=50).collect::<Vec<_>>());
    store.del(vec![k, attempts]).await.unwrap();
}

async fn attempts_count_within_a_window_started_by_the_first(store: &Ephemeral) {
    let k = key("attempts");
    assert_eq!(store.count_attempt(&k, 1).await.unwrap(), 1);
    assert_eq!(
        store.ttl(&k).await.unwrap(),
        Ttl::Left(Duration::from_secs(1))
    );
    assert_eq!(store.count_attempt(&k, 100).await.unwrap(), 2);
    // Later attempts do not move the window.
    assert!(
        matches!(store.pttl(&k).await.unwrap(), Ttl::Left(left) if left <= Duration::from_secs(1))
    );
    tokio::time::sleep(Duration::from_millis(1_200)).await;
    assert_eq!(store.count_attempt(&k, 1).await.unwrap(), 1);
    // A window of nothing removes the count as it is made, and the next starts again.
    let none = key("attempts-none");
    assert_eq!(store.count_attempt(&none, 0).await.unwrap(), 1);
    assert!(!store.exists(&none).await.unwrap());
    assert_eq!(store.count_attempt(&none, 0).await.unwrap(), 1);
    // A count that is not an integer is refused.
    store.set(&none, "words", None, None).await.unwrap();
    assert!(store.count_attempt(&none, 10).await.is_err());
    store.del(vec![k, none]).await.unwrap();
}

async fn attention_lasts_until_its_time_and_is_asked_in_order(store: &Ephemeral) {
    let (a, b, absent) = (key("attended-a"), key("attended-b"), key("attended-none"));
    store
        .attention_mark(&a, "phone", Duration::from_millis(300))
        .await
        .unwrap();
    store
        .attention_mark(&b, "desktop", Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(
        store
            .attention_ask(vec![a.clone(), absent.clone(), b.clone()])
            .await
            .unwrap(),
        [true, false, true]
    );
    assert!(store.attention_ask(Vec::new()).await.unwrap().is_empty());
    // The set lives as long as its newest member.
    assert!(matches!(
        store.pttl(&a).await.unwrap(),
        Ttl::Left(left) if left <= Duration::from_millis(300)
    ));
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(store.attention_ask(vec![a.clone()]).await.unwrap(), [false]);
    // Marking again forgets members whose time has passed and keeps the rest.
    store
        .attention_mark(&b, "phone", Duration::from_millis(200))
        .await
        .unwrap();
    store
        .attention_mark(&b, "desktop", Duration::from_secs(5))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    store
        .attention_mark(&b, "laptop", Duration::from_secs(5))
        .await
        .unwrap();
    let members: Vec<String> = zrange(store, &b)
        .await
        .into_iter()
        .map(|(member, _)| member)
        .collect();
    assert_eq!(members, ["desktop", "laptop"]);
    // The set's expiry is the latest mark's, however long its other members last.
    store
        .attention_mark(&b, "watch", Duration::from_millis(200))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!store.exists(&b).await.unwrap());
    assert_eq!(store.attention_ask(vec![b.clone()]).await.unwrap(), [false]);
    store
        .attention_mark(&b, "desktop", Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(store.zrem(&b, "desktop").await.unwrap(), 1);
    assert_eq!(store.zrem(&b, "desktop").await.unwrap(), 0);
    assert_eq!(store.attention_ask(vec![b.clone()]).await.unwrap(), [false]);
    // Lasting no time, the mark is gone with its set.
    store
        .attention_mark(&b, "blink", Duration::ZERO)
        .await
        .unwrap();
    assert!(!store.exists(&b).await.unwrap());
    // A key holding something else is refused, asked or marked.
    store.set(&absent, "words", None, None).await.unwrap();
    assert!(store.attention_ask(vec![absent.clone()]).await.is_err());
    assert!(
        store
            .attention_mark(&absent, "phone", Duration::from_secs(1))
            .await
            .is_err()
    );
    store.del(absent).await.unwrap();
}

async fn a_badge_grows_only_while_kept(store: &Ephemeral) {
    let k = key("badge");
    assert_eq!(store.badge_add(&k).await.unwrap(), None);
    assert!(!store.exists(&k).await.unwrap());
    store
        .set(&k, 4, Some(Expiry::Seconds(300)), None)
        .await
        .unwrap();
    assert_eq!(store.badge_add(&k).await.unwrap(), Some(5));
    assert_eq!(store.get::<i64>(&k).await.unwrap(), Some(5));
    assert!(
        matches!(store.ttl(&k).await.unwrap(), Ttl::Left(left) if left > Duration::from_secs(290))
    );
    store.set(&k, "words", None, None).await.unwrap();
    assert!(store.badge_add(&k).await.is_err());
    store.del(k).await.unwrap();
}

/// A pool of 100 mana earning a thousandth a tick, so that ticks passing while a test runs do
/// not move it.
const RATES: ManaRates = ManaRates {
    per_tick: 1,
    max: 100_000,
};

fn mana_keys() -> ManaKeys {
    let pool = key("mana");
    ManaKeys {
        credit: format!("{pool}:credit"),
        sent: format!("{pool}:sent"),
        pool,
    }
}

async fn credit_of(store: &Ephemeral, keys: &ManaKeys, author: &str) -> i64 {
    hget(store, &keys.credit, author)
        .await
        .map_or(0, |credit| credit.parse().unwrap())
}

async fn forget(store: &Ephemeral, keys: ManaKeys) {
    store
        .del(vec![keys.pool, keys.credit, keys.sent])
        .await
        .unwrap();
}

fn turn(author: &str, cost: i64, weight: u32) -> ManaTurn {
    ManaTurn {
        author: author.to_string(),
        cost,
        weight,
    }
}

async fn a_pool_starts_full_spends_as_asked_and_is_forgotten_when_full(store: &Ephemeral) {
    let keys = mana_keys();
    let spend = |cost, how, sender| store.mana_pool(&keys, RATES, cost, how, sender, 60_000);
    // Peeking spends nothing, whatever the cost, and a full pool keeps no key.
    let peeked = spend(150_000, ManaSpend::Peek, Some("a")).await.unwrap();
    assert!(!peeked.spent);
    assert_eq!(peeked.level, RATES.max);
    assert_eq!(peeked.at_ms % 200, 0);
    assert!(!store.exists(&keys.pool).await.unwrap());
    // A full pool pays anything, down to half its capacity in debt.
    let paid = spend(150_000, ManaSpend::Pay, Some("a")).await.unwrap();
    assert!(paid.spent);
    assert_eq!(paid.level, -RATES.max / 2);
    let left = spend(1_000, ManaSpend::Pay, Some("b")).await.unwrap();
    assert!(!left.spent);
    assert!(left.level <= -RATES.max / 2 + 10);
    let bypassed = spend(1_000, ManaSpend::Bypass, Some("b")).await.unwrap();
    assert!(bypassed.spent);
    assert_eq!(bypassed.level, -RATES.max / 2);
    // Spending nothing is not recorded, and a sender that is empty is none.
    assert!(spend(0, ManaSpend::Bypass, Some("c")).await.unwrap().spent);
    assert!(
        spend(1_000, ManaSpend::Bypass, Some(""))
            .await
            .unwrap()
            .spent
    );
    // The pool expires once it would be full again, and a second more.
    assert!(
        matches!(store.pttl(&keys.pool).await.unwrap(), Ttl::Left(left) if left > Duration::from_secs(1000))
    );
    let pool_level: i64 = hget(store, &keys.pool, "level")
        .await
        .unwrap()
        .parse()
        .unwrap();
    assert!(pool_level <= -RATES.max / 2 + 10);
    // What was spent is credited to its sender in whole mana; what was refused is not.
    assert_eq!(
        zrange(store, &keys.sent).await,
        [("b".to_string(), 1.0), ("a".to_string(), 150.0)]
    );
    assert!(
        matches!(store.pttl(&keys.sent).await.unwrap(), Ttl::Left(left) if left <= Duration::from_secs(60))
    );
    forget(store, keys).await;
}

async fn a_spend_under_one_mana_names_its_sender_at_zero(store: &Ephemeral) {
    let keys = mana_keys();
    let paid = store
        .mana_pool(&keys, RATES, 999, ManaSpend::Pay, Some("a"), 60_000)
        .await
        .unwrap();
    assert!(paid.spent);
    assert_eq!(paid.level, RATES.max - 999);
    assert_eq!(zrange(store, &keys.sent).await, [("a".to_string(), 0.0)]);
    forget(store, keys).await;
}

async fn credit_held_counts_toward_the_pool_and_caps_it(store: &Ephemeral) {
    let keys = mana_keys();
    hset(store, &keys.credit, "a", "40000").await;
    let peeked = store
        .mana_pool(&keys, RATES, 0, ManaSpend::Peek, None, 60_000)
        .await
        .unwrap();
    assert_eq!(peeked.level, RATES.max);
    // The pool itself holds the capacity less the credit, and pays from that alone.
    let refused = store
        .mana_pool(&keys, RATES, 70_000, ManaSpend::Pay, Some("b"), 60_000)
        .await
        .unwrap();
    assert!(!refused.spent);
    assert_eq!(refused.level, RATES.max);
    assert_eq!(
        hget(store, &keys.pool, "level").await.as_deref(),
        Some("60000")
    );
    forget(store, keys).await;
}

async fn credit_that_is_not_a_number(store: &Ephemeral) {
    let keys = mana_keys();
    hset(store, &keys.credit, "a", "lots").await;
    hset(store, &keys.credit, "b", "500").await;
    // The pool cannot add it up.
    assert!(
        store
            .mana_pool(&keys, RATES, 0, ManaSpend::Peek, None, 60_000)
            .await
            .is_err()
    );
    // Sharing reads it as none, and writes back only what it read.
    let answer = store
        .mana_allot(
            &keys,
            RATES,
            ManaAllot::Share,
            60_000,
            &[turn("a", 1_000, 3)],
        )
        .await
        .unwrap();
    assert_eq!(answer.answers, [true]);
    assert_eq!(hget(store, &keys.credit, "b").await, None);
    assert_eq!(credit_of(store, &keys, "a").await, RATES.max);
    forget(store, keys).await;
}

async fn the_first_three_waiting_are_credited_a_third_each_and_paid_from_it(store: &Ephemeral) {
    let keys = mana_keys();
    let allot = |how, turns: Vec<ManaTurn>| {
        let keys = keys.clone();
        let store = store.clone();
        async move {
            store
                .mana_allot(&keys, RATES, how, 60_000, &turns)
                .await
                .unwrap()
        }
    };
    // A full pool, shared by three: a third each, the remainder to the first.
    let answer = allot(
        ManaAllot::Share,
        vec![
            turn("a", 30_000, 1),
            turn("b", 40_000, 1),
            turn("c", 20_000, 1),
        ],
    )
    .await;
    assert_eq!(answer.level, RATES.max, "the level counts what is credited");
    // b's row costs more than its credit, which is its whole share of the pool, so it may post
    // into debt.
    assert_eq!(answer.answers, [true, true, true]);
    assert_eq!(credit_of(store, &keys, "a").await, 33_334);
    assert_eq!(credit_of(store, &keys, "b").await, 33_333);
    assert_eq!(credit_of(store, &keys, "c").await, 33_333);
    assert_eq!(hget(store, &keys.pool, "level").await.as_deref(), Some("0"));
    assert!(
        matches!(store.pttl(&keys.credit).await.unwrap(), Ttl::Left(left) if left <= Duration::from_secs(60))
    );
    // Paid from their credit, the rest staying theirs.
    let paid = allot(ManaAllot::Pay, vec![turn("a", 30_000, 1)]).await;
    assert_eq!(paid.answers, [true]);
    assert_eq!(credit_of(store, &keys, "a").await, 3_334);
    assert!(paid.level <= RATES.max - 30_000 + 2 * RATES.per_tick);
    assert_eq!(zrange(store, &keys.sent).await, [("a".to_string(), 30.0)]);
    // One with no credit is refused, and others keep theirs.
    let refused = allot(ManaAllot::Pay, vec![turn("d", 1_000, 1)]).await;
    assert_eq!(refused.answers, [false]);
    assert_eq!(credit_of(store, &keys, "b").await, 33_333);
    // Out of the first three, a's credit goes back to the pool, and is shared again.
    let answer = allot(
        ManaAllot::Share,
        vec![
            turn("b", 40_000, 1),
            turn("c", 20_000, 1),
            turn("d", 1_000, 1),
        ],
    )
    .await;
    assert_eq!(credit_of(store, &keys, "a").await, 0);
    assert!(credit_of(store, &keys, "d").await >= 1_111);
    assert!(answer.level <= RATES.max);
    // Paid exactly, the credit at zero is not kept.
    let exact = credit_of(store, &keys, "c").await;
    let paid = allot(ManaAllot::Pay, vec![turn("c", exact, 1)]).await;
    assert_eq!(paid.answers, [true]);
    assert_eq!(hget(store, &keys.credit, "c").await, None);
    // With no one waiting, everything goes back.
    let answer = allot(ManaAllot::Share, vec![]).await;
    assert!(answer.answers.is_empty());
    for author in ["a", "b", "c", "d"] {
        assert_eq!(credit_of(store, &keys, author).await, 0);
    }
    assert!(!store.exists(&keys.credit).await.unwrap());
    forget(store, keys).await;
}

async fn two_waiting_take_two_thirds_and_one(store: &Ephemeral) {
    let keys = mana_keys();
    store
        .mana_allot(
            &keys,
            RATES,
            ManaAllot::Share,
            60_000,
            &[turn("a", 90_000, 2), turn("b", 90_000, 1)],
        )
        .await
        .unwrap();
    assert_eq!(credit_of(store, &keys, "a").await, 66_667);
    assert_eq!(credit_of(store, &keys, "b").await, 33_333);
    forget(store, keys).await;
}

async fn a_row_larger_than_a_share_of_the_pool_posts_into_debt_once_the_share_is_held(
    store: &Ephemeral,
) {
    let keys = mana_keys();
    let allot = |how, turns: Vec<ManaTurn>| {
        let keys = keys.clone();
        let store = store.clone();
        async move {
            store
                .mana_allot(&keys, RATES, how, 60_000, &turns)
                .await
                .unwrap()
        }
    };
    // Alone, with the whole pool credited: a row costing more than the pool posts, into debt
    // down to half the capacity.
    let answer = allot(ManaAllot::Share, vec![turn("a", 150_000, 3)]).await;
    assert_eq!(answer.answers, [true]);
    let paid = allot(ManaAllot::Pay, vec![turn("a", 150_000, 3)]).await;
    assert_eq!(paid.answers, [true]);
    assert!(
        paid.level <= -RATES.max / 2 + 2 * RATES.per_tick,
        "{}",
        paid.level
    );
    assert_eq!(credit_of(store, &keys, "a").await, 0);
    // In debt nothing is shared out, and nothing more posts into debt.
    let answer = allot(ManaAllot::Share, vec![turn("b", 150_000, 3)]).await;
    assert_eq!(answer.answers, [false]);
    assert_eq!(credit_of(store, &keys, "b").await, 0);
    let refused = allot(ManaAllot::Pay, vec![turn("b", 150_000, 3)]).await;
    assert_eq!(refused.answers, [false]);
    forget(store, keys).await;
}

async fn paying_with_no_one_waiting_is_refused(store: &Ephemeral) {
    let keys = mana_keys();
    hset(store, &keys.credit, "a", "500").await;
    assert!(
        store
            .mana_allot(&keys, RATES, ManaAllot::Pay, 60_000, &[])
            .await
            .is_err()
    );
    // It stopped before writing anything.
    assert_eq!(credit_of(store, &keys, "a").await, 500);
    assert!(!store.exists(&keys.pool).await.unwrap());
    forget(store, keys).await;
}

async fn the_top_spenders_are_summed_over_minutes_and_kept_a_while(store: &Ephemeral) {
    let (cache, first, second) = (key("top"), key("minute-1"), key("minute-2"));
    zincrby(store, &first, 5.0, "a").await;
    zincrby(store, &second, 2.0, "a").await;
    zincrby(store, &second, 6.0, "b").await;
    zincrby(store, &first, 1.0, "c").await;
    let minutes = vec![first.clone(), second.clone(), key("minute-3")];
    let top = store
        .mana_top(&cache, minutes.clone(), 2, Duration::from_millis(300))
        .await
        .unwrap();
    assert_eq!(top, [("a".to_string(), 7.0), ("b".to_string(), 6.0)]);
    // Read from what was summed while it lasts.
    zincrby(store, &first, 100.0, "c").await;
    let top = store
        .mana_top(&cache, minutes.clone(), 2, Duration::from_millis(300))
        .await
        .unwrap();
    assert_eq!(top[0].0, "a");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let top = store
        .mana_top(&cache, minutes.clone(), 2, Duration::from_millis(300))
        .await
        .unwrap();
    assert_eq!(top[0], ("c".to_string(), 101.0));
    // Of one score, the member that sorts last comes first.
    zincrby(store, &second, 1.0, "b").await;
    let tied = key("top-tied");
    let top = store
        .mana_top(&tied, minutes, 3, Duration::from_secs(5))
        .await
        .unwrap();
    assert_eq!(
        top,
        [
            ("c".to_string(), 101.0),
            ("b".to_string(), 7.0),
            ("a".to_string(), 7.0)
        ]
    );
    // Nothing spent sums to nothing, and nothing is kept.
    let none = key("top-none");
    assert!(
        store
            .mana_top(&none, vec![key("empty")], 2, Duration::from_secs(5))
            .await
            .unwrap()
            .is_empty()
    );
    assert!(!store.exists(&none).await.unwrap());
    store.del(vec![cache, first, second, tied]).await.unwrap();
}

async fn the_top_of_no_minutes_is_refused_and_a_count_of_zero_is_everyone(store: &Ephemeral) {
    let cache = key("top-zero");
    assert!(
        store
            .mana_top(&cache, Vec::new(), 2, Duration::from_secs(5))
            .await
            .is_err()
    );
    let (minute, members) = (key("minute"), key("members"));
    zincrby(store, &minute, 3.0, "a").await;
    zincrby(store, &minute, 1.0, "b").await;
    // A set's members count one each.
    store
        .sadd(&members, vec!["b".into(), "c".into()])
        .await
        .unwrap();
    let top = store
        .mana_top(
            &cache,
            vec![minute.clone(), members.clone()],
            0,
            Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(
        top,
        [
            ("a".to_string(), 3.0),
            ("b".to_string(), 2.0),
            ("c".to_string(), 1.0)
        ]
    );
    // A minute holding text is refused.
    let text = key("text");
    store.set(&text, "words", None, None).await.unwrap();
    assert!(
        store
            .mana_top(
                &key("top-text"),
                vec![text.clone()],
                2,
                Duration::from_secs(5)
            )
            .await
            .is_err()
    );
    store.del(vec![cache, minute, members, text]).await.unwrap();
}

// What only `Memory` can show: exact times, on a clock the test sets.

/// 2027-01-15T08:00:00Z, a tick's start, in Unix microseconds.
const START_MICROS: i64 = 1_800_000_000_000_000;

fn clocked(max_bytes: usize) -> (Ephemeral, Arc<AtomicI64>) {
    let clock = Arc::new(AtomicI64::new(START_MICROS));
    (
        Ephemeral::Memory(Memory::with_clock(max_bytes, clock.clone())),
        clock,
    )
}

fn advance(clock: &AtomicI64, millis: i64) {
    clock.fetch_add(millis * 1000, Ordering::SeqCst);
}

#[tokio::test]
async fn a_key_lasts_through_its_deadline_and_is_gone_a_millisecond_after() {
    let (store, clock) = clocked(1 << 20);
    store
        .set("k", 1, Some(Expiry::Millis(100)), None)
        .await
        .unwrap();
    advance(&clock, 100);
    assert!(store.exists("k").await.unwrap());
    assert_eq!(store.pttl("k").await.unwrap(), Ttl::Left(Duration::ZERO));
    assert_eq!(store.ttl("k").await.unwrap(), Ttl::Left(Duration::ZERO));
    advance(&clock, 1);
    assert!(!store.exists("k").await.unwrap());
    assert_eq!(store.pttl("k").await.unwrap(), Ttl::NoKey);
    // A deadline of now itself is written and lasts the millisecond.
    let now = START_MICROS / 1000 + 101;
    store
        .set("at", 1, Some(Expiry::AtMillis(now)), None)
        .await
        .unwrap();
    assert!(store.exists("at").await.unwrap());
    advance(&clock, 1);
    assert!(!store.exists("at").await.unwrap());
    // Counting a key that has just expired starts again, with no expiry.
    store
        .set("count", 5, Some(Expiry::Millis(10)), None)
        .await
        .unwrap();
    advance(&clock, 11);
    assert_eq!(store.incr::<i64>("count").await.unwrap(), 1);
    assert_eq!(store.ttl("count").await.unwrap(), Ttl::NoExpiry);
}

#[tokio::test]
async fn ttl_rounds_to_the_nearest_second() {
    let (store, clock) = clocked(1 << 20);
    store
        .set("k", 1, Some(Expiry::Millis(1_500)), None)
        .await
        .unwrap();
    assert_eq!(
        store.ttl("k").await.unwrap(),
        Ttl::Left(Duration::from_secs(2))
    );
    advance(&clock, 1);
    assert_eq!(
        store.ttl("k").await.unwrap(),
        Ttl::Left(Duration::from_secs(1))
    );
    assert_eq!(
        store.pttl("k").await.unwrap(),
        Ttl::Left(Duration::from_millis(1_499))
    );
    advance(&clock, 999);
    assert_eq!(
        store.ttl("k").await.unwrap(),
        Ttl::Left(Duration::from_secs(1))
    );
    advance(&clock, 1);
    assert_eq!(store.ttl("k").await.unwrap(), Ttl::Left(Duration::ZERO));
}

#[tokio::test]
async fn a_gcra_bucket_waits_exactly_and_refills_on_time() {
    let (store, clock) = clocked(1 << 20);
    let rate = GcraRate {
        emission_ms: 1_000,
        tolerance_ms: 2_000,
    };
    for _ in 0..3 {
        assert_eq!(store.gcra_take("g", rate).await.unwrap(), None);
    }
    assert_eq!(
        store.gcra_take("g", rate).await.unwrap(),
        Some(Duration::from_millis(1_000))
    );
    assert_eq!(
        store.pttl("g").await.unwrap(),
        Ttl::Left(Duration::from_millis(3_000))
    );
    advance(&clock, 999);
    assert_eq!(
        store.gcra_peek("g", rate).await.unwrap(),
        Some(Duration::from_millis(1))
    );
    advance(&clock, 1);
    assert_eq!(store.gcra_peek("g", rate).await.unwrap(), None);
    assert_eq!(store.gcra_take("g", rate).await.unwrap(), None);
    assert_eq!(
        store.gcra_take("g", rate).await.unwrap(),
        Some(Duration::from_millis(1_000))
    );
    // Refunded, the bucket ends a request's time sooner.
    store.gcra_refund("g", rate).await.unwrap();
    assert_eq!(
        store.pttl("g").await.unwrap(),
        Ttl::Left(Duration::from_millis(2_000))
    );
    // Refunded to exactly now, it is removed.
    store.gcra_refund("g", rate).await.unwrap();
    store.gcra_refund("g", rate).await.unwrap();
    assert!(!store.exists("g").await.unwrap());
}

#[tokio::test]
async fn a_window_of_attempts_lasts_through_its_last_millisecond() {
    let (store, clock) = clocked(1 << 20);
    assert_eq!(store.count_attempt("a", 2).await.unwrap(), 1);
    advance(&clock, 2_000);
    assert_eq!(store.count_attempt("a", 2).await.unwrap(), 2);
    advance(&clock, 1);
    assert_eq!(store.count_attempt("a", 2).await.unwrap(), 1);
}

#[tokio::test]
async fn attention_ends_at_its_time_though_its_set_lasts_the_millisecond() {
    let (store, clock) = clocked(1 << 20);
    store
        .attention_mark("u", "phone", Duration::from_millis(300))
        .await
        .unwrap();
    advance(&clock, 299);
    assert_eq!(store.attention_ask(vec!["u".into()]).await.unwrap(), [true]);
    advance(&clock, 1);
    assert_eq!(
        store.attention_ask(vec!["u".into()]).await.unwrap(),
        [false]
    );
    assert!(store.exists("u").await.unwrap());
    // Marked again now, the member whose time is now is forgotten.
    store
        .attention_mark("u", "desktop", Duration::from_millis(300))
        .await
        .unwrap();
    let Ephemeral::Memory(memory) = &store else {
        unreachable!("clocked is Memory")
    };
    assert_eq!(
        memory.zrange("u").unwrap(),
        [("desktop".to_string(), 1_800_000_000_600.0)]
    );
}

#[tokio::test]
async fn a_pool_earns_each_tick_and_expires_when_it_would_be_full() {
    let (store, clock) = clocked(1 << 20);
    let keys = mana_keys();
    let rates = ManaRates {
        per_tick: 10,
        max: 1_001,
    };
    let spend = |cost, how| store.mana_pool(&keys, rates, cost, how, Some("a"), 60_000);
    // Half the capacity in debt is floor(1001 / 2) below zero.
    let paid = spend(5_000, ManaSpend::Bypass).await.unwrap();
    assert_eq!(paid.level, -500);
    assert_eq!(paid.at_ms, START_MICROS / 1000);
    // ceil(1501 / 10) ticks of 200 ms, and a second.
    assert_eq!(
        store.pttl(&keys.pool).await.unwrap(),
        Ttl::Left(Duration::from_millis(151 * 200 + 1_000))
    );
    // Within a tick nothing is earned; at its end, one tick's.
    advance(&clock, 199);
    assert_eq!(spend(0, ManaSpend::Peek).await.unwrap().level, -500);
    advance(&clock, 1);
    let peeked = spend(0, ManaSpend::Peek).await.unwrap();
    assert_eq!(peeked.level, -490);
    assert_eq!(peeked.at_ms, START_MICROS / 1000 + 200);
    // Earning past full, it is full, and forgotten.
    advance(&clock, 200 * 200);
    assert_eq!(spend(0, ManaSpend::Peek).await.unwrap().level, 1_001);
    assert!(!store.exists(&keys.pool).await.unwrap());
    // A full pool pays more than it holds, but a pool not full only what it holds.
    assert!(spend(2_000, ManaSpend::Pay).await.unwrap().spent);
    assert!(!spend(1, ManaSpend::Pay).await.unwrap().spent);
    assert_eq!(
        zrange(&store, &keys.sent).await,
        [("a".to_string(), 7.0)],
        "floor(5000 / 1000) + floor(2000 / 1000)"
    );
}

// The bound on bytes.

/// What a key holding `value` as text counts as.
fn text_bytes(key: &str, value: &str) -> usize {
    memory::key_bytes(key) + value.len()
}

fn used(store: &Ephemeral) -> usize {
    let Ephemeral::Memory(memory) = store else {
        unreachable!("the bound is Memory's")
    };
    memory.used_bytes().unwrap()
}

#[tokio::test]
async fn a_full_store_evicts_the_soonest_to_expire_first() {
    let value = "x".repeat(100);
    let size = text_bytes("soon", &value);
    let (store, _clock) = clocked(4 * size);
    store
        .set("late", value.as_str(), Some(Expiry::Seconds(50)), None)
        .await
        .unwrap();
    store
        .set("soon", value.as_str(), Some(Expiry::Seconds(10)), None)
        .await
        .unwrap();
    store
        .set("mid1", value.as_str(), Some(Expiry::Seconds(30)), None)
        .await
        .unwrap();
    store.set("last", value.as_str(), None, None).await.unwrap();
    assert_eq!(used(&store), 4 * size);
    store.set("next", value.as_str(), None, None).await.unwrap();
    assert!(!store.exists("soon").await.unwrap());
    for kept in ["late", "mid1", "last", "next"] {
        assert!(store.exists(kept).await.unwrap(), "{kept}");
    }
    // Growing one key evicts as many as its growth needs, soonest first.
    let bigger = "x".repeat(100 + size + 1);
    store
        .set("next", bigger.as_str(), None, None)
        .await
        .unwrap();
    assert!(!store.exists("mid1").await.unwrap());
    assert!(!store.exists("late").await.unwrap());
    assert!(store.exists("last").await.unwrap());
    assert!(used(&store) <= 4 * size);
}

#[tokio::test]
async fn a_full_store_of_keys_that_never_expire_refuses_and_keeps_what_it_holds() {
    let value = "x".repeat(100);
    let size = text_bytes("k1", &value);
    let (store, _clock) = clocked(3 * size);
    store.set("k1", value.as_str(), None, None).await.unwrap();
    store.set("k2", value.as_str(), None, None).await.unwrap();
    store
        .set("k3", value.as_str(), Some(Expiry::Seconds(10)), None)
        .await
        .unwrap();
    // Evicting k3 would not make room for twice a key, so nothing is evicted.
    let double = "x".repeat(100 + size);
    let refused = store.set("k4", double.as_str(), None, None).await;
    assert!(matches!(refused, Err(Error::Full { max_bytes }) if max_bytes == 3 * size));
    assert!(store.exists("k3").await.unwrap());
    assert!(!store.exists("k4").await.unwrap());
    assert_eq!(used(&store), 3 * size);
    // A key never evicts itself to grow.
    let refused = store
        .set("k3", double.as_str(), Some(Expiry::Seconds(10)), None)
        .await;
    assert!(matches!(refused, Err(Error::Full { .. })));
    assert_eq!(
        store.get::<String>("k3").await.unwrap(),
        Some(value.clone())
    );
    // Every kind of write is bounded.
    assert!(matches!(
        store.sadd("s", vec!["a".repeat(size)]).await,
        Err(Error::Full { .. })
    ));
    assert!(matches!(
        store.count_attempt(&"c".repeat(size), 10).await,
        Err(Error::Full { .. })
    ));
    let mut pipeline = Pipeline::new();
    pipeline.zadd("z", vec![(1.0, "a".repeat(size))]);
    assert!(matches!(store.run(pipeline).await, Err(Error::Full { .. })));
    assert!(!store.exists("s").await.unwrap());
    assert!(!store.exists("z").await.unwrap());
    // Shrinking needs no room, and what it frees is used before anything is evicted.
    store.set("k1", "", None, None).await.unwrap();
    assert_eq!(used(&store), 3 * size - value.len());
    store.set("k4", "", None, None).await.unwrap();
    assert!(store.exists("k3").await.unwrap());
    assert_eq!(used(&store), 3 * size);
    // Past what is free, the key that expires goes.
    store.set("k4", "x".repeat(10), None, None).await.unwrap();
    assert!(!store.exists("k3").await.unwrap());
    assert_eq!(
        used(&store),
        2 * size - value.len() + text_bytes("k4", "0123456789")
    );
}

#[tokio::test]
async fn keys_past_their_deadline_give_their_bytes_back() {
    let (store, clock) = clocked(1 << 20);
    for i in 0..30 {
        store
            .set(&format!("k{i}"), "value", Some(Expiry::Millis(10)), None)
            .await
            .unwrap();
    }
    store.set("kept", "value", None, None).await.unwrap();
    let kept = text_bytes("kept", "value");
    advance(&clock, 11);
    // Each operation sweeps a handful, soonest first, whatever it reads.
    assert!(store.exists("kept").await.unwrap());
    let after_one = used(&store);
    assert!(after_one < kept + 30 * text_bytes("k10", "value"));
    assert!(after_one > kept);
    store.get::<String>("kept").await.unwrap();
    assert_eq!(used(&store), kept);
    // A key read past its deadline reads as gone, its bytes given back.
    store
        .set("brief", "value", Some(Expiry::Millis(10)), None)
        .await
        .unwrap();
    store
        .set("later", "value", Some(Expiry::Millis(5)), None)
        .await
        .unwrap();
    advance(&clock, 11);
    assert_eq!(store.get::<String>("brief").await.unwrap(), None);
    assert_eq!(used(&store), kept);
}

#[tokio::test]
async fn a_script_refused_for_room_partway_is_undone() {
    let keys = ManaKeys {
        pool: "pool".into(),
        credit: "credit".into(),
        sent: "sent".into(),
    };
    let tick = (START_MICROS / 1000 / 200).to_string();
    // What spending 1 mana of a full pool of 100 writes: the sender in `sent`, then the pool.
    let sent = memory::key_bytes(&keys.sent) + memory::sorted_member_bytes("a");
    let pool = memory::key_bytes(&keys.pool)
        + memory::field_bytes("level", "99000")
        + memory::field_bytes("tick", &tick);
    let filler = text_bytes("filler", "f");
    // A key that expires, larger than the pool, which `sent` evicts to make room.
    let other = "o".repeat(pool);
    let other_bytes = text_bytes("other", &other);
    let (store, _clock) = clocked(filler + sent + pool - 1);
    store.set("filler", "f", None, None).await.unwrap();
    store
        .set("other", other.as_str(), Some(Expiry::Seconds(60)), None)
        .await
        .unwrap();
    assert_eq!(used(&store), filler + other_bytes);
    let refused = store
        .mana_pool(&keys, RATES, 1_000, ManaSpend::Pay, Some("a"), 60_000)
        .await;
    assert!(matches!(refused, Err(Error::Full { .. })), "{refused:?}");
    // Everything is as it was: `sent` never written, `other` back with its expiry.
    assert!(!store.exists(&keys.sent).await.unwrap());
    assert!(!store.exists(&keys.pool).await.unwrap());
    assert_eq!(store.get::<String>("other").await.unwrap(), Some(other));
    assert_eq!(
        store.ttl("other").await.unwrap(),
        Ttl::Left(Duration::from_secs(60))
    );
    assert_eq!(used(&store), filler + other_bytes);
    // With a byte more, it fits once `other` is evicted.
    let (store, _clock) = clocked(filler + sent + pool);
    store.set("filler", "f", None, None).await.unwrap();
    store
        .set("other", "o".repeat(pool), Some(Expiry::Seconds(60)), None)
        .await
        .unwrap();
    let paid = store
        .mana_pool(&keys, RATES, 1_000, ManaSpend::Pay, Some("a"), 60_000)
        .await
        .unwrap();
    assert!(paid.spent);
    assert!(!store.exists("other").await.unwrap());
    assert_eq!(used(&store), filler + sent + pool);
}

#[tokio::test]
async fn clones_hold_the_same_keys() {
    let store = memory();
    let clone = store.clone();
    store.set("k", 1, None, None).await.unwrap();
    assert_eq!(clone.get::<i64>("k").await.unwrap(), Some(1));
}
