//! Putting a deployment on a machine: `install` (the user it runs as, its data directory, and the
//! systemd unit that runs it), `init` (its `aspen-ul.toml`), and running every other command as
//! the user that owns the data directory when started as root.

use std::ffi::CString;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// The user `install` makes, and its group.
pub const USER: &str = "aspen-ul";
/// The systemd unit `install` writes.
const UNIT: &str = "/etc/systemd/system/aspen-ul.service";

#[derive(clap::Args, Debug)]
pub struct InstallArgs {
    /// Where the binary is installed.
    #[arg(long, default_value = "/usr/local/bin/aspen-ul")]
    pub bin: PathBuf,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TlsMode {
    /// A certificate from Let's Encrypt, renewed while the deployment runs.
    Acme,
    /// A certificate and key the operator keeps in files (--cert, --key).
    Files,
    /// Plain HTTP, for a proxy on this machine that terminates TLS.
    Off,
}

#[derive(clap::Args, Debug)]
pub struct InitArgs {
    /// The deployment's domain, for good: its accounts' passkeys and other deployments know it by
    /// this name, and it can never change.
    #[arg(long)]
    pub domain: String,
    /// Where Let's Encrypt writes about the certificate.
    #[arg(long)]
    pub email: Option<String>,
    #[arg(long, value_enum, default_value_t = TlsMode::Acme)]
    pub tls: TlsMode,
    #[arg(long, required_if_eq("tls", "files"))]
    pub cert: Option<PathBuf>,
    #[arg(long, required_if_eq("tls", "files"))]
    pub key: Option<PathBuf>,
    /// The port aspen-ul listens at. People reach the deployment at 443 whatever it is: a router
    /// forwarding 443 here, or with `--tls off` the proxy in front, which reaches aspen-ul at this
    /// port on this machine's loopback addresses.
    #[arg(long, default_value_t = 443)]
    pub port: u16,
    /// Where uploaded files are served, `media.<domain>` when not given.
    #[arg(long)]
    pub media_domain: Option<String>,
    /// The address clients send call media to, when it is not this machine's own (behind NAT,
    /// the router's public address).
    #[arg(long)]
    pub announced_address: Option<String>,
    /// Have creating an account take an invite, and print one for the first administrator.
    #[arg(long)]
    pub invite_only: bool,
    /// Take the domain as given, without asking again.
    #[arg(long)]
    pub yes: bool,
}

fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// When started as root for anything but `install`, becomes the user that owns `data`, as the
/// deployment and PostgreSQL run, so `sudo aspen-ul <command>` reads and writes as they do.
/// Called before any thread starts.
pub fn become_owner(data: &Path) -> Result<()> {
    if !is_root() {
        return Ok(());
    }
    use std::os::unix::fs::MetadataExt;
    let owner = std::fs::metadata(data)
        .map(|metadata| (metadata.uid(), metadata.gid()))
        .ok()
        .filter(|(uid, _)| *uid != 0);
    let Some((uid, gid)) = owner else {
        bail!(
            "aspen-ul does not run as root, and neither does PostgreSQL. `aspen-ul install` makes \
             the user {USER} and its data directory, {}, and every command run as root afterwards \
             runs as that directory's owner",
            data.display()
        );
    };
    // SAFETY: each call has no preconditions beyond its arguments, and no other thread runs yet.
    unsafe {
        let groups = [gid];
        if libc::setgroups(1, groups.as_ptr()) != 0
            || libc::setgid(gid) != 0
            || libc::setuid(uid) != 0
        {
            return Err(std::io::Error::last_os_error())
                .context("could not become the data directory's owner");
        }
        let entry = libc::getpwuid(uid);
        if !entry.is_null() {
            let home = std::ffi::CStr::from_ptr((*entry).pw_dir)
                .to_string_lossy()
                .into_owned();
            std::env::set_var("HOME", home);
        }
    }
    Ok(())
}

fn run(program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .status()
        .with_context(|| format!("could not run {program}"))?;
    if !status.success() {
        bail!("{program} {} failed ({status})", args.join(" "));
    }
    Ok(())
}

fn ids_of(user: &str) -> Option<(u32, u32)> {
    let name = CString::new(user).ok()?;
    // SAFETY: the entry is read before any other call into the password database.
    unsafe {
        let entry = libc::getpwnam(name.as_ptr());
        (!entry.is_null()).then(|| ((*entry).pw_uid, (*entry).pw_gid))
    }
}

/// The unit that runs the deployment at `data` with the binary at `bin`: as [`USER`], allowed to
/// listen at ports below 1024 and nothing else root may do, restarted when it fails, and stopped
/// by SIGTERM to UL alone, which stops PostgreSQL itself.
pub fn unit(bin: &Path, data: &Path) -> String {
    format!(
        "[Unit]\n\
         Description=Aspen Ultra Lite\n\
         Documentation=https://github.com/aspen-chat/aspen-chat-server/tree/main/docs/operators\n\
         After=network-online.target\n\
         Wants=network-online.target\n\
         \n\
         [Service]\n\
         User={USER}\n\
         Group={USER}\n\
         ExecStart={bin} --data {data}\n\
         Restart=on-failure\n\
         RestartSec=5\n\
         KillMode=mixed\n\
         TimeoutStopSec=90\n\
         AmbientCapabilities=CAP_NET_BIND_SERVICE\n\
         CapabilityBoundingSet=CAP_NET_BIND_SERVICE\n\
         NoNewPrivileges=true\n\
         LimitNOFILE=1048576\n\
         ProtectSystem=strict\n\
         ReadWritePaths={data}\n\
         ProtectHome=true\n\
         PrivateTmp=true\n\
         PrivateDevices=true\n\
         ProtectKernelTunables=true\n\
         ProtectKernelModules=true\n\
         ProtectControlGroups=true\n\
         RestrictSUIDSGID=true\n\
         LockPersonality=true\n\
         \n\
         [Install]\n\
         WantedBy=multi-user.target\n",
        bin = bin.display(),
        data = data.display(),
    )
}

/// Writes `bytes` to `path` whole, so nothing reading or running it sees part of it.
fn replace(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let partial = path.with_extension("partial");
    std::fs::write(&partial, bytes)
        .with_context(|| format!("could not write {}", partial.display()))?;
    std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(mode))?;
    std::fs::rename(&partial, path)?;
    Ok(())
}

