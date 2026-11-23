//! [`Bus::Memory`](crate::Bus::Memory): the bus inside one process, for a deployment that is one
//! server. Everything a subscriber, stream, or bucket holds is in this process's memory and goes
//! with it; nothing is shared with another process, so there is nothing to connect to, reconnect,
//! or keep in step.
//!
//! What it keeps is bounded: a stream by its own `max_age`, `max_bytes`, and `max_messages`, and
//! every stream kept in memory together by the limit [`Memory::new`] is given, as NATS bounds its
//! memory streams by its account's; a subscriber's undelivered messages by
//! [`SUBSCRIBER_CAPACITY`], past which further messages are dropped for it, as NATS drops them for
//! a slow consumer; a bucket by its `max_age`.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::StreamExt;
use tokio::sync::{Notify, broadcast, mpsc};

use crate::{
    AccountMemory, BoxError, Bucket, BucketConfig, Deliver, Deliveries, Delivery, DurableConfig,
    Entry, Error, Headers, Message, Operation, Ordered, PublishAck, Result, Retention, Start,
    Storage, StreamConfig, StreamInfo, StreamMessage, StreamState, Subscriber, Watch,
};

/// The most messages one subscriber may have waiting to be read.
pub const SUBSCRIBER_CAPACITY: usize = 65_536;
/// How long a request waits for its answer, as NATS's client does by default.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How many changes a bucket's watcher may fall behind by before its watch ends with an error,
/// which the watcher answers by watching again and reading the key.
const WATCH_CAPACITY: usize = 256;

fn failed(doing: &'static str, why: impl Into<BoxError>) -> Error {
    Error::Failed {
        doing,
        source: why.into(),
    }
}

/// Whether `subject` is one `filter` matches: token by token, `*` matching any one token and a
/// final `>` one or more.
pub(crate) fn matches(filter: &str, subject: &str) -> bool {
    let (mut filter, mut subject) = (filter.split('.'), subject.split('.'));
    loop {
        match (filter.next(), subject.next()) {
            (Some(">"), Some(_)) => return true,
            (Some("*"), Some(_)) => {}
            (Some(wanted), Some(token)) => {
                if wanted != token {
                    return false;
                }
            }
            (None, None) => return true,
            (Some(_), None) | (None, Some(_)) => return false,
        }
    }
}

/// The bus inside this process. Cheap to clone; every clone shares what it holds, and differs
/// only in the inbox its requests are answered at ([`Memory::with_inbox_prefix`]).
#[derive(Clone)]
pub struct Memory {
    broker: Arc<Broker>,
    inbox_prefix: Arc<str>,
}

struct Broker {
    subscriptions: Mutex<Subscriptions>,
    streams: Mutex<HashMap<String, Arc<Stream>>>,
    buckets: Mutex<HashMap<String, Arc<MemoryBucket>>>,
    /// The sequence every stream made starts at: the moment the bus was made, in Unix
    /// milliseconds, times a thousand. It is above any sequence an earlier run of the process
    /// gave, so a client resuming from one is never taken for one still in the stream, and below
    /// 2^53, which clients read sequences as, for centuries.
    first_sequence: u64,
    /// The most every stream kept in memory may reserve together, the sum of their `max_bytes`.
    memory_limit: Option<u64>,
    next_inbox: AtomicU64,
}

#[derive(Default)]
struct Subscriptions {
    list: Vec<Subscription>,
    /// Which member of each queue group the next message goes to.
    turns: HashMap<String, usize>,
}

struct Subscription {
    filter: String,
    group: Option<String>,
    sender: mpsc::Sender<Message>,
}

impl Memory {
    /// An empty bus whose streams kept in memory may reserve at most `memory_limit` bytes
    /// together, or any amount without one.
    pub fn new(memory_limit: Option<u64>) -> Self {
        let millis = time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
        Self {
            broker: Arc::new(Broker {
                subscriptions: Mutex::default(),
                streams: Mutex::default(),
                buckets: Mutex::default(),
                first_sequence: u64::try_from(millis).unwrap_or(0).saturating_mul(1000),
                memory_limit,
                next_inbox: AtomicU64::new(0),
            }),
            inbox_prefix: Arc::from("_INBOX"),
        }
    }

