//! This example demonstrates an HTTP server that serves files from a directory.
//!
//! Checkout the `README.md` for guidance.

use std::{
    env, fs, io, net::SocketAddr, panic, path::PathBuf, sync::Arc, time::Duration,
};

use anyhow::{Context, Result, bail};
use aspen_protocol::CommunityMailboxManager;
use clap::Parser;
use diesel::{
    PgConnection,
    r2d2::{ConnectionManager, Pool},
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::runtime;
use tracing::{error, info};

mod aspen_protocol;
mod api;
mod database;
mod handle_request;

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
    if options.keylog {
        server_crypto.key_log = Arc::new(rustls::KeyLogFile::new());
    }


    let community_mailbox_manager = CommunityMailboxManager::new();
    let app = api::make_router();

    let listener = tokio::net::TcpListener::bind(options.listen).await?;
    info!("listening on {}", listener.local_addr()?);
    // TODO: Use configured HTTPS certs/keys
    axum::serve(listener, app).await?;

    Ok(())
}
