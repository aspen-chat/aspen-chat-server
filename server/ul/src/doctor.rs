//! `aspen-ul doctor`: what stands between this machine and a working deployment, each problem
//! with what to do about it. It reads what it can see from here: the machine, the running
//! deployment, the addresses DNS gives its names, and what the internet is answered with at them.
//! A machine behind a router whose outside address its own requests cannot reach finds those last
//! checks failing from inside the network alone, which it says.

use std::collections::BTreeSet;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::config::{Config, Tls};

/// Services that say the address a request came from, in Cloudflare's `trace` form (`ip=…`).
const ADDRESS_SERVICES: [(Family, &str); 2] = [
    (Family::V4, "https://1.1.1.1/cdn-cgi/trace"),
    (Family::V6, "https://[2606:4700:4700::1111]/cdn-cgi/trace"),
];
const GIB: u64 = 1 << 30;
const TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    V4,
    V6,
}

impl Family {
    fn of(address: &IpAddr) -> Self {
        match address {
            IpAddr::V4(_) => Self::V4,
            IpAddr::V6(_) => Self::V6,
        }
    }

    fn record(self) -> &'static str {
        match self {
            Self::V4 => "A",
            Self::V6 => "AAAA",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::V4 => "IPv4",
            Self::V6 => "IPv6",
        }
    }
}

#[derive(Default)]
struct Report {
    to_fix: usize,
}

impl Report {
    fn section(&self, title: &str) {
        println!("\n{title}");
    }

    fn ok(&self, what: impl AsRef<str>) {
        println!("  ok    {}", what.as_ref());
    }

    fn fix(&mut self, what: impl AsRef<str>, how: impl AsRef<str>) {
        self.to_fix += 1;
        println!("  FIX   {}\n        {}", what.as_ref(), how.as_ref());
    }

    fn note(&self, what: impl AsRef<str>, how: Option<&str>) {
        println!("  note  {}", what.as_ref());
        if let Some(how) = how {
            println!("        {how}");
        }
    }
}

/// The disk `directory` is on and the kind the kernel says it is (`SD`, `MMC`), when it can tell.
fn data_disk(directory: &Path) -> Option<(String, String)> {
    use std::os::unix::fs::MetadataExt;
    let device = std::fs::metadata(directory).ok()?.dev();
    let (major, minor) = (libc::major(device), libc::minor(device));
    let mut block = std::fs::canonicalize(format!("/sys/dev/block/{major}:{minor}")).ok()?;
    if block.join("partition").exists() {
        block = block.parent()?.to_path_buf();
    }
    let kind = std::fs::read_to_string(block.join("device").join("type")).ok()?;
    Some((
        block.file_name()?.to_string_lossy().into_owned(),
        kind.trim().to_string(),
    ))
}

