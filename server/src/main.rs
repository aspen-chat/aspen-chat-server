//! The Aspen API server's process: its allocator, command line, logging, and runtime. Without a
//! subcommand it serves the API (`aspen_chat_server::serve`); the subcommands are what an
//! operator runs against a deployment (`aspen_chat_server::run_command`).

use std::{
    io,
    net::{IpAddr, SocketAddr as StdSocketAddr},
    panic,
    path::PathBuf,
    time::Duration,
};

use anyhow::{Result, bail};
use clap::Parser;
use tokio::{runtime, sync::oneshot};
use tracing::{error, info, level_filters::LevelFilter, warn};
use tracing_subscriber::Layer as _;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

use aspen_api as api;
use aspen_app as app;
use aspen_app::aspen_config;
use aspen_chat_server::Command;

/// jemalloc for the whole process (it also replaces `malloc`), which keeps memory from
/// fragmenting across threads and reports what it holds (`aspen_metrics::memory`).
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// jemalloc's options, read when it starts. Freed memory goes back to the system on its own
/// schedule through a background thread; without it, pages are returned only while the program
/// allocates, and a server gone idle after a busy hour keeps its peak resident size.
#[unsafe(export_name = "malloc_conf")]
pub static MALLOC_CONF: &[u8; 23] = b"background_thread:true\0";

#[derive(Parser, Debug)]
#[clap(name = "server")]
struct Opt {
    /// file to log TLS keys to for debugging
    #[clap(long)]
    keylog: bool,
    /// TLS private key in PEM format
    #[clap(short = 'k', long, requires = "cert", conflicts_with = "no_https")]
    key: Option<PathBuf>,
    /// TLS certificate in PEM format
    #[clap(short = 'c', long, requires = "key", conflicts_with = "no_https")]
    cert: Option<PathBuf>,
    /// Address(es) to listen on, can be either IPv4 or IPv6.
    /// Pass multiple times to listen on more than one address. Each one given must bind, and an
    /// IPv6 address takes IPv6 alone. Given none, the server listens on every interface over
    /// IPv4 and IPv6 (0.0.0.0 and ::), leaving out a family this machine does not have.
    #[clap(long)]
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
    /// Run without serving requests: open no listening socket but the health checks' (`[health]
    /// listen_addr`), need no web client, and do only the background work every API server
    /// shares (sending mail and making digests, voice reports, standing checks, plugins'
    /// observers and timers, push). The metrics endpoint is still served when it is on.
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

// tokio-console reads instrumentation Tokio compiles only under this flag.
#[cfg(all(feature = "console", not(tokio_unstable)))]
compile_error!(
    "the console feature needs Tokio's instrumentation: build with RUSTFLAGS=\"--cfg tokio_unstable\""
);

/// The stack of each of the runtime's threads. Tokio gives them 2 MiB, which a debug build
/// overruns: unoptimized, each `async fn` keeps every future it awaits on the stack as well as in
/// its own state, so a chain of polls through the app layer takes hundreds of KiB per frame, and
/// the job runner posting a held message (`jobs::run` down to the query that checks who may post)
/// was measured at 2 MiB on the way into diesel. A thread's stack pages are committed only as it
/// touches them, so what a release build leaves unused costs nothing.
const RUNTIME_THREAD_STACK: usize = 8 << 20;

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
        .thread_stack_size(RUNTIME_THREAD_STACK)
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
        return aspen_chat_server::run_command(command).await;
    }
    let role = if options.private_worker {
        app::context::Role::PrivateWorker
    } else {
        app::context::Role::Public
    };
    let tls_acceptor = if options.private_worker || options.gen_openapi_schema || options.no_https {
        None
    } else {
        let (Some(key), Some(cert)) = (&options.key, &options.cert) else {
            bail!(
                "no certificate: give --cert and --key (scripts/dev_ca.py issues them for \
                 development), or --no-https behind a proxy that terminates TLS"
            );
        };
        Some(aspen_chat_server::tls_from_files(
            key,
            cert,
            options.keylog,
        )?)
    };
    let app = api::start(options.gen_openapi_schema, role).await?;
    let (exit_tx, exit_rx) = oneshot::channel();
    let mut exit_tx = Some(exit_tx);
    ctrlc::set_handler(move || {
        let Some(exit_tx) = exit_tx.take() else {
            return;
        };
        let _ = exit_tx.send(());
        info!("Shutdown signal received, shutting down...");
    })?;
    let Some(serving) = app else {
        info!("running as a private worker: serving nothing, doing the background work");
        let _ = exit_rx.await;
        return Ok(());
    };
    let listen_required = !options.listen_addr.is_empty();
    let listen_addrs: Vec<StdSocketAddr> = if listen_required {
        &options.listen_addr[..]
    } else {
        &aspen_sockets::EVERY_INTERFACE[..]
    }
    .iter()
    .map(|&addr| StdSocketAddr::new(addr, options.port))
    .collect();
    let listeners = aspen_sockets::tcp_listeners(&listen_addrs, listen_required)?;
    for listener in &listeners {
        info!("listening on {}", listener.local_addr()?);
    }
    if options.no_https {
        warn!("--no-https enabled, server is not encrypting anything in transit");
    }
    aspen_chat_server::serve(serving, listeners, tls_acceptor, exit_rx).await
}
