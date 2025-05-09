//! This example demonstrates an HTTP server that serves files from a directory.
//!
//! Checkout the `README.md` for guidance.

use std::{
    env, error::Error, fs, io, net::SocketAddr, panic, path::PathBuf, sync::Arc, time::Duration,
};

use anyhow::{Context, Result, anyhow, bail};
use aspen_protocol::CommunityMailboxManager;
use clap::Parser;
use diesel::{
    PgConnection,
    r2d2::{ConnectionManager, Pool},
};
use handle_request::{SessionContext, SubscribeCommand};
use quinn_proto::crypto::rustls::QuicServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::{
    runtime,
    sync::{RwLock, mpsc, oneshot},
};
use tokio_stream::{StreamExt as _, StreamMap, wrappers::BroadcastStream};
use tracing::{error, info, info_span};
use tracing_futures::Instrument as _;

use quinn::{Endpoint, ServerConfig, VarInt};

mod aspen_protocol;
mod database;
mod handle_request;

/// Constructs a QUIC endpoint configured to listen for incoming connections on a certain address
/// and port.
///
/// ## Returns
///
/// - a stream of incoming QUIC connections
/// - server certificate serialized into DER format
pub fn make_server_endpoint(
    bind_addr: SocketAddr,
) -> Result<(Endpoint, CertificateDer<'static>), Box<dyn Error + Send + Sync + 'static>> {
    let (server_config, server_cert) = configure_server()?;
    let endpoint = Endpoint::server(server_config, bind_addr)?;
    Ok((endpoint, server_cert))
}

/// Returns default server configuration along with its certificate.
fn configure_server()
-> Result<(ServerConfig, CertificateDer<'static>), Box<dyn Error + Send + Sync + 'static>> {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let cert_der = CertificateDer::from(cert.cert);
    let priv_key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());

    let mut server_config =
        ServerConfig::with_single_cert(vec![cert_der.clone()], priv_key.into())?;
    let transport_config = Arc::get_mut(&mut server_config.transport).unwrap();
    transport_config.max_concurrent_uni_streams(0_u8.into());

    Ok((server_config, cert_der))
}

pub const ALPN_QUIC_HTTP: &[&[u8]] = &[b"hq-29"];

#[derive(Parser, Debug)]
#[clap(name = "server")]
struct Opt {
    /// file to log TLS keys to for debugging
    #[clap(long = "keylog")]
    keylog: bool,
    /// TLS private key in PEM format
    #[clap(short = 'k', long = "key", requires = "cert")]
    key: Option<PathBuf>,
    /// TLS certificate in PEM format
    #[clap(short = 'c', long = "cert", requires = "key")]
    cert: Option<PathBuf>,
    /// Enable stateless retries
    #[clap(long = "stateless-retry")]
    stateless_retry: bool,
    /// Address to listen on
    #[clap(long = "listen", default_value = "[::1]:4433")]
    listen: SocketAddr,
    /// Client address to block
    #[clap(long = "block")]
    block: Option<SocketAddr>,
    /// Maximum number of concurrent connections to allow
    #[clap(long = "connection-limit")]
    connection_limit: Option<usize>,
}

fn main() {
    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .finish(),
    )
    .unwrap();
    panic::set_hook(Box::new(tracing_panic::panic_hook));
    let opt = Opt::parse();
    let runtime = runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("unable to init tokio runtime");
    let code = {
        if let Err(e) = runtime.block_on(run(opt)) {
            error!("ERROR: {e}");
            1
        } else {
            0
        }
    };
    runtime.shutdown_timeout(Duration::from_secs(5));
    ::std::process::exit(code);
}

