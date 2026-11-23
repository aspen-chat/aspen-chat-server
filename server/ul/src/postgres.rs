//! The deployment's PostgreSQL: the bundled server, run as a child of this process from the
//! runtime's `pgsql/`, its cluster in the data directory's `postgres/<major>/`, reached over a
//! Unix socket in `run/` alone (it listens on no network address), with peer authentication as
//! the user UL runs as.
//!
//! The cluster is made on first start with every locale category `C.UTF-8` of PostgreSQL's
//! builtin provider, which needs no locale of the machine's and treats every script's letters as
//! letters (`pg_trgm` finds no trigrams in non-Latin text under `C`). What it may hold in memory
//! follows the memory the deployment is given ([`Sizes`]).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::process::{Child, Command};

/// The PostgreSQL major version the runtime holds, whose cluster lives in `postgres/<major>/`.
pub const MAJOR: &str = "18";
/// The database the deployment's rows are in.
pub const DATABASE: &str = "aspen";
/// The longest path a Unix socket may have on Linux, its terminating zero included.
const SOCKET_PATH_MAX: usize = 108;
/// How long PostgreSQL has to start, and to stop once asked.
const START_WITHIN: Duration = Duration::from_secs(120);
const STOP_WITHIN: Duration = Duration::from_secs(60);

/// What the deployment's memory sizes, the database's share of it among them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizes {
    /// The API server's database pool. Each connection grows to 13 to 17 MiB of the database's
    /// memory and keeps it, so the pool is small where memory is.
    pub pool: usize,
    pub shared_buffers_mib: u64,
    pub maintenance_work_mem_mib: u64,
    pub effective_cache_size_mib: u64,
}

impl Sizes {
    pub fn for_memory(memory_mib: u64) -> Self {
        Self {
            pool: if memory_mib < 2048 { 6 } else { 12 },
            shared_buffers_mib: (memory_mib / 8).clamp(64, 512),
            maintenance_work_mem_mib: (memory_mib / 32).max(16),
            effective_cache_size_mib: memory_mib / 2,
        }
    }
}

/// The user this process runs as, which the cluster's one role is named for.
fn os_user() -> Result<String> {
    // SAFETY: getpwuid's answer is read before any other call into the password database.
    unsafe {
        let entry = libc::getpwuid(libc::geteuid());
        if entry.is_null() {
            bail!("this process's user has no entry in the system's users");
        }
        Ok(std::ffi::CStr::from_ptr((*entry).pw_name)
            .to_string_lossy()
            .into_owned())
    }
}

pub struct Postgres {
    bin: PathBuf,
    cluster: PathBuf,
    socket_dir: PathBuf,
    log: PathBuf,
    user: String,
    child: Option<Child>,
}

impl Postgres {
    pub fn new(runtime: &Path, data: &Path) -> Result<Self> {
        let socket_dir = data.join("run");
        let socket = socket_dir.join(".s.PGSQL.5432.lock");
        if socket.as_os_str().len() >= SOCKET_PATH_MAX {
            bail!(
                "the data directory's path is too long for PostgreSQL's socket in {}: keep it \
                 under {} characters",
                socket_dir.display(),
                SOCKET_PATH_MAX - ".s.PGSQL.5432.lock".len() - "/run/".len()
            );
        }
        Ok(Self {
            bin: runtime.join("pgsql").join("bin"),
            cluster: data.join("postgres").join(MAJOR),
            socket_dir,
            log: data.join("logs").join("postgres.log"),
            user: os_user()?,
            child: None,
        })
    }

    /// The database's address, as `aspen.toml`'s `database_url` takes it.
    pub fn url(&self, database: &str) -> String {
        format!(
            "host={} dbname={database} user={} sslmode=disable",
            self.socket_dir.display(),
            self.user
        )
    }

    fn command(&self, program: &str) -> Command {
        let mut command = Command::new(self.bin.join(program));
        // The cluster's locale is its own; the machine's is never read.
        command.env_remove("LANG").env("LC_ALL", "C");
        command
    }

    /// Makes the cluster when there is none.
    async fn ensure_cluster(&self) -> Result<()> {
        if self.cluster.join("PG_VERSION").exists() {
            return Ok(());
        }
        tracing::info!(cluster = %self.cluster.display(), "making the database cluster");
        tokio::fs::create_dir_all(&self.cluster).await?;
        let made = self
            .command("initdb")
            .arg("--pgdata")
            .arg(&self.cluster)
            .args(["--username", &self.user, "--auth=peer", "--encoding=UTF8"])
            .args([
                "--locale-provider=builtin",
                "--locale=C.UTF-8",
                "--no-instructions",
            ])
            .stdin(Stdio::null())
            .output()
            .await
            .context("could not run the bundled initdb")?;
        if !made.status.success() {
            bail!(
                "initdb could not make the cluster in {}: {}",
                self.cluster.display(),
                String::from_utf8_lossy(&made.stderr).trim()
            );
        }
        Ok(())
    }