    /// This bus, its requests answered at subjects under `prefix`, as a NATS client's are with a
    /// custom inbox prefix.
    pub fn with_inbox_prefix(&self, prefix: impl Into<String>) -> Self {
        Self {
            broker: Arc::clone(&self.broker),
            inbox_prefix: Arc::from(prefix.into()),
        }
    }

    pub(crate) fn connected(&self) -> bool {
        true
    }

    /// Hands `message` to every subscriber whose filter matches it, and to one member of each
    /// queue group; whether any subscriber took it.
    fn deliver(&self, message: Message) -> bool {
        let mut guard = self
            .broker
            .subscriptions
            .lock()
            .expect("subscriptions lock");
        let subscriptions = &mut *guard;
        subscriptions.list.retain(|s| !s.sender.is_closed());
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut taken = false;
        for (index, subscription) in subscriptions.list.iter().enumerate() {
            if !matches(&subscription.filter, &message.subject) {
                continue;
            }
            match &subscription.group {
                Some(group) => groups.entry(group.clone()).or_default().push(index),
                None => {
                    taken = true;
                    if subscription.sender.try_send(message.clone()).is_err() {
                        tracing::warn!(
                            subject = message.subject,
                            "a slow subscriber missed a message"
                        );
                    }
                }
            }
        }
        for (group, members) in groups {
            let turn = subscriptions.turns.entry(group).or_default();
            let index = members[*turn % members.len()];
            *turn = turn.wrapping_add(1);
            taken = true;
            if subscriptions.list[index]
                .sender
                .try_send(message.clone())
                .is_err()
            {
                tracing::warn!(
                    subject = message.subject,
                    "a slow subscriber missed a message"
                );
            }
        }
        taken
    }

    pub(crate) async fn publish(&self, subject: String, payload: Bytes) -> Result<()> {
        self.deliver(Message {
            subject,
            reply: None,
            headers: Headers::new(),
            payload,
        });
        Ok(())
    }

    pub(crate) async fn flush(&self) -> Result<()> {
        Ok(())
    }