async fn run(options: Opt) -> Result<()> {
    let _ = dotenvy::dotenv();
    let conn_manager = ConnectionManager::<PgConnection>::new(
        &env::var("DATABASE_URL").expect("DATABASE_URL must be set in environment or .env file"),
    );
    let conn_pool = Pool::builder().build(conn_manager)?;

    let (certs, key) = if let (Some(key_path), Some(cert_path)) = (&options.key, &options.cert) {
        let key = fs::read(key_path).context("failed to read private key")?;
        let key = if key_path.extension().is_some_and(|x| x == "der") {
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key))
        } else {
            rustls_pemfile::private_key(&mut &*key)
                .context("malformed PKCS #1 private key")?
                .ok_or_else(|| anyhow::Error::msg("no private keys found"))?
        };
        let cert_chain = fs::read(cert_path).context("failed to read certificate chain")?;
        let cert_chain = if cert_path.extension().is_some_and(|x| x == "der") {
            vec![CertificateDer::from(cert_chain)]
        } else {
            rustls_pemfile::certs(&mut &*cert_chain)
                .collect::<Result<_, _>>()
                .context("invalid PEM-encoded certificate")?
        };

        (cert_chain, key)
    } else {
        let dirs =
            directories_next::ProjectDirs::from("org", "aspen-chat", "aspen-server").unwrap();
        let path = dirs.data_local_dir();
        let cert_path = path.join("cert.der");
        let key_path = path.join("key.der");
        let (cert, key) = match fs::read(&cert_path).and_then(|x| Ok((x, fs::read(&key_path)?))) {
            Ok((cert, key)) => (
                CertificateDer::from(cert),
                PrivateKeyDer::try_from(key).map_err(anyhow::Error::msg)?,
            ),
            Err(ref e) if e.kind() == io::ErrorKind::NotFound => {
                info!("generating self-signed certificate");
                let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
                let key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
                let cert = cert.cert.into();
                fs::create_dir_all(path).context("failed to create certificate directory")?;
                fs::write(&cert_path, &cert).context("failed to write certificate")?;
                fs::write(&key_path, key.secret_pkcs8_der())
                    .context("failed to write private key")?;
                (cert, key.into())
            }
            Err(e) => {
                bail!("failed to read certificate: {}", e);
            }
        };

        (vec![cert], key)
    };

    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    server_crypto.alpn_protocols = ALPN_QUIC_HTTP.iter().map(|&x| x.into()).collect();
    if options.keylog {
        server_crypto.key_log = Arc::new(rustls::KeyLogFile::new());
    }

    let mut server_config =
        quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(server_crypto)?));
    let transport_config = Arc::get_mut(&mut server_config.transport).unwrap();
    transport_config.max_concurrent_uni_streams(0_u8.into());

    let community_mailbox_manager = CommunityMailboxManager::new();

    let endpoint = quinn::Endpoint::server(server_config, options.listen)?;
    eprintln!("listening on {}", endpoint.local_addr()?);

    while let Some(conn) = endpoint.accept().await {
        if options
            .connection_limit
            .is_some_and(|n| endpoint.open_connections() >= n)
        {
            info!("refusing due to open connection limit");
            conn.refuse();
        } else if Some(conn.remote_address()) == options.block {
            info!("refusing blocked client IP address");
            conn.refuse();
        } else if options.stateless_retry && !conn.remote_address_validated() {
            info!("requiring connection to validate its address");
            conn.retry().unwrap();
        } else {
            info!("accepting connection");
            let (subscribe_cmd_send, subscribe_cmd_receive) = mpsc::channel(16);
            let session_context = Arc::new(RwLock::new(SessionContext::new(
                conn_pool.clone(),
                subscribe_cmd_send,
            )));
            let cmm = community_mailbox_manager.clone();
            tokio::spawn(async move {
                if let Err(e) =
                    handle_connection(conn, cmm, subscribe_cmd_receive, session_context).await
                {
                    error!("connection failed: {reason}", reason = e.to_string())
                }
            });
        }
    }

    Ok(())
}

async fn handle_connection(
    conn: quinn::Incoming,
    community_mailbox_manager: CommunityMailboxManager,
    mut subscribe_cmd_recv: mpsc::Receiver<Vec<SubscribeCommand>>,
    session_context: Arc<RwLock<SessionContext>>,
) -> Result<()> {
    let connection = conn.await?;
    let span = info_span!(
        "connection",
        remote = %connection.remote_address(),
        protocol = %connection
            .handshake_data()
            .unwrap()
            .downcast::<quinn::crypto::rustls::HandshakeData>().unwrap()
            .protocol
            .map_or_else(|| "<none>".into(), |x| String::from_utf8_lossy(&x).into_owned())
    );
    info!("established");
    let (lagged_send, mut lagged_recv) = oneshot::channel();
    // Setup event stream
    tokio::spawn({
        let connection = connection.clone();
        async move {
            let mut subscriptions = StreamMap::new();
            loop {
                tokio::select! {
                    cmds = subscribe_cmd_recv.recv() => {
                        let Some(cmds) = cmds else { break; };
                        for cmd in cmds {
                            if cmd.desire_subscribed {
                                subscriptions.insert(
                                    cmd.community,
                                    BroadcastStream::new(community_mailbox_manager.subscribe_mailbox(&cmd.community))
                                );
                            } else {
                                subscriptions.remove(&cmd.community);
                            }
                        }
                    }
                    event = subscriptions.next() => {
                        let Some((_community, event)) = event else { break; };
                        let Ok(event) = event else { let _ = lagged_send.send(()); break; };
                        match connection.open_uni().await {
                            Ok(mut send) => {
                                let mut to_send = Vec::new();
                                match ciborium::into_writer(&*event, &mut to_send) {
                                    Ok(()) => {
                                        // Send along
                                        if let Err(e) = send.write_all(&to_send).await {
                                            error!("failed to write server event to client {e}"); 
                                        }
                                    }
                                    Err(e) => {
                                        error!("failed to serialize server event {e}");
                                    }
                                }
                            },
                            Err(e) => {
                                error!("sending server event to client failed {e}");
                                break;
                            }
                        }
                    }
                }
            }
        }
    });
    let conn_result = async {
        // Each stream initiated by the client constitutes a new request. We intentionally only
        // process one stream at a time in the hopes that this will make delivery more in order.
        loop {
            tokio::select! {
                stream = connection.accept_bi() => {
                    let stream = match stream {
                        Err(quinn::ConnectionError::ApplicationClosed { .. }) => {
                            info!("connection closed");
                            break Ok(());
                        }
                        Err(e) => {
                            break Err(e.into());
                        }
                        Ok(s) => s,
                    };
                    let fut = handle_request::handle_request(Arc::clone(&session_context), stream);
                    if let Err(e) = fut.await {
                        error!("failed: {reason}", reason = e.to_string());
                    }
                }
                _ = &mut lagged_recv => {
                    break Err(anyhow!("connection not keeping up with server events, disconnecting"));
                }
            }
        }
    }
    .instrument(span)
    .await;
    match &conn_result {
        Ok(()) => connection.close(VarInt::from_u32(0), b""),
        Err(e) => {
            let reason = format!("QUIC error {e}");
            connection.close(VarInt::from_u32(1), reason.as_bytes());
        }
    }
    conn_result.map_err(|e| anyhow!("quinn connection error {e}"))
}
