//! The deployment's certificate from an ACME authority (Let's Encrypt by default), answered by
//! TLS-ALPN-01 on the port the deployment is served at, so nothing but that port needs to reach
//! this machine for it, and renewed while the deployment runs.
//!
//! What it keeps is in the data directory's `acme/`: the account (`account.json`, with the
//! authority it is an account of) and the certificate chain and its key (`certificate.pem`,
//! `key.pem`), each written whole and renamed into place, readable by UL's user alone. A
//! certificate is renewed once two thirds of its lifetime have passed, or at once when it does not
//! name every name the deployment answers.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, NewAccount,
    NewOrder, OrderStatus, RetryPolicy,
};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// The protocol an authority's TLS-ALPN-01 validation connects with (RFC 8737).
pub const ACME_ALPN: &[u8] = b"acme-tls/1";
/// Let's Encrypt's production directory.
pub const LETS_ENCRYPT: &str = "https://acme-v02.api.letsencrypt.org/directory";
/// The longest the renewal task sleeps before looking again, so a clock change or a certificate
/// replaced by hand is noticed within it.
const LOOK_AT_MOST_EVERY: Duration = Duration::from_secs(12 * 60 * 60);
/// How long after a failed issuance it is tried again, doubling to [`RETRY_AT_MOST`]. An
/// authority limits failed validations (Let's Encrypt, five an hour per account and name), so
/// retrying faster gains nothing.
const RETRY_FIRST: Duration = Duration::from_secs(60);
const RETRY_AT_MOST: Duration = Duration::from_secs(6 * 60 * 60);

/// What the deployment's TLS answers with: the certificate it holds, and while an order is open,
/// the authority's challenges.
#[derive(Debug, Default)]
pub struct Certificates {
    live: RwLock<Option<Arc<CertifiedKey>>>,
    challenges: RwLock<HashMap<String, Arc<CertifiedKey>>>,
}

impl ResolvesServerCert for Certificates {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let validating = hello
            .alpn()
            .is_some_and(|mut protocols| protocols.any(|protocol| protocol == ACME_ALPN));
        if validating {
            let name = hello.server_name()?;
            return self
                .challenges
                .read()
                .expect("not poisoned")
                .get(name)
                .cloned();
        }
        self.live.read().expect("not poisoned").clone()
    }
}

