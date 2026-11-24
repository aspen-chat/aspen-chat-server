//! The message bus Aspen's servers share: subjects to publish to and subscribe on, streams that
//! keep what is published to them for their consumers, and key-value buckets. Each server reaches
//! it through a [`Bus`], whose types are Aspen's own, so what a server does with the bus does not
//! depend on what carries it. [`Bus::Nats`] is NATS JetStream; [`Bus::Memory`] keeps it all inside
//! one process.
//!
//! Subjects are dot-separated tokens; a filter's `*` matches one token and a final `>` one or
//! more, as in NATS.

mod memory;
mod nats;
pub mod remote;

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::Stream;

pub use memory::{Memory, MemoryBucket};
pub use nats::Nats;
pub use remote::Remote;

/// A connection to the message bus. Cheap to clone; every clone shares the connection.
#[derive(Clone)]
pub enum Bus {
    Nats(Nats),
    /// The bus inside this process, for a deployment that is one server.
    Memory(Memory),
    /// Another process's memory bus, for a command run beside that one server.
    Remote(Remote),
}

/// What went wrong on the bus.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The stream named does not exist.
    #[error("the stream {0} does not exist")]
    NoSuchStream(String),
    /// The bus has no room for a stream of the size asked for.
    #[error("the message bus has no room for that stream: {0}")]
    NoRoom(#[source] BoxError),
    /// A consumer was removed, or is not of the kind it was opened as, so reading it ends.
    #[error("the consumer ended: {0}")]
    ConsumerEnded(#[source] BoxError),
    /// Anything else, with what was being done.
    #[error("message bus error while {doing}: {source}")]
    Failed {
        doing: &'static str,
        #[source]
        source: BoxError,
    },
}

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T> = std::result::Result<T, Error>;

/// The headers of a message. A name may repeat; [`Headers::get`] answers one of its values.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Headers(Vec<(String, String)>);

impl Headers {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, name: impl Into<String>, value: impl Into<String>) {
        self.0.push((name.into(), value.into()));
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(held, _)| held == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.0
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

/// A message published on a subject, as a subscriber receives it.
#[derive(Debug, Clone)]
pub struct Message {
    pub subject: String,
    /// Where an answer goes, for a request.
    pub reply: Option<String>,
    pub headers: Headers,
    pub payload: Bytes,
}

/// What a subscription receives, until the bus ends it.
pub struct Subscriber(Pin<Box<dyn Stream<Item = Message> + Send>>);

impl Stream for Subscriber {
    type Item = Message;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Message>> {
        self.0.as_mut().poll_next(cx)
    }
}

/// What happens to a stream's messages once its consumers have them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// Each is kept until its age or the stream's size lets it go.
    Limits,
    /// Each is let go once every consumer has acknowledged it. None of Aspen's streams is.
    Interest,
    /// Each is let go once a consumer acknowledges it.
    WorkQueue,
}

/// Where a stream keeps its messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    Memory,
    File,
}

/// A stream: which subjects it keeps, and for how long.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamConfig {
    pub name: String,
    pub subjects: Vec<String>,
    pub retention: Retention,
    pub storage: Storage,
    /// The oldest a message is kept.
    pub max_age: Duration,
    /// The most the stream holds, its oldest going first past it.
    pub max_bytes: u64,
    /// The most messages it holds, its oldest going first past it; `None` for no limit.
    pub max_messages: Option<u64>,
    /// The most a consumer may have delivered and not yet acknowledged, and how long one may
    /// go unread before it is removed, for each consumer that does not say.
    pub consumer_limits: Option<ConsumerLimits>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsumerLimits {
    pub max_ack_pending: i64,
    pub inactive_threshold: Duration,
}

/// A stream as the bus holds it now.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamInfo {
    pub config: StreamConfig,
    pub state: StreamState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamState {
    pub messages: u64,
    pub bytes: u64,
    /// The sequence of the last message it took, held or not; 0 before its first.
    pub last_sequence: u64,
    /// When its oldest message held was published; meaningless while it holds none.
    pub first_published: time::OffsetDateTime,
}

