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
    #[error("error while querying NATS stream state {0}")]
    NatsRequest(#[from] async_nats::jetstream::context::RequestError),
    #[error("error while creating NATS stream consumer {0}")]
    NatsConsumer(#[from] async_nats::jetstream::stream::ConsumerError),
    #[error("error while reading from NATS stream consumer {0}")]
    NatsStream(#[from] async_nats::jetstream::consumer::StreamError),
    #[error("error while publishing to NATS event stream {0}")]
    NatsPublish(#[from] async_nats::jetstream::context::PublishError),
    #[error("error while subscribing to a NATS subject {0}")]
    NatsSubscribe(String),
    #[error("could not send a voice command: {0}")]
    VoiceCommand(String),
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
    #[error("the poll is closed")]
    PollClosed,
    #[error("password does not meet requirement {0:?}")]
    PasswordRequirement(crate::api::error::PasswordRequirement),
    #[error("error putting object to media store {0}")]
    S3PutObject(
        #[from] Box<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::put_object::PutObjectError>>,
    ),
    #[error("error deleting object from media store {0}")]
    S3DeleteObject(
        #[from]
        Box<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::delete_object::DeleteObjectError>>,
    ),
    #[error("error inspecting object in media store {0}")]
    S3HeadObject(
        #[from]
        Box<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::head_object::HeadObjectError>>,
    ),
    #[error("error presigning media store request {0}")]
    S3Presign(#[from] aws_sdk_s3::presigning::PresigningConfigError),
    #[error("user not authorized")]
    Unauthorized,
    #[error("tokio join error {0}")]
    TokioJoin(#[from] tokio::task::JoinError),
}

pub type Result<T> = std::result::Result<T, app::Error>;