    /// Starts the server, making its cluster first if needed, and waits until it answers.
    pub async fn start(&mut self, sizes: Sizes) -> Result<()> {
        self.ensure_cluster().await?;
        for directory in [
            &self.socket_dir,
            &self.log.parent().map(Path::to_path_buf).unwrap_or_default(),
        ] {
            tokio::fs::create_dir_all(directory).await?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&self.socket_dir, std::fs::Permissions::from_mode(0o700))
                .await?;
        }
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log)
            .with_context(|| format!("could not open {}", self.log.display()))?;
        let settings = [
            ("listen_addresses", String::new()),
            (
                "unix_socket_directories",
                self.socket_dir.display().to_string(),
            ),
            ("unix_socket_permissions", "0700".to_string()),
            // The pool, a migration's or a command's connection beside it, and the background
            // jobs' own.
            ("max_connections", (sizes.pool + 10).to_string()),
            ("shared_buffers", format!("{}MB", sizes.shared_buffers_mib)),
            (
                "effective_cache_size",
                format!("{}MB", sizes.effective_cache_size_mib),
            ),
            (
                "maintenance_work_mem",
                format!("{}MB", sizes.maintenance_work_mem_mib),
            ),
            ("work_mem", "4MB".to_string()),
            ("autovacuum_max_workers", "2".to_string()),
            ("max_parallel_workers_per_gather", "0".to_string()),
            ("max_wal_size", "512MB".to_string()),
            ("io_workers", "1".to_string()),
            ("huge_pages", "off".to_string()),
        ];
        let mut command = self.command("postgres");
        command.arg("-D").arg(&self.cluster);
        for (name, value) in settings {
            command.arg("-c").arg(format!("{name}={value}"));
        }
        command
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .kill_on_drop(false);
        // Should this process die without stopping it, PostgreSQL is asked to stop at once, as
        // `pg_ctl stop -m fast` asks.
        // SAFETY: prctl is async-signal-safe, and nothing else runs between fork and exec.
        unsafe {
            command.pre_exec(|| {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGINT) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .context("could not start the bundled postgres")?;
        let deadline = tokio::time::Instant::now() + START_WITHIN;
        loop {
            if let Some(status) = child.try_wait()? {
                bail!(
                    "PostgreSQL stopped as it started ({status}); its log, {}, says why",
                    self.log.display()
                );
            }
            let ready = self
                .command("pg_isready")
                .arg("--quiet")
                .arg("--host")
                .arg(&self.socket_dir)
                .args(["--username", &self.user, "--dbname", "postgres"])
                .status()
                .await?;
            if ready.success() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                let _ = child.start_kill();
                bail!(
                    "PostgreSQL did not answer within {} s; its log, {}, says why",
                    START_WITHIN.as_secs(),
                    self.log.display()
                );
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.child = Some(child);
        self.ensure_database().await
    }

    /// Makes the deployment's database when the cluster has none.
    async fn ensure_database(&self) -> Result<()> {
        use diesel_async::{RunQueryDsl, SimpleAsyncConnection};
        #[derive(diesel::QueryableByName)]
        struct Found {
            #[diesel(sql_type = diesel::sql_types::Bool)]
            found: bool,
        }
        let mut conn = aspen_app::database::connect(&self.url("postgres"))
            .await
            .context("could not reach the bundled PostgreSQL")?;
        let Found { found } = diesel::sql_query(
            "SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1) AS found",
        )
        .bind::<diesel::sql_types::Text, _>(DATABASE)
        .get_result(&mut conn)
        .await?;
        if !found {
            conn.batch_execute(&format!("CREATE DATABASE {DATABASE}"))
                .await?;
        }
        Ok(())
    }

    /// The server started, for stopping it after this process's runtime has ended.
    pub fn running(&mut self) -> Option<Running> {
        let child = self.child.take()?;
        // Its exit is waited for by `Running::stop`, not by the runtime's.
        let pid = child.id()?;
        std::mem::forget(child);
        Some(Running {
            pid: pid as libc::pid_t,
        })
    }

    /// Completes if the server stops of its own accord.
    pub async fn stopped(&mut self) -> std::io::Result<std::process::ExitStatus> {
        match self.child.as_mut() {
            Some(child) => child.wait().await,
            None => std::future::pending().await,
        }
    }
}

/// The server, still running once the runtime that started it has ended.
pub struct Running {
    pid: libc::pid_t,
}

impl Running {
    /// Asks the server to stop at once, ending every connection, as `pg_ctl stop -m fast` asks,
    /// and waits for it.
    pub fn stop(self) -> Result<()> {
        // SAFETY: the pid is this process's child, which nothing has waited for.
        unsafe { libc::kill(self.pid, libc::SIGINT) };
        let deadline = std::time::Instant::now() + STOP_WITHIN;
        loop {
            let mut status = 0;
            // SAFETY: as above.
            let waited = unsafe { libc::waitpid(self.pid, &mut status, libc::WNOHANG) };
            if waited == self.pid || waited == -1 {
                return Ok(());
            }
            if std::time::Instant::now() >= deadline {
                // SAFETY: as above.
                unsafe { libc::kill(self.pid, libc::SIGKILL) };
                bail!("PostgreSQL did not stop within {} s", STOP_WITHIN.as_secs());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_machines_get_a_small_pool() {
        assert_eq!(Sizes::for_memory(512).pool, 6);
        assert_eq!(Sizes::for_memory(1024).pool, 6);
        assert_eq!(Sizes::for_memory(2047).pool, 6);
        assert_eq!(Sizes::for_memory(2048).pool, 12);
        assert_eq!(Sizes::for_memory(64 * 1024).pool, 12);
        assert_eq!(Sizes::for_memory(256).shared_buffers_mib, 64);
        assert_eq!(Sizes::for_memory(1024).shared_buffers_mib, 128);
        assert_eq!(Sizes::for_memory(64 * 1024).shared_buffers_mib, 512);
    }
}
