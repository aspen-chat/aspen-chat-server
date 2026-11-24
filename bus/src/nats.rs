//! [`Bus::Nats`](crate::Bus::Nats): the bus as NATS JetStream.

use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream::{self, AckKind, ErrorCode, consumer, kv, stream};
use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};

use crate::{
    AccountMemory, BoxError, Bucket, BucketConfig, ConsumerLimits, Deliver, Deliveries, Delivery,
    DurableConfig, Entry, Error, Headers, Message, Operation, Ordered, PublishAck, Result,
    Retention, Start, Storage, StreamConfig, StreamInfo, StreamMessage, StreamState, Subscriber,
    Watch,
};

/// A NATS connection and its JetStream context.
#[derive(Clone)]
pub struct Nats(pub(crate) Arc<Connection>);

pub(crate) struct Connection {
    client: async_nats::Client,
    pub(crate) jetstream: jetstream::Context,
}

fn failed(doing: &'static str) -> impl FnOnce(BoxError) -> Error {
    move |source| Error::Failed { doing, source }
}

fn boxed<E: std::error::Error + Send + Sync + 'static>(error: E) -> BoxError {
    Box::new(error)
}

/// What an ordered reading asks for at a time: enough to catch up quickly, bounded so that a
/// server catching up a busy stream holds no more than this of it at once.
const ORDERED_BATCH: i64 = 1000;
const ORDERED_BATCH_BYTES: i64 = 8 * 1024 * 1024;
const ORDERED_EXPIRES: Duration = Duration::from_secs(5);

impl Nats {
    pub fn new(client: async_nats::Client) -> Self {
        let jetstream = jetstream::new(client.clone());
        Self(Arc::new(Connection { client, jetstream }))
    }

    pub(crate) fn connected(&self) -> bool {
        self.0.client.connection_state() == async_nats::connection::State::Connected
    }

    pub(crate) async fn publish(&self, subject: String, payload: Bytes) -> Result<()> {
        self.0
            .client
            .publish(subject, payload)
            .await
            .map_err(|e| failed("publishing")(boxed(e)))
    }

    pub(crate) async fn flush(&self) -> Result<()> {
        self.0
            .client
            .flush()
            .await
            .map_err(|e| failed("flushing")(boxed(e)))
    }

    pub(crate) async fn subscribe(
        &self,
        subject: String,
        group: Option<String>,
    ) -> Result<Subscriber> {
        let subscriber = match group {
            Some(group) => self.0.client.queue_subscribe(subject, group).await,
            None => self.0.client.subscribe(subject).await,
        }
        .map_err(|e| failed("subscribing")(boxed(e)))?;
        Ok(Subscriber(Box::pin(subscriber.map(message))))
    }

    pub(crate) async fn request(&self, subject: String, payload: Bytes) -> Result<Message> {
        self.0
            .client
            .request(subject, payload)
            .await
            .map(message)
            .map_err(|e| failed("asking")(boxed(e)))
    }

    pub(crate) async fn publish_to_stream(
        &self,
        subject: String,
        headers: Headers,
        payload: Bytes,
    ) -> Result<PublishAck> {
        let ack = self
            .0
            .jetstream
            .publish_with_headers(subject, header_map(&headers), payload)
            .await
            .map_err(|e| failed("publishing to a stream")(boxed(e)))?;
        Ok(PublishAck(Box::pin(async move {
            ack.await
                .map(|ack| ack.sequence)
                .map_err(|e| failed("waiting for a stream's acknowledgement")(boxed(e)))
        })))
    }

    pub(crate) async fn create_or_update_stream(&self, config: StreamConfig) -> Result<StreamInfo> {
        let info = self
            .0
            .jetstream
            .create_or_update_stream(stream_config(&config))
            .await
            .map_err(|e| match e.kind() {
                jetstream::context::CreateStreamErrorKind::JetStream(refusal)
                    if refusal.error_code() == ErrorCode::MEMORY_RESOURCES_EXCEEDED =>
                {
                    Error::NoRoom(boxed(e))
                }
                _ => failed("making or changing a stream")(boxed(e)),
            })?;
        stream_info(&info)
    }