/// The TLS the deployment is served with: `certificates`, over HTTP/2 or HTTP/1.1, and the
/// authority's validations over [`ACME_ALPN`].
pub fn acceptor(certificates: Arc<Certificates>) -> tokio_rustls::TlsAcceptor {
    let mut config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_cert_resolver(certificates);
    config.alpn_protocols = vec![
        b"h2".to_vec(),
        b"http/1.1".to_vec(),
        b"http/1.0".to_vec(),
        ACME_ALPN.to_vec(),
    ];
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

/// Where the authority is, and who to tell of the certificate's problems.
#[derive(Debug, Clone)]
pub struct Authority {
    pub directory: String,
    pub email: Option<String>,
    /// Authorities trusted for reaching it besides the system's, for one of the operator's own.
    pub ca_file: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct SavedAccount {
    directory: String,
    credentials: AccountCredentials,
}

/// Keeps the deployment's certificate for `names` in `dir`, issuing and renewing it.
pub struct Acme {
    dir: PathBuf,
    names: BTreeSet<String>,
    authority: Authority,
    certificates: Arc<Certificates>,
}

/// What is known of the certificate held.
struct Held {
    names: BTreeSet<String>,
    renew_after: time::OffsetDateTime,
    expires: time::OffsetDateTime,
}

impl Acme {
    /// Loads the certificate `dir` holds, if any, into `certificates`.
    pub fn new(
        dir: &Path,
        names: BTreeSet<String>,
        authority: Authority,
        certificates: Arc<Certificates>,
    ) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        restrict(dir, 0o700)?;
        let acme = Self {
            dir: dir.to_path_buf(),
            names,
            authority,
            certificates,
        };
        match acme.load() {
            Ok(Some(held)) => info!(
                names = ?held.names,
                expires = %held.expires,
                "serving the certificate held"
            ),
            Ok(None) => info!("no certificate yet; asking for one"),
            Err(e) => {
                warn!("the certificate held could not be read, so another is asked for: {e:#}")
            }
        }
        Ok(acme)
    }

    fn certificate_path(&self) -> PathBuf {
        self.dir.join("certificate.pem")
    }

    fn key_path(&self) -> PathBuf {
        self.dir.join("key.pem")
    }

    /// Reads the certificate held and, when it serves, starts answering with it.
    fn load(&self) -> Result<Option<Held>> {
        let Ok(chain_pem) = std::fs::read(self.certificate_path()) else {
            return Ok(None);
        };
        let key_pem = std::fs::read(self.key_path()).context("the certificate's key is missing")?;
        let (certified, held) = parse(&chain_pem, &key_pem)?;
        *self.certificates.live.write().expect("not poisoned") = Some(certified);
        Ok(Some(held))
    }

    /// Issues and renews for as long as the process runs.
    pub async fn run(self) {
        let mut retry = RETRY_FIRST;
        loop {
            let wait = match self.due() {
                Ok(None) => match self.issue().await {
                    Ok(()) => {
                        retry = RETRY_FIRST;
                        continue;
                    }
                    Err(e) => {
                        warn!(
                            retry_in_seconds = retry.as_secs(),
                            "could not get a certificate for {:?} from {}: {e:#}",
                            self.names,
                            self.authority.directory
                        );
                        let wait = retry;
                        retry = (retry * 2).min(RETRY_AT_MOST);
                        wait
                    }
                },
                Ok(Some(wait)) => wait,
                Err(e) => {
                    warn!("the certificate held could not be read: {e:#}");
                    retry
                }
            };
            tokio::time::sleep(wait.min(LOOK_AT_MOST_EVERY)).await;
        }
    }

    /// How long until the held certificate is to be renewed; `None` when it is now.
    fn due(&self) -> Result<Option<Duration>> {
        let Some(held) = self.load()? else {
            return Ok(None);
        };
        if held.names != self.names {
            info!(held = ?held.names, wanted = ?self.names, "the certificate names other names");
            return Ok(None);
        }
        let left = held.renew_after - time::OffsetDateTime::now_utc();
        Ok(u64::try_from(left.whole_seconds())
            .ok()
            .filter(|seconds| *seconds > 0)
            .map(Duration::from_secs))
    }

    async fn account(&self) -> Result<Account> {
        let builder = || match &self.authority.ca_file {
            Some(ca_file) => Account::builder_with_root(ca_file),
            None => Account::builder(),
        };
        let path = self.dir.join("account.json");
        if let Ok(text) = std::fs::read(&path) {
            let saved: SavedAccount = serde_json::from_slice(&text)
                .with_context(|| format!("{} is not an account", path.display()))?;
            if saved.directory == self.authority.directory {
                return Ok(builder()?.from_credentials(saved.credentials).await?);
            }
            info!(directory = %self.authority.directory, "making an account with another authority");
        }
        let contact = self
            .authority
            .email
            .as_ref()
            .map(|email| format!("mailto:{email}"));
        let contact: Vec<&str> = contact.iter().map(String::as_str).collect();
        let (account, credentials) = builder()?
            .create(
                &NewAccount {
                    contact: &contact,
                    terms_of_service_agreed: true,
                    only_return_existing: false,
                },
                self.authority.directory.clone(),
                None,
            )
            .await?;
        let saved = serde_json::to_vec_pretty(&SavedAccount {
            directory: self.authority.directory.clone(),
            credentials,
        })?;
        write_private(&path, &saved)?;
        Ok(account)
    }

    async fn issue(&self) -> Result<()> {
        info!(names = ?self.names, directory = %self.authority.directory, "asking for a certificate");
        let account = self.account().await?;
        let identifiers: Vec<_> = self.names.iter().cloned().map(Identifier::Dns).collect();
        let mut order = account.new_order(&NewOrder::new(&identifiers)).await?;
        let answered = self.answer_challenges(&mut order).await;
        let finished = match answered {
            Ok(()) => self.finish(&mut order).await,
            Err(e) => Err(e),
        };
        self.certificates
            .challenges
            .write()
            .expect("not poisoned")
            .clear();
        let (chain_pem, key_pem) = finished?;
        let (certified, held) = parse(chain_pem.as_bytes(), key_pem.as_bytes())?;
        write_private(&self.key_path(), key_pem.as_bytes())?;
        write_private(&self.certificate_path(), chain_pem.as_bytes())?;
        *self.certificates.live.write().expect("not poisoned") = Some(certified);
        info!(names = ?held.names, expires = %held.expires, "serving a new certificate");
        Ok(())
    }

    async fn answer_challenges(&self, order: &mut instant_acme::Order) -> Result<()> {
        let mut authorizations = order.authorizations();
        while let Some(authorization) = authorizations.next().await {
            let mut authorization = authorization?;
            match authorization.status {
                AuthorizationStatus::Pending => {}
                AuthorizationStatus::Valid => continue,
                status @ (AuthorizationStatus::Invalid
                | AuthorizationStatus::Revoked
                | AuthorizationStatus::Expired
                | AuthorizationStatus::Deactivated) => {
                    bail!("the authority holds this name's authorization {status:?}")
                }
            }
            let mut challenge = authorization
                .challenge(ChallengeType::TlsAlpn01)
                .context("the authority offers no TLS-ALPN-01 challenge")?;
            let name = challenge.identifier().to_string();
            let digest = challenge.key_authorization().digest();
            let mut params = rcgen::CertificateParams::new(vec![name.clone()])?;
            params.custom_extensions =
                vec![rcgen::CustomExtension::new_acme_identifier(digest.as_ref())];
            let key = rcgen::KeyPair::generate()?;
            let certificate = params.self_signed(&key)?;
            let answer = certified(
                vec![certificate.der().clone()],
                PrivateKeyDer::try_from(key.serialize_der()).map_err(|e| anyhow!(e))?,
            )?;
            self.certificates
                .challenges
                .write()
                .expect("not poisoned")
                .insert(name, answer);
            challenge.set_ready().await?;
        }
        Ok(())
    }

    /// The chain and its key, once the authority validated every name.
    async fn finish(&self, order: &mut instant_acme::Order) -> Result<(String, String)> {
        let status = order.poll_ready(&RetryPolicy::default()).await?;
        if status != OrderStatus::Ready {
            let problems: Vec<String> = order
                .state()
                .error
                .iter()
                .map(|problem| problem.to_string())
                .collect();
            bail!(
                "the authority could not validate every name ({status:?}{}). Each name must \
                 resolve to this machine, and the port the deployment is served at must reach it \
                 from the internet",
                if problems.is_empty() {
                    String::new()
                } else {
                    format!(": {}", problems.join("; "))
                }
            );
        }
        let key_pem = order.finalize().await?;
        let chain_pem = order.poll_certificate(&RetryPolicy::default()).await?;
        Ok((chain_pem, key_pem))
    }
}

fn certified(
    chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<Arc<CertifiedKey>> {
    let signer = rustls::crypto::aws_lc_rs::sign::any_supported_type(&key)?;
    Ok(Arc::new(CertifiedKey::new(chain, signer)))
}

/// The chain and key, ready to serve, and what the leaf says of itself.
fn parse(chain_pem: &[u8], key_pem: &[u8]) -> Result<(Arc<CertifiedKey>, Held)> {
    let chain = CertificateDer::pem_slice_iter(chain_pem).collect::<Result<Vec<_>, _>>()?;
    let leaf = chain
        .first()
        .context("the certificate file holds no certificate")?;
    let (_, parsed) = x509_parser::parse_x509_certificate(leaf)
        .map_err(|e| anyhow!("the certificate cannot be read: {e}"))?;
    let validity = parsed.validity();
    let issued = validity.not_before.to_datetime();
    let expires = validity.not_after.to_datetime();
    let names = parsed
        .subject_alternative_name()
        .map_err(|e| anyhow!("the certificate's names cannot be read: {e}"))?
        .map(|names| {
            names
                .value
                .general_names
                .iter()
                // Only DNS names are ever asked for, so only they are compared.
                .filter_map(|name| {
                    if let x509_parser::extensions::GeneralName::DNSName(name) = name {
                        Some(name.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let key = PrivateKeyDer::from_pem_slice(key_pem)?;
    let certified = certified(chain, key)?;
    Ok((
        certified,
        Held {
            names,
            renew_after: issued + (expires - issued) * 2 / 3,
            expires,
        },
    ))
}

fn restrict(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

/// Writes `bytes` to `path` whole, readable by this user alone.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let partial = path.with_extension("partial");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&partial)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    std::fs::rename(&partial, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn self_signed(names: &[&str]) -> (String, String) {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params =
            rcgen::CertificateParams::new(names.iter().map(|n| n.to_string()).collect::<Vec<_>>())
                .unwrap();
        params.not_before = rcgen::date_time_ymd(2026, 1, 1);
        params.not_after = rcgen::date_time_ymd(2026, 4, 1);
        let certificate = params.self_signed(&key).unwrap();
        (certificate.pem(), key.serialize_pem())
    }

    #[test]
    fn a_certificate_is_renewed_two_thirds_through_its_life() {
        let (chain, key) = self_signed(&["chat.example.org", "media.chat.example.org"]);
        let (_, held) = parse(chain.as_bytes(), key.as_bytes()).unwrap();
        assert_eq!(
            held.names,
            BTreeSet::from(["chat.example.org".into(), "media.chat.example.org".into()])
        );
        // 90 days from 2026-01-01: renewed 60 days in.
        assert_eq!(
            held.renew_after,
            time::macros::datetime!(2026-03-02 00:00 UTC)
        );
        assert_eq!(held.expires, time::macros::datetime!(2026-04-01 00:00 UTC));
    }

    #[test]
    fn a_key_that_is_not_one_is_refused() {
        let (chain, _) = self_signed(&["chat.example.org"]);
        assert!(parse(chain.as_bytes(), b"not a key").is_err());
    }
}
