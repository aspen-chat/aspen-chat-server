//! The Aspen API server. Without a subcommand it serves the API; the subcommands are what an
//! operator runs against a deployment (`operator`).

use std::{
    cell::RefCell,
    fs, io,
    net::{IpAddr, Ipv4Addr, SocketAddr as StdSocketAddr},
    panic,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use futures_util::{StreamExt, stream::FuturesUnordered};
use hyper::{Request, body::Incoming};
use hyper_util::{
    rt::{TokioExecutor, TokioIo, TokioTimer},
    server,
};
use rand::SeedableRng as _;
use rand::rngs::SysRng;
use rand_chacha::ChaCha20Rng;
use rust_i18n::i18n;
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use tokio::{net::TcpListener, runtime, sync::oneshot};
use tokio_rustls::TlsAcceptor;
use tower::Service as _;
use tracing::{error, info, level_filters::LevelFilter, warn};
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

mod api;
mod app;
mod aspen_config;
mod connections;
mod database;
mod operator;

/// jemalloc for the whole process (it also replaces `malloc`), which keeps memory from
/// fragmenting across threads and reports what it holds (`aspen_metrics::memory`).
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// jemalloc's options, read when it starts. Freed memory goes back to the system on its own
/// schedule through a background thread; without it, pages are returned only while the program
/// allocates, and a server gone idle after a busy hour keeps its peak resident size.
#[unsafe(export_name = "malloc_conf")]
pub static MALLOC_CONF: &[u8; 23] = b"background_thread:true\0";

/// How often an idle HTTP/2 connection is pinged, and how long its answer may take before the
/// connection is closed.
const HTTP2_PING_INTERVAL: Duration = Duration::from_secs(30);
const HTTP2_PING_TIMEOUT: Duration = Duration::from_secs(20);

/// The first and longest waits before accepting again after accepting failed.
const ACCEPT_BACKOFF_MIN: Duration = Duration::from_millis(10);
const ACCEPT_BACKOFF_MAX: Duration = Duration::from_secs(1);

#[derive(Parser, Debug)]
#[clap(name = "server")]
struct Opt {
    /// file to log TLS keys to for debugging
    #[clap(long)]
    keylog: bool,
    /// TLS private key in PEM format
    #[clap(short = 'k', long, requires = "cert")]
    key: Option<PathBuf>,
    /// TLS certificate in PEM format
    #[clap(short = 'c', long, requires = "key")]
    cert: Option<PathBuf>,
    /// Address(es) to listen on, can be either IPv4 or IPv6.
    /// Pass multiple times to listen on more than one address.
    #[clap(long, default_values_t = [IpAddr::V4(Ipv4Addr::UNSPECIFIED)])]
    listen_addr: Vec<IpAddr>,

    /// The network port to use for all given listen addresses.
    #[clap(long, default_value_t = 443)]
    port: u16,
    /// By default Aspen mandates the use of HTTPS with TLS for all communications.
    /// Aspen won't even issue a redirect over an insecure connection.
    /// --no-https will invert this behavior, instead Aspen will never encrypt communications.
    /// Exposing this mode to production users is wildly insecure and strongly discouraged. However,
    /// if Aspen is behind a reverse proxy or other middleware that provides its own HTTPS you may
    /// find this useful in a production environment.
    #[clap(long)]
    no_https: bool,

    #[clap(long)]
    gen_openapi_schema: bool,
    /// Run without serving anything: open no listening socket, need no web client, and do only
    /// the background work every API server shares (sending mail and making digests, voice
    /// reports, standing checks, plugins' observers and timers, push). The metrics endpoint is
    /// still served when it is on.
    #[clap(long, conflicts_with_all = ["key", "cert", "listen_addr", "port", "no_https", "keylog"])]
    private_worker: bool,
    /// Run a one-off companion migration task and exit.
    /// Current supported values:
    /// - icon-storage-key-backfill-v1
    #[clap(long)]
    run_companion_migration: Option<String>,
    #[clap(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
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
    /// Operator commands for the registry of voice servers.
    VoiceServers {
        #[clap(subcommand)]
        action: operator::VoiceServersCommand,
    },
    /// Operator commands for plugins: installing, configuring, ordering, and removing them.
    Plugins {
        #[clap(subcommand)]
        action: operator::PluginsCommand,
    },
}

thread_local! {
    pub static CHACHA_RNG: RefCell<ChaCha20Rng> = RefCell::new(ChaCha20Rng::try_from_rng(&mut SysRng).expect("failed to initialize system randomness"));
}

i18n!(
    "locales",
    fallback = "en",
    backend = app::locale::PseudoLocales::default()
);

/// `rust_i18n::t!` in the locale of the request being handled (`app::locale`). Every
/// client-facing string goes through this one, never `rust_i18n::t!` directly.
macro_rules! t {
    ($key:expr) => {
        rust_i18n::t!($key, locale = $crate::app::locale::current())
    };
    ($key:expr, $($rest:tt)+) => {
        rust_i18n::t!($key, locale = $crate::app::locale::current(), $($rest)+)
    };
}
pub(crate) use t;

// tokio-console reads instrumentation Tokio compiles only under this flag.
#[cfg(all(feature = "console", not(tokio_unstable)))]
compile_error!(
    "the console feature needs Tokio's instrumentation: build with RUSTFLAGS=\"--cfg tokio_unstable\""
);

fn main() {
    let opt = Opt::parse();
    // Writing the API schemas reads no configuration, so it runs where there is none, as in CI.
    if !opt.gen_openapi_schema
        && let Err(e) = aspen_config::load_config()
    {
        eprintln!("failed to load config from aspen.toml or environment. {e}");
        std::process::exit(2);
    }
    // `init` also routes `log` records into tracing, which is how dependencies that log through
    // the `log` crate (the Valkey client among them) show up under `ASPEN_LOG=...,fred=debug`.
    // An operator command's answer is its standard output, so its logs go to standard error.
    let operator_command = opt.command.is_some();
    let log = tracing_subscriber::fmt::layer()
        .with_writer(move || -> Box<dyn io::Write> {
            if operator_command {
                Box::new(io::stderr())
            } else {
                Box::new(io::stdout())
            }
        })
        .with_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .with_env_var("ASPEN_LOG")
                .from_env()
                .expect("invalid logging filter set in env var ASPEN_LOG"),
        );
    let logging = tracing_subscriber::registry().with(log);
    // tokio-console's server, on 127.0.0.1:6669 unless `TOKIO_CONSOLE_BIND` says otherwise. It
    // reads Tokio's own instrumentation, which `ASPEN_LOG` does not filter.
    #[cfg(feature = "console")]
    let logging = logging.with(console_subscriber::spawn());
    logging.init();
    panic::set_hook(Box::new(tracing_panic::panic_hook));
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
    std::process::exit(code);
}

