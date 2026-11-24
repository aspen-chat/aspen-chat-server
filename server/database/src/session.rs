//! Whether a connection is a PostgreSQL session of its own, or reaches one through a pooler.
//!
//! A pooler (PgBouncer, Supavisor, RDS Proxy, PlanetScale's port 6432) lends its server
//! connections to one client after another, so what a session holds beyond a transaction (a
//! session-level advisory lock, a `SET`, a prepared statement) can land on a server connection
//! the next client is given, and outlives the client that took it. Each pooler answers a client's
//! startup with a cancellation key of its own making, since a cancel request has to come back to
//! it to be routed, while PostgreSQL's key names the backend process serving the session. So a
//! connection is its own session exactly when the process its key names is the one its statements
//! run in.
//!
//! tokio-postgres keeps the key private, and gives it out only as a cancel request written to a
//! stream the caller supplies (`CancelToken::cancel_query_raw`). [`session`] supplies one end of
//! an in-memory pipe, plays the server on the other, and reads the process id out of the request,
//! whose layout the protocol fixes: its length (16), the cancel code, the process id, and the
//! secret. No request reaches the database.

use diesel::sql_types::Integer;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use std::future::{Ready, ready};
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, ReadBuf};
use tokio_postgres::CancelToken;
use tokio_postgres::tls::{ChannelBinding, TlsConnect, TlsStream};

/// The code of the startup message that asks for TLS.
const SSL_REQUEST: i32 = 80_877_103;
/// The code of the startup message that asks for a query to be cancelled.
const CANCEL_REQUEST: i32 = 80_877_102;

/// Who answers a connection's statements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    /// PostgreSQL itself: the session lasts as long as the connection.
    Own,
    /// A pooler, which announced `announced` as the process while `serving` runs the statements.
    Pooled { announced: i32, serving: i32 },
}

/// Why [`session`] could not tell.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("failed to ask which process serves the connection: {0}")]
    Query(#[from] diesel::result::Error),
    #[error("failed to read the connection's cancellation key: {0}")]
    Key(#[from] tokio_postgres::Error),
    #[error("failed to read the connection's cancellation key: {0}")]
    Io(#[from] io::Error),
    #[error("the connection's cancel request is not one the protocol describes: {0}")]
    Malformed(&'static str),
}

/// Tells whether `conn` is a session of PostgreSQL's own (see the module's documentation).
pub async fn session(conn: &mut AsyncPgConnection) -> Result<Session, SessionError> {
    let announced = announced_process(conn.cancel_token()).await?;
    let serving = diesel::select(diesel::dsl::sql::<Integer>("pg_backend_pid()"))
        .get_result::<i32>(conn)
        .await?;
    Ok(if announced == serving {
        Session::Own
    } else {
        Session::Pooled { announced, serving }
    })
}

/// The process id in the cancellation key the server gave `token`'s connection at startup.
async fn announced_process(token: CancelToken) -> Result<i32, SessionError> {
    let (client, server) = tokio::io::duplex(64);
    let (written, read) = tokio::join!(
        token.cancel_query_raw(client, Plain),
        read_cancel_request(server)
    );
    // The reader's error says more than the writer's broken pipe.
    let process = read?;
    written?;
    Ok(process)
}

/// Answers the client's side of the cancel request as a server that offers TLS would, and reads
/// the process id out of it.
async fn read_cancel_request(mut server: DuplexStream) -> Result<i32, SessionError> {
    let (mut length, mut code) = (server.read_i32().await?, server.read_i32().await?);
    if (length, code) == (8, SSL_REQUEST) {
        server.write_all(b"S").await?;
        (length, code) = (server.read_i32().await?, server.read_i32().await?);
    }
    if code != CANCEL_REQUEST {
        return Err(SessionError::Malformed(
            "its code is not a cancel request's",
        ));
    }
    if length < 12 {
        return Err(SessionError::Malformed(
            "it is too short to hold a process id",
        ));
    }
    Ok(server.read_i32().await?)
}

/// What tokio-postgres takes for TLS on the in-memory pipe: the pipe itself, since the request
/// never leaves the process.
struct Plain;

impl TlsConnect<DuplexStream> for Plain {
    type Stream = PlainStream;
    type Error = io::Error;
    type Future = Ready<io::Result<PlainStream>>;

    fn connect(self, stream: DuplexStream) -> Self::Future {
        ready(Ok(PlainStream(stream)))
    }
}

struct PlainStream(DuplexStream);

impl TlsStream for PlainStream {
    fn channel_binding(&self) -> ChannelBinding {
        ChannelBinding::none()
    }
}

impl AsyncRead for PlainStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for PlainStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the client's side writes to the pipe, `ssl` first when it asks for TLS.
    async fn read(ssl: bool, length: i32, code: i32) -> Result<i32, SessionError> {
        let (mut client, server) = tokio::io::duplex(64);
        let reading = tokio::spawn(read_cancel_request(server));
        if ssl {
            client.write_i32(8).await.unwrap();
            client.write_i32(SSL_REQUEST).await.unwrap();
            assert_eq!(client.read_u8().await.unwrap(), b'S');
        }
        for word in [length, code, 4242, -7] {
            client.write_i32(word).await.unwrap();
        }
        drop(client);
        reading.await.unwrap()
    }

    #[tokio::test]
    async fn reads_the_process_with_and_without_asking_for_tls() {
        assert_eq!(read(true, 16, CANCEL_REQUEST).await.unwrap(), 4242);
        assert_eq!(read(false, 16, CANCEL_REQUEST).await.unwrap(), 4242);
    }

    #[tokio::test]
    async fn refuses_what_is_not_a_cancel_request() {
        assert!(matches!(
            read(false, 16, SSL_REQUEST + 1).await,
            Err(SessionError::Malformed(_))
        ));
        assert!(matches!(
            read(true, 8, CANCEL_REQUEST).await,
            Err(SessionError::Malformed(_))
        ));
    }

    /// Runs against the database `DATABASE_URL` names, which must be PostgreSQL itself; without
    /// one it checks nothing.
    #[tokio::test]
    async fn a_direct_connection_is_its_own_session() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        let mut conn = crate::connect(&url).await.unwrap();
        assert_eq!(session(&mut conn).await.unwrap(), Session::Own);
        // The key is read again each time, and the connection still answers afterwards.
        assert_eq!(session(&mut conn).await.unwrap(), Session::Own);
    }
}