pub fn install(data: &Path, args: &InstallArgs) -> Result<()> {
    if !is_root() {
        bail!("aspen-ul install makes a user and a system service, so it runs as root (sudo)");
    }
    if ids_of(USER).is_none() {
        run(
            "useradd",
            &[
                "--system",
                "--user-group",
                "--home-dir",
                &data.display().to_string(),
                "--no-create-home",
                "--shell",
                "/usr/sbin/nologin",
                USER,
            ],
        )?;
        println!("made the user {USER}");
    }
    let (uid, gid) = ids_of(USER).context("the user useradd made is not there")?;
    std::fs::create_dir_all(data)?;
    std::os::unix::fs::chown(data, Some(uid), Some(gid))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(data, std::fs::Permissions::from_mode(0o750))?;
    }
    let exe = std::env::current_exe()?;
    if std::fs::canonicalize(&exe).ok() != std::fs::canonicalize(&args.bin).ok() {
        let binary =
            std::fs::read(&exe).with_context(|| format!("could not read {}", exe.display()))?;
        if let Some(parent) = args.bin.parent() {
            std::fs::create_dir_all(parent)?;
        }
        replace(&args.bin, &binary, 0o755)?;
        println!("installed {}", args.bin.display());
    }
    let configured = data.join("aspen-ul.toml").exists();
    if Path::new("/run/systemd/system").is_dir() {
        replace(Path::new(UNIT), unit(&args.bin, data).as_bytes(), 0o644)?;
        run("systemctl", &["daemon-reload"])?;
        run("systemctl", &["enable", "aspen-ul"])?;
        println!("wrote {UNIT}, started at boot");
        if configured {
            run("systemctl", &["restart", "aspen-ul"])?;
            println!("restarted aspen-ul");
        }
    } else {
        println!(
            "this machine runs no systemd, so no service was made: run `{} --data {}` as {USER} \
             under the supervisor it has",
            args.bin.display(),
            data.display()
        );
    }
    if !configured {
        println!(
            "next: sudo {} init --domain <the deployment's domain>",
            args.bin.display()
        );
    }
    Ok(())
}

/// The domain as `public_url`'s host, refused unless it is one: lowercase letters, digits, and
/// hyphens in labels, or any script's letters, which URL parsing turns into its ASCII form.
fn domain_of(given: &str) -> Result<String> {
    let url = url::Url::parse(&format!("https://{given}/"))
        .with_context(|| format!("{given:?} is not a domain"))?;
    if let (Some(url::Host::Domain(domain)), "/", None) = (url.host(), url.path(), url.port())
        && domain.contains('.')
    {
        return Ok(domain.to_string());
    }
    bail!("{given:?} is not a domain: give it alone, without a scheme, port, or path")
}

fn toml_string(text: &str) -> String {
    serde_json::to_string(text).expect("a string is JSON")
}

