//! Aspen Ultra Lite: a whole deployment in one program on one machine. It supervises the bundled
//! PostgreSQL, holds the bus and short-lived store in its own memory, keeps the files in its data
//! directory and serves them at their own origin, and runs the voice server inside itself, on the
//! one port the API is served at, under `/voice-server`.
//!
//! Everything it keeps is in the data directory (`--data`): `aspen-ul.toml`, what the operator
//! says ([`config`]); `postgres/`, the cluster; `media/`, the files; `logs/`; and `run/`, the
//! database's socket.

mod config;
mod postgres;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tracing::{error, info, warn};

/// jemalloc for the whole process (it also replaces `malloc`), which keeps memory from
/// fragmenting across threads and reports what it holds (`aspen_metrics::memory`).
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

/// jemalloc's options, read when it starts. Freed memory goes back to the system on its own
/// schedule through a background thread, so a server gone idle gives back its peak.
#[unsafe(export_name = "malloc_conf")]
pub static MALLOC_CONF: &[u8; 23] = b"background_thread:true\0";

/// Where the voice server's routes are, under the deployment's own address.
const VOICE_PATH: &str = "/voice-server";
/// The name the deployment's one voice server is registered under.
const VOICE_NAME: &str = "aspen-ul";
/// What the bus and short-lived store may hold, each, of the deployment's memory: a sixteenth,
/// between 16 and 256 MiB.
fn in_process_share(memory_mib: u64) -> u64 {
    (memory_mib / 16).clamp(16, 256) * 1024 * 1024
}
/// The stack of each of the runtime's threads, as the API server's (`aspen-chat-server`).
const RUNTIME_THREAD_STACK: usize = 8 << 20;

#[derive(Parser, Debug)]
#[command(name = "aspen-ul", about = "A whole Aspen deployment in one program")]
struct Opt {
    /// The data directory, which holds `aspen-ul.toml` and everything the deployment keeps.
    #[arg(long, default_value = "/var/lib/aspen-ul")]
    data: PathBuf,
    /// The runtime: the bundled PostgreSQL (`pgsql/`) and the web client (`web-client/`).
    #[arg(long)]
    runtime: PathBuf,
    #[command(subcommand)]
    command: Option<UlCommand>,
}

#[derive(clap::Subcommand, Debug)]
enum UlCommand {
    /// Serve the deployment, which is what aspen-ul does given no command.
    Serve,
    /// An operator command of `aspen-chat-server`'s, run against this deployment: through the
    /// running deployment's bus when it runs, so that what it changes reaches everyone connected.
    #[command(flatten)]
    Operator(aspen_chat_server::Command),
}

/// Where the running deployment serves its bus to commands, in the data directory's `run/`.
fn control_socket(data: &Path) -> PathBuf {
    data.join("run").join("control.sock")
}

fn main() {
    let opt = Opt::parse();
    // An operator command's answer is its standard output, so its logs go to standard error.
    let to_stderr = matches!(opt.command, Some(UlCommand::Operator(_)));
    tracing_subscriber::fmt()
        .with_writer(move || -> Box<dyn std::io::Write> {
            if to_stderr {
                Box::new(std::io::stderr())
            } else {
                Box::new(std::io::stdout())
            }
        })
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("ASPEN_LOG")
                .unwrap_or_else(|_| "info,str0m=warn".into()),
        )
        .init();
    std::panic::set_hook(Box::new(tracing_panic::panic_hook));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(RUNTIME_THREAD_STACK)
        .build()
        .expect("a Tokio runtime");
    let mut database = None;
    let ran = runtime.block_on(run(opt, &mut database));
    // Every task of the API and voice servers ends before the database they use does.
    runtime.shutdown_timeout(Duration::from_secs(5));
    let stopped = database.map_or(Ok(()), postgres::Running::stop);
    let code = match ran.and(stopped) {
        Ok(()) => 0,
        Err(e) => {
            error!("{e:#}");
            1
        }
    };
    std::process::exit(code);
}