async fn run(options: Opt) -> Result<()> {
    if let Some(command) = options.command {
        let config = aspen_config::load_config()?;
        return match command {
            Command::Limits { action } => operator::limits(&config, action).await,
            Command::Bench { action } => operator::bench(&config, action).await,
            Command::Admin { action } => operator::admin(&config, action).await,
            Command::Invites { action } => operator::invites(&config, action).await,
            Command::Communities { action } => operator::communities(&config, action).await,
            Command::Federation { action } => operator::federation(&config, action).await,
            Command::Settings { action } => operator::settings(&config, action).await,
            Command::VoiceServers { action } => operator::voice_servers(&config, action).await,
            Command::Plugins { action } => operator::plugins(&config, action).await,
        };
    }
    let role = if options.private_worker {
        app::context::Role::PrivateWorker
    } else {
        app::context::Role::Public
    };
    let app = api::start(options.gen_openapi_schema, role).await?;
    let (exit_tx, mut exit_rx) = oneshot::channel();
    let mut exit_tx = Some(exit_tx);
    ctrlc::set_handler(move || {
        let Some(exit_tx) = exit_tx.take() else {
            return;
        };
        let _ = exit_tx.send(());
        info!("Shutdown signal received, shutting down...");
    })?;
    let Some((app, config)) = app else {
        info!("running as a private worker: serving nothing, doing the background work");
        let _ = exit_rx.await;
        return Ok(());
    };
    // TODO: Hot reload these files when they change. certbot and things like it will update the data periodically. It'd be nice to not require
    // a server reboot to start using the new cert and key.
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
                let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
                let key = PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
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

    let tls_acceptor = (!options.no_https)
        .then(|| -> Result<TlsAcceptor> {
            // Setup TLS config
            let mut server_config = ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(certs, key)?;
            server_config.alpn_protocols =
                vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];
            if options.keylog {
                server_config.key_log = Arc::new(rustls::KeyLogFile::new());
            }
            Ok(TlsAcceptor::from(Arc::new(server_config)))
        })
        .transpose()?;

    let mut listeners = Vec::new();
    for addr in options.listen_addr {
        match TcpListener::bind(StdSocketAddr::new(addr, options.port)).await {
            Ok(listener) => {
                info!("listening on {}", listener.local_addr()?);
                listeners.push(listener);
            }
            Err(e) => warn!("unable to bind listen address {addr} due to {e}"),
        }
    }

    if listeners.is_empty() {
        bail!("all listen addresses failed to bind")
    }

    if options.no_https {
        warn!("--no-https enabled, server is not encrypting anything in transit");
    }

    let limits = &config.connections;
    let addresses = aspen_limits::ClientAddresses::new(
        &config.rate_limits.trusted_proxies,
        config.rate_limits.ipv6_prefix,
    )
    .map_err(anyhow::Error::msg)?;
    let gate = connections::Gate::new(limits, addresses);
    let handshake_timeout = Duration::from_secs(limits.handshake_seconds);
    // Without a timer hyper keeps no time at all: a client could take forever over its headers.
    // HTTP/2 connections are pinged while idle, and closed when a ping goes unanswered.
    let mut http = server::conn::auto::Builder::new(TokioExecutor::new());
    http.http1()
        .timer(TokioTimer::new())
        .header_read_timeout(Duration::from_secs(limits.header_read_seconds));
    http.http2()
        .timer(TokioTimer::new())
        .keep_alive_interval(Some(HTTP2_PING_INTERVAL))
        .keep_alive_timeout(HTTP2_PING_TIMEOUT);
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
        // The socket holds its place within the limits until it closes, through an upgrade to
        // a WebSocket too, which keeps it.
        let socket = connections::Counted::new(socket, admitted);
        let tls_acceptor = tls_acceptor.clone();
        let service = app.clone();
        let http = http.clone();
        tokio::spawn(async move {
            let hyper_service =
                hyper::service::service_fn(move |mut request: Request<Incoming>| {
                    // Rate limits count by the client's address, which starts from the peer's.
                    request
                        .extensions_mut()
                        .insert(api::rate_limit::PeerAddr(remote_addr));
                    service.clone().call(request)
                });

            /// Using a macro to do compile time duck typing over TlsStream and TcpStream.
            macro_rules! handle_stream {
                ($stream:expr) => {{
                    let socket = TokioIo::new($stream);

                    if let Err(e) = http
                        .serve_connection_with_upgrades(socket, hyper_service)
                        .await
                    {
                        error!("failed to serve connection {e}");
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
                        Ok(Err(e)) => {
                            error!("error establishing TLS {e}");
                        }
                        Err(_) => {
                            info!("{remote_addr} took too long over its TLS handshake");
                        }
                    }
                }
                None => {
                    // no_https enabled, send unencrypted.
                    handle_stream!(socket)
                }
            }
        });
    }
}