    pub(crate) async fn subscribe(
        &self,
        subject: String,
        group: Option<String>,
    ) -> Result<Subscriber> {
        let (sender, receiver) = mpsc::channel(SUBSCRIBER_CAPACITY);
        {
            let mut subscriptions = self
                .broker
                .subscriptions
                .lock()
                .expect("subscriptions lock");
            subscriptions.list.push(Subscription {
                filter: subject,
                group,
                sender,
            });
        }
        let messages = futures_util::stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|message| (message, receiver))
        });
        Ok(Subscriber(Box::pin(messages)))
    }

    pub(crate) async fn request(&self, subject: String, payload: Bytes) -> Result<Message> {
        let inbox = format!(
            "{}.{}",
            self.inbox_prefix,
            self.broker.next_inbox.fetch_add(1, Ordering::Relaxed)
        );
        let mut answers = self.subscribe(inbox.clone(), None).await?;
        let asked = self.deliver(Message {
            subject,
            reply: Some(inbox),
            headers: Headers::new(),
            payload,
        });
        if !asked {
            return Err(failed("asking", "no one answers that subject"));
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, answers.next()).await {
            Ok(Some(answer)) => Ok(answer),
            Ok(None) => Err(failed("asking", "the answer's subscription ended")),
            Err(_) => Err(failed("asking", "no answer came in time")),
        }
    }

    fn stream_for_subject(&self, subject: &str) -> Option<Arc<Stream>> {
        let streams = self.broker.streams.lock().expect("streams lock");
        streams
            .values()
            .find(|stream| {
                let held = stream.held.lock().expect("stream lock");
                held.config
                    .subjects
                    .iter()
                    .any(|filter| matches(filter, subject))
            })
            .cloned()
    }

    fn stream(&self, name: &str) -> Result<Arc<Stream>> {
        self.broker
            .streams
            .lock()
            .expect("streams lock")
            .get(name)
            .cloned()
            .ok_or_else(|| Error::NoSuchStream(name.to_string()))
    }

    pub(crate) async fn publish_to_stream(
        &self,
        subject: String,
        headers: Headers,
        payload: Bytes,
    ) -> Result<PublishAck> {
        let stream = self.stream_for_subject(&subject).ok_or_else(|| {
            failed(
                "publishing to a stream",
                format!("no stream takes {subject}"),
            )
        })?;
        let sequence = stream.append(subject, headers, payload);
        Ok(PublishAck(Box::pin(async move { Ok(sequence) })))
    }

    pub(crate) async fn create_or_update_stream(&self, config: StreamConfig) -> Result<StreamInfo> {
        let mut streams = self.broker.streams.lock().expect("streams lock");
        if let Some(limit) = self.broker.memory_limit
            && config.storage == Storage::Memory
        {
            let others: u64 = streams
                .values()
                .filter_map(|stream| {
                    let held = stream.held.lock().expect("stream lock");
                    (held.config.name != config.name && held.config.storage == Storage::Memory)
                        .then_some(held.config.max_bytes)
                })
                .sum();
            if others.saturating_add(config.max_bytes) > limit {
                return Err(Error::NoRoom(
                    format!(
                        "{} bytes asked for {}, {others} reserved by the others, of {limit}",
                        config.max_bytes, config.name
                    )
                    .into(),
                ));
            }
        }
        let stream = match streams.get(&config.name) {
            Some(stream) => {
                let mut held = stream.held.lock().expect("stream lock");
                held.config = config;
                held.trim(time::OffsetDateTime::now_utc());
                drop(held);
                Arc::clone(stream)
            }
            None => {
                let stream = Arc::new(Stream {
                    held: Mutex::new(Held {
                        config: config.clone(),
                        messages: VecDeque::new(),
                        bytes: 0,
                        last_sequence: self.broker.first_sequence.saturating_sub(1),
                        consumers: HashMap::new(),
                    }),
                    changed: Notify::new(),
                });
                streams.insert(config.name.clone(), Arc::clone(&stream));
                stream
            }
        };
        drop(streams);
        stream.changed.notify_waiters();
        Ok(stream.info())
    }

    pub(crate) async fn stream_info(&self, name: &str) -> Result<Option<StreamInfo>> {
        match self.stream(name) {
            Ok(stream) => Ok(Some(stream.info())),
            Err(Error::NoSuchStream(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub(crate) async fn purge(&self, stream: &str, filter: String) -> Result<()> {
        let stream = self.stream(stream)?;
        let mut held = stream.held.lock().expect("stream lock");
        let Held {
            messages, bytes, ..
        } = &mut *held;
        messages.retain(|stored| {
            let keep = !matches(&filter, &stored.subject);
            if !keep {
                *bytes = bytes.saturating_sub(stored.size);
            }
            keep
        });
        Ok(())
    }

    pub(crate) async fn ordered(&self, stream: &str, start: Start) -> Result<(Ordered, u64)> {
        let stream = self.stream(stream)?;
        let (next, pending) = {
            let mut held = stream.held.lock().expect("stream lock");
            held.trim(time::OffsetDateTime::now_utc());
            let index = match start {
                Start::BySequence(sequence) => held
                    .messages
                    .partition_point(|stored| stored.sequence < sequence),
                Start::ByTime(at) => held
                    .messages
                    .partition_point(|stored| stored.published < at),
            };
            let next = held
                .messages
                .get(index)
                .map_or(held.last_sequence.saturating_add(1), |stored| {
                    stored.sequence
                });
            (next, (held.messages.len() - index) as u64)
        };
        let reading = Arc::new(Mutex::new(next));
        let messages = async_stream(move || {
            let (stream, reading) = (Arc::clone(&stream), Arc::clone(&reading));
            async move {
                loop {
                    let changed = stream.changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    {
                        let mut held = stream.held.lock().expect("stream lock");
                        held.trim(time::OffsetDateTime::now_utc());
                        let mut next = reading.lock().expect("reading lock");
                        let index = held
                            .messages
                            .partition_point(|stored| stored.sequence < *next);
                        if let Some(stored) = held.messages.get(index) {
                            *next = stored.sequence + 1;
                            let pending = (held.messages.len() - index - 1) as u64;
                            return Some(Ok(stored.read(pending, 1)));
                        }
                    }
                    changed.await;
                }
            }
        });
        Ok((Ordered(Box::pin(messages)), pending))
    }

    pub(crate) async fn durable(&self, stream: &str, config: DurableConfig) -> Result<Deliveries> {
        let stream = self.stream(stream)?;
        let name = config.name.clone();
        {
            let mut held = stream.held.lock().expect("stream lock");
            held.trim(time::OffsetDateTime::now_utc());
            let next = match config.deliver {
                Deliver::All => held
                    .messages
                    .front()
                    .map_or(held.last_sequence.saturating_add(1), |stored| {
                        stored.sequence
                    }),
                Deliver::New => held.last_sequence.saturating_add(1),
            };
            held.consumers
                .entry(name.clone())
                .or_insert_with(|| Consumer {
                    config,
                    next,
                    outstanding: BTreeMap::new(),
                });
        }
        let deliveries = async_stream(move || {
            let (stream, name) = (Arc::clone(&stream), name.clone());
            async move {
                loop {
                    let changed = stream.changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    let wait = {
                        let mut held = stream.held.lock().expect("stream lock");
                        match held.next_delivery(&name, Instant::now()) {
                            Next::Deliver(message) => {
                                return Some(Ok(Delivery {
                                    acker: crate::Acker::Memory(Acker {
                                        stream: Arc::clone(&stream),
                                        consumer: name.clone(),
                                        sequence: message.sequence,
                                    }),
                                    message,
                                }));
                            }
                            Next::Ended => {
                                return Some(Err(Error::ConsumerEnded(
                                    format!("the consumer {name} is gone").into(),
                                )));
                            }
                            Next::WaitUntil(at) => at,
                        }
                    };
                    match wait {
                        Some(at) => {
                            tokio::select! {
                                () = changed => {}
                                () = tokio::time::sleep_until(at.into()) => {}
                            }
                        }
                        None => changed.await,
                    }
                }
            }
        });
        Ok(Deliveries(Box::pin(deliveries)))
    }

    pub(crate) async fn account_memory(&self) -> Result<AccountMemory> {
        let streams = self.broker.streams.lock().expect("streams lock");
        let (mut used, mut reserved) = (0, 0);
        for stream in streams.values() {
            let held = stream.held.lock().expect("stream lock");
            match held.config.storage {
                Storage::Memory => {
                    used += held.bytes;
                    reserved += held.config.max_bytes;
                }
                Storage::File => {}
            }
        }
        Ok(AccountMemory {
            used,
            reserved,
            limit: self.broker.memory_limit,
        })
    }

    pub(crate) async fn bucket(&self, config: BucketConfig) -> Result<Bucket> {
        let mut buckets = self.broker.buckets.lock().expect("buckets lock");
        let bucket = buckets
            .entry(config.name.clone())
            .or_insert_with(|| {
                Arc::new(MemoryBucket {
                    max_age: config.max_age,
                    entries: Mutex::default(),
                    changes: broadcast::channel(WATCH_CAPACITY).0,
                })
            })
            .clone();
        Ok(Bucket::Memory(bucket))
    }
}

/// An async stream of what `next` answers each time, ending when it answers `None`.
fn async_stream<T, F, N>(mut next: N) -> impl futures_util::Stream<Item = T> + Send
where
    T: Send,
    F: std::future::Future<Output = Option<T>> + Send,
    N: FnMut() -> F + Send,
{
    futures_util::stream::unfold((), move |()| {
        let next = next();
        async move { next.await.map(|item| (item, ())) }
    })
}

struct Stream {
    held: Mutex<Held>,
    /// Told whenever the stream takes a message or what its consumers may be delivered changes.
    changed: Notify,
}

struct Held {
    config: StreamConfig,
    /// In order of sequence; sequences rise with gaps where messages were let go.
    messages: VecDeque<Stored>,
    bytes: u64,
    last_sequence: u64,
    consumers: HashMap<String, Consumer>,
}

struct Stored {
    sequence: u64,
    subject: String,
    headers: Headers,
    payload: Bytes,
    published: time::OffsetDateTime,
    /// What it counts toward the stream's `max_bytes`: its subject, headers, and payload.
    size: u64,
}

impl Stored {
    fn read(&self, pending: u64, delivered: i64) -> StreamMessage {
        StreamMessage {
            subject: self.subject.clone(),
            headers: self.headers.clone(),
            payload: self.payload.clone(),
            sequence: self.sequence,
            published: self.published,
            pending,
            delivered,
        }
    }
}

struct Consumer {
    config: DurableConfig,
    /// The sequence from which messages not yet delivered are looked for.
    next: u64,
    /// What has been delivered and not acknowledged, by sequence.
    outstanding: BTreeMap<u64, Outstanding>,
}

struct Outstanding {
    deliveries: i64,
    /// When it is delivered again unless acknowledged: its `ack_wait` after a delivery or a
    /// progress report, or a refusal's delay after it.
    due: Instant,
}

enum Next {
    Deliver(StreamMessage),
    /// Nothing is due; look again when the stream changes, or at this moment.
    WaitUntil(Option<Instant>),
    Ended,
}

impl Stream {
    fn append(&self, subject: String, headers: Headers, payload: Bytes) -> u64 {
        let now = time::OffsetDateTime::now_utc();
        let sequence = {
            let mut held = self.held.lock().expect("stream lock");
            held.trim(now);
            held.last_sequence += 1;
            let size = (subject.len()
                + headers
                    .iter()
                    .map(|(n, v)| n.len() + v.len())
                    .sum::<usize>()
                + payload.len()) as u64;
            let sequence = held.last_sequence;
            held.messages.push_back(Stored {
                sequence,
                subject,
                headers,
                payload,
                published: now,
                size,
            });
            held.bytes += size;
            held.trim(now);
            sequence
        };
        self.changed.notify_waiters();
        sequence
    }

    fn info(&self) -> StreamInfo {
        let mut held = self.held.lock().expect("stream lock");
        let now = time::OffsetDateTime::now_utc();
        held.trim(now);
        StreamInfo {
            config: held.config.clone(),
            state: StreamState {
                messages: held.messages.len() as u64,
                bytes: held.bytes,
                last_sequence: held.last_sequence,
                first_published: held.messages.front().map_or(now, |stored| stored.published),
            },
        }
    }
}

impl Held {
    /// Lets go of the oldest messages past the stream's age, size, and count.
    fn trim(&mut self, now: time::OffsetDateTime) {
        let max_age = self.config.max_age;
        let max_messages = self.config.max_messages.unwrap_or(u64::MAX);
        while let Some(oldest) = self.messages.front() {
            let aged = !max_age.is_zero() && now - oldest.published >= max_age;
            let over =
                self.bytes > self.config.max_bytes || self.messages.len() as u64 > max_messages;
            if !(aged || over) {
                break;
            }
            self.bytes = self.bytes.saturating_sub(oldest.size);
            self.messages.pop_front();
        }
    }

    fn position(&self, sequence: u64) -> Option<usize> {
        let index = self
            .messages
            .partition_point(|stored| stored.sequence < sequence);
        self.messages
            .get(index)
            .is_some_and(|stored| stored.sequence == sequence)
            .then_some(index)
    }

    /// What the consumer `name` delivers next: a message whose acknowledgement is overdue or
    /// whose refusal's delay has passed, then the next new one its filter matches while it has
    /// room under `max_ack_pending`.
    fn next_delivery(&mut self, name: &str, now: Instant) -> Next {
        self.trim(time::OffsetDateTime::now_utc());
        let stream_max_ack_pending = self
            .config
            .consumer_limits
            .map(|limits| limits.max_ack_pending);
        let Some(consumer) = self.consumers.get_mut(name) else {
            return Next::Ended;
        };
        // What the stream no longer holds can be delivered no more.
        let messages = &self.messages;
        let held = |sequence: u64| {
            let index = messages.partition_point(|stored| stored.sequence < sequence);
            messages
                .get(index)
                .filter(|stored| stored.sequence == sequence)
        };
        consumer
            .outstanding
            .retain(|&sequence, _| held(sequence).is_some());
        let max_deliver = consumer.config.max_deliver.filter(|most| *most > 0);
        if let Some((&sequence, outstanding)) = consumer
            .outstanding
            .iter_mut()
            .find(|(_, outstanding)| outstanding.due <= now)
        {
            if max_deliver.is_some_and(|most| outstanding.deliveries >= most) {
                consumer.outstanding.remove(&sequence);
            } else {
                outstanding.deliveries += 1;
                outstanding.due = now + consumer.config.ack_wait;
                let deliveries = outstanding.deliveries;
                let stored = held(sequence).expect("kept above");
                let pending = self.messages.len() as u64;
                return Next::Deliver(stored.read(pending, deliveries));
            }
        }
        let room = consumer
            .config
            .max_ack_pending
            .or(stream_max_ack_pending)
            .filter(|most| *most > 0)
            .is_none_or(|most| (consumer.outstanding.len() as i64) < most);
        if room {
            let start = self
                .messages
                .partition_point(|stored| stored.sequence < consumer.next);
            if let Some((offset, stored)) = self
                .messages
                .iter()
                .skip(start)
                .enumerate()
                .find(|(_, stored)| matches(&consumer.config.filter, &stored.subject))
            {
                consumer.next = stored.sequence + 1;
                consumer.outstanding.insert(
                    stored.sequence,
                    Outstanding {
                        deliveries: 1,
                        due: now + consumer.config.ack_wait,
                    },
                );
                let pending = (self.messages.len() - start - offset - 1) as u64;
                return Next::Deliver(stored.read(pending, 1));
            }
            consumer.next = self.last_sequence.saturating_add(1).max(consumer.next);
        }
        Next::WaitUntil(
            consumer
                .outstanding
                .values()
                .map(|outstanding| outstanding.due)
                .min(),
        )
    }
}

/// Says what became of a message a durable consumer delivered.
pub(crate) struct Acker {
    stream: Arc<Stream>,
    consumer: String,
    sequence: u64,
}

impl Acker {
    fn with_outstanding(&self, change: impl FnOnce(&mut Held, Option<&mut Outstanding>)) {
        {
            let mut held = self.stream.held.lock().expect("stream lock");
            let held = &mut *held;
            let mut outstanding = held
                .consumers
                .get_mut(&self.consumer)
                .and_then(|consumer| consumer.outstanding.remove(&self.sequence));
            change(held, outstanding.as_mut());
            if let Some(outstanding) = outstanding
                && let Some(consumer) = held.consumers.get_mut(&self.consumer)
            {
                consumer.outstanding.insert(self.sequence, outstanding);
            }
        }
        self.stream.changed.notify_waiters();
    }

    pub(crate) async fn ack(&self) -> Result<()> {
        let sequence = self.sequence;
        let stream = Arc::clone(&self.stream);
        {
            let mut held = stream.held.lock().expect("stream lock");
            if let Some(consumer) = held.consumers.get_mut(&self.consumer) {
                consumer.outstanding.remove(&sequence);
            }
            match held.config.retention {
                Retention::WorkQueue => {
                    if let Some(index) = held.position(sequence)
                        && let Some(stored) = held.messages.remove(index)
                    {
                        held.bytes = held.bytes.saturating_sub(stored.size);
                    }
                }
                Retention::Limits | Retention::Interest => {}
            }
        }
        stream.changed.notify_waiters();
        Ok(())
    }

    pub(crate) async fn nak(&self, delay: Option<Duration>) -> Result<()> {
        let due = Instant::now() + delay.unwrap_or_default();
        self.with_outstanding(|_, outstanding| {
            if let Some(outstanding) = outstanding {
                outstanding.due = due;
            }
        });
        Ok(())
    }

    pub(crate) async fn progress(&self) -> Result<()> {
        let now = Instant::now();
        self.with_outstanding(|held, outstanding| {
            let ack_wait = held
                .consumers
                .get(&self.consumer)
                .map(|consumer| consumer.config.ack_wait)
                .unwrap_or_default();
            if let Some(outstanding) = outstanding {
                outstanding.due = now + ack_wait;
            }
        });
        Ok(())
    }
}

/// A bucket inside this process.
pub struct MemoryBucket {
    max_age: Option<Duration>,
    entries: Mutex<HashMap<String, (Bytes, Instant)>>,
    changes: broadcast::Sender<Entry>,
}

impl MemoryBucket {
    fn live(&self, written: Instant, now: Instant) -> bool {
        self.max_age
            .is_none_or(|age| now.duration_since(written) < age)
    }

    pub(crate) fn get(&self, key: &str) -> Result<Option<Bytes>> {
        let now = Instant::now();
        let mut entries = self.entries.lock().expect("bucket lock");
        match entries.get(key) {
            Some((value, written)) if self.live(*written, now) => Ok(Some(value.clone())),
            Some(_) => {
                entries.remove(key);
                Ok(None)
            }
            None => Ok(None),
        }
    }

    pub(crate) fn put(&self, key: &str, value: Bytes) -> Result<()> {
        let now = Instant::now();
        {
            let mut entries = self.entries.lock().expect("bucket lock");
            entries.retain(|_, (_, written)| self.live(*written, now));
            entries.insert(key.to_string(), (value.clone(), now));
        }
        let _ = self.changes.send(Entry {
            key: key.to_string(),
            value,
            operation: Operation::Put,
        });
        Ok(())
    }

    pub(crate) fn purge(&self, key: &str) -> Result<()> {
        self.entries.lock().expect("bucket lock").remove(key);
        let _ = self.changes.send(Entry {
            key: key.to_string(),
            value: Bytes::new(),
            operation: Operation::Purge,
        });
        Ok(())
    }

    pub(crate) fn keys(&self) -> Result<Vec<String>> {
        let now = Instant::now();
        let mut entries = self.entries.lock().expect("bucket lock");
        entries.retain(|_, (_, written)| self.live(*written, now));
        Ok(entries.keys().cloned().collect())
    }

    pub(crate) fn watch(&self, key: &str) -> Result<Watch> {
        let key = key.to_string();
        let changes = Arc::new(tokio::sync::Mutex::new(self.changes.subscribe()));
        let watch = async_stream(move || {
            let (changes, key) = (Arc::clone(&changes), key.clone());
            async move {
                let mut changes = changes.lock().await;
                loop {
                    match changes.recv().await {
                        Ok(entry) if entry.key == key => return Some(Ok(entry)),
                        Ok(_) => {}
                        Err(broadcast::error::RecvError::Lagged(missed)) => {
                            return Some(Err(failed(
                                "watching a bucket",
                                format!("the watch fell {missed} changes behind"),
                            )));
                        }
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            }
        });
        Ok(Watch(Box::pin(watch)))
    }
}