fn free_bytes(directory: &Path) -> Option<u64> {
    let path = std::ffi::CString::new(directory.as_os_str().as_encoded_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` is a valid C string and `stat` is written by the call alone.
    (unsafe { libc::statvfs(path.as_ptr(), &mut stat) } == 0)
        .then(|| stat.f_bavail as u64 * stat.f_frsize as u64)
}

fn check_machine(data: &Path, report: &mut Report) -> Result<()> {
    report.section("The machine");
    let memory = crate::memory_mib(None)?;
    if memory < 1000 {
        report.fix(
            format!("{memory} MiB of memory"),
            "A deployment needs 1 GiB of memory or more: use a larger machine.",
        );
    } else {
        report.ok(format!("{:.1} GiB of memory", memory as f64 / 1024.0));
    }
    match free_bytes(data) {
        Some(free) if free >= 5 * GIB => {
            report.ok(format!("{} GiB free where the data is kept", free / GIB))
        }
        Some(free) => report.note(
            format!("{} GiB free where the data is kept", free / GIB),
            Some("Files, the database, and a backup's staging all grow there; give it room."),
        ),
        None => report.note(
            "Could not tell how much disk is free where the data is kept",
            None,
        ),
    }
    if let Some((disk, kind)) = data_disk(data)
        && kind == "SD"
    {
        report.note(
            format!("The data is on an SD card ({disk})"),
            Some(
                "SD cards wear out under a database and lose writes when power is cut. Keep the \
                 data directory on an SSD, and run `aspen-ul backup` daily.",
            ),
        );
    }
    Ok(())
}

fn client(resolve: Option<(&str, SocketAddr)>) -> Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .redirect(reqwest::redirect::Policy::none());
    if let Some((name, address)) = resolve {
        builder = builder.resolve(name, address);
    }
    Ok(builder.build()?)
}

/// The certificate the deployment holds: its names and when it expires.
fn held_certificate(data: &Path) -> Option<(BTreeSet<String>, time::OffsetDateTime)> {
    let pem = std::fs::read(data.join("acme").join("certificate.pem")).ok()?;
    let leaf = rustls::pki_types::pem::PemObject::pem_slice_iter(&pem)
        .next()?
        .ok()
        .map(|der: rustls::pki_types::CertificateDer<'static>| der)?;
    let (_, parsed) = x509_parser::parse_x509_certificate(&leaf).ok()?;
    let names = parsed
        .subject_alternative_name()
        .ok()??
        .value
        .general_names
        .iter()
        .filter_map(|name| {
            if let x509_parser::extensions::GeneralName::DNSName(name) = name {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect();
    Some((names, parsed.validity().not_after.to_datetime()))
}

async fn check_deployment(config: &Config, data: &Path, report: &mut Report) -> Result<()> {
    report.section("The deployment");
    if aspen_bus::Remote::connect(&crate::control_socket(data))
        .await
        .is_err()
    {
        report.fix(
            "aspen-ul is not running",
            "Start it (sudo systemctl start aspen-ul); `journalctl -u aspen-ul` says why it stopped.",
        );
        return Ok(());
    }
    report.ok("aspen-ul is running");
    let public = url::Url::parse(&config.public_url)?;
    let host = public.host_str().unwrap_or("localhost").to_string();
    let listen = config
        .listen
        .first()
        .copied()
        .unwrap_or(IpAddr::from([127, 0, 0, 1]));
    let local = SocketAddr::new(listen, config.port);
    let scheme = match config.tls {
        Tls::Off => "http",
        Tls::Files { .. } | Tls::Acme { .. } => "https",
    };
    let local_url = format!(
        "{scheme}://{host}:{}{}/health/ready",
        config.port,
        aspen_api::API_PREFIX
    );
    // Asked of this machine, under the deployment's own name, before its certificate is judged:
    // whether it serves at all.
    let answered = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .resolve(&host, local)
        .danger_accept_invalid_certs(true)
        .build()?
        .get(&local_url)
        .send()
        .await;
    match answered {
        Ok(response) if response.status() == reqwest::StatusCode::NO_CONTENT => {
            report.ok(format!("It answers at port {}, and is ready", config.port));
        }
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            report.fix(
                format!(
                    "It answers at port {} that it is not ready ({status})",
                    config.port
                ),
                format!("It says: {}", body.trim()),
            );
        }
        Err(e) => report.fix(
            format!("It does not answer at {local}"),
            format!("{e}. `journalctl -u aspen-ul` says what it is doing."),
        ),
    }
    if let Tls::Acme { .. } = config.tls {
        match held_certificate(data) {
            None => report.fix(
                "It holds no certificate yet",
                "The ACME authority must reach port 443 at each name below for it to be issued; \
                 `journalctl -u aspen-ul` says what the authority answered.",
            ),
            Some((names, expires)) => {
                let wanted = config.names()?;
                let left = (expires - time::OffsetDateTime::now_utc()).whole_days();
                if names != wanted {
                    report.fix(
                        format!("Its certificate names {names:?}, not {wanted:?}"),
                        "It asks for a new one at once; `journalctl -u aspen-ul` says what the \
                         authority answered.",
                    );
                } else if left < 14 {
                    report.fix(
                        format!("Its certificate expires in {left} days"),
                        "It is renewed two thirds through its life, so renewing has been failing: \
                         `journalctl -u aspen-ul` says what the authority answered.",
                    );
                } else {
                    report.ok(format!("Its certificate is good for {left} more days"));
                }
            }
        }
    }
    Ok(())
}

/// This machine's address as the internet sees it, of `family`, when it has one.
async fn public_address(family: Family) -> Option<IpAddr> {
    let url = ADDRESS_SERVICES.iter().find(|(f, _)| *f == family)?.1;
    let text = client(None)
        .ok()?
        .get(url)
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix("ip="))?
        .parse()
        .ok()
        .filter(|address| Family::of(address) == family)
}

/// The address this machine sends from over `family`, which is its public one only when no
/// router stands between.
fn own_address(family: Family) -> Option<IpAddr> {
    let (bind, to): (SocketAddr, SocketAddr) = match family {
        Family::V4 => (([0, 0, 0, 0], 0).into(), ([1, 1, 1, 1], 443).into()),
        Family::V6 => (
            (std::net::Ipv6Addr::UNSPECIFIED, 0).into(),
            (
                "2606:4700:4700::1111".parse::<std::net::Ipv6Addr>().ok()?,
                443,
            )
                .into(),
        ),
    };
    let socket = std::net::UdpSocket::bind(bind).ok()?;
    socket.connect(to).ok()?;
    Some(socket.local_addr().ok()?.ip())
}

fn ports(config: &Config) -> String {
    let voice = &config.voice;
    let mut ports = vec![format!("TCP 443 to {}", config.port)];
    if !matches!(config.tls, Tls::Off) && config.redirect_port != 0 {
        ports.push(format!("TCP 80 to {}", config.redirect_port));
    }
    ports.push(format!("UDP and TCP {}", voice.media_port));
    ports.push(format!("UDP and TCP {}", voice.transfer_port));
    ports.push(format!(
        "UDP {}-{}",
        voice.relay_ports.0, voice.relay_ports.1
    ));
    ports.join(", ")
}

/// Checks the addresses, and returns whether every name points here.
async fn check_network(config: &Config, report: &mut Report) -> Result<bool> {
    report.section("The network");
    let mut public = Vec::new();
    for family in [Family::V4, Family::V6] {
        match public_address(family).await {
            Some(address) => {
                report.ok(format!("{} address {address}", family.name()));
                public.push(address);
            }
            None if family == Family::V4 => report.note(
                "No public IPv4 address found",
                Some("The machine did not reach the internet over IPv4, or the service that tells its address."),
            ),
            None => report.note("No public IPv6 address found", None),
        }
    }
    let behind_router = public
        .iter()
        .any(|address| own_address(Family::of(address)) != Some(*address));
    if behind_router {
        report.note(
            "This machine is behind a router",
            Some(&format!(
                "The router must forward to it: {}. If the router's own outside address is not \
                 the one above, the provider shares addresses between customers and nothing can \
                 be forwarded.",
                ports(config)
            )),
        );
        let announced: Vec<IpAddr> = config
            .voice
            .announced_address
            .iter()
            .filter_map(|address| address.parse().ok())
            .collect();
        let missing: Vec<String> = public
            .iter()
            .filter(|address| !announced.contains(address))
            .map(IpAddr::to_string)
            .collect();
        if config.voice.announced_address.is_empty() || !missing.is_empty() {
            report.fix(
                "Calls are offered at this machine's own address, which the internet cannot reach",
                format!(
                    "In aspen-ul.toml: [voice] announced_address = [{}], then restart aspen-ul.",
                    public
                        .iter()
                        .map(|address| format!("\"{address}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
    }
    let mut every_name_here = true;
    for name in config.names()? {
        let found: BTreeSet<IpAddr> = tokio::net::lookup_host((name.as_str(), 443))
            .await
            .map(|addresses| addresses.map(|address| address.ip()).collect())
            .unwrap_or_default();
        for family in [Family::V4, Family::V6] {
            let of_family: Vec<String> = found
                .iter()
                .filter(|address| Family::of(address) == family)
                .map(IpAddr::to_string)
                .collect();
            let Some(here) = public.iter().find(|address| Family::of(address) == family) else {
                if !of_family.is_empty() {
                    report.fix(
                        format!("{name} has {} {}, and this machine has no {} address", family.record(), of_family.join(", "), family.name()),
                        "Remove that record at your DNS provider: whoever is given it tries an address nothing answers at.",
                    );
                }
                continue;
            };
            if found.contains(here) {
                report.ok(format!("{name} {} {here}", family.record()));
            } else {
                every_name_here = false;
                report.fix(
                    if of_family.is_empty() {
                        format!("{name} has no {} record", family.record())
                    } else {
                        format!("{name} has {} {}", family.record(), of_family.join(", "))
                    },
                    format!(
                        "In your domain's DNS: {name}.  {}  {here}. A record just written can take some minutes to be seen.",
                        family.record()
                    ),
                );
            }
        }
    }
    Ok(every_name_here)
}

async fn check_from_outside(config: &Config, report: &mut Report, names_here: bool) -> Result<()> {
    report.section("From the internet");
    if !names_here {
        report.note("Not checked until the names above point here", None);
        return Ok(());
    }
    let unreachable_hint = "Port 443 must reach this machine: check the router and any firewall. A \
                            router that cannot reach its own outside address makes this fail from \
                            inside the network alone: try from a phone off the Wi-Fi.";
    let browser = client(None)?;
    let health = format!(
        "{}{}/health",
        config.public_url.trim_end_matches('/'),
        aspen_api::API_PREFIX
    );
    match browser.get(&health).send().await {
        Ok(response) if response.status() == reqwest::StatusCode::NO_CONTENT => {
            report.ok(format!(
                "{} answers, with a certificate browsers accept",
                config.public_url
            ));
        }
        Ok(response) => report.fix(
            format!("{health} answered {}", response.status()),
            unreachable_hint,
        ),
        Err(e) => report.fix(format!("{health} did not answer: {e}"), unreachable_hint),
    }
    if !config.files_in_s3() {
        let media = config.media_url()?;
        match browser.get(format!("{media}/")).send().await {
            Ok(response) if response.headers().contains_key("x-content-type-options") => {
                report.ok(format!(
                    "{media} answers, with a certificate browsers accept"
                ));
            }
            Ok(response) => report.fix(
                format!(
                    "{media} answered {} without the files' headers",
                    response.status()
                ),
                "Something other than aspen-ul answers there: its name must point at this machine.",
            ),
            Err(e) => report.fix(format!("{media} did not answer: {e}"), unreachable_hint),
        }
    }
    report.note(
        "Not checked: whether the internet reaches the voice server's media ports",
        Some("Join a call from a phone off the Wi-Fi. If it connects and no one is heard, those ports are not forwarded."),
    );
    if matches!(config.tls, Tls::Off) || config.redirect_port == 0 {
        return Ok(());
    }
    let plain = url::Url::parse(&config.public_url)?;
    let plain = format!("http://{}", plain.host_str().unwrap_or_default());
    match browser.get(format!("{plain}/")).send().await {
        Ok(response)
            if response.status() == reqwest::StatusCode::PERMANENT_REDIRECT
                && response
                    .headers()
                    .get("location")
                    .and_then(|location| location.to_str().ok())
                    .is_some_and(|location| {
                        location.starts_with(config.public_url.trim_end_matches('/'))
                    }) =>
        {
            match browser
                .get(format!("{plain}{}/health", aspen_api::API_PREFIX))
                .send()
                .await
            {
                Ok(response) if response.status() == reqwest::StatusCode::FORBIDDEN => {
                    report.ok("Plain HTTP is sent on to HTTPS, but for the API, which refuses it");
                }
                Ok(response) => report.fix(
                    format!(
                        "{plain}{}/health answered {}, rather than refusing",
                        aspen_api::API_PREFIX,
                        response.status()
                    ),
                    "Something other than aspen-ul answers port 80 at that address.",
                ),
                Err(e) => report.fix(
                    format!("{plain} did not answer the API: {e}"),
                    "Port 80 must reach this machine.",
                ),
            }
        }
        Ok(response) => report.fix(
            format!(
                "{plain} answered {}, rather than sending browsers on to {}",
                response.status(),
                config.public_url
            ),
            "Something other than aspen-ul answers port 80 at that address.",
        ),
        Err(e) => report.fix(
            format!("{plain} did not answer: {e}"),
            "Someone who types the name without https:// gets nothing. Port 80 must reach this \
             machine: check the router and any firewall.",
        ),
    }
    Ok(())
}

pub async fn doctor(config: &Config, data: &Path) -> Result<()> {
    let mut report = Report::default();
    check_machine(data, &mut report)?;
    check_deployment(config, data, &mut report).await?;
    let names_here = check_network(config, &mut report).await?;
    check_from_outside(config, &mut report, names_here).await?;
    println!();
    if report.to_fix > 0 {
        bail!(
            "{} thing{} to fix, above",
            report.to_fix,
            if report.to_fix == 1 { "" } else { "s" }
        );
    }
    println!("Nothing to fix.");
    Ok(())
}