/// The memory the deployment may use: `memory_mib`, or the machine's, or its cgroup's when less.
fn memory_mib(given: Option<u64>) -> Result<u64> {
    if let Some(given) = given {
        return Ok(given);
    }
    let meminfo =
        std::fs::read_to_string("/proc/meminfo").context("could not read /proc/meminfo")?;
    let total_kib: u64 = meminfo
        .lines()
        .find_map(|line| line.strip_prefix("MemTotal:"))
        .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse().ok())
        .context("/proc/meminfo gives no MemTotal")?;
    let cgroup = std::fs::read_to_string("/sys/fs/cgroup/memory.max")
        .ok()
        .and_then(|max| max.trim().parse::<u64>().ok());
    Ok(cgroup.map_or(total_kib / 1024, |bytes| {
        (bytes / 1024 / 1024).min(total_kib / 1024)
    }))
}

/// TOML's quoted string of `text`.
fn quoted(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Runs the deployment, leaving in `database` the PostgreSQL it started, for `main` to stop once
/// nothing uses it.
async fn run(opt: Opt, database: &mut Option<postgres::Running>) -> Result<()> {
    // SAFETY: geteuid has no preconditions.
    if unsafe { libc::geteuid() } == 0 {
        bail!(
            "aspen-ul does not run as root, and neither does PostgreSQL: run it as a user of its \
             own that owns {}",
            opt.data.display()
        );
    }
    let config_path = opt.data.join("aspen-ul.toml");
    let config = config::Config::read(&config_path)?;
    let memory = memory_mib(config.memory_mib)?;
    let sizes = postgres::Sizes::for_memory(memory);
    let runtime = std::fs::canonicalize(&opt.runtime)
        .with_context(|| format!("the runtime {} is not there", opt.runtime.display()))?;
    let data = std::fs::canonicalize(&opt.data)?;

    let mut postgres = postgres::Postgres::new(&runtime, &data)?;
    match opt.command {
        None | Some(UlCommand::Serve) => {
            info!(memory_mib = memory, pool = sizes.pool, "starting");
            postgres.start(sizes).await?;
            let served = serve(&config, &runtime, &data, &mut postgres, sizes, memory).await;
            *database = postgres.running();
            served
        }
        Some(UlCommand::Operator(command)) => {
            let socket = control_socket(&data);
            match aspen_bus::Remote::connect(&socket).await {
                // The deployment runs, with its database, and hears what the command publishes.
                Ok(remote) => {
                    aspen_app::aspen_config::use_process_bus(aspen_bus::Bus::Remote(remote));
                    let server_config = server_config(&config, &runtime, &data, &postgres, sizes)?;
                    aspen_chat_server::run_command_with(&server_config, command).await
                }
                // It does not: the command starts the database itself, and what it publishes
                // reaches no one, since no one is connected to hear it.
                Err(_) => {
                    postgres.start(sizes).await?;
                    aspen_app::aspen_config::use_process_bus(aspen_bus::Bus::Memory(
                        aspen_bus::Memory::new(None),
                    ));
                    let server_config = server_config(&config, &runtime, &data, &postgres, sizes)?;
                    let ran = aspen_chat_server::run_command_with(&server_config, command).await;
                    *database = postgres.running();
                    ran
                }
            }
        }
    }
}

/// The API server's configuration: what UL derives from `config`, the machine, and where it keeps
/// things, with `[server]` laid over it.
fn server_config(
    config: &config::Config,
    runtime: &Path,
    data: &Path,
    database: &postgres::Postgres,
    sizes: postgres::Sizes,
) -> Result<aspen_app::aspen_config::AspenConfig> {
    let media_url = config.media_url()?;
    // The bus and short-lived store are this process's, so `nats_url`, `nats_auth_token`, and
    // `valkey_url`, which a server reaching NATS and Valkey needs, are given addresses that name
    // nothing (`.invalid` is reserved for that) and are never read.
    let derived = format!(
        "public_url = {public}\n\
         database_url = {database}\n\
         database_pool_size = {pool}\n\
         nats_url = \"nats://in-process.invalid\"\n\
         nats_auth_token = \"in-process\"\n\
         valkey_url = \"redis://in-process.invalid\"\n\
         event_retained_mib = 32\n\
         [web_client]\n\
         dir = {web}\n\
         [media.directory]\n\
         path = {media}\n\
         public_base_url = {media_url}\n\
         [metrics]\n\
         enabled = false\n",
        public = quoted(&config.public_url),
        database = quoted(&database.url(postgres::DATABASE)),
        pool = sizes.pool,
        web = quoted(&runtime.join("web-client").display().to_string()),
        media = quoted(&data.join("media").display().to_string()),
        media_url = quoted(&media_url),
    );
    aspen_app::aspen_config::load_config_sources(vec![
        Box::new(::config::File::from_str(
            &derived,
            ::config::FileFormat::Toml,
        )),
        Box::new(config.server_settings()?),
    ])
    .context("the API server's settings, as aspen-ul.toml gives them, are not right")
}

async fn serve(
    config: &config::Config,
    runtime: &Path,
    data: &Path,
    database: &mut postgres::Postgres,
    sizes: postgres::Sizes,
    memory: u64,
) -> Result<()> {
    {
        let mut conn = aspen_app::database::connect(&database.url(postgres::DATABASE)).await?;
        aspen_migrate::migration::lock(&mut conn).await?;
        aspen_migrate::migration::up::run_up(&mut conn, aspen_migrate::registry::MIGRATIONS)
            .await?;
    }
    let server_config = server_config(config, runtime, data, database, sizes)?;
    let share = in_process_share(memory);
    let bus = aspen_bus::Memory::new(Some(share));
    let backends = aspen_app::context::Backends::InProcess {
        bus: aspen_bus::Bus::Memory(bus.clone()),
        ephemeral: aspen_ephemeral::Ephemeral::Memory(aspen_ephemeral::Memory::new(share as usize)),
    };
    aspen_app::aspen_config::use_process_bus(aspen_bus::Bus::Memory(bus.clone()));
    let (context, serving) =
        aspen_api::start_with(aspen_app::context::Role::Public, server_config, backends).await?;
    let socket = control_socket(data);
    // A socket left by a deployment that stopped without removing it; one answering is a
    // deployment running now, which this must not serve beside.
    if aspen_bus::Remote::connect(&socket).await.is_ok() {
        bail!(
            "another aspen-ul serves this data directory ({}); stop it first",
            data.display()
        );
    }
    let _ = std::fs::remove_file(&socket);
    let control = tokio::net::UnixListener::bind(&socket)
        .with_context(|| format!("could not listen at {}", socket.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
    }
    tokio::spawn(aspen_bus::remote::serve(
        control,
        aspen_bus::Bus::Memory(bus.clone()),
    ));
    let mut serving = serving.context("a public server serves")?;

    let voice_url = format!("{}{VOICE_PATH}", config.public_url.trim_end_matches('/'));
    let voice_id = register_voice_server(&context, &voice_url, config.voice.capacity).await?;
    let voice = start_voice(config, voice_id, &bus).await?;
    serving.router = with_voice(serving.router, voice.router.clone());

    let tls = match &config.tls {
        config::Tls::Off => {
            warn!("tls is off: serving plain HTTP, for a proxy that terminates TLS");
            None
        }
        config::Tls::Files { cert, key } => {
            Some(aspen_chat_server::tls_from_files(key, cert, false)?)
        }
    };
    let addresses: Vec<SocketAddr> = if config.listen.is_empty() {
        &aspen_sockets::EVERY_INTERFACE[..]
    } else {
        &config.listen[..]
    }
    .iter()
    .map(|&ip| SocketAddr::new(ip, config.port))
    .collect();
    let listeners = aspen_sockets::tcp_listeners(&addresses, !config.listen.is_empty())?;
    for listener in &listeners {
        info!(address = %listener.local_addr()?, "listening");
    }
    let (exit_tx, exit_rx) = tokio::sync::oneshot::channel();
    let mut exit_tx = Some(exit_tx);
    ctrlc::set_handler(move || {
        if let Some(exit_tx) = exit_tx.take() {
            info!("stopping");
            let _ = exit_tx.send(());
        }
    })?;
    let served = tokio::select! {
        served = aspen_chat_server::serve(serving, listeners, tls, exit_rx) => served,
        failure = voice.failed() => match failure {
            voice_server::Failure::MediaThread => Err(anyhow::anyhow!("a media thread of the voice server stopped")),
            voice_server::Failure::Unregistered => Err(anyhow::anyhow!("the voice server's registration is gone")),
        },
        status = database.stopped() => Err(anyhow::anyhow!(
            "PostgreSQL stopped of its own accord ({}); its log in {} says why",
            status.map_or_else(|e| e.to_string(), |status| status.to_string()),
            data.join("logs").display()
        )),
    };
    voice.shutdown().await;
    served
}

/// `web`, with the requests under [`VOICE_PATH`] taken by `voice` instead, their paths as the
/// voice server's own: it names its routes by them (`voice_server::signalling::limit_http`), as it
/// would on a machine of its own, so they reach its router with the prefix taken off and matched
/// there alone, not nested beneath another router's match.
fn with_voice(web: axum::Router, voice: axum::Router) -> axum::Router {
    use tower::ServiceExt;
    axum::Router::new().fallback_service(tower::service_fn(
        move |mut request: axum::extract::Request| {
            let (web, voice) = (web.clone(), voice.clone());
            async move {
                let path = request.uri().path();
                let rest = path
                    .strip_prefix(VOICE_PATH)
                    .filter(|rest| rest.is_empty() || rest.starts_with('/'));
                let response = match rest {
                    Some(rest) => {
                        let rest = if rest.is_empty() { "/" } else { rest };
                        let path_and_query = match request.uri().query() {
                            Some(query) => format!("{rest}?{query}"),
                            None => rest.to_string(),
                        };
                        let mut parts = request.uri().clone().into_parts();
                        parts.path_and_query = path_and_query.parse().ok();
                        if let Ok(uri) = axum::http::Uri::from_parts(parts) {
                            *request.uri_mut() = uri;
                        }
                        voice.oneshot(request).await
                    }
                    None => web.oneshot(request).await,
                };
                Ok::<_, std::convert::Infallible>(
                    response.unwrap_or_else(|infallible| match infallible {}),
                )
            }
        },
    ))
}

/// The deployment's one voice server's id, registered at `url` under [`VOICE_NAME`] the first
/// time, and moved to `url` on any start that finds it elsewhere.
async fn register_voice_server(
    context: &aspen_app::context::GlobalServerContext,
    url: &str,
    capacity: i32,
) -> Result<uuid::Uuid> {
    use aspen_app::voice as servers;
    let found = servers::list_servers(context)
        .await?
        .into_iter()
        .find(|server| server.name == VOICE_NAME);
    let server = match found {
        Some(server) if server.url == url && server.capacity == capacity => server,
        Some(server) => {
            servers::update_server(
                context,
                server.id,
                servers::VoiceServerChangeset {
                    name: None,
                    url: Some(url.to_string()),
                    capacity: Some(capacity),
                    enabled: None,
                },
            )
            .await?
        }
        None => {
            servers::create_server(context, VOICE_NAME.to_string(), url.to_string(), capacity)
                .await?
        }
    };
    Ok(server.id.0)
}

async fn start_voice(
    config: &config::Config,
    id: uuid::Uuid,
    bus: &aspen_bus::Memory,
) -> Result<voice_server::VoiceServer> {
    let voice = &config.voice;
    let announced = voice
        .announced_address
        .iter()
        .map(|address| quoted(address))
        .collect::<Vec<_>>()
        .join(", ");
    // The voice server's bus is this process's, so `nats_url` names nothing and is never read.
    let text = format!(
        "id = \"{id}\"\n\
         nats_url = \"nats://in-process.invalid\"\n\
         nats_auth_token = \"in-process\"\n\
         media_threads = 1\n\
         [rtc]\n\
         min_port = {media}\n\
         max_port = {media}\n\
         announced_address = [{announced}]\n\
         [transfer]\n\
         port = {transfer}\n\
         relay_min_port = {relay_min}\n\
         relay_max_port = {relay_max}\n\
         [metrics]\n\
         enabled = false\n",
        media = voice.media_port,
        transfer = voice.transfer_port,
        relay_min = voice.relay_ports.0,
        relay_max = voice.relay_ports.1,
    );
    let voice_config = voice_server::config::load_config_text(&text)
        .context("the voice server's settings, as aspen-ul.toml gives them, are not right")?;
    let link = voice_server::Link::new(aspen_bus::Bus::Memory(
        bus.with_inbox_prefix(voice_protocol::control::inbox_prefix(id)),
    ));
    voice_server::VoiceServer::start(&voice_config, link).await
}
