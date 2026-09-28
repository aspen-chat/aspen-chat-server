//! Calls to other deployments: HTTPS only, no redirects, a short timeout, a capped body, and
//! only public addresses (`app::outbound`) unless `[federation.development]` says otherwise.

use crate::app::federation::{DeploymentDocument, Domain};
use crate::app::{self, outbound::PublicResolver};
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
        .map_err(|error| {
            tracing::info!(%domain, error = %error_chain(&error), "could not reach a deployment");
            unreachable(t!("federationNoAnswer", domain = domain.as_str()))
        })?;
    if !response.status().is_success() {
        tracing::info!(%domain, status = %response.status(), "a deployment has no document");
        return Err(unreachable(t!(
            "federationNoDocument",
            domain = domain.as_str()
        )));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            tracing::info!(%domain, error = %error_chain(&error), "a deployment's document broke off");
            unreachable(t!("federationNoAnswer", domain = domain.as_str()))
        })?;
        if body.len() + chunk.len() > MAX_DOCUMENT_BYTES {
            return Err(unreachable(t!("federationDocumentInvalid")));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|error| {
        tracing::info!(%domain, %error, "a deployment's document did not parse");
        unreachable(t!("federationDocumentInvalid"))
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
