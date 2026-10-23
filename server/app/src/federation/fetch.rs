//! Calls to other deployments: HTTPS only, no redirects, a short timeout, a capped body, and
//! only public addresses (`app::outbound`) unless `[federation.development]` says otherwise.

use crate::aspen_config::FederationConfig;
use crate::federation::{DeploymentDocument, Domain};
use crate::outbound::PublicResolver;
use crate::t;
use futures_util::StreamExt;
use std::time::Duration;

/// How long one call to another deployment may take, connecting included.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The largest document read from another deployment.
const MAX_DOCUMENT_BYTES: usize = 64 * 1024;

/// The client every call to another deployment is made with, built once per process from
/// `[federation]`. It fails only on an unreadable development certificate.
pub fn client(config: &FederationConfig) -> crate::Result<reqwest::Client> {
    let development = &config.development;
    if development.allow_private_addresses {
        tracing::warn!("federation may call deployments at private network addresses");
    }
    builder(
        config,
        PublicResolver {
            allow_private: development.allow_private_addresses,
            allow_loopback: false,
        },
    )?
    .user_agent(concat!(
        "Aspen/",
        env!("CARGO_PKG_VERSION"),
        " (federation)"
    ))
    .build()
    .map_err(|e| config_error(format!("building the federation client: {e}")))
}

fn config_error(message: String) -> crate::Error {
    crate::Error::Config(config::ConfigError::Message(message))
}

/// What every client that calls hosts named by others shares: HTTPS only, no redirects, a short
/// timeout, addresses as `resolver` allows, and `[federation.development]`'s certificate
/// authorities trusted besides the system's.
pub fn builder(
    config: &FederationConfig,
    resolver: PublicResolver,
) -> crate::Result<reqwest::ClientBuilder> {
    let development = &config.development;
    let mut roots = Vec::new();
    for path in &development.extra_root_certificates {
        let unreadable = |e: &dyn std::fmt::Display| {
            config_error(format!(
                "federation.development.extra_root_certificates: {}: {e}",
                path.display()
            ))
        };
        let pem = std::fs::read(path).map_err(|e| unreadable(&e))?;
        roots.extend(reqwest::Certificate::from_pem_bundle(&pem).map_err(|e| unreadable(&e))?);
    }
    if !roots.is_empty() {
        tracing::warn!(
            count = roots.len(),
            "federation trusts extra development certificate authorities"
        );
    }
    Ok(reqwest::Client::builder()
        // A proxy would resolve names itself, past `PublicResolver`.
        .no_proxy()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .connect_timeout(TIMEOUT)
        .dns_resolver(resolver)
        .tls_certs_merge(roots))
}

fn unreachable(detail: std::borrow::Cow<'static, str>) -> crate::Error {
    crate::Error::DeploymentUnreachable(detail)
}

/// Why a call to `domain` failed, as the person who asked for it can act on: which of the
/// ways a call can fail it was, and what to check.
pub fn failure(domain: &Domain, error: &reqwest::Error) -> crate::Error {
    use crate::outbound::{CertificateFailure as Certificate, Failure};
    tracing::info!(%domain, error = %crate::outbound::error_chain(error), "could not reach a deployment");
    let domain = domain.as_str();
    unreachable(match crate::outbound::classify(error) {
        Failure::NotFound => t!("federationNotFound", domain = domain),
        Failure::NoPublicAddress => t!("federationPrivateAddress", domain = domain),
        Failure::Certificate(Certificate::Expired) => {
            t!("federationCertificateExpired", domain = domain)
        }
        Failure::Certificate(Certificate::NotYetValid) => {
            t!("federationCertificateNotYetValid", domain = domain)
        }
        Failure::Certificate(Certificate::WrongName) => {
            t!("federationCertificateWrongName", domain = domain)
        }
        Failure::Certificate(Certificate::Untrusted) => {
            t!("federationCertificateUntrusted", domain = domain)
        }
        Failure::Certificate(Certificate::Invalid) => {
            t!("federationCertificateInvalid", domain = domain)
        }
        Failure::Tls => t!("federationTls", domain = domain),
        Failure::ConnectionRefused => t!("federationConnectionRefused", domain = domain),
        Failure::TimedOut => t!(
            "federationTimedOut",
            domain = domain,
            seconds = TIMEOUT.as_secs()
        ),
        Failure::NoAnswer => t!("federationNoAnswer", domain = domain),
    })
}

pub async fn document(
    client: &reqwest::Client,
    domain: &Domain,
) -> crate::Result<DeploymentDocument> {
    let response = client
        .get(domain.document_url())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| failure(domain, &error))?;
    let status = response.status();
    if status.is_redirection() {
        let to = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("?")
            .to_string();
        return Err(unreachable(t!(
            "federationRedirected",
            domain = domain.as_str(),
            to = to
        )));
    }
    if !status.is_success() {
        tracing::info!(%domain, %status, "a deployment has no document");
        return Err(unreachable(t!(
            "federationNoDocument",
            domain = domain.as_str(),
            status = status.as_u16()
        )));
    }
    let body = read_capped(domain, response, MAX_DOCUMENT_BYTES)
        .await?
        .ok_or_else(|| {
            unreachable(t!(
                "federationDocumentTooLarge",
                domain = domain.as_str(),
                max = MAX_DOCUMENT_BYTES / 1024
            ))
        })?;
    serde_json::from_slice(&body).map_err(|error| {
        tracing::info!(%domain, %error, "a deployment's document did not parse");
        // What did not parse is quoted as the parser put it, as a path or a name would be.
        unreachable(t!(
            "federationDocumentUnreadable",
            domain = domain.as_str(),
            detail = error.to_string()
        ))
    })
}

/// The body of `domain`'s `response`, or `None` once it runs past `max` bytes, which stops the
/// read there.
pub async fn read_capped(
    domain: &Domain,
    response: reqwest::Response,
    max: usize,
) -> crate::Result<Option<Vec<u8>>> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| failure(domain, &error))?;
        if body.len() + chunk.len() > max {
            return Ok(None);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some(body))
}
