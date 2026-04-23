use crate::app;
use diesel_async::pooled_connection::deadpool;
use std::borrow::Cow;

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
    NatsConnect(#[from] async_nats::ConnectError),
    #[error("error while creating or updating NATS stream {0}")]
    NatsCreateStream(#[from] async_nats::jetstream::context::CreateStreamError),
    #[error("error while getting NATS stream {0}")]
    NatsGetStream(#[from] async_nats::jetstream::context::GetStreamError),
    #[error("error while creating NATS stream consumer {0}")]
    NatsConsumer(#[from] async_nats::jetstream::stream::ConsumerError),
    #[error("error while reading from NATS stream consumer {0}")]
    NatsStream(#[from] async_nats::jetstream::consumer::StreamError),
    #[error("error while publishing to NATS event stream {0}")]
    NatsPublish(#[from] async_nats::jetstream::context::PublishError),
    #[error("error serializing as YAML {0}")]
    SerdeNorway(#[from] serde_norway::Error),
    #[error("error serializing as JSON {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("I/O error {0}")]
    Io(#[from] std::io::Error),
    #[error("valkey error {0}")]
    Valkey(#[from] fred::error::Error),
    #[error("failed to build connection pool: {0}")]
    DeadpoolBuild(#[from] deadpool::BuildError),
    #[error("validation error: {0}")]
    Validation(Cow<'static, str>),
    #[error("error putting object to media store {0}")]
    S3PutObject(
        #[from] Box<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::put_object::PutObjectError>>,
    ),
    #[error("error reading object from media store {0}")]
    S3GetObject(
        #[from] Box<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::get_object::GetObjectError>>,
    ),
    #[error("error deleting object from media store {0}")]
    S3DeleteObject(
        #[from]
        Box<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::delete_object::DeleteObjectError>>,
    ),
    #[error("error reading media bytes {0}")]
    S3ByteStream(#[from] aws_sdk_s3::primitives::ByteStreamError),
    #[error("user not authorized")]
    Unauthorized,
    #[error("tokio join error {0}")]
    TokioJoinError(#[from] tokio::task::JoinError),
}

pub type Result<T> = std::result::Result<T, app::Error>;
