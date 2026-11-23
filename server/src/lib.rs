//! The Aspen API server's listener and operator commands, which the `aspen-chat-server` process
//! runs and another process can run too: [`serve`] serves what `api::start` built on listeners
//! it is given, and [`run_command`] runs an operator command.

use std::{fs, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use clap::Subcommand;
use futures_util::{StreamExt, stream::FuturesUnordered};
use hyper::{Request, body::Incoming};
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server,
};
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer},
};
use tokio::sync::oneshot;
use tokio_rustls::TlsAcceptor;
use tower::Service as _;
use tracing::{debug, error, info, warn};

use aspen_api as api;
use aspen_app as app;
use aspen_app::aspen_config;
pub mod connections;
pub mod operator;

/// How often an idle HTTP/2 connection is pinged, and how long its answer may take before the
/// connection is closed.
const HTTP2_PING_INTERVAL: Duration = Duration::from_secs(30);
const HTTP2_PING_TIMEOUT: Duration = Duration::from_secs(20);

/// The first and longest waits before accepting again after accepting failed.
const ACCEPT_BACKOFF_MIN: Duration = Duration::from_millis(10);
const ACCEPT_BACKOFF_MAX: Duration = Duration::from_secs(1);

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Operator commands for rate limits.
    Limits {
        #[clap(subcommand)]
        action: operator::LimitsCommand,
    },
    /// Operator commands for benchmark populations.
    Bench {
        #[clap(subcommand)]
        action: operator::BenchCommand,
    },
    /// Operator commands for the files kept of deleted messages for reviewing reports.
    Attachments {
        #[clap(subcommand)]
        action: operator::AttachmentsCommand,
    },
    /// Operator commands for deployment roles: the first administrator, and the top role.
    Admin {
        #[clap(subcommand)]
        action: operator::AdminCommand,
    },
    /// Operator commands for communities.
    Communities {
        #[clap(subcommand)]
        action: operator::CommunitiesCommand,
    },
    /// Operator commands for background jobs.
    Jobs {
        #[clap(subcommand)]
        action: operator::JobsCommand,
    },
    /// Operator commands for the deployment's Privacy Policy and Terms of Use.
    Documents {
        #[clap(subcommand)]
        action: operator::DocumentsCommand,
    },
    /// Operator commands for registration invites.
    Invites {
        #[clap(subcommand)]
        action: operator::InvitesCommand,
    },
    /// Operator commands for federation: the directory of other deployments, and this
    /// deployment's key.
    Federation {
        #[clap(subcommand)]
        action: operator::FederationCommand,
    },
    /// Operator commands for the deployment's settings: its name, policies, and federation
    /// gates.
    Settings {
        #[clap(subcommand)]
        action: operator::SettingsCommand,
    },
    /// Operator commands for the deployment's file storage: importing files kept in a directory.
    Storage {
        #[clap(subcommand)]
        action: operator::StorageCommand,
    },
    /// Operator commands for the registry of voice servers.
    VoiceServers {
        #[clap(subcommand)]
        action: operator::VoiceServersCommand,
    },
    /// Operator commands for the OpenID Connect providers people sign in through.
    IdentityProviders {
        #[clap(subcommand)]
        action: operator::IdentityProvidersCommand,
    },
    /// Operator commands for plugins: installing, configuring, ordering, and removing them.
    Plugins {
        #[clap(subcommand)]
        action: operator::PluginsCommand,
    },
}

/// Whether a key or certificate file is PEM, which is told by its contents: PEM marks what it
/// holds with a line that begins so, and DER is the bare binary structure.
fn is_pem(file: &[u8]) -> bool {
    const BOUNDARY: &[u8] = b"-----BEGIN";
    file.windows(BOUNDARY.len())
        .any(|window| window == BOUNDARY)
}