/// The memory the bus holds for this deployment's streams, as the bus reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountMemory {
    /// What its streams hold now.
    pub used: u64,
    /// What its streams have reserved, the sum of their `max_bytes` for those in memory.
    pub reserved: u64,
    /// The most it may reserve, when it is limited.
    pub limit: Option<u64>,
}

/// A message read from a stream.
#[derive(Debug, Clone)]
pub struct StreamMessage {
    pub subject: String,
    pub headers: Headers,
    pub payload: Bytes,
    pub sequence: u64,
    /// When the stream took it, on the bus's clock.
    pub published: time::OffsetDateTime,
    /// How many more of the consumer's are still to be delivered after it.
    pub pending: u64,
    /// How many times it has been delivered to this consumer, this time included.
    pub delivered: i64,
}

/// Where an ordered reading of a stream starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Start {
    /// The first message taken at or after this moment, on the bus's clock.
    ByTime(time::OffsetDateTime),
    /// The message of this sequence, or the first after it the stream still holds.
    BySequence(u64),
}

/// A reading of a stream in order, from where it started, delivering each message once.
pub struct Ordered(Pin<Box<dyn Stream<Item = Result<StreamMessage>> + Send>>);

impl Stream for Ordered {
    type Item = Result<StreamMessage>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.as_mut().poll_next(cx)
    }
}

/// Which messages a durable consumer starts with when it is first made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Deliver {
    /// Every message the stream holds.
    All,
    /// Only those published after it is made.
    New,
}

/// A consumer that keeps its place under its name, which every server reading it shares, so each
/// message goes to one of them, until acknowledged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DurableConfig {
    pub name: String,
    /// The subjects it reads.
    pub filter: String,
    pub deliver: Deliver,
    /// How long a delivery may go unacknowledged before it is delivered again.
    pub ack_wait: Duration,
    /// How many times a message is delivered before it is given up; `None` for no limit.
    pub max_deliver: Option<i64>,
    /// How many may be delivered and not yet acknowledged at once; `None` for the stream's own.
    pub max_ack_pending: Option<i64>,
}

/// A message delivered by a durable consumer, which says what became of it.
pub struct Delivery {
    pub message: StreamMessage,
    acker: Acker,
}

enum Acker {
    Nats(nats::Acker),
    Memory(memory::Acker),
}

impl Delivery {
    /// Done with: it is not delivered again.
    pub async fn ack(&self) -> Result<()> {
        match &self.acker {
            Acker::Nats(acker) => acker.ack().await,
            Acker::Memory(acker) => acker.ack().await,
        }
    }

    /// Not done with: delivered again after `delay`, or at once.
    pub async fn nak(&self, delay: Option<Duration>) -> Result<()> {
        match &self.acker {
            Acker::Nats(acker) => acker.nak(delay).await,
            Acker::Memory(acker) => acker.nak(delay).await,
        }
    }

    /// Still being worked on: its `ack_wait` starts again.
    pub async fn progress(&self) -> Result<()> {
        match &self.acker {
            Acker::Nats(acker) => acker.progress().await,
            Acker::Memory(acker) => acker.progress().await,
        }
    }
}

/// What a durable consumer delivers, for as long as it is read.
pub struct Deliveries(Pin<Box<dyn Stream<Item = Result<Delivery>> + Send>>);

impl Stream for Deliveries {
    type Item = Result<Delivery>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.as_mut().poll_next(cx)
    }
}

/// A message handed to a stream, whose acknowledgement says it was kept.
pub struct PublishAck(Pin<Box<dyn Future<Output = Result<u64>> + Send>>);

impl IntoFuture for PublishAck {
    /// The sequence the stream gave it.
    type Output = Result<u64>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<u64>> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        self.0
    }
}

/// A key-value bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketConfig {
    pub name: String,
    pub description: String,
    /// How long an entry is kept after it was last written; `None` for ever.
    pub max_age: Option<Duration>,
}

/// What became of a key, as a watch reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Put,
    Delete,
    Purge,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub key: String,
    pub value: Bytes,
    pub operation: Operation,
}

