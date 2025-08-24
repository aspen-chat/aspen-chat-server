//! This example demonstrates an HTTP server that serves files from a directory.
//!
//! Checkout the `README.md` for guidance.

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
use clap::Parser;
use futures_util::{StreamExt, stream::FuturesUnordered};
use hyper::{Request, body::Incoming};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server,
};
use rand::SeedableRng as _;
use rand_chacha::ChaCha20Rng;
use rustls::{
    ServerConfig,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};
use tokio::{
    net::TcpListener,
    runtime,
    sync::oneshot,
};
use tokio_rustls::TlsAcceptor;
use tower::Service as _;
use tracing::{error, info, level_filters::LevelFilter, warn};

mod api;
mod aspen_config;
mod database;
mod nats_connection_manager;

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
}

thread_local! {
    pub static CHACHA_RNG: RefCell<ChaCha20Rng> = RefCell::new(ChaCha20Rng::from_os_rng());
}

fn main() {
    if let Err(e) = aspen_config::load_config() {
        eprintln!("failed to load config from aspen.toml or environment. {e}");
        ::std::process::exit(2);
    }
    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::EnvFilter::builder()
                    .with_default_directive(LevelFilter::INFO.into())
                    .with_env_var("ASPEN_LOG")
                    .from_env()
                    .expect("invalid logging filter set in env var ASPEN_LOG"),
            )
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
    let (exit_tx, mut exit_rx) = oneshot::channel();
    let mut exit_tx = Some(exit_tx);
    ctrlc::set_handler(move || {
        let Some(exit_tx) = exit_tx.take() else {
            return;
        };
        let _ = exit_tx.send(());
        info!("Shutdown signal received, shutting down...");
    })?;
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

    let app = api::make_router().await;
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

    loop {
        let mut listeners = FuturesUnordered::from_iter(listeners.iter().map(|l| l.accept()));
        let socket = tokio::select! {
            maybe_socket = listeners.next() => {
                match maybe_socket {
                    Some(Ok((socket, _remote_addr))) => socket,
                    Some(Err(e)) => {
                        error!("TCP I/O error {e}");
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
        let tls_acceptor = tls_acceptor.clone();
        let service = app.clone();
        tokio::spawn(async move {
            let hyper_service = hyper::service::service_fn(move |request: Request<Incoming>| {
                service.clone().call(request)
            });

            /// Using a macro to do compile time duck typing over TlsStream and TcpStream.
            macro_rules! handle_stream {
                ($stream:expr) => {{
                    let socket = TokioIo::new($stream);

                    if let Err(e) = server::conn::auto::Builder::new(TokioExecutor::new())
                        .serve_connection_with_upgrades(socket, hyper_service)
                        .await
                    {
                        error!("failed to serve connection {e}");
                    }
                }};
            }

            match &tls_acceptor {
                Some(tls_acceptor) => match tls_acceptor.accept(socket).await {
                    Ok(tls_stream) => {
                        handle_stream!(tls_stream)
                    }
                    Err(e) => {
                        error!("error establishing TLS {e}");
                        return;
                    }
                },
                None => {
                    // no_https enabled, send unencrypted.
                    handle_stream!(socket)
                }
            }
        });
    }
}