    async fn stream(&self, name: &str) -> Result<jetstream::stream::Stream> {
        self.0
            .jetstream
            .get_stream(name)
            .await
            .map_err(|e| match e.kind() {
                jetstream::context::GetStreamErrorKind::JetStream(refusal)
                    if refusal.error_code() == ErrorCode::STREAM_NOT_FOUND =>
                {
                    Error::NoSuchStream(name.to_string())
                }
                _ => failed("opening a stream")(boxed(e)),
            })
    }

    pub(crate) async fn stream_info(&self, name: &str) -> Result<Option<StreamInfo>> {
        match self.stream(name).await {
            Ok(stream) => stream_info(stream.cached_info()).map(Some),
            Err(Error::NoSuchStream(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub(crate) async fn purge(&self, stream: &str, filter: String) -> Result<()> {
        self.stream(stream)
            .await?
            .purge()
            .filter(filter)
            .await
            .map(|_| ())
            .map_err(|e| failed("purging a stream")(boxed(e)))
    }

    pub(crate) async fn ordered(&self, stream: &str, start: Start) -> Result<(Ordered, u64)> {
        let deliver_policy = match start {
            Start::ByTime(start_time) => consumer::DeliverPolicy::ByStartTime { start_time },
            Start::BySequence(start_sequence) => {
                consumer::DeliverPolicy::ByStartSequence { start_sequence }
            }
        };
        let consumer = self
            .stream(stream)
            .await?
            .create_consumer(consumer::pull::OrderedConfig {
                deliver_policy,
                replay_policy: consumer::ReplayPolicy::Instant,
                max_batch: ORDERED_BATCH,
                max_bytes: ORDERED_BATCH_BYTES,
                max_expires: ORDERED_EXPIRES,
                ..Default::default()
            })
            .await
            .map_err(|e| failed("opening an ordered consumer")(boxed(e)))?;
        let pending = consumer.cached_info().num_pending;
        let messages = consumer
            .messages()
            .await
            .map_err(|e| failed("reading an ordered consumer")(boxed(e)))?
            .filter_map(|message| async move {
                match message {
                    Ok(message) => stream_message(&message),
                    Err(e) => Some(Err(failed("reading an ordered consumer")(boxed(e)))),
                }
            });
        Ok((Ordered(Box::pin(messages)), pending))
    }

    pub(crate) async fn durable(&self, stream: &str, config: DurableConfig) -> Result<Deliveries> {
        let DurableConfig {
            name,
            filter,
            deliver,
            ack_wait,
            max_deliver,
            max_ack_pending,
        } = config;
        let consumer = self
            .stream(stream)
            .await?
            .get_or_create_consumer(
                &name,
                consumer::pull::Config {
                    durable_name: Some(name.clone()),
                    filter_subject: filter,
                    deliver_policy: match deliver {
                        Deliver::All => consumer::DeliverPolicy::All,
                        Deliver::New => consumer::DeliverPolicy::New,
                    },
                    ack_policy: consumer::AckPolicy::Explicit,
                    ack_wait,
                    max_deliver: max_deliver.unwrap_or(-1),
                    max_ack_pending: max_ack_pending.unwrap_or(0),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| failed("opening a durable consumer")(boxed(e)))?;
        let deliveries = consumer
            .messages()
            .await
            .map_err(|e| failed("reading a durable consumer")(boxed(e)))?
            .filter_map(|delivered| async move {
                match delivered {
                    Ok(delivered) => match stream_message(&delivered) {
                        Some(message) => Some(message.map(|message| Delivery {
                            message,
                            acker: crate::Acker::Nats(Acker(Box::new(delivered))),
                        })),
                        // Never delivered again, so it cannot hold up a consumer that takes
                        // one message at a time.
                        None => {
                            if let Err(e) = delivered.ack_with(AckKind::Term).await {
                                tracing::error!(
                                    "could not let go of a message without its metadata: {e}"
                                );
                            }
                            None
                        }
                    },
                    Err(e) => Some(Err(match e.kind() {
                        consumer::pull::MessagesErrorKind::ConsumerDeleted
                        | consumer::pull::MessagesErrorKind::PushBasedConsumer => {
                            Error::ConsumerEnded(boxed(e))
                        }
                        consumer::pull::MessagesErrorKind::MissingHeartbeat
                        | consumer::pull::MessagesErrorKind::Pull
                        | consumer::pull::MessagesErrorKind::NoResponders
                        | consumer::pull::MessagesErrorKind::Other => {
                            failed("reading a durable consumer")(boxed(e))
                        }
                    })),
                }
            });
        Ok(Deliveries(Box::pin(deliveries)))
    }

    pub(crate) async fn account_memory(&self) -> Result<AccountMemory> {
        let account = self
            .0
            .jetstream
            .query_account()
            .await
            .map_err(|e| failed("reading the account")(boxed(e)))?;
        Ok(AccountMemory {
            used: account.memory,
            reserved: account.reserved_memory,
            limit: account
                .limits
                .max_memory
                .and_then(|limit| u64::try_from(limit).ok()),
        })
    }

    pub(crate) async fn bucket(&self, config: BucketConfig) -> Result<Bucket> {
        if let Ok(store) = self.0.jetstream.get_key_value(&config.name).await {
            return Ok(Bucket::Nats(Box::new(store)));
        }
        let BucketConfig {
            name,
            description,
            max_age,
        } = config;
        self.0
            .jetstream
            .create_key_value(kv::Config {
                bucket: name,
                description,
                history: 1,
                max_age: max_age.unwrap_or_default(),
                ..Default::default()
            })
            .await
            .map(|store| Bucket::Nats(Box::new(store)))
            .map_err(|e| failed("opening a bucket")(boxed(e)))
    }
}

/// Says what became of a delivered message.
pub(crate) struct Acker(Box<jetstream::Message>);

impl Acker {
    pub(crate) async fn ack(&self) -> Result<()> {
        self.0.ack().await.map_err(|e| failed("acknowledging")(e))
    }

    pub(crate) async fn nak(&self, delay: Option<Duration>) -> Result<()> {
        self.0
            .ack_with(AckKind::Nak(delay))
            .await
            .map_err(|e| failed("refusing a delivery")(e))
    }

    pub(crate) async fn progress(&self) -> Result<()> {
        self.0
            .ack_with(AckKind::Progress)
            .await
            .map_err(|e| failed("saying a delivery is in progress")(e))
    }
}

fn message(message: async_nats::Message) -> Message {
    Message {
        subject: message.subject.to_string(),
        reply: message.reply.map(|reply| reply.to_string()),
        headers: message.headers.as_ref().map(headers).unwrap_or_default(),
        payload: message.payload,
    }
}

fn header_map(headers: &Headers) -> async_nats::HeaderMap {
    let mut map = async_nats::HeaderMap::new();
    for (name, value) in headers.iter() {
        map.append(name, value);
    }
    map
}

fn headers(map: &async_nats::HeaderMap) -> Headers {
    let mut headers = Headers::new();
    for (name, values) in map.iter() {
        for value in values {
            headers.insert(name.to_string(), value.as_str());
        }
    }
    headers
}

/// A message read from a stream, or `None` for one that came without the metadata that places
/// it in its stream, which JetStream always gives: it is logged and skipped, and a durable
/// consumer's is terminated so that it is not delivered again.
fn stream_message(message: &jetstream::Message) -> Option<Result<StreamMessage>> {
    let info = match message.info() {
        Ok(info) => info,
        Err(e) => {
            tracing::error!("a stream message came without its metadata: {e}");
            return None;
        }
    };
    Some(Ok(StreamMessage {
        subject: message.subject.to_string(),
        headers: message.headers.as_ref().map(headers).unwrap_or_default(),
        payload: message.payload.clone(),
        sequence: info.stream_sequence,
        published: info.published,
        pending: info.pending,
        delivered: info.delivered,
    }))
}

fn stream_config(config: &StreamConfig) -> stream::Config {
    let StreamConfig {
        name,
        subjects,
        retention,
        storage,
        max_age,
        max_bytes,
        max_messages,
        consumer_limits,
    } = config;
    stream::Config {
        name: name.clone(),
        subjects: subjects.clone(),
        retention: match retention {
            Retention::Limits => stream::RetentionPolicy::Limits,
            Retention::Interest => stream::RetentionPolicy::Interest,
            Retention::WorkQueue => stream::RetentionPolicy::WorkQueue,
        },
        storage: match storage {
            Storage::Memory => stream::StorageType::Memory,
            Storage::File => stream::StorageType::File,
        },
        max_age: *max_age,
        max_bytes: i64::try_from(*max_bytes).unwrap_or(i64::MAX),
        max_messages: max_messages.map_or(-1, |most| i64::try_from(most).unwrap_or(i64::MAX)),
        discard: stream::DiscardPolicy::Old,
        consumer_limits: consumer_limits.map(|limits| stream::ConsumerLimits {
            max_ack_pending: limits.max_ack_pending,
            inactive_threshold: limits.inactive_threshold,
        }),
        ..Default::default()
    }
}

fn stream_info(info: &stream::Info) -> Result<StreamInfo> {
    let config = &info.config;
    Ok(StreamInfo {
        config: StreamConfig {
            name: config.name.clone(),
            subjects: config.subjects.clone(),
            retention: match config.retention {
                stream::RetentionPolicy::Limits => Retention::Limits,
                stream::RetentionPolicy::Interest => Retention::Interest,
                stream::RetentionPolicy::WorkQueue => Retention::WorkQueue,
            },
            storage: match config.storage {
                stream::StorageType::Memory => Storage::Memory,
                stream::StorageType::File => Storage::File,
            },
            max_age: config.max_age,
            max_bytes: u64::try_from(config.max_bytes).unwrap_or(0),
            max_messages: u64::try_from(config.max_messages).ok(),
            consumer_limits: config
                .consumer_limits
                .as_ref()
                .map(|limits| ConsumerLimits {
                    max_ack_pending: limits.max_ack_pending,
                    inactive_threshold: limits.inactive_threshold,
                }),
        },
        state: StreamState {
            messages: info.state.messages,
            bytes: info.state.bytes,
            last_sequence: info.state.last_sequence,
            first_published: info.state.first_timestamp,
        },
    })
}

pub(crate) async fn get(store: &kv::Store, key: &str) -> Result<Option<Bytes>> {
    store
        .get(key)
        .await
        .map_err(|e| failed("reading a bucket")(boxed(e)))
}

pub(crate) async fn put(store: &kv::Store, key: &str, value: Bytes) -> Result<()> {
    store
        .put(key, value)
        .await
        .map(|_| ())
        .map_err(|e| failed("writing to a bucket")(boxed(e)))
}

pub(crate) async fn purge(store: &kv::Store, key: &str) -> Result<()> {
    store
        .purge(key)
        .await
        .map_err(|e| failed("purging a bucket's key")(boxed(e)))
}

pub(crate) async fn keys(store: &kv::Store) -> Result<Vec<String>> {
    store
        .keys()
        .await
        .map_err(|e| failed("listing a bucket's keys")(boxed(e)))?
        .map_err(|e| failed("listing a bucket's keys")(boxed(e)))
        .try_collect()
        .await
}

pub(crate) async fn watch(store: &kv::Store, key: &str) -> Result<Watch> {
    let watch = store
        .watch(key)
        .await
        .map_err(|e| failed("watching a bucket")(boxed(e)))?
        .map(|entry| {
            entry
                .map(|entry| Entry {
                    key: entry.key,
                    value: entry.value,
                    operation: match entry.operation {
                        kv::Operation::Put => Operation::Put,
                        kv::Operation::Delete => Operation::Delete,
                        kv::Operation::Purge => Operation::Purge,
                    },
                })
                .map_err(|e| failed("watching a bucket")(boxed(e)))
        });
    Ok(Watch(Box::pin(watch)))
}