/// The private key a `--key` file holds, PEM or DER, as PKCS #8, PKCS #1, or SEC1.
fn private_key(file: Vec<u8>) -> Result<PrivateKeyDer<'static>> {
    if is_pem(&file) {
        rustls_pemfile::private_key(&mut &*file)
            .context("malformed PEM-encoded private key")?
            .context("the PEM file holds no private key")
    } else {
        PrivateKeyDer::try_from(file).map_err(|error| {
            anyhow::anyhow!("the private key is neither PEM nor a DER-encoded key: {error}")
        })
    }
}

/// The certificates a `--cert` file holds: every one of a PEM file, or the one a DER file is.
fn certificate_chain(file: Vec<u8>) -> Result<Vec<CertificateDer<'static>>> {
    if is_pem(&file) {
        rustls_pemfile::certs(&mut &*file)
            .collect::<Result<_, _>>()
            .context("invalid PEM-encoded certificate")
    } else {
        Ok(vec![CertificateDer::from(file)])
    }
}

/// TLS answered with the certificate chain and private key in the files at `cert` and `key`, PEM
/// or DER, offering HTTP/2 and HTTP/1.1, and logging its keys for debugging with `keylog`.
pub fn tls_from_files(key: &Path, cert: &Path, keylog: bool) -> Result<TlsAcceptor> {
    // TODO: Hot reload these files when they change. certbot and things like it will update the data periodically. It'd be nice to not require
    // a server reboot to start using the new cert and key.
    let key = private_key(fs::read(key).context("failed to read private key")?)?;
    let cert_chain =
        certificate_chain(fs::read(cert).context("failed to read certificate chain")?)?;
    let mut server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)?;
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];
    if keylog {
        server_config.key_log = Arc::new(rustls::KeyLogFile::new());
    }
    Ok(TlsAcceptor::from(Arc::new(server_config)))
}
/// Runs an operator command against the deployment `aspen.toml` and the environment name, then
/// wakes the servers' job runners for any job it saved.
pub async fn run_command(command: Command) -> Result<()> {
    run_command_with(&aspen_config::load_config()?, command).await
}

/// As [`run_command`], against the deployment `config` names.
pub async fn run_command_with(config: &aspen_config::AspenConfig, command: Command) -> Result<()> {
    let ran = match command {
        Command::Limits { action } => operator::limits(config, action).await,
        Command::Bench { action } => operator::bench(config, action).await,
        Command::Admin { action } => operator::admin(config, action).await,
        Command::Attachments { action } => operator::attachments(config, action).await,
        Command::Invites { action } => operator::invites(config, action).await,
        Command::Documents { action } => operator::documents(config, action).await,
        Command::Communities { action } => operator::communities(config, action).await,
        Command::Jobs { action } => operator::jobs(config, action).await,
        Command::Federation { action } => operator::federation(config, action).await,
        Command::Settings { action } => operator::settings(config, action).await,
        Command::Storage { action } => operator::storage(config, action).await,
        Command::VoiceServers { action } => operator::voice_servers(config, action).await,
        Command::IdentityProviders { action } => operator::identity_providers(config, action).await,
        Command::Plugins { action } => operator::plugins(config, action).await,
    };
    // What the command decided may be a job (a plugin's purge, signing out a blocked
    // deployment's users), which the servers start at once rather than at their next look.
    if ran.is_ok() {
        app::jobs::wake_from(config).await;
    }
    ran
}

