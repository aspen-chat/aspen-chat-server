use crate::app;
use diesel_async::pooled_connection::deadpool;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("diesel error {0}")]
    Diesel(#[from] diesel::result::Error),
    #[error("deadpool error {0}")]
    Deadpool(#[from] deadpool::PoolError),
    #[error("config error {0}")]
    Config(#[from] config::ConfigError),
    #[error("argon2 password hash error {0}")]
    Argon2(#[from] argon2::password_hash::Error),
    #[error("error while connecting to NATS message broker {0}")]
    NatsConnectError(#[from] async_nats::ConnectError),
    #[error("error while creating or updating NATS stream {0}")]
    NatsCreateStreamError(#[from] async_nats::jetstream::context::CreateStreamError),
    #[error("error while getting NATS stream {0}")]
    NatsGetStreamError(#[from] async_nats::jetstream::context::GetStreamError),
    #[error("error while creating NATS stream consumer {0}")]
    NatsConsumerError(#[from] async_nats::jetstream::stream::ConsumerError),
    #[error("error while reading from NATS stream consumer {0}")]
    NatsStreamError(#[from] async_nats::jetstream::consumer::StreamError),
    #[error("error while publishing to NATS event stream {0}")]
    NatsPublishError(#[from] async_nats::jetstream::context::PublishError),
    #[error("error serializing as YAML {0}")]
    SerdeNorway(#[from] serde_norway::Error),
    #[error("error serializing as JSON {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("I/O error {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, app::Error>;
