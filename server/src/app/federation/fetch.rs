//! Calls to other deployments: HTTPS only, no redirects, a short timeout, a capped body, and
//! only public addresses (`app::outbound`) unless `[federation.development]` says otherwise.

use crate::app;
use crate::app::federation::{DeploymentDocument, Domain};
use crate::app::outbound::{PublicResolver, ResolveError};
use crate::aspen_config::FederationConfig;
use futures_util::StreamExt;
use rust_i18n::t;
use std::time::Duration;

/// How long one call to another deployment may take, connecting included.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The largest document read from another deployment.
const MAX_DOCUMENT_BYTES: usize = 64 * 1024;

/// The client every call to another deployment is made with, built once per process from
/// `[federation]`. It fails only on an unreadable development certificate.
pub fn client(config: &FederationConfig) -> app::Result<reqwest::Client> {
    let development = &config.development;
    let config_error = |message: String| app::Error::Config(config::ConfigError::Message(message));
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
    if development.allow_private_addresses {
        tracing::warn!("federation may call deployments at private network addresses");
    }
    reqwest::Client::builder()
        .user_agent(concat!(
            "Aspen/",
            env!("CARGO_PKG_VERSION"),
            " (federation)"
        ))
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .connect_timeout(TIMEOUT)
        .dns_resolver(PublicResolver {
            allow_private: development.allow_private_addresses,
        })
        .tls_certs_merge(roots)
        .build()
        .map_err(|e| config_error(format!("building the federation client: {e}")))
}

fn unreachable(detail: std::borrow::Cow<'static, str>) -> app::Error {
    app::Error::DeploymentUnreachable(detail)
}

/// Why a call to `domain` failed, as the person who asked for it can act on: which of the
/// ways a call can fail it was, and what to check.
pub fn failure(domain: &Domain, error: &reqwest::Error) -> app::Error {
    tracing::info!(%domain, error = %error_chain(error), "could not reach a deployment");
    let domain_text = domain.as_str();
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(cause) = source {
        if let Some(resolve) = cause.downcast_ref::<ResolveError>() {
            return unreachable(match resolve {
                ResolveError::NotFound(_) => t!("federationNotFound", domain = domain_text),
                ResolveError::NoPublicAddress(_) => {
                    t!("federationPrivateAddress", domain = domain_text)
                }
            });
        }
        if let Some(tls) = cause.downcast_ref::<rustls::Error>() {
            return unreachable(certificate_failure(domain_text, tls));
        }
        if let Some(io) = cause.downcast_ref::<std::io::Error>() {
            if let Some(tls) = io.get_ref().and_then(|e| e.downcast_ref::<rustls::Error>()) {
                return unreachable(certificate_failure(domain_text, tls));
            }
            if io.kind() == std::io::ErrorKind::ConnectionRefused {
                return unreachable(t!("federationConnectionRefused", domain = domain_text));
            }
        }
        source = cause.source();
    }
    if error.is_timeout() {
        return unreachable(t!(
            "federationTimedOut",
            domain = domain_text,
            seconds = TIMEOUT.as_secs()
        ));
    }
    unreachable(t!("federationNoAnswer", domain = domain_text))
}

/// A certificate `domain` presented that this server does not trust, by what is wrong with it.
fn certificate_failure(domain: &str, error: &rustls::Error) -> std::borrow::Cow<'static, str> {
    use rustls::CertificateError as Certificate;
    match error {
        rustls::Error::InvalidCertificate(certificate) => match certificate {
            Certificate::Expired | Certificate::ExpiredContext { .. } => {
                t!("federationCertificateExpired", domain = domain)
            }
            Certificate::NotValidYet | Certificate::NotValidYetContext { .. } => {
                t!("federationCertificateNotYetValid", domain = domain)
            }
            Certificate::NotValidForName | Certificate::NotValidForNameContext { .. } => {
                t!("federationCertificateWrongName", domain = domain)
            }
            Certificate::UnknownIssuer => t!("federationCertificateUntrusted", domain = domain),
            _ => t!("federationCertificateInvalid", domain = domain),
        },
        _ => t!("federationTls", domain = domain),
    }
}

/// `domain`'s published document.
pub async fn document(
    client: &reqwest::Client,
    domain: &Domain,
) -> app::Result<DeploymentDocument> {
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
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| failure(domain, &error))?;
        if body.len() + chunk.len() > MAX_DOCUMENT_BYTES {
            return Err(unreachable(t!(
                "federationDocumentTooLarge",
                domain = domain.as_str(),
                max = MAX_DOCUMENT_BYTES / 1024
            )));
        }
        body.extend_from_slice(&chunk);
    }
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

/// An error and its causes on one line, which is where a TLS or DNS failure's reason is.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}