/// Every change to a key from when the watch is in place; what it held before is read with
/// [`Bucket::get`], after the watch starts so that no change falls between the two.
pub struct Watch(Pin<Box<dyn Stream<Item = Result<Entry>> + Send>>);

impl Stream for Watch {
    type Item = Result<Entry>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.0.as_mut().poll_next(cx)
    }
}

/// A key-value bucket, opened.
#[derive(Clone)]
pub enum Bucket {
    Nats(Box<async_nats::jetstream::kv::Store>),
    Memory(std::sync::Arc<MemoryBucket>),
    Remote(Remote, BucketConfig),
}

impl Bus {
    /// Whether the bus is reachable now.
    pub fn connected(&self) -> bool {
        match self {
            Bus::Nats(nats) => nats.connected(),
            Bus::Memory(memory) => memory.connected(),
            Bus::Remote(_) => true,
        }
    }

    /// Publishes `payload` on `subject` to whoever subscribes now; nothing keeps it.
    pub async fn publish(&self, subject: impl Into<String>, payload: Bytes) -> Result<()> {
        match self {
            Bus::Nats(nats) => nats.publish(subject.into(), payload).await,
            Bus::Memory(memory) => memory.publish(subject.into(), payload).await,
            Bus::Remote(remote) => remote.publish(subject.into(), payload).await,
        }
    }

    /// Waits until what this connection published has reached the bus.
    pub async fn flush(&self) -> Result<()> {
        match self {
            Bus::Nats(nats) => nats.flush().await,
            Bus::Memory(memory) => memory.flush().await,
            Bus::Remote(_) => Ok(()),
        }
    }

    pub async fn subscribe(&self, subject: impl Into<String>) -> Result<Subscriber> {
        match self {
            Bus::Nats(nats) => nats.subscribe(subject.into(), None).await,
            Bus::Memory(memory) => memory.subscribe(subject.into(), None).await,
            Bus::Remote(_) => Err(remote::refused("subscribing")),
        }
    }

    /// Subscribes as one of `group`, so each message goes to one subscriber of the group.
    pub async fn queue_subscribe(
        &self,
        subject: impl Into<String>,
        group: impl Into<String>,
    ) -> Result<Subscriber> {
        match self {
            Bus::Nats(nats) => nats.subscribe(subject.into(), Some(group.into())).await,
            Bus::Memory(memory) => memory.subscribe(subject.into(), Some(group.into())).await,
            Bus::Remote(_) => Err(remote::refused("subscribing")),
        }
    }

    /// Publishes `payload` on `subject` and waits for the first answer.
    pub async fn request(&self, subject: impl Into<String>, payload: Bytes) -> Result<Message> {
        match self {
            Bus::Nats(nats) => nats.request(subject.into(), payload).await,
            Bus::Memory(memory) => memory.request(subject.into(), payload).await,
            Bus::Remote(_) => Err(remote::refused("asking")),
        }
    }

    /// Hands `payload` to the stream that keeps `subject`, answering an acknowledgement to wait
    /// for. Copies handed over one after another are kept in that order.
    pub async fn publish_to_stream(
        &self,
        subject: impl Into<String>,
        headers: Headers,
        payload: Bytes,
    ) -> Result<PublishAck> {
        match self {
            Bus::Nats(nats) => {
                nats.publish_to_stream(subject.into(), headers, payload)
                    .await
            }
            Bus::Memory(memory) => {
                memory
                    .publish_to_stream(subject.into(), headers, payload)
                    .await
            }
            Bus::Remote(remote) => {
                remote
                    .publish_to_stream(subject.into(), headers, payload)
                    .await
            }
        }
    }

    /// Makes the stream `config` describes, or changes it to match.
    pub async fn create_or_update_stream(&self, config: StreamConfig) -> Result<StreamInfo> {
        match self {
            Bus::Nats(nats) => nats.create_or_update_stream(config).await,
            Bus::Memory(memory) => memory.create_or_update_stream(config).await,
            Bus::Remote(remote) => remote.create_or_update_stream(config).await,
        }
    }