/// `aspen-ul.toml` for `args`.
pub fn config_text(args: &InitArgs) -> Result<String> {
    let domain = domain_of(&args.domain)?;
    let mut text = format!(
        "# The deployment's address, for good: passkeys and other deployments know it by it.\n\
         public_url = {}\n",
        toml_string(&format!("https://{domain}"))
    );
    if let Some(media) = &args.media_domain {
        let media = domain_of(media)?;
        text += &format!("media_url = {}\n", toml_string(&format!("https://{media}")));
    }
    if args.port != 443 {
        text += &format!("port = {}\n", args.port);
    }
    if args.tls == TlsMode::Off {
        text += "# The proxy in front, on this machine, reaches aspen-ul here.\n\
                 listen = [\"127.0.0.1\", \"::1\"]\n";
    }
    text += "\n[tls]\n";
    match args.tls {
        TlsMode::Acme => {
            text += "mode = \"acme\"\n";
            if let Some(email) = &args.email {
                text += &format!("email = {}\n", toml_string(email));
            }
        }
        TlsMode::Files => {
            let (Some(cert), Some(key)) = (&args.cert, &args.key) else {
                bail!("--tls files takes --cert and --key");
            };
            text += &format!(
                "mode = \"files\"\ncert = {}\nkey = {}\n",
                toml_string(&cert.display().to_string()),
                toml_string(&key.display().to_string())
            );
        }
        TlsMode::Off => text += "mode = \"off\"\n",
    }
    if let Some(address) = &args.announced_address {
        text += &format!(
            "\n[voice]\nannounced_address = [{}]\n",
            toml_string(address)
        );
    }
    Ok(text)
}

/// Asks for the domain again, as it can never change; refused without a terminal unless `--yes`.
fn confirm(args: &InitArgs, public_url: &str) -> Result<()> {
    if args.yes {
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        bail!(
            "the deployment's address, {public_url}, can never change: passkeys and other \
             deployments know it by it. Run again with --yes to take it"
        );
    }
    println!(
        "The deployment's address will be {public_url}, for good: its accounts' passkeys and \
         other deployments know it by it, and it can never change."
    );
    print!("Type the domain again to take it: ");
    std::io::stdout().flush()?;
    let mut typed = String::new();
    std::io::stdin().read_line(&mut typed)?;
    if domain_of(typed.trim()).ok() != domain_of(&args.domain).ok() {
        bail!("that is not the same domain; nothing was written");
    }
    Ok(())
}

/// Writes `aspen-ul.toml` in `data`, refusing to replace one. Returns the deployment's address.
pub fn init(data: &Path, args: &InitArgs) -> Result<String> {
    let path = data.join("aspen-ul.toml");
    if path.exists() {
        bail!(
            "{} already describes a deployment; edit it instead",
            path.display()
        );
    }
    let text = config_text(args)?;
    let public_url = text
        .lines()
        .find_map(|line| line.strip_prefix("public_url = "))
        .and_then(|quoted| serde_json::from_str::<String>(quoted).ok())
        .context("the written configuration names no address")?;
    confirm(args, &public_url)?;
    std::fs::create_dir_all(data)?;
    replace(&path, text.as_bytes(), 0o640)?;
    println!("wrote {}", path.display());
    Ok(public_url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        init: InitArgs,
    }

    fn written(args: &[&str]) -> Result<String> {
        let cli = Cli::try_parse_from(std::iter::once("init").chain(args.iter().copied()))?;
        let text = config_text(&cli.init)?;
        let path =
            std::env::temp_dir().join(format!("aspen-ul-init-{}.toml", uuid::Uuid::now_v7()));
        std::fs::write(&path, &text)?;
        let read = crate::config::Config::read(&path);
        std::fs::remove_file(&path)?;
        read?;
        Ok(text)
    }

    #[test]
    fn what_init_writes_is_read_back() {
        let text =
            written(&["--domain", "Chat.Example.org", "--email", "ops@example.org"]).unwrap();
        assert!(
            text.contains("public_url = \"https://chat.example.org\"\n"),
            "{text}"
        );
        assert!(
            text.contains("mode = \"acme\"\nemail = \"ops@example.org\"\n"),
            "{text}"
        );
        let text = written(&[
            "--domain",
            "chat.example.org",
            "--tls",
            "off",
            "--port",
            "8080",
        ])
        .unwrap();
        assert!(
            text.contains("public_url = \"https://chat.example.org\"\n"),
            "{text}"
        );
        assert!(text.contains("port = 8080\n"), "{text}");
        assert!(
            text.contains("listen = [\"127.0.0.1\", \"::1\"]\n"),
            "{text}"
        );
        let text = written(&[
            "--domain",
            "chat.example.org",
            "--media-domain",
            "files.example.org",
            "--announced-address",
            "203.0.113.7",
        ])
        .unwrap();
        assert!(
            text.contains("media_url = \"https://files.example.org\"\n"),
            "{text}"
        );
        assert!(
            text.contains("announced_address = [\"203.0.113.7\"]\n"),
            "{text}"
        );
    }

    #[test]
    fn a_domain_in_any_script_is_taken_in_its_ascii_form() {
        assert_eq!(domain_of("пример.рф").unwrap(), "xn--e1afmkfd.xn--p1ai");
    }

    #[test]
    fn what_is_not_a_domain_is_refused() {
        for given in [
            "https://chat.example.org",
            "chat.example.org/x",
            "chat.example.org:8443",
            "localhost",
            "192.0.2.1",
            "a b.org",
        ] {
            assert!(domain_of(given).is_err(), "{given}");
        }
        assert!(written(&["--domain", "chat.example.org", "--tls", "files"]).is_err());
    }
}
