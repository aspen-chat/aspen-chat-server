//! What each operation does, against the Valkey of `docker-compose.yaml` or `VALKEY_URL`: the
//! behaviour any other store behind [`Ephemeral`] must match. Each test is skipped, saying so,
//! when Valkey cannot be reached.

use std::time::Duration;

use fred::interfaces::{HashesInterface, SortedSetsInterface};

use crate::*;

async fn store() -> Option<Ephemeral> {
    let url = std::env::var("VALKEY_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
    match Ephemeral::valkey_at(&url).await {
        Ok(store) => Some(store),
        Err(e) => {
            eprintln!("skipping: no Valkey at {url}: {e}");
            None
        }
    }
}

fn client(store: &Ephemeral) -> &fred::clients::Client {
    match store {
        Ephemeral::Valkey(client) => client,
    }
}

/// A key no other test or run uses.
fn key(name: &str) -> String {
    format!("aspen_ephemeral_test:{}:{name}", uuid::Uuid::now_v7())
}

#[tokio::test]
async fn set_writes_as_its_condition_allows_and_answers_the_previous_value() {
    let Some(store) = store().await else { return };
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
    let previous: Option<String> = store
        .set_previous(&k, 7, None, Some(Condition::Present))
        .await
        .unwrap();
    assert_eq!(previous.as_deref(), Some("a"));
    // Written with no expiry, it lost the one it had; an integer reads as either.
    assert_eq!(store.ttl(&k).await.unwrap(), Ttl::NoExpiry);
    assert_eq!(store.get::<i64>(&k).await.unwrap(), Some(7));
    assert_eq!(store.get::<String>(&k).await.unwrap().as_deref(), Some("7"));
    // The previous value is answered whether or not the condition let it write.
    let previous: Option<i64> = store
        .set_previous(&k, 8, None, Some(Condition::Absent))
        .await
        .unwrap();
    assert_eq!(previous, Some(7));
    assert_eq!(store.get::<i64>(&k).await.unwrap(), Some(7));
    let previous: Option<i64> = store
        .set_previous(&key("fresh"), 1, None, None)
        .await
        .unwrap();
    assert_eq!(previous, None);
    assert_eq!(store.getdel::<i64>(&k).await.unwrap(), Some(7));
    assert_eq!(store.getdel::<i64>(&k).await.unwrap(), None);
    assert!(!store.exists(&k).await.unwrap());
}

#[tokio::test]
async fn keys_expire_as_written_and_say_how_long_they_have() {
    let Some(store) = store().await else { return };
    let (seconds, millis, at) = (key("ex"), key("px"), key("pxat"));
    store
        .set(&seconds, 1, Some(Expiry::Seconds(100)), None)
        .await
        .unwrap();
    store
        .set(&millis, 1, Some(Expiry::Millis(200)), None)
        .await
        .unwrap();
    let now_ms = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis(),
    )
    .unwrap();
    store
        .set(&at, 1, Some(Expiry::AtMillis(now_ms + 200)), None)
        .await
        .unwrap();
    assert!(
        matches!(store.ttl(&seconds).await.unwrap(), Ttl::Left(left) if left > Duration::from_secs(98))
    );
    assert!(
        matches!(store.pttl(&millis).await.unwrap(), Ttl::Left(left) if left <= Duration::from_millis(200))
    );
    assert_eq!(store.pttl(&key("absent")).await.unwrap(), Ttl::NoKey);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(!store.exists(&millis).await.unwrap());
    assert!(!store.exists(&at).await.unwrap());
    assert_eq!(store.ttl(&millis).await.unwrap(), Ttl::NoKey);
    assert!(store.expire(&seconds, 5).await.unwrap());
    assert!(
        matches!(store.ttl(&seconds).await.unwrap(), Ttl::Left(left) if left <= Duration::from_secs(5))
    );
    assert!(!store.expire(&millis, 5).await.unwrap());
    assert_eq!(store.del(vec![seconds, millis]).await.unwrap(), 1);
}

#[tokio::test]
async fn counters_start_from_zero_and_refuse_what_is_not_an_integer() {
    let Some(store) = store().await else { return };
    let k = key("count");
    assert_eq!(store.incr::<i64>(&k).await.unwrap(), 1);
    assert_eq!(store.incr::<u64>(&k).await.unwrap(), 2);
    assert_eq!(store.incr_by(&k, 5).await.unwrap(), 7);
    // A counter made by counting has no expiry until it is given one.
    assert_eq!(store.ttl(&k).await.unwrap(), Ttl::NoExpiry);
    let text = key("text");
    store.set(&text, "words", None, None).await.unwrap();
    assert!(store.incr::<i64>(&text).await.is_err());
    store.del(vec![k, text]).await.unwrap();
}

