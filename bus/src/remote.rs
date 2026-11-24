//! [`Bus::Remote`](crate::Bus::Remote): another process's [`Memory`](crate::Memory) bus, reached
//! over a Unix socket it serves ([`serve`]), for a command run beside a deployment of one server
//! (`aspen-ul`) whose bus is that server's memory. A command publishes, sizes and purges streams,
//! and reads and writes buckets; what only a server does, subscribing and reading streams through
//! consumers, is refused over it.
//!
//! Each call is one connection: a request, a line of JSON, and its answer, another.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::{
    AccountMemory, BucketConfig, Bus, ConsumerLimits, Error, Headers, PublishAck, Result,
    Retention, Storage, StreamConfig, StreamInfo, StreamState,
};

/// The most a request or an answer may hold.
const MAX_LINE: usize = 16 * 1024 * 1024;

/// A bus served at a socket by the process that holds it.
#[derive(Clone, Debug)]
pub struct Remote {
    socket: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
enum Request {
    Publish {
        subject: String,
        payload: Vec<u8>,
    },
    PublishToStream {
        subject: String,
        headers: Vec<(String, String)>,
        payload: Vec<u8>,
    },
    CreateOrUpdateStream {
        config: WireStreamConfig,
    },
    StreamInfo {
        name: String,
    },
    Purge {
        stream: String,
        filter: String,
    },
    AccountMemory,
    BucketGet {
        bucket: WireBucket,
        key: String,
    },
    BucketPut {
        bucket: WireBucket,
        key: String,
        value: Vec<u8>,
    },
    BucketPurge {
        bucket: WireBucket,
        key: String,
    },
    BucketKeys {
        bucket: WireBucket,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Answer {
    Done,
    Sequence(u64),
    Stream(Option<WireStreamInfo>),
    Memory {
        used: u64,
        reserved: u64,
        limit: Option<u64>,
    },
    Value(Option<Vec<u8>>),
    Keys(Vec<String>),
    NoSuchStream(String),
    NoRoom(String),
    Failed(String),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireBucket {
    name: String,
    description: String,
    max_age_millis: Option<u64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum WireRetention {
    Limits,
    Interest,
    WorkQueue,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum WireStorage {
    Memory,
    File,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireStreamConfig {
    name: String,
    subjects: Vec<String>,
    retention: WireRetention,
    storage: WireStorage,
    max_age_millis: u64,
    max_bytes: u64,
    max_messages: Option<u64>,
    consumer_limits: Option<(i64, u64)>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireStreamInfo {
    config: WireStreamConfig,
    messages: u64,
    bytes: u64,
    last_sequence: u64,
    first_published_nanos: i128,
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

impl From<&StreamConfig> for WireStreamConfig {
    fn from(config: &StreamConfig) -> Self {
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
        Self {
            name: name.clone(),
            subjects: subjects.clone(),
            retention: match retention {
                Retention::Limits => WireRetention::Limits,
                Retention::Interest => WireRetention::Interest,
                Retention::WorkQueue => WireRetention::WorkQueue,
            },
            storage: match storage {
                Storage::Memory => WireStorage::Memory,
                Storage::File => WireStorage::File,
            },
            max_age_millis: millis(*max_age),
            max_bytes: *max_bytes,
            max_messages: *max_messages,
            consumer_limits: consumer_limits
                .map(|limits| (limits.max_ack_pending, millis(limits.inactive_threshold))),
        }
    }
}

impl From<WireStreamConfig> for StreamConfig {
    fn from(config: WireStreamConfig) -> Self {
        let WireStreamConfig {
            name,
            subjects,
            retention,
            storage,
            max_age_millis,
            max_bytes,
            max_messages,
            consumer_limits,
        } = config;
        Self {
            name,
            subjects,
            retention: match retention {
                WireRetention::Limits => Retention::Limits,
                WireRetention::Interest => Retention::Interest,
                WireRetention::WorkQueue => Retention::WorkQueue,
            },
            storage: match storage {
                WireStorage::Memory => Storage::Memory,
                WireStorage::File => Storage::File,
            },
            max_age: Duration::from_millis(max_age_millis),
            max_bytes,
            max_messages,
            consumer_limits: consumer_limits.map(|(max_ack_pending, inactive)| ConsumerLimits {
                max_ack_pending,
                inactive_threshold: Duration::from_millis(inactive),
            }),
        }
    }
}

impl From<&StreamInfo> for WireStreamInfo {
    fn from(info: &StreamInfo) -> Self {
        Self {
            config: (&info.config).into(),
            messages: info.state.messages,
            bytes: info.state.bytes,
            last_sequence: info.state.last_sequence,
            first_published_nanos: info.state.first_published.unix_timestamp_nanos(),
        }
    }
}

impl From<WireStreamInfo> for StreamInfo {
    fn from(info: WireStreamInfo) -> Self {
        Self {
            config: info.config.into(),
            state: StreamState {
                messages: info.messages,
                bytes: info.bytes,
                last_sequence: info.last_sequence,
                first_published: time::OffsetDateTime::from_unix_timestamp_nanos(
                    info.first_published_nanos,
                )
                .unwrap_or(time::OffsetDateTime::UNIX_EPOCH),
            },
        }
    }
}

impl From<&BucketConfig> for WireBucket {
    fn from(config: &BucketConfig) -> Self {
        Self {
            name: config.name.clone(),
            description: config.description.clone(),
            max_age_millis: config.max_age.map(millis),
        }
    }
}

impl From<WireBucket> for BucketConfig {
    fn from(bucket: WireBucket) -> Self {
        Self {
            name: bucket.name,
            description: bucket.description,
            max_age: bucket.max_age_millis.map(Duration::from_millis),
        }
    }
}

fn failed(doing: &'static str, why: impl Into<crate::BoxError>) -> Error {
    Error::Failed {
        doing,
        source: why.into(),
    }
}

/// What only a server does with the bus, which a command does not.
pub(crate) fn refused(doing: &'static str) -> Error {
    failed(
        doing,
        "a command reaches the deployment's bus only to publish, size streams, and use buckets",
    )
}

impl Remote {
    /// The bus `socket` serves, which this checks is there by asking it nothing.
    pub async fn connect(socket: &Path) -> Result<Self> {
        UnixStream::connect(socket)
            .await
            .map_err(|e| failed("reaching the deployment's bus", e))?;
        Ok(Self {
            socket: socket.to_path_buf(),
        })
    }

    async fn ask(&self, request: &Request) -> Result<Answer> {
        let mut stream = UnixStream::connect(&self.socket)
            .await
            .map_err(|e| failed("reaching the deployment's bus", e))?;
        let mut line = serde_json::to_vec(request).map_err(|e| failed("asking the bus", e))?;
        line.push(b'\n');
        stream
            .write_all(&line)
            .await
            .map_err(|e| failed("asking the bus", e))?;
        let mut answer = String::new();
        BufReader::new(stream)
            .take(MAX_LINE as u64)
            .read_line(&mut answer)
            .await
            .map_err(|e| failed("reading the bus's answer", e))?;
        let answer: Answer =
            serde_json::from_str(&answer).map_err(|e| failed("reading the bus's answer", e))?;
        match answer {
            Answer::NoSuchStream(name) => Err(Error::NoSuchStream(name)),
            Answer::NoRoom(why) => Err(Error::NoRoom(why.into())),
            Answer::Failed(why) => Err(failed("using the deployment's bus", why)),
            answer @ (Answer::Done
            | Answer::Sequence(_)
            | Answer::Stream(_)
            | Answer::Memory { .. }
            | Answer::Value(_)
            | Answer::Keys(_)) => Ok(answer),
        }
    }

    pub(crate) async fn publish(&self, subject: String, payload: Bytes) -> Result<()> {
        self.ask(&Request::Publish {
            subject,
            payload: payload.to_vec(),
        })
        .await
        .map(|_| ())
    }

    pub(crate) async fn publish_to_stream(
        &self,
        subject: String,
        headers: Headers,
        payload: Bytes,
    ) -> Result<PublishAck> {
        let headers = headers
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        match self
            .ask(&Request::PublishToStream {
                subject,
                headers,
                payload: payload.to_vec(),
            })
            .await?
        {
            Answer::Sequence(sequence) => Ok(PublishAck(Box::pin(async move { Ok(sequence) }))),
            _ => Err(failed(
                "publishing to a stream",
                "the bus answered something else",
            )),
        }
    }

    pub(crate) async fn create_or_update_stream(&self, config: StreamConfig) -> Result<StreamInfo> {
        match self
            .ask(&Request::CreateOrUpdateStream {
                config: (&config).into(),
            })
            .await?
        {
            Answer::Stream(Some(info)) => Ok(info.into()),
            _ => Err(failed(
                "making or changing a stream",
                "the bus answered something else",
            )),
        }
    }

    pub(crate) async fn stream_info(&self, name: &str) -> Result<Option<StreamInfo>> {
        match self
            .ask(&Request::StreamInfo {
                name: name.to_string(),
            })
            .await?
        {
            Answer::Stream(info) => Ok(info.map(StreamInfo::from)),
            _ => Err(failed(
                "reading a stream",
                "the bus answered something else",
            )),
        }
    }

    pub(crate) async fn purge(&self, stream: &str, filter: String) -> Result<()> {
        self.ask(&Request::Purge {
            stream: stream.to_string(),
            filter,
        })
        .await
        .map(|_| ())
    }

    pub(crate) async fn account_memory(&self) -> Result<AccountMemory> {
        match self.ask(&Request::AccountMemory).await? {
            Answer::Memory {
                used,
                reserved,
                limit,
            } => Ok(AccountMemory {
                used,
                reserved,
                limit,
            }),
            _ => Err(failed(
                "reading the bus's memory",
                "the bus answered something else",
            )),
        }
    }

    pub(crate) async fn bucket_get(
        &self,
        bucket: &BucketConfig,
        key: &str,
    ) -> Result<Option<Bytes>> {
        match self
            .ask(&Request::BucketGet {
                bucket: bucket.into(),
                key: key.to_string(),
            })
            .await?
        {
            Answer::Value(value) => Ok(value.map(Bytes::from)),
            _ => Err(failed(
                "reading a bucket",
                "the bus answered something else",
            )),
        }
    }

    pub(crate) async fn bucket_put(
        &self,
        bucket: &BucketConfig,
        key: &str,
        value: Bytes,
    ) -> Result<()> {
        self.ask(&Request::BucketPut {
            bucket: bucket.into(),
            key: key.to_string(),
            value: value.to_vec(),
        })
        .await
        .map(|_| ())
    }

    pub(crate) async fn bucket_purge(&self, bucket: &BucketConfig, key: &str) -> Result<()> {
        self.ask(&Request::BucketPurge {
            bucket: bucket.into(),
            key: key.to_string(),
        })
        .await
        .map(|_| ())
    }

    pub(crate) async fn bucket_keys(&self, bucket: &BucketConfig) -> Result<Vec<String>> {
        match self
            .ask(&Request::BucketKeys {
                bucket: bucket.into(),
            })
            .await?
        {
            Answer::Keys(keys) => Ok(keys),
            _ => Err(failed(
                "listing a bucket's keys",
                "the bus answered something else",
            )),
        }
    }
}

/// Answers every request made at `listener` on `bus`, for as long as the process runs.
pub async fn serve(listener: UnixListener, bus: Bus) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(e) => {
                tracing::warn!("the bus's socket could not accept: {e}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let bus = bus.clone();
        tokio::spawn(async move {
            if let Err(e) = answer_one(stream, &bus).await {
                tracing::warn!("a command's request to the bus failed: {e}");
            }
        });
    }
}

async fn answer_one(stream: UnixStream, bus: &Bus) -> std::io::Result<()> {
    let (read, mut write) = stream.into_split();
    let mut line = String::new();
    BufReader::new(read)
        .take(MAX_LINE as u64)
        .read_line(&mut line)
        .await?;
    // A process asking only whether the deployment runs connects and closes, sending nothing.
    if line.is_empty() {
        return Ok(());
    }
    let answer = match serde_json::from_str::<Request>(&line) {
        Ok(request) => answered(bus, request).await,
        Err(e) => Answer::Failed(format!("the request could not be read: {e}")),
    };
    let mut line = serde_json::to_vec(&answer).map_err(std::io::Error::other)?;
    line.push(b'\n');
    write.write_all(&line).await
}

async fn answered(bus: &Bus, request: Request) -> Answer {
    let result = match request {
        Request::Publish { subject, payload } => bus
            .publish(subject, Bytes::from(payload))
            .await
            .map(|()| Answer::Done),
        Request::PublishToStream {
            subject,
            headers,
            payload,
        } => {
            let mut held = Headers::new();
            for (name, value) in headers {
                held.insert(name, value);
            }
            match bus
                .publish_to_stream(subject, held, Bytes::from(payload))
                .await
            {
                Ok(ack) => ack.await.map(Answer::Sequence),
                Err(e) => Err(e),
            }
        }
        Request::CreateOrUpdateStream { config } => bus
            .create_or_update_stream(config.into())
            .await
            .map(|info| Answer::Stream(Some((&info).into()))),
        Request::StreamInfo { name } => bus
            .stream_info(&name)
            .await
            .map(|info| Answer::Stream(info.as_ref().map(WireStreamInfo::from))),
        Request::Purge { stream, filter } => {
            bus.purge(&stream, filter).await.map(|()| Answer::Done)
        }
        Request::AccountMemory => bus.account_memory().await.map(|memory| Answer::Memory {
            used: memory.used,
            reserved: memory.reserved,
            limit: memory.limit,
        }),
        Request::BucketGet { bucket, key } => match bus.bucket(bucket.into()).await {
            Ok(bucket) => bucket
                .get(&key)
                .await
                .map(|value| Answer::Value(value.map(|v| v.to_vec()))),
            Err(e) => Err(e),
        },
        Request::BucketPut { bucket, key, value } => match bus.bucket(bucket.into()).await {
            Ok(bucket) => bucket
                .put(&key, Bytes::from(value))
                .await
                .map(|()| Answer::Done),
            Err(e) => Err(e),
        },
        Request::BucketPurge { bucket, key } => match bus.bucket(bucket.into()).await {
            Ok(bucket) => bucket.purge(&key).await.map(|()| Answer::Done),
            Err(e) => Err(e),
        },
        Request::BucketKeys { bucket } => match bus.bucket(bucket.into()).await {
            Ok(bucket) => bucket.keys().await.map(Answer::Keys),
            Err(e) => Err(e),
        },
    };
    match result {
        Ok(answer) => answer,
        Err(Error::NoSuchStream(name)) => Answer::NoSuchStream(name),
        Err(Error::NoRoom(why)) => Answer::NoRoom(why.to_string()),
        Err(e @ (Error::ConsumerEnded(_) | Error::Failed { .. })) => Answer::Failed(e.to_string()),
    }
}
