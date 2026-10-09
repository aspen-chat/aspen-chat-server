//! TLS to the services the servers use: the certificate files an operator gives for each
//! (`[nats.tls]`, `[valkey.tls]`, `[media.s3.tls]`, `[email.tls]`), and NATS connections made
//! with them, which both the API and voice servers open.
//!
//! Each service's client checks the service's certificate against the system's authorities and
//! the host name it is reached by; `ca_file` adds authorities, for a service whose certificate
//! a private authority issued. NATS, S3, and SMTP clients read the files themselves; fred takes
//! a whole rustls client, which [`TlsFiles::rustls_client_config`] makes.

use async_nats::ToServerAddrs;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The certificate files for TLS to one service.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TlsFiles {
    /// PEM certificates of authorities to trust besides the system's.
    pub ca_file: Option<PathBuf>,
    /// A PEM client certificate (its chain, leaf first), for a service that authenticates
    /// clients by certificate. Given with `key_file`.
    pub cert_file: Option<PathBuf>,
    /// The PEM private key of `cert_file`.
    pub key_file: Option<PathBuf>,
}

impl TlsFiles {
    /// The client certificate and its key, refusing one without the other; `section` names
    /// where they were given, such as `nats.tls`.
    pub fn identity(&self, section: &str) -> Result<Option<(&Path, &Path)>, String> {
        match (&self.cert_file, &self.key_file) {
            (None, None) => Ok(None),
            (Some(cert), Some(key)) => Ok(Some((cert, key))),
            _ => Err(format!(
                "{section} gives one of cert_file and key_file; a client certificate needs both"
            )),
        }
    }
}

impl TlsFiles {
    /// A rustls client checking certificates against the system's authorities and `ca_file`'s,
    /// presenting the client certificate when one is given, for a client that takes one whole
    /// (fred's). `section` names where the files were given, such as `valkey.tls`.
    pub fn rustls_client_config(&self, section: &str) -> Result<rustls::ClientConfig, String> {
        let unusable =
            |path: &Path, e: &dyn std::fmt::Display| format!("{section}: {}: {e}", path.display());
        let mut roots = rustls::RootCertStore::empty();
        let system = rustls_native_certs::load_native_certs();
        roots.add_parsable_certificates(system.certs);
        if let Some(ca) = &self.ca_file {
            let mut found = false;
            for cert in CertificateDer::pem_file_iter(ca).map_err(|e| unusable(ca, &e))? {
                roots
                    .add(cert.map_err(|e| unusable(ca, &e))?)
                    .map_err(|e| unusable(ca, &e))?;
                found = true;
            }
            if !found {
                return Err(unusable(ca, &"holds no certificate"));
            }
        }
        if roots.is_empty() {
            return Err(format!(
                "{section}: no certificate authorities could be read from the system; give \
                 ca_file"
            ));
        }
        let builder = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("{section}: {e}"))?
        .with_root_certificates(roots);
        match self.identity(section)? {
            None => Ok(builder.with_no_client_auth()),
            Some((cert, key)) => {
                let chain = CertificateDer::pem_file_iter(cert)
                    .map_err(|e| unusable(cert, &e))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| unusable(cert, &e))?;
                let key = PrivateKeyDer::from_pem_file(key).map_err(|e| unusable(key, &e))?;
                builder
                    .with_client_auth_cert(chain, key)
                    .map_err(|e| format!("{section}'s client certificate: {e}"))
            }
        }
    }
}

/// `options` with `tls`'s files, requiring TLS whatever `nats_url` names, so that no one between
/// the servers and NATS can strip it. The client certificate is presented when both its files
/// are given, which each server's config checks ([`TlsFiles::identity`]).
pub fn nats_options(
    options: async_nats::ConnectOptions,
    tls: Option<&TlsFiles>,
) -> async_nats::ConnectOptions {
    let Some(tls) = tls else {
        return options;
    };
    let mut options = options.require_tls(true);
    if let Some(ca) = &tls.ca_file {
        options = options.add_root_certificates(ca.clone());
    }
    if let (Some(cert), Some(key)) = (&tls.cert_file, &tls.key_file) {
        options = options.add_client_certificate(cert.clone(), key.clone());
    }
    options
}

/// Warns when `client`, connected to `nats_url`, is not encrypted and a server it may reach is
/// not on this machine: then the NATS password and every event cross the network readable. It is
/// encrypted when every address in `nats_url` names `tls://` or `wss://`, `[nats.tls]` requires
/// it, or the NATS server requires it, which async-nats then obeys.
pub fn warn_if_nats_unencrypted(
    nats_url: &str,
    tls: Option<&TlsFiles>,
    client: &async_nats::Client,
) {
    let Ok(addresses) = nats_url.to_server_addrs() else {
        return;
    };
    let addresses: Vec<_> = addresses.collect();
    let encrypted = tls.is_some()
        || client.server_info().tls_required
        || addresses.iter().all(async_nats::ServerAddr::tls_required);
    let remote = addresses.iter().any(|address| !is_loopback(address.host()));
    if !encrypted && remote {
        tracing::warn!(
            nats_url,
            "NATS is reached without TLS, so its password and every event cross the network \
             readable; name it with tls:// or give [nats.tls]"
        );
    }
}

/// Whether `host` (a name, or an address with or without IPv6's brackets) is this machine.
pub fn is_loopback(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    host.parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or_else(|_| {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            host == "localhost" || host.ends_with(".localhost")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_this_machine() {
        for host in [
            "127.0.0.1",
            "127.8.0.1",
            "::1",
            "[::1]",
            "localhost",
            "LocalHost.",
            "a.localhost",
        ] {
            assert!(is_loopback(host), "{host}");
        }
        for host in ["10.0.0.1", "nats.internal", "localhost.example.org", "::2"] {
            assert!(!is_loopback(host), "{host}");
        }
    }

    #[test]
    fn a_client_certificate_needs_its_key() {
        let half = TlsFiles {
            cert_file: Some("c.pem".into()),
            ..TlsFiles::default()
        };
        assert!(half.identity("nats.tls").is_err());
        assert!(TlsFiles::default().identity("nats.tls").unwrap().is_none());
    }
}