#[tokio::test]
async fn sets_and_values_are_read_back_in_bulk() {
    let Some(store) = store().await else { return };
    let set = key("set");
    assert_eq!(
        store
            .sadd(&set, vec!["a".into(), "b".into(), "a".into()])
            .await
            .unwrap(),
        2
    );
    let mut members = store.smembers(&set).await.unwrap();
    members.sort();
    assert_eq!(members, ["a", "b"]);
    assert!(store.smembers(&key("none")).await.unwrap().is_empty());
    let (one, two) = (key("one"), key("two"));
    store.set(&one, "1", None, None).await.unwrap();
    assert_eq!(
        store.mget(vec![one.clone(), two.clone()]).await.unwrap(),
        [Some("1".to_string()), None]
    );
    store.del(vec![set, one]).await.unwrap();
}

#[tokio::test]
async fn a_pipeline_answers_each_command_where_it_was_queued() {
    let Some(store) = store().await else { return };
    let (sorted, plain) = (key("sorted"), key("plain"));
    let mut pipeline = Pipeline::new();
    pipeline.zadd(&sorted, vec![(1.0, "old".into()), (5.0, "new".into())]);
    pipeline.expire(&sorted, 60);
    pipeline.set(&plain, 1, Some(Expiry::Seconds(60)), None);
    let previous = pipeline.set_previous::<i64>(&plain, 2, None, Some(Condition::Present));
    let left = pipeline.pttl(&sorted);
    let none = pipeline.pttl(key("absent"));
    let values = pipeline.mget(vec![plain.clone(), key("absent")]);
    pipeline.zremrangebyscore(&sorted, f64::NEG_INFINITY, 2.0);
    let ranged = pipeline.zrangebyscore(&sorted, 0.0, f64::INFINITY);
    let mut replies = store.run(pipeline).await.unwrap();
    assert_eq!(replies.take(previous).unwrap(), Some(1));
    assert!(matches!(replies.take(left).unwrap(), Ttl::Left(_)));
    assert_eq!(replies.take(none).unwrap(), Ttl::NoKey);
    assert_eq!(replies.take(values).unwrap(), [Some("2".to_string()), None]);
    assert_eq!(replies.take(ranged).unwrap(), ["new"]);
    // An empty pipeline sends nothing and answers nothing.
    store.run(Pipeline::new()).await.unwrap();
    assert_eq!(store.zrem(&sorted, "new").await.unwrap(), 1);
    store.del(plain).await.unwrap();
}

