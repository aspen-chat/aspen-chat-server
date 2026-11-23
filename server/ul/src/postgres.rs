//! The deployment's PostgreSQL: the bundled server, run as a child of this process from the
//! runtime's `pgsql/`, its cluster in the data directory's `postgres/<major>/`, reached over a
//! Unix socket in `run/` alone (it listens on no network address), with peer authentication as
//! the user UL runs as.
//!
//! The cluster is made on first start with every locale category `C.UTF-8` of PostgreSQL's
//! builtin provider, which needs no locale of the machine's and treats every script's letters as
//! letters (`pg_trgm` finds no trigrams in non-Latin text under `C`). What it may hold in memory
//! follows the memory the deployment is given ([`Sizes`]).
//!
//! A cluster lives in `postgres/<major>/`, by the major version of the runtime's PostgreSQL. A
//! runtime of a newer major finds the older cluster and upgrades it with `pg_upgrade` and the
//! previous runtime's binaries, which `bundle` keeps for this, into a new cluster beside it, made
//! under a scratch name and renamed into place only once whole; the old cluster is removed once
//! the new one has started. A cluster newer than the runtime is refused.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::process::{Child, Command};

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
    major: u32,
    data: PathBuf,
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
        let bin = runtime.join("pgsql").join("bin");
        let major = major_of(&bin)?;
        Ok(Self {
            cluster: data.join("postgres").join(major.to_string()),
            bin,
            major,
            data: data.to_path_buf(),
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

    /// A program of the bundled PostgreSQL's, run with no locale of the machine's.
    pub fn tool(&self, program: &str) -> Command {
        let mut command = Command::new(self.bin.join(program));
        // The cluster's locale is its own; the machine's is never read.
        command.env_remove("LANG").env("LC_ALL", "C");
        command
    }

    /// The major version of the runtime's PostgreSQL, and so of its cluster.
    pub fn major(&self) -> u32 {
        self.major
    }

    /// Makes a cluster at `cluster`, as every cluster of a deployment is made.
    async fn initdb(&self, cluster: &Path) -> Result<()> {
        tokio::fs::create_dir_all(cluster).await?;
        let made = self
            .tool("initdb")
            .arg("--pgdata")
            .arg(cluster)
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
                cluster.display(),
                String::from_utf8_lossy(&made.stderr).trim()
            );
        }
        Ok(())
    }

    /// The clusters in `postgres/` other than this runtime's, by major version.
    fn other_clusters(&self) -> Result<Vec<(u32, PathBuf)>> {
        let mut found = Vec::new();
        let Ok(entries) = std::fs::read_dir(self.data.join("postgres")) else {
            return Ok(found);
        };
        for entry in entries {
            let path = entry?.path();
            let Ok(version) = std::fs::read_to_string(path.join("PG_VERSION")) else {
                continue;
            };
            let major: u32 = version.trim().parse().with_context(|| {
                format!("{} names no version", path.join("PG_VERSION").display())
            })?;
            if path != self.cluster {
                found.push((major, path));
            }
        }
        found.sort();
        Ok(found)
    }

    /// Beside an upgraded cluster until its first start: the path of the cluster it was upgraded
    /// from, which that start removes.
    fn upgraded_marker(&self) -> PathBuf {
        self.cluster.with_extension("upgraded-from")
    }

    /// Makes this runtime's cluster when there is none: a new one, or the older cluster upgraded.
    async fn ensure_cluster(&self, sizes: Sizes) -> Result<()> {
        if self.cluster.join("PG_VERSION").exists() {
            return Ok(());
        }
        let others = self.other_clusters()?;
        if let Some((newer, path)) = others.iter().rev().find(|(major, _)| *major > self.major) {
            bail!(
                "the database is PostgreSQL {newer} ({}), newer than this aspen-ul's {}; run the \
                 aspen-ul that last ran it, or a newer one",
                path.display(),
                self.major
            );
        }
        let Some((old_major, old_cluster)) = others.last().cloned() else {
            tracing::info!(cluster = %self.cluster.display(), "making the database cluster");
            self.initdb(&self.cluster).await?;
            return Ok(());
        };
        self.upgrade(old_major, &old_cluster, sizes).await
    }

    /// The `bin/` of a kept runtime whose PostgreSQL is `major`.
    fn runtime_of(&self, major: u32) -> Option<PathBuf> {
        std::fs::read_dir(self.data.join("runtime"))
            .ok()?
            .filter_map(|entry| Some(entry.ok()?.path().join("pgsql").join("bin")))
            .find(|bin| major_of(bin).ok() == Some(major))
    }

    /// Upgrades the cluster of PostgreSQL `old_major` at `old_cluster` into this runtime's.
    async fn upgrade(&self, old_major: u32, old_cluster: &Path, sizes: Sizes) -> Result<()> {
        let old_bin = self.runtime_of(old_major).with_context(|| {
            format!(
                "the database is PostgreSQL {old_major} and this aspen-ul's is {}, and the runtime \
                 of the aspen-ul before it, which upgrading needs, is no longer in {}. Run that \
                 aspen-ul once more, then this one",
                self.major,
                self.data.join("runtime").display()
            )
        })?;
        tracing::info!(
            from = old_major,
            to = self.major,
            "upgrading the database to a new major version of PostgreSQL"
        );
        // pg_upgrade takes only a cluster that was shut down cleanly: one that was not is started
        // and stopped by its own PostgreSQL, which recovers it.
        let mut old = Postgres {
            bin: old_bin.clone(),
            major: old_major,
            data: self.data.clone(),
            cluster: old_cluster.to_path_buf(),
            socket_dir: self.socket_dir.clone(),
            log: self.log.clone(),
            user: self.user.clone(),
            child: None,
        };
        old.launch(sizes).await?;
        if let Some(running) = old.running() {
            running.stop()?;
        }
        let partial = self.cluster.with_extension("partial");
        if partial.exists() {
            tokio::fs::remove_dir_all(&partial).await?;
        }
        self.initdb(&partial).await?;
        let logs = self.log.parent().map(Path::to_path_buf).unwrap_or_default();
        let upgrading = logs.join(format!("pg_upgrade-{old_major}-to-{}", self.major));
        tokio::fs::create_dir_all(&upgrading).await?;
        let output = self
            .tool("pg_upgrade")
            .current_dir(&upgrading)
            .arg("--old-bindir")
            .arg(&old_bin)
            .arg("--new-bindir")
            .arg(&self.bin)
            .arg("--old-datadir")
            .arg(old_cluster)
            .arg("--new-datadir")
            .arg(&partial)
            .arg("--socketdir")
            .arg(&self.socket_dir)
            .args(["--username", &self.user, "--copy"])
            .stdin(Stdio::null())
            .output()
            .await
            .context("could not run the bundled pg_upgrade")?;
        tokio::fs::write(upgrading.join("output.log"), &output.stdout).await?;
        if !output.status.success() {
            bail!(
                "pg_upgrade could not upgrade the database from PostgreSQL {old_major} to {} \
                 ({}); the old cluster, {}, is unchanged, and {} says why",
                self.major,
                output.status,
                old_cluster.display(),
                upgrading.display()
            );
        }
        tokio::fs::write(
            self.upgraded_marker(),
            old_cluster.as_os_str().as_encoded_bytes(),
        )
        .await?;
        tokio::fs::rename(&partial, &self.cluster).await?;
        Ok(())
    }

    /// Starts the server, making its cluster first if needed, and waits until it answers.
    pub async fn start(&mut self, sizes: Sizes) -> Result<()> {
        self.ensure_cluster(sizes).await?;
        self.launch(sizes).await?;
        let marker = self.upgraded_marker();
        if let Ok(old) = tokio::fs::read(&marker).await {
            let old = PathBuf::from(std::ffi::OsString::from(
                String::from_utf8(old).context("the upgrade's marker names no path")?,
            ));
            tracing::info!(old = %old.display(), "the upgraded database started; removing the old one");
            if old.exists() {
                tokio::fs::remove_dir_all(&old).await?;
            }
            tokio::fs::remove_file(&marker).await?;
        }
        self.ensure_database().await
    }

    /// Starts the server on its cluster, which exists, and waits until it answers.
    async fn launch(&mut self, sizes: Sizes) -> Result<()> {
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
            ("huge_pages", "off".to_string()),
        ];
        // The I/O workers, which a small machine needs few of, are named by the version.
        let io_workers: &[(&str, &str)] = if self.major <= 18 {
            &[("io_workers", "1")]
        } else {
            &[("io_min_workers", "1"), ("io_max_workers", "2")]
        };
        let mut command = self.tool("postgres");
        command.arg("-D").arg(&self.cluster);
        for (name, value) in settings
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .chain(io_workers.iter().copied())
        {
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
                .tool("pg_isready")
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
        Ok(())
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

/// The major version of the PostgreSQL whose programs are in `bin`, as `postgres --version` says
/// it (`postgres (PostgreSQL) 18.7`, `19rc2`).
pub fn major_of(bin: &Path) -> Result<u32> {
    let output = std::process::Command::new(bin.join("postgres"))
        .arg("--version")
        .env("LC_ALL", "C")
        .output()
        .with_context(|| format!("could not run {}", bin.join("postgres").display()))?;
    let text = String::from_utf8_lossy(&output.stdout);
    major_in(&text).with_context(|| format!("{} gives no version: {text:?}", bin.display()))
}

fn major_in(version: &str) -> Option<u32> {
    let number = version.split_whitespace().last()?;
    let digits: String = number.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
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
    fn the_major_version_is_read_from_what_postgres_says() {
        assert_eq!(major_in("postgres (PostgreSQL) 18.7\n"), Some(18));
        assert_eq!(major_in("postgres (PostgreSQL) 19rc2\n"), Some(19));
        assert_eq!(major_in("postgres (PostgreSQL) 19beta4"), Some(19));
        assert_eq!(major_in(""), None);
    }

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
