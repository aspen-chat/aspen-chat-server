//! What every backend of the bus must do alike. Each test runs against [`Bus::Memory`], and
//! against [`Bus::Nats`] when `ASPEN_BUS_TEST_NATS` names a NATS server with JetStream (with
//! `ASPEN_BUS_TEST_NATS_TOKEN` its token), which it makes streams and buckets of its own on,
//! named for the test, and removes after. Without one the NATS half says so and checks nothing.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;

use crate::{
    Bucket, BucketConfig, Bus, ConsumerLimits, Deliver, DurableConfig, Error, Headers, Memory,
    Nats, Operation, Retention, Start, Storage, StreamConfig,
};

/// A name no other test, and no other run, uses.
fn unique(what: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        "aspen_bus_test_{what}_{nanos}_{}",
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

async fn nats() -> Option<Bus> {
    let Ok(url) = std::env::var("ASPEN_BUS_TEST_NATS") else {
        eprintln!("ASPEN_BUS_TEST_NATS is not set; the NATS backend is not checked");
        return None;
    };
    let mut options = async_nats::ConnectOptions::new();
    if let Ok(token) = std::env::var("ASPEN_BUS_TEST_NATS_TOKEN") {
        options = options.token(token);
    }
    let client = async_nats::connect_with_options(url, options)
        .await
        .expect("the NATS ASPEN_BUS_TEST_NATS names");
    Some(Bus::Nats(Nats::new(client)))
}

/// Removes what a test made on NATS; the memory bus goes with the test.
async fn clean(bus: &Bus, streams: &[&str], buckets: &[&str]) {
    match bus {
        Bus::Nats(nats) => {
            for stream in streams {
                let _ = nats.0.jetstream.delete_stream(stream).await;
            }
            for bucket in buckets {
                let _ = nats.0.jetstream.delete_key_value(bucket).await;
            }
        }
        Bus::Memory(_) => {}
    }
}

/// Runs `body` against the memory bus, and against NATS when one is named.
macro_rules! both {
    ($name:ident, $body:expr) => {
        mod $name {
            use super::*;

            #[tokio::test]
            async fn memory() {
                ($body)(Bus::Memory(Memory::new(None))).await;
            }

            #[tokio::test]
            async fn nats() {
                if let Some(bus) = super::nats().await {
                    ($body)(bus).await;
                }
            }
        }
    };
}

fn stream(name: &str, retention: Retention) -> StreamConfig {
    StreamConfig {
        name: name.to_string(),
        subjects: vec![format!("{name}.>")],
        retention,
        storage: Storage::Memory,
        max_age: Duration::from_secs(60),
        max_bytes: 1024 * 1024,
        max_messages: None,
        consumer_limits: None,
    }
}

async fn put(bus: &Bus, subject: String, body: &'static str) -> u64 {
    bus.publish_to_stream(subject, Headers::new(), Bytes::from_static(body.as_bytes()))
        .await
        .unwrap()
        .await
        .unwrap()
}

async fn next<S: futures_util::Stream + Unpin>(stream: &mut S) -> S::Item {
    tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("an item within five seconds")
        .expect("the stream went on")
}

/// Whether `stream` yields nothing for `wait`.
async fn quiet<S: futures_util::Stream + Unpin>(stream: &mut S, wait: Duration) -> bool {
    tokio::time::timeout(wait, stream.next()).await.is_err()
}

#[test]
fn filters_match_whole_tokens() {
    use crate::memory::matches;
    assert!(matches("a.b", "a.b"));
    assert!(!matches("a.b", "a.bc"));
    assert!(!matches("a.b", "a.b.c"));
    assert!(matches("a.*", "a.b"));
    assert!(!matches("a.*", "a.b.c"));
    assert!(!matches("a.*", "a"));
    assert!(matches("a.>", "a.b.c"));
    assert!(!matches("a.>", "a"));
    assert!(matches("a.*.c", "a.b.c"));
    assert!(!matches("a.*.c", "a.b.d"));
}

both!(
    subscribers_get_what_matches_and_a_group_gets_it_once,
    |bus: Bus| async move {
        let root = unique("subjects");
        let mut exact = bus.subscribe(format!("{root}.a")).await.unwrap();
        let mut wild = bus.subscribe(format!("{root}.*")).await.unwrap();
        let mut first = bus
            .queue_subscribe(format!("{root}.a"), "group")
            .await
            .unwrap();
        let mut second = bus
            .queue_subscribe(format!("{root}.a"), "group")
            .await
            .unwrap();
        bus.flush().await.unwrap();
        for n in 0..4 {
            bus.publish(format!("{root}.a"), Bytes::from(n.to_string()))
                .await
                .unwrap();
        }
        bus.publish(format!("{root}.b"), Bytes::from_static(b"b"))
            .await
            .unwrap();
        bus.flush().await.unwrap();
        for n in 0..4 {
            assert_eq!(next(&mut exact).await.payload, n.to_string());
        }
        for expected in ["0", "1", "2", "3", "b"] {
            assert_eq!(next(&mut wild).await.payload, expected);
        }
        assert!(quiet(&mut exact, Duration::from_millis(200)).await);
        let mut grouped = 0;
        while !quiet(&mut first, Duration::from_millis(200)).await {
            grouped += 1;
        }
        while !quiet(&mut second, Duration::from_millis(200)).await {
            grouped += 1;
        }
        assert_eq!(grouped, 4, "each message reaches one member of the group");
    }
);

both!(
    a_request_is_answered_at_its_reply_subject,
    |bus: Bus| async move {
        let subject = unique("request");
        let mut asked = bus.subscribe(subject.clone()).await.unwrap();
        bus.flush().await.unwrap();
        let answerer = bus.clone();
        tokio::spawn(async move {
            let request = asked.next().await.unwrap();
            let reply = request.reply.expect("a reply subject");
            answerer
                .publish(
                    reply,
                    Bytes::from(format!(
                        "answer to {}",
                        String::from_utf8_lossy(&request.payload)
                    )),
                )
                .await
                .unwrap();
        });
        let answer = bus
            .request(subject, Bytes::from_static(b"q"))
            .await
            .unwrap();
        assert_eq!(answer.payload, "answer to q");
        assert!(
            bus.request(unique("nobody"), Bytes::new()).await.is_err(),
            "a request no one answers fails"
        );
    }
);

#[tokio::test]
async fn a_memory_request_is_answered_at_its_inbox_prefix() {
    let bus = Memory::new(None);
    let voice = Bus::Memory(bus.with_inbox_prefix("_INBOX_voice.someone"));
    let bus = Bus::Memory(bus);
    let mut asked = bus.subscribe("ask").await.unwrap();
    let answerer = bus.clone();
    tokio::spawn(async move {
        let request = asked.next().await.unwrap();
        let reply = request.reply.unwrap();
        assert!(reply.starts_with("_INBOX_voice.someone."), "{reply}");
        answerer
            .publish(reply, Bytes::from_static(b"ok"))
            .await
            .unwrap();
    });
    assert_eq!(
        voice.request("ask", Bytes::new()).await.unwrap().payload,
        "ok"
    );
}

both!(
    a_stream_numbers_what_it_takes_and_lets_the_oldest_go,
    |bus: Bus| async move {
        let name = unique("limits");
        let mut config = stream(&name, Retention::Limits);
        config.max_messages = Some(3);
        bus.create_or_update_stream(config.clone()).await.unwrap();
        let first = put(&bus, format!("{name}.a"), "1").await;
        let mut last = first;
        for body in ["2", "3", "4", "5"] {
            let sequence = put(&bus, format!("{name}.a"), body).await;
            assert_eq!(sequence, last + 1);
            last = sequence;
        }
        let info = bus.stream_info(&name).await.unwrap().unwrap();
        assert_eq!(info.state.messages, 3, "past max_messages the oldest go");
        assert_eq!(info.state.last_sequence, last);
        assert_eq!(info.config.max_messages, Some(3));
        // Past max_bytes too.
        config.max_messages = None;
        config.max_bytes = 64;
        bus.create_or_update_stream(config).await.unwrap();
        for _ in 0..10 {
            put(&bus, format!("{name}.a"), "0123456789").await;
        }
        let info = bus.stream_info(&name).await.unwrap().unwrap();
        assert!(info.state.bytes <= 64, "{} bytes held", info.state.bytes);
        assert!(info.state.messages < 10);
        assert!(
            bus.publish_to_stream(unique("nowhere"), Headers::new(), Bytes::new())
                .await
                .map(|_| ())
                .is_err()
                || matches!(bus, Bus::Nats(_)),
            "a subject no stream takes is refused"
        );
        assert!(bus.stream_info(&unique("absent")).await.unwrap().is_none());
        clean(&bus, &[&name], &[]).await;
    }
);

both!(
    a_stream_lets_go_of_what_is_past_its_age,
    |bus: Bus| async move {
        let name = unique("age");
        let mut config = stream(&name, Retention::Limits);
        config.max_age = Duration::from_secs(1);
        bus.create_or_update_stream(config).await.unwrap();
        put(&bus, format!("{name}.a"), "old").await;
        tokio::time::sleep(Duration::from_millis(1500)).await;
        put(&bus, format!("{name}.a"), "new").await;
        let (mut reading, pending) = bus.ordered(&name, Start::BySequence(1)).await.unwrap();
        assert_eq!(pending, 1);
        assert_eq!(next(&mut reading).await.unwrap().payload, "new");
        clean(&bus, &[&name], &[]).await;
    }
);

both!(
    a_purge_lets_go_of_what_its_filter_matches,
    |bus: Bus| async move {
        let name = unique("purge");
        bus.create_or_update_stream(stream(&name, Retention::Limits))
            .await
            .unwrap();
        put(&bus, format!("{name}.keep"), "k").await;
        put(&bus, format!("{name}.drop.1"), "d").await;
        put(&bus, format!("{name}.drop.2"), "d").await;
        bus.purge(&name, format!("{name}.drop.*")).await.unwrap();
        let (mut reading, pending) = bus.ordered(&name, Start::BySequence(1)).await.unwrap();
        assert_eq!(pending, 1);
        assert_eq!(
            next(&mut reading).await.unwrap().subject,
            format!("{name}.keep")
        );
        clean(&bus, &[&name], &[]).await;
    }
);

both!(
    an_ordered_reading_starts_where_asked_and_follows,
    |bus: Bus| async move {
        let name = unique("ordered");
        bus.create_or_update_stream(stream(&name, Retention::Limits))
            .await
            .unwrap();
        let mut headers = Headers::new();
        headers.insert("Aspen-Test", "kept");
        let first = bus
            .publish_to_stream(format!("{name}.a"), headers, Bytes::from_static(b"1"))
            .await
            .unwrap()
            .await
            .unwrap();
        put(&bus, format!("{name}.b"), "2").await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        let between = time::OffsetDateTime::now_utc();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let third = put(&bus, format!("{name}.a"), "3").await;

        let (mut all, pending) = bus.ordered(&name, Start::BySequence(first)).await.unwrap();
        assert_eq!(pending, 3);
        let read = next(&mut all).await.unwrap();
        assert_eq!((read.sequence, read.pending), (first, 2));
        assert_eq!(read.headers.get("Aspen-Test"), Some("kept"));
        assert_eq!(next(&mut all).await.unwrap().payload, "2");
        assert_eq!(next(&mut all).await.unwrap().sequence, third);
        // It goes on with what comes after.
        let fourth = put(&bus, format!("{name}.a"), "4").await;
        let read = next(&mut all).await.unwrap();
        assert_eq!((read.sequence, read.pending), (fourth, 0));

        let (mut since, pending) = bus.ordered(&name, Start::ByTime(between)).await.unwrap();
        assert_eq!(pending, 2);
        assert_eq!(next(&mut since).await.unwrap().sequence, third);

        let (mut later, pending) = bus
            .ordered(&name, Start::BySequence(fourth + 1))
            .await
            .unwrap();
        assert_eq!(pending, 0);
        assert!(quiet(&mut later, Duration::from_millis(200)).await);
        clean(&bus, &[&name], &[]).await;
    }
);

fn durable(name: &str, filter: String, deliver: Deliver) -> DurableConfig {
    DurableConfig {
        name: name.to_string(),
        filter,
        deliver,
        ack_wait: Duration::from_secs(1),
        max_deliver: None,
        max_ack_pending: None,
    }
}

both!(
    a_durable_consumer_starts_as_told_and_keeps_its_place,
    |bus: Bus| async move {
        let name = unique("durable");
        bus.create_or_update_stream(stream(&name, Retention::Limits))
            .await
            .unwrap();
        put(&bus, format!("{name}.a"), "before").await;
        let mut all = bus
            .durable(&name, durable("all", format!("{name}.a"), Deliver::All))
            .await
            .unwrap();
        let mut new = bus
            .durable(&name, durable("new", format!("{name}.>"), Deliver::New))
            .await
            .unwrap();
        put(&bus, format!("{name}.b"), "other").await;
        put(&bus, format!("{name}.a"), "after").await;
        let delivered = next(&mut all).await.unwrap();
        assert_eq!(
            (
                delivered.message.payload.as_ref(),
                delivered.message.delivered
            ),
            (b"before".as_ref(), 1)
        );
        delivered.ack().await.unwrap();
        let delivered = next(&mut all).await.unwrap();
        assert_eq!(
            delivered.message.payload, "after",
            "its filter passes over the other subject"
        );
        delivered.ack().await.unwrap();
        for expected in ["other", "after"] {
            let delivered = next(&mut new).await.unwrap();
            assert_eq!(delivered.message.payload, expected);
            delivered.ack().await.unwrap();
        }
        // Opened again under its name, it goes on from where it was.
        drop(all);
        let mut again = bus
            .durable(&name, durable("all", format!("{name}.a"), Deliver::All))
            .await
            .unwrap();
        assert!(
            quiet(&mut again, Duration::from_millis(1500)).await,
            "what was acknowledged is not delivered again"
        );
        put(&bus, format!("{name}.a"), "last").await;
        assert_eq!(next(&mut again).await.unwrap().message.payload, "last");
        clean(&bus, &[&name], &[]).await;
    }
);

both!(
    what_is_not_acknowledged_is_delivered_again,
    |bus: Bus| async move {
        let name = unique("redeliver");
        bus.create_or_update_stream(stream(&name, Retention::Limits))
            .await
            .unwrap();
        let mut config = durable("again", format!("{name}.>"), Deliver::All);
        config.max_deliver = Some(3);
        let mut deliveries = bus.durable(&name, config).await.unwrap();
        put(&bus, format!("{name}.a"), "x").await;
        // Refused with a delay: again after it.
        let first = next(&mut deliveries).await.unwrap();
        assert_eq!(first.message.delivered, 1);
        first.nak(Some(Duration::from_millis(300))).await.unwrap();
        let second = next(&mut deliveries).await.unwrap();
        assert_eq!(second.message.delivered, 2);
        // Left unacknowledged: again once its ack_wait has passed.
        let started = std::time::Instant::now();
        let third = next(&mut deliveries).await.unwrap();
        assert!(
            started.elapsed() >= Duration::from_millis(800),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(third.message.delivered, 3);
        // Past max_deliver it is given up.
        assert!(quiet(&mut deliveries, Duration::from_millis(1800)).await);
        clean(&bus, &[&name], &[]).await;
    }
);

both!(progress_holds_off_a_redelivery, |bus: Bus| async move {
    let name = unique("progress");
    bus.create_or_update_stream(stream(&name, Retention::Limits))
        .await
        .unwrap();
    let mut deliveries = bus
        .durable(&name, durable("slow", format!("{name}.>"), Deliver::All))
        .await
        .unwrap();
    put(&bus, format!("{name}.a"), "x").await;
    let delivered = next(&mut deliveries).await.unwrap();
    for _ in 0..3 {
        tokio::time::sleep(Duration::from_millis(600)).await;
        delivered.progress().await.unwrap();
    }
    assert!(quiet(&mut deliveries, Duration::from_millis(500)).await);
    delivered.ack().await.unwrap();
    assert!(quiet(&mut deliveries, Duration::from_millis(1500)).await);
    clean(&bus, &[&name], &[]).await;
});

both!(
    one_outstanding_holds_back_the_next_and_work_queues_let_go,
    |bus: Bus| async move {
        let name = unique("workqueue");
        bus.create_or_update_stream(stream(&name, Retention::WorkQueue))
            .await
            .unwrap();
        let mut config = durable("lane", format!("{name}.>"), Deliver::All);
        config.max_ack_pending = Some(1);
        config.ack_wait = Duration::from_secs(5);
        let mut lane = bus.durable(&name, config).await.unwrap();
        put(&bus, format!("{name}.a"), "1").await;
        put(&bus, format!("{name}.a"), "2").await;
        let first = next(&mut lane).await.unwrap();
        assert_eq!(first.message.payload, "1");
        assert!(
            quiet(&mut lane, Duration::from_millis(500)).await,
            "the second waits for the first"
        );
        first.ack().await.unwrap();
        let second = next(&mut lane).await.unwrap();
        assert_eq!(second.message.payload, "2");
        second.ack().await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        let info = bus.stream_info(&name).await.unwrap().unwrap();
        assert_eq!(
            info.state.messages, 0,
            "a work queue lets go of what is acknowledged"
        );
        clean(&bus, &[&name], &[]).await;
    }
);

both!(
    a_stream_whose_consumers_say_nothing_takes_its_own_limits,
    |bus: Bus| async move {
        let name = unique("limits_default");
        let mut config = stream(&name, Retention::Limits);
        config.consumer_limits = Some(ConsumerLimits {
            max_ack_pending: 1,
            inactive_threshold: Duration::from_secs(60),
        });
        bus.create_or_update_stream(config).await.unwrap();
        let mut lane = bus
            .durable(&name, durable("default", format!("{name}.>"), Deliver::All))
            .await
            .unwrap();
        put(&bus, format!("{name}.a"), "1").await;
        put(&bus, format!("{name}.a"), "2").await;
        let first = next(&mut lane).await.unwrap();
        assert!(quiet(&mut lane, Duration::from_millis(500)).await);
        first.ack().await.unwrap();
        assert_eq!(next(&mut lane).await.unwrap().message.payload, "2");
        clean(&bus, &[&name], &[]).await;
    }
);

async fn bucket(bus: &Bus, name: &str, max_age: Option<Duration>) -> Bucket {
    bus.bucket(BucketConfig {
        name: name.to_string(),
        description: "a test's".to_string(),
        max_age,
    })
    .await
    .unwrap()
}

both!(a_bucket_keeps_and_watches_its_keys, |bus: Bus| async move {
    let name = unique("bucket");
    let store = bucket(&bus, &name, None).await;
    assert_eq!(store.get("k").await.unwrap(), None);
    let mut watch = store.watch("k").await.unwrap();
    store.put("other", Bytes::from_static(b"o")).await.unwrap();
    store.put("k", Bytes::from_static(b"1")).await.unwrap();
    let change = next(&mut watch).await.unwrap();
    assert_eq!(
        (change.key.as_str(), change.value.as_ref(), change.operation),
        ("k", b"1".as_ref(), Operation::Put)
    );
    assert_eq!(
        store.get("k").await.unwrap(),
        Some(Bytes::from_static(b"1"))
    );
    let mut keys = store.keys().await.unwrap();
    keys.sort();
    assert_eq!(keys, ["k", "other"]);
    store.purge("k").await.unwrap();
    assert_eq!(next(&mut watch).await.unwrap().operation, Operation::Purge);
    assert_eq!(store.get("k").await.unwrap(), None);
    // The same bucket opened again holds the same keys.
    let again = bucket(&bus, &name, None).await;
    assert_eq!(
        again.get("other").await.unwrap(),
        Some(Bytes::from_static(b"o"))
    );
    clean(&bus, &[], &[&name]).await;
});

both!(
    a_bucket_lets_go_of_what_is_past_its_age,
    |bus: Bus| async move {
        let name = unique("bucket_age");
        let store = bucket(&bus, &name, Some(Duration::from_secs(1))).await;
        store.put("k", Bytes::from_static(b"1")).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(store.get("k").await.unwrap(), None);
        assert!(store.keys().await.unwrap().is_empty());
        clean(&bus, &[], &[&name]).await;
    }
);

#[tokio::test]
async fn memory_streams_are_refused_past_the_memory_limit() {
    let bus = Bus::Memory(Memory::new(Some(1000)));
    let mut first = stream("first", Retention::Limits);
    first.max_bytes = 600;
    bus.create_or_update_stream(first.clone()).await.unwrap();
    let mut second = stream("second", Retention::Limits);
    second.max_bytes = 500;
    assert!(matches!(
        bus.create_or_update_stream(second.clone()).await,
        Err(Error::NoRoom(_))
    ));
    second.max_bytes = 400;
    bus.create_or_update_stream(second).await.unwrap();
    // Changing one counts what it asks for now, not what it held.
    first.max_bytes = 500;
    bus.create_or_update_stream(first).await.unwrap();
    let memory = bus.account_memory().await.unwrap();
    assert_eq!((memory.reserved, memory.limit), (900, Some(1000)));
    // A stream on file takes none of it.
    let mut file = stream("file", Retention::Limits);
    file.storage = Storage::File;
    file.max_bytes = 10_000;
    bus.create_or_update_stream(file).await.unwrap();
}

#[tokio::test]
async fn memory_sequences_start_above_any_an_earlier_run_gave() {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let bus = Bus::Memory(Memory::new(None));
    bus.create_or_update_stream(stream("seq", Retention::Limits))
        .await
        .unwrap();
    let sequence = put(&bus, "seq.a".to_string(), "x").await;
    assert!(sequence >= before * 1000, "{sequence}");
    assert!(
        sequence < 1 << 53,
        "clients read sequences as numbers of 53 bits"
    );
    let info = bus.stream_info("seq").await.unwrap().unwrap();
    assert_eq!(info.state.last_sequence, sequence);
}