#[tokio::test]
async fn a_gcra_bucket_allows_its_burst_then_waits_and_gives_back_what_is_refunded() {
    let Some(store) = store().await else { return };
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

#[tokio::test]
async fn a_gcra_bucket_refills_by_expiring() {
    let Some(store) = store().await else { return };
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
}

#[tokio::test]
async fn attempts_count_within_a_window_started_by_the_first() {
    let Some(store) = store().await else { return };
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
    store.del(k).await.unwrap();
}

#[tokio::test]
async fn attention_lasts_until_its_time_and_is_asked_in_order() {
    let Some(store) = store().await else { return };
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
    let members: Vec<String> = client(&store)
        .zrange(&b, 0, -1, None, false, None, false)
        .await
        .unwrap();
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
    assert_eq!(store.attention_ask(vec![b]).await.unwrap(), [false]);
}

#[tokio::test]
async fn a_badge_grows_only_while_kept() {
    let Some(store) = store().await else { return };
    let k = key("badge");
    assert_eq!(store.badge_add(&k).await.unwrap(), None);
    assert!(!store.exists(&k).await.unwrap());
    store
        .set(&k, 4, Some(Expiry::Seconds(300)), None)
        .await
        .unwrap();
    assert_eq!(store.badge_add(&k).await.unwrap(), Some(5));
    assert!(
        matches!(store.ttl(&k).await.unwrap(), Ttl::Left(left) if left > Duration::from_secs(290))
    );
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
    let credit: Option<i64> = client(store).hget(&keys.credit, author).await.unwrap();
    credit.unwrap_or(0)
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

#[tokio::test]
async fn a_pool_starts_full_spends_as_asked_and_is_forgotten_when_full() {
    let Some(store) = store().await else { return };
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
    // The pool expires once it would be full again, and a second more.
    assert!(
        matches!(store.pttl(&keys.pool).await.unwrap(), Ttl::Left(left) if left > Duration::from_secs(1000))
    );
    // What was spent is credited to its sender in whole mana; what was refused is not.
    let sent: Vec<(String, f64)> = client(&store)
        .zrange(&keys.sent, 0, -1, None, false, None, true)
        .await
        .unwrap();
    assert_eq!(sent, [("b".to_string(), 1.0), ("a".to_string(), 150.0)]);
    assert!(
        matches!(store.pttl(&keys.sent).await.unwrap(), Ttl::Left(left) if left <= Duration::from_secs(60))
    );
    forget(&store, keys).await;
}

#[tokio::test]
async fn credit_held_counts_toward_the_pool_and_caps_it() {
    let Some(store) = store().await else { return };
    let keys = mana_keys();
    client(&store)
        .hset::<(), _, _>(&keys.credit, ("a", 40_000))
        .await
        .unwrap();
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
    forget(&store, keys).await;
}

#[tokio::test]
async fn the_first_three_waiting_are_credited_a_third_each_and_paid_from_it() {
    let Some(store) = store().await else { return };
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
    assert_eq!(credit_of(&store, &keys, "a").await, 33_334);
    assert_eq!(credit_of(&store, &keys, "b").await, 33_333);
    assert!(
        matches!(store.pttl(&keys.credit).await.unwrap(), Ttl::Left(left) if left <= Duration::from_secs(60))
    );
    // Paid from their credit, the rest staying theirs.
    let paid = allot(ManaAllot::Pay, vec![turn("a", 30_000, 1)]).await;
    assert_eq!(paid.answers, [true]);
    assert_eq!(credit_of(&store, &keys, "a").await, 3_334);
    assert!(paid.level <= RATES.max - 30_000 + 2 * RATES.per_tick);
    // One with no credit is refused, and others keep theirs.
    let refused = allot(ManaAllot::Pay, vec![turn("d", 1_000, 1)]).await;
    assert_eq!(refused.answers, [false]);
    assert_eq!(credit_of(&store, &keys, "b").await, 33_333);
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
    assert_eq!(credit_of(&store, &keys, "a").await, 0);
    assert!(credit_of(&store, &keys, "d").await >= 1_111);
    assert!(answer.level <= RATES.max);
    // With no one waiting, everything goes back.
    let answer = allot(ManaAllot::Share, vec![]).await;
    assert!(answer.answers.is_empty());
    for author in ["a", "b", "c", "d"] {
        assert_eq!(credit_of(&store, &keys, author).await, 0);
    }
    forget(&store, keys).await;
}

#[tokio::test]
async fn two_waiting_take_two_thirds_and_one() {
    let Some(store) = store().await else { return };
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
    assert_eq!(credit_of(&store, &keys, "a").await, 66_667);
    assert_eq!(credit_of(&store, &keys, "b").await, 33_333);
    forget(&store, keys).await;
}

#[tokio::test]
async fn a_row_larger_than_a_share_of_the_pool_posts_into_debt_once_the_share_is_held() {
    let Some(store) = store().await else { return };
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
    assert_eq!(credit_of(&store, &keys, "a").await, 0);
    // In debt nothing is shared out, and nothing more posts into debt.
    let answer = allot(ManaAllot::Share, vec![turn("b", 150_000, 3)]).await;
    assert_eq!(answer.answers, [false]);
    assert_eq!(credit_of(&store, &keys, "b").await, 0);
    forget(&store, keys).await;
}

#[tokio::test]
async fn the_top_spenders_are_summed_over_minutes_and_kept_a_while() {
    let Some(store) = store().await else { return };
    let (cache, first, second) = (key("top"), key("minute-1"), key("minute-2"));
    let add = |minute: &str, member: &str, by: f64| {
        let client = client(&store).clone();
        let (minute, member) = (minute.to_string(), member.to_string());
        async move {
            let _: f64 = client.zincrby(minute, by, member).await.unwrap();
        }
    };
    add(&first, "a", 5.0).await;
    add(&second, "a", 2.0).await;
    add(&second, "b", 6.0).await;
    add(&first, "c", 1.0).await;
    let minutes = vec![first.clone(), second.clone(), key("minute-3")];
    let top = store
        .mana_top(&cache, minutes.clone(), 2, Duration::from_millis(300))
        .await
        .unwrap();
    assert_eq!(top, [("a".to_string(), 7.0), ("b".to_string(), 6.0)]);
    // Read from what was summed while it lasts.
    add(&first, "c", 100.0).await;
    let top = store
        .mana_top(&cache, minutes.clone(), 2, Duration::from_millis(300))
        .await
        .unwrap();
    assert_eq!(top[0].0, "a");
    tokio::time::sleep(Duration::from_millis(400)).await;
    let top = store
        .mana_top(&cache, minutes, 2, Duration::from_millis(300))
        .await
        .unwrap();
    assert_eq!(top[0], ("c".to_string(), 101.0));
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
    store.del(vec![cache, first, second]).await.unwrap();
}
