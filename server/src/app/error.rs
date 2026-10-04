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
    NatsSubscribe(#[from] async_nats::SubscribeError),
    #[error("error while opening a NATS key-value bucket {0}")]
    NatsKeyValue(#[from] async_nats::jetstream::context::CreateKeyValueError),
    #[error("error while listing a NATS key-value bucket's keys {0}")]
    NatsKeys(#[from] async_nats::jetstream::kv::WatchError),
    #[error("error while writing to a NATS key-value bucket {0}")]
    NatsKeyValuePut(#[from] async_nats::jetstream::kv::PutError),
    #[error("error while watching a NATS key-value bucket {0}")]
    NatsKeyValueWatch(#[from] async_nats::jetstream::kv::WatcherError),
    #[error("could not send a voice command: {0}")]
    VoiceCommand(#[source] async_nats::client::PublishError),
    #[error("event published with the wrong scope: {0}")]
    EventRouting(String),
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
    #[error("an invite is required to create an account")]
    RegistrationInviteRequired,
    #[error("the registration invite is not valid")]
    RegistrationInviteInvalid,
    #[error("only the deployment's administrators may do this")]
    AdminRequired,
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
    #[error("error reading object from media store {0}")]
    S3GetObject(
        #[from] Box<aws_sdk_s3::error::SdkError<aws_sdk_s3::operation::get_object::GetObjectError>>,
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
    #[error("forbidden: {0}")]
    Forbidden(Cow<'static, str>),
    /// A block stands between the caller and the person they would message (`app::block`).
    #[error("a block stands between these people")]
    Blocked,
    /// The caller is banned from the community they would join (`app::ban`), with the reason
    /// given to them, if one was.
    #[error("banned from the community")]
    Banned { reason: Option<String> },
    /// The account is banned from the deployment (`app::user_ban`), with the reason given to
    /// them and when the ban ends, if either.
    #[error("banned from the deployment")]
    DeploymentBanned {
        reason: Option<String>,
        until: Option<chrono::DateTime<chrono::Utc>>,
    },
    /// The caller has already reported this, and their report awaits review (`app::report`).
    #[error("already reported")]
    AlreadyReported,
    #[error("the session must verify its user again before changing security settings")]
    ReauthenticationRequired,
    #[error("the password or code presented was wrong")]
    VerificationFailed,
    #[error("conflict: {0}")]
    Conflict(Cow<'static, str>),
    #[error("too many failed attempts")]
    TooManyAttempts,
    #[error("the server requires a second factor, so the last one cannot be removed")]
    LastSecondFactor,
    #[error("passkeys are not configured on this server")]
    PasskeysUnavailable,
    #[error("the passkey was not accepted: {0}")]
    PasskeyRejected(String),
    #[error("the sign-in ticket is unknown, expired, or used")]
    InvalidTicket,
    /// A device link (`app::device_link`) is unknown, expired, already claimed, or not the
    /// caller's.
    #[error("the sign-in code is unknown or expired")]
    DeviceLinkExpired,
    /// A device link was already scanned by another device.
    #[error("the sign-in code was already scanned")]
    DeviceLinkUsed,
    #[error("the request needs a session")]
    Unauthenticated,
    #[error("webauthn error {0}")]
    Webauthn(#[from] webauthn_rs::prelude::WebauthnError),
    #[error("authenticator app secret error {0}")]
    Totp(String),
    #[error("the server is too busy to do this now")]
    Busy,
    /// Another deployment could not be reached, or did not answer as a deployment does; the
    /// reason is localized for the administrator who asked.
    #[error("another deployment could not be reached: {0}")]
    DeploymentUnreachable(Cow<'static, str>),
    /// Federation refuses the crossing; the reason is localized for the person refused.
    #[error("federation refused: {0}")]
    FederationRefused(Cow<'static, str>),
    /// A statement from another deployment (an assertion, a notice) is malformed, forged,
    /// expired, used, or not for this deployment; the reason says which, and what to do.
    #[error("the statement is not valid: {0}")]
    AssertionInvalid(Cow<'static, str>),
    /// This deployment requires two factors, and the sign-in at home proved only a password.
    #[error("a sign-in stronger than a password is required")]
    StrongerSignInRequired,
    #[error("the server's event feed has stopped")]
    EventFeedStopped,
    #[error("tokio join error {0}")]
    TokioJoin(#[from] tokio::task::JoinError),
}

pub type Result<T> = std::result::Result<T, app::Error>;
