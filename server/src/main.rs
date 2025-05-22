//! This example demonstrates an HTTP server that serves files from a directory.
//!
//! Checkout the `README.md` for guidance.

use std::{
    cell::RefCell, fs, io, net::SocketAddr, panic, path::PathBuf, sync::Arc, time::Duration,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
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
use tokio::runtime;
use tokio_rustls::TlsAcceptor;
use tower::Service as _;
use tracing::{error, info};

mod api;
mod database;

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
    /// Address to listen on
    #[clap(long = "listen", default_value = "[::1]:4433")]
    listen: SocketAddr,
    /// Maximum number of concurrent connections to allow
    #[clap(long = "connection-limit")]
    connection_limit: Option<usize>,
}

thread_local! {
    pub static CHACHA_RNG: RefCell<ChaCha20Rng> = RefCell::new(ChaCha20Rng::from_os_rng());
}

fn main() {
    let _ = dotenvy::dotenv();
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

    let app = api::make_router();

    let listener = tokio::net::TcpListener::bind(options.listen).await?;

    // Setup TLS config
    let mut server_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    server_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec(), b"http/1.0".to_vec()];
    if options.keylog {
        server_config.key_log = Arc::new(rustls::KeyLogFile::new());
    }
    let tls_acceptor = TlsAcceptor::from(Arc::new(server_config));

    info!("listening on {}", listener.local_addr()?);
    loop {
        let (socket, _remote_addr) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                error!("TCP I/O error {e}");
                continue;
            }
        };
        let tls_acceptor = tls_acceptor.clone();
        let service = app.clone();
        tokio::spawn(async move {
            let tls_stream = match tls_acceptor.accept(socket).await {
                Ok(tls_stream) => tls_stream,
                Err(e) => {
                    error!("error establishing TLS {e}");
                    return;
                }
            };
            let socket = TokioIo::new(tls_stream);

            let hyper_service = hyper::service::service_fn(move |request: Request<Incoming>| {
                service.clone().call(request)
            });

            if let Err(e) = server::conn::auto::Builder::new(TokioExecutor::new())
                .serve_connection_with_upgrades(socket, hyper_service)
                .await
            {
                error!("failed to serve connection {e}");
            }
        });
    }
}
