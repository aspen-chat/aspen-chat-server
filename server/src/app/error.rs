use diesel_async::pooled_connection::deadpool;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("diesel error {0}")]
    Diesel(#[from] diesel::result::Error),
    #[error("deadpool error {0}")]
    Deadpool(#[from] deadpool::PoolError),
    #[error("argon2 password hash error {0}")]
    Argon2(#[from] argon2::password_hash::Error),
    #[error("error while connecting to NATS message broker {0}")]
    NatsConnectError(#[from] async_nats::ConnectError),
    #[error("error serializing as YAML {0}")]
    SerdeNorway(#[from] serde_norway::Error),
    #[error("I/O error {0}")]
    Io(#[from] std::io::Error),
}