/// Serves `serving` on `listeners`, over `tls` when given, until `exit` is told: each connection
/// within the limits of `[connections]`, closed when idle, and asked to finish when it lasts.
pub async fn serve(
    serving: api::Serving,
    listeners: Vec<tokio::net::TcpListener>,
    tls_acceptor: Option<TlsAcceptor>,
    mut exit_rx: oneshot::Receiver<()>,
) -> Result<()> {
    let api::Serving {
        router: app,
        config,
        suspension,
    } = serving;
    let limits = &config.connections;
    let addresses = aspen_limits::ClientAddresses::new(
        &config.rate_limits.trusted_proxies,
        config.rate_limits.ipv6_prefix,
    )
    .map_err(anyhow::Error::msg)?;
    let gate = connections::Gate::new(limits, addresses, suspension);
    let handshake_timeout = Duration::from_secs(limits.handshake_seconds);
    let idle_timeout = Duration::from_secs(limits.idle_seconds);
    let shutdown_timeout = Duration::from_secs(limits.shutdown_seconds);
    // Without a timer hyper keeps no time at all: a client could take forever over its headers.
    // HTTP/2 connections are pinged while idle, and closed when a ping goes unanswered.
    let mut http = server::conn::auto::Builder::new(TokioExecutor::new());
    http.http1()
        .timer(TokioTimer::new())
        .header_read_timeout(Duration::from_secs(limits.header_read_seconds));
    http.http2()
        .timer(TokioTimer::new())
        .keep_alive_interval(Some(HTTP2_PING_INTERVAL))
        .keep_alive_timeout(HTTP2_PING_TIMEOUT)
        .max_concurrent_streams(limits.max_streams);
    let http = Arc::new(http);
    let mut accept_backoff = ACCEPT_BACKOFF_MIN;

    loop {
        let mut listeners = FuturesUnordered::from_iter(listeners.iter().map(|l| l.accept()));
        let (socket, remote_addr) = tokio::select! {
            maybe_socket = listeners.next() => {
                match maybe_socket {
                    Some(Ok((socket, remote_addr))) => {
                        accept_backoff = ACCEPT_BACKOFF_MIN;
                        (socket, remote_addr)
                    }
                    Some(Err(e)) => {
                        // Out of file descriptors or memory, accepting again at once fails
                        // again at once; waiting lets connections close first.
                        error!("TCP I/O error {e}");
                        tokio::time::sleep(accept_backoff).await;
                        accept_backoff = (accept_backoff * 2).min(ACCEPT_BACKOFF_MAX);
                        continue;
                    }
                    None => {
                        unreachable!("listeners is not empty, and exactly one value is pulled from it.")
                    }
                }
            },
            _ = &mut exit_rx => {
                break Ok(());
            }
        };
        // Closed at once, by dropping it, when over a limit.
        let Some(admitted) = gate.admit(remote_addr.ip()) else {
            continue;
        };
        // Event frames are small and each is flushed as it is written; without this, one written
        // while the previous is still unacknowledged waits for the client's delayed ACK.
        if let Err(e) = socket.set_nodelay(true) {
            warn!("could not turn off Nagle's algorithm for {remote_addr}: {e}");
        }
        let place = api::rate_limit::Connection(admitted.place());
        // The socket holds its place within the limits until it closes, through an upgrade to
        // a WebSocket too, which keeps it.
        let socket = connections::Counted::new(socket, admitted);
        let tls_acceptor = tls_acceptor.clone();
        let service = app.clone();
        let http = http.clone();
        tokio::spawn(async move {
            let activity = connections::Activity::new();
            let hyper_service = {
                let activity = activity.clone();
                hyper::service::service_fn(move |mut request: Request<Incoming>| {
                    // Rate limits count by the client's address, which starts from the peer's.
                    request
                        .extensions_mut()
                        .insert(api::rate_limit::PeerAddr(remote_addr));
                    request.extensions_mut().insert(place.clone());
                    // What a router merged beside the API's reads its peer by, as axum serves it.
                    request
                        .extensions_mut()
                        .insert(axum::extract::ConnectInfo(remote_addr));
                    let busy = activity.begin();
                    let response = service.clone().call(request);
                    async move {
                        response.await.map(|r| {
                            let progress = busy.responded();
                            r.map(|body| connections::Tracked::new(body, progress))
                        })
                    }
                })
            };

            /// Using a macro to do compile time duck typing over TlsStream and TcpStream.
            macro_rules! handle_stream {
                ($stream:expr) => {{
                    let socket = TokioIo::new($stream);
                    let connection = http.serve_connection_with_upgrades(socket, hyper_service);
                    tokio::pin!(connection);
                    // A connection with nothing happening on it for the idle time is asked to
                    // close: HTTP/2 sends GOAWAY and finishes what is in flight, for at most
                    // the shutdown time, after which it is dropped with whatever is left.
                    let served = tokio::select! {
                        served = connection.as_mut() => served,
                        () = activity.idle(idle_timeout) => {
                            connection.as_mut().graceful_shutdown();
                            match tokio::time::timeout(shutdown_timeout, connection).await {
                                Ok(served) => served,
                                Err(_) => {
                                    debug!("{remote_addr} did not finish closing in time");
                                    Ok(())
                                }
                            }
                        }
                    };
                    match served {
                        Ok(()) => {}
                        Err(e) if api::disconnect::is_disconnect(&*e) => {
                            debug!("{remote_addr} disconnected: {e}");
                        }
                        Err(e) => error!("failed to serve connection {e}"),
                    }
                }};
            }

            match &tls_acceptor {
                Some(tls_acceptor) => {
                    match tokio::time::timeout(handshake_timeout, tls_acceptor.accept(socket)).await
                    {
                        Ok(Ok(tls_stream)) => {
                            handle_stream!(tls_stream)
                        }
                        Ok(Err(e)) if api::disconnect::is_disconnect(&e) => {
                            debug!("{remote_addr} disconnected during its TLS handshake: {e}");
                        }
                        Ok(Err(e)) => {
                            error!("error establishing TLS {e}");
                        }
                        Err(_) => {
                            info!("{remote_addr} took too long over its TLS handshake");
                        }
                    }
                }
                None => {
                    handle_stream!(socket)
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An Ed25519 key as PKCS #8: the algorithm, then the key's 32 bytes.
    fn pkcs8() -> Vec<u8> {
        let mut der = vec![
            0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22,
            0x04, 0x20,
        ];
        der.extend(0..32);
        der
    }

    /// A P-256 key as SEC1: the key's 32 bytes, then the curve.
    fn sec1() -> Vec<u8> {
        let mut der = vec![0x30, 0x31, 0x02, 0x01, 0x01, 0x04, 0x20];
        der.extend(0..32);
        der.extend([
            0xa0, 0x0a, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07,
        ]);
        der
    }

    #[test]
    fn a_key_file_is_read_by_what_it_holds() {
        assert!(matches!(
            private_key(pkcs8()).unwrap(),
            PrivateKeyDer::Pkcs8(_)
        ));
        assert!(matches!(
            private_key(sec1()).unwrap(),
            PrivateKeyDer::Sec1(_)
        ));
        let pem = "comment\n-----BEGIN PRIVATE KEY-----\n\
                   MC4CAQAwBQYDK2VwBCIEIAABAgMEBQYHCAkKCwwNDg8QERITFBUWFxgZGhscHR4f\n\
                   -----END PRIVATE KEY-----\n";
        match private_key(pem.as_bytes().to_vec()).unwrap() {
            PrivateKeyDer::Pkcs8(key) => assert_eq!(key.secret_pkcs8_der(), pkcs8()),
            other => panic!("{other:?}"),
        }
        assert!(private_key(b"not a key".to_vec()).is_err());
        assert!(
            private_key(b"-----BEGIN CERTIFICATE-----\n-----END CERTIFICATE-----\n".to_vec())
                .is_err()
        );
    }

    #[test]
    fn a_certificate_file_is_read_by_what_it_holds() {
        let der = vec![0x30, 0x03, 0x02, 0x01, 0x00];
        assert_eq!(
            certificate_chain(der.clone()).unwrap(),
            [CertificateDer::from(der)]
        );
        let pem = "-----BEGIN CERTIFICATE-----\nMAMCAQA=\n-----END CERTIFICATE-----\n".repeat(2);
        assert_eq!(certificate_chain(pem.into_bytes()).unwrap().len(), 2);
    }
}