    /// The stream `name` as the bus holds it now; `None` when there is none.
    pub async fn stream_info(&self, name: &str) -> Result<Option<StreamInfo>> {
        match self {
            Bus::Nats(nats) => nats.stream_info(name).await,
            Bus::Memory(memory) => memory.stream_info(name).await,
            Bus::Remote(remote) => remote.stream_info(name).await,
        }
    }

    /// Lets go of every message of `stream` on a subject `filter` matches.
    pub async fn purge(&self, stream: &str, filter: impl Into<String>) -> Result<()> {
        match self {
            Bus::Nats(nats) => nats.purge(stream, filter.into()).await,
            Bus::Memory(memory) => memory.purge(stream, filter.into()).await,
            Bus::Remote(remote) => remote.purge(stream, filter.into()).await,
        }
    }

    /// Reads `stream` in order from `start`, answering how many messages it has to deliver
    /// before it reaches the end of the stream.
    pub async fn ordered(&self, stream: &str, start: Start) -> Result<(Ordered, u64)> {
        match self {
            Bus::Nats(nats) => nats.ordered(stream, start).await,
            Bus::Memory(memory) => memory.ordered(stream, start).await,
            Bus::Remote(_) => Err(remote::refused("reading a stream")),
        }
    }

    /// Reads `stream` through the durable consumer `config` names, making it if it is not there.
    pub async fn durable(&self, stream: &str, config: DurableConfig) -> Result<Deliveries> {
        match self {
            Bus::Nats(nats) => nats.durable(stream, config).await,
            Bus::Memory(memory) => memory.durable(stream, config).await,
            Bus::Remote(_) => Err(remote::refused("reading a stream")),
        }
    }

    /// The memory the bus holds for this deployment's streams.
    pub async fn account_memory(&self) -> Result<AccountMemory> {
        match self {
            Bus::Nats(nats) => nats.account_memory().await,
            Bus::Memory(memory) => memory.account_memory().await,
            Bus::Remote(remote) => remote.account_memory().await,
        }
    }

    /// Opens the bucket `config` describes, making it on first use.
    pub async fn bucket(&self, config: BucketConfig) -> Result<Bucket> {
        match self {
            Bus::Nats(nats) => nats.bucket(config).await,
            Bus::Memory(memory) => memory.bucket(config).await,
            Bus::Remote(remote) => Ok(Bucket::Remote(remote.clone(), config)),
        }
    }
}

impl Bucket {
    pub async fn get(&self, key: &str) -> Result<Option<Bytes>> {
        match self {
            Bucket::Nats(store) => nats::get(store, key).await,
            Bucket::Memory(bucket) => bucket.get(key),
            Bucket::Remote(remote, config) => remote.bucket_get(config, key).await,
        }
    }

    pub async fn put(&self, key: &str, value: Bytes) -> Result<()> {
        match self {
            Bucket::Nats(store) => nats::put(store, key, value).await,
            Bucket::Memory(bucket) => bucket.put(key, value),
            Bucket::Remote(remote, config) => remote.bucket_put(config, key, value).await,
        }
    }

    /// Removes `key` and what came before it.
    pub async fn purge(&self, key: &str) -> Result<()> {
        match self {
            Bucket::Nats(store) => nats::purge(store, key).await,
            Bucket::Memory(bucket) => bucket.purge(key),
            Bucket::Remote(remote, config) => remote.bucket_purge(config, key).await,
        }
    }

    /// Every key the bucket holds now.
    pub async fn keys(&self) -> Result<Vec<String>> {
        match self {
            Bucket::Nats(store) => nats::keys(store).await,
            Bucket::Memory(bucket) => bucket.keys(),
            Bucket::Remote(remote, config) => remote.bucket_keys(config).await,
        }
    }

    pub async fn watch(&self, key: &str) -> Result<Watch> {
        match self {
            Bucket::Nats(store) => nats::watch(store, key).await,
            Bucket::Memory(bucket) => bucket.watch(key),
            Bucket::Remote(..) => Err(remote::refused("watching a bucket")),
        }
    }
}

#[cfg(test)]
mod tests;
