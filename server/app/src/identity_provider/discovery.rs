//! Calling identity providers: HTTPS only, no redirects, a short timeout, answers of bounded size,
//! and only public addresses (`app::outbound`) unless `[identity_providers]
//! allow_private_addresses` says otherwise. Each provider's discovery document, with its keys, is
//! cached for [`CACHED_FOR`], and fetched afresh when a token is signed with a key the cache lacks,
//! at most once per [`REFRESH_AT_MOST_EVERY`] per issuer ([`document_for_unknown_key`]).

use crate::aspen_config::AspenConfig;
use crate::outbound::{CertificateFailure as Certificate, Failure, PublicResolver};
use crate::t;
use futures_util::StreamExt;
use openidconnect::core::CoreProviderMetadata;
use openidconnect::{HttpRequest, HttpResponse, IssuerUrl};
use std::borrow::Cow;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// How long one call to a provider may take, connecting included.
const TIMEOUT: Duration = Duration::from_secs(10);
/// The largest answer read from a provider: a discovery document, its keys, or a token.
const MAX_ANSWER_BYTES: usize = 1024 * 1024;
/// How long a provider's discovery document and keys are used before they are fetched again.
const CACHED_FOR: Duration = Duration::from_secs(60 * 60);
/// How many issuers' documents one server keeps: every provider a deployment has, and room for
/// those changed or removed lately.
const MAX_CACHED: u64 = 64;
/// How often one server reads a provider's document afresh because a token was signed with a key
/// its cached copy lacks. Anyone may post a logout token naming any key, so without this bound
/// each would cost the provider two calls.
const REFRESH_AT_MOST_EVERY: Duration = Duration::from_secs(60);

/// The client every call to a provider is made with, and the documents fetched with it.
struct Outbound {
    client: reqwest::Client,
    /// `[identity_providers] allow_private_addresses`, for URLs naming an address outright, which
    /// the resolver never sees.
    allow_private: bool,
    documents: moka::sync::Cache<String, Arc<CoreProviderMetadata>>,
    /// Per issuer, when its document was last read afresh for an unknown key. Its lock is held
    /// while that read is made, so tokens arriving meanwhile wait for it rather than each making
    /// their own.
    refreshes: moka::sync::Cache<String, Arc<tokio::sync::Mutex<Option<Instant>>>>,
}

static OUTBOUND: OnceLock<Outbound> = OnceLock::new();

/// Builds the client from `[identity_providers]`, once per process; the server calls it as it
/// starts, so an unreadable `ca_file` stops it there rather than at the first sign-in.
pub fn prepare(config: &AspenConfig) -> crate::Result<()> {
    outbound(config).map(|_| ())
}

fn outbound(config: &AspenConfig) -> crate::Result<&'static Outbound> {
    if let Some(outbound) = OUTBOUND.get() {
        return Ok(outbound);
    }
    let settings = &config.identity_providers;
    if settings.allow_private_addresses {
        tracing::warn!("identity providers may be called at private network addresses");
    }
    let mut roots = Vec::new();
    if let Some(path) = &settings.ca_file {
        let unreadable = |e: &dyn std::fmt::Display| {
            crate::Error::Config(config::ConfigError::Message(format!(
                "identity_providers.ca_file: {}: {e}",
                path.display()
            )))
        };
        let pem = std::fs::read(path).map_err(|e| unreadable(&e))?;
        roots.extend(reqwest::Certificate::from_pem_bundle(&pem).map_err(|e| unreadable(&e))?);
    }
    let client = reqwest::Client::builder()
        // A proxy would resolve names itself, past `PublicResolver`.
        .no_proxy()
        .https_only(true)
        // A provider's endpoints are named in its document; one that redirects is refused rather
        // than followed to wherever it points.
        .redirect(reqwest::redirect::Policy::none())
        .timeout(TIMEOUT)
        .connect_timeout(TIMEOUT)
        .dns_resolver(PublicResolver {
            allow_private: settings.allow_private_addresses,
            allow_loopback: false,
        })
        .tls_certs_merge(roots)
        .user_agent(concat!(
            "Aspen/",
            env!("CARGO_PKG_VERSION"),
            " (identity providers)"
        ))
        .build()
        .map_err(|e| {
            crate::Error::Config(config::ConfigError::Message(format!(
                "building the identity provider client: {e}"
            )))
        })?;
    // Two first calls at once each build one; the first stored is kept.
    Ok(OUTBOUND.get_or_init(|| Outbound {
        client,
        allow_private: settings.allow_private_addresses,
        documents: moka::sync::Cache::builder()
            .max_capacity(MAX_CACHED)
            .time_to_live(CACHED_FOR)
            .build(),
        refreshes: moka::sync::Cache::builder()
            .max_capacity(MAX_CACHED)
            .build(),
    }))
}

/// Why a call to a provider failed, beyond what `reqwest` says.
#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error("the answer was larger than {MAX_ANSWER_BYTES} bytes")]
    TooLarge,
    /// The URL names an address that may not be called: a loopback or unspecified one ever, or
    /// one inside a network unless private addresses are allowed.
    #[error("{0} names an address that may not be called")]
    InsideAddress(String),
}

/// Whether `url` names, outright, an address this server may not call: loopback or unspecified
/// (which reaches this machine) whatever the settings say, and one inside a network unless
/// `allow_private`. A name is checked by `PublicResolver` as it resolves instead.
fn refuses_address(url: &url::Url, allow_private: bool) -> bool {
    use std::net::IpAddr;
    let ip = match url.host() {
        Some(url::Host::Ipv4(v4)) => IpAddr::V4(v4),
        Some(url::Host::Ipv6(v6)) => IpAddr::V6(v6),
        Some(url::Host::Domain(_)) | None => return false,
    };
    let this_machine = |ip: IpAddr| ip.is_loopback() || ip.is_unspecified();
    let carried = match ip {
        IpAddr::V6(v6) => v6.to_ipv4().map(IpAddr::V4),
        IpAddr::V4(_) => None,
    };
    if this_machine(ip) || carried.is_some_and(this_machine) {
        return true;
    }
    !allow_private && crate::outbound::names_inside_address(url)
}

/// Makes one call `openidconnect` asks for, with the providers' client. The endpoints are those a
/// provider's discovery document names, so each is checked as the issuer is.
async fn call(
    client: &reqwest::Client,
    allow_private: bool,
    request: HttpRequest,
) -> Result<HttpResponse, HttpError> {
    let (parts, body) = request.into_parts();
    let target = parts.uri.to_string();
    if url::Url::parse(&target).map_or(true, |url| refuses_address(&url, allow_private)) {
        return Err(HttpError::InsideAddress(target));
    }
    let response = client
        .request(parts.method, target)
        .headers(parts.headers)
        .body(body)
        .send()
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len() + chunk.len() > MAX_ANSWER_BYTES {
            return Err(HttpError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    let mut answer = HttpResponse::new(body);
    *answer.status_mut() = status;
    *answer.headers_mut() = headers;
    Ok(answer)
}

/// The function `openidconnect` makes its calls to providers through.
pub fn http(
    config: &AspenConfig,
) -> crate::Result<
    impl Fn(HttpRequest) -> futures_util::future::BoxFuture<'static, Result<HttpResponse, HttpError>>,
> {
    let outbound = outbound(config)?;
    let client = outbound.client.clone();
    let allow_private = outbound.allow_private;
    Ok(move |request: HttpRequest| {
        let client = client.clone();
        Box::pin(async move { call(&client, allow_private, request).await })
            as futures_util::future::BoxFuture<'static, Result<HttpResponse, HttpError>>
    })
}

/// Checks an issuer as administrators give it: an `https` URL with a host, no query or fragment,
/// and not naming outright an address [`refuses_address`] refuses (a name is checked as it
/// resolves).
pub fn issuer_url(config: &AspenConfig, issuer: &str) -> crate::Result<IssuerUrl> {
    let invalid = || crate::Error::Validation(t!("identityProviderIssuerInvalid", issuer = issuer));
    let url = url::Url::parse(issuer).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || !url.has_host()
        || url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid());
    }
    if refuses_address(&url, config.identity_providers.allow_private_addresses) {
        return Err(crate::Error::IdentityProviderUnreachable(t!(
            "identityProviderPrivateAddress",
            issuer = issuer
        )));
    }
    IssuerUrl::new(issuer.to_string()).map_err(|_| invalid())
}

/// `issuer`'s discovery document and keys: the cached copy, unless `fresh` asks for them
/// afresh, as adding a provider and a token signed with an unknown key do.
pub async fn document(
    config: &AspenConfig,
    issuer: &str,
    fresh: bool,
) -> crate::Result<Arc<CoreProviderMetadata>> {
    let outbound = outbound(config)?;
    if !fresh && let Some(document) = outbound.documents.get(issuer) {
        return Ok(document);
    }
    let issuer_url = issuer_url(config, issuer)?;
    let client = http(config)?;
    let document = CoreProviderMetadata::discover_async(issuer_url, &client)
        .await
        .map_err(|error| discovery_failure(issuer, error))?;
    let document = Arc::new(document);
    outbound
        .documents
        .insert(issuer.to_string(), document.clone());
    Ok(document)
}

/// `issuer`'s document, read afresh for a token signed with a key the cached copy lacks, unless
/// this server did so in the last [`REFRESH_AT_MOST_EVERY`]: then the copy that read left (which
/// a token naming a key the provider has since published is checked against in a minute at most),
/// or none.
pub async fn document_for_unknown_key(
    config: &AspenConfig,
    issuer: &str,
) -> crate::Result<Arc<CoreProviderMetadata>> {
    let outbound = outbound(config)?;
    let gate = outbound
        .refreshes
        .get_with(issuer.to_string(), Default::default);
    let mut last = gate.lock().await;
    if last.is_some_and(|at| at.elapsed() < REFRESH_AT_MOST_EVERY) {
        drop(last);
        return outbound
            .documents
            .get(issuer)
            .ok_or_else(|| unreachable(t!("identityProviderNoAnswer", issuer = issuer)));
    }
    // Stamped whether or not the read succeeds, so a provider that is down is not called for
    // every token either.
    *last = Some(Instant::now());
    document(config, issuer, true).await
}

fn unreachable(detail: Cow<'static, str>) -> crate::Error {
    crate::Error::IdentityProviderUnreachable(detail)
}

/// Why fetching `issuer`'s discovery document failed, as the person who asked can act on.
fn discovery_failure(
    issuer: &str,
    error: openidconnect::DiscoveryError<HttpError>,
) -> crate::Error {
    use openidconnect::DiscoveryError;
    tracing::info!(
        issuer,
        error = %crate::outbound::error_chain(&error),
        "could not read an identity provider's discovery document"
    );
    match error {
        DiscoveryError::Request(error) => request_failure(issuer, &error),
        DiscoveryError::Response(status, _, _) => unreachable(t!(
            "identityProviderDiscoveryStatus",
            issuer = issuer,
            status = status.as_u16()
        )),
        DiscoveryError::Parse(error) => unreachable(t!(
            "identityProviderDiscoveryUnreadable",
            issuer = issuer,
            detail = error.to_string()
        )),
        DiscoveryError::Validation(detail) => unreachable(t!(
            "identityProviderDiscoveryInvalid",
            issuer = issuer,
            detail = detail
        )),
        DiscoveryError::UrlParse(_) | DiscoveryError::Other(_) => {
            crate::Error::Validation(t!("identityProviderIssuerInvalid", issuer = issuer))
        }
        _ => unreachable(t!("identityProviderNoAnswer", issuer = issuer)),
    }
}

/// Why a call to `issuer`'s provider failed, by which of the ways a call can fail it was.
pub fn request_failure(issuer: &str, error: &HttpError) -> crate::Error {
    let reqwest_error = match error {
        HttpError::Request(error) => error,
        HttpError::TooLarge => {
            return unreachable(t!(
                "identityProviderAnswerTooLarge",
                issuer = issuer,
                max = MAX_ANSWER_BYTES / 1024
            ));
        }
        HttpError::InsideAddress(url) => {
            tracing::info!(
                issuer,
                url,
                "refused to call an identity provider's endpoint at an inside address"
            );
            return unreachable(t!("identityProviderPrivateAddress", issuer = issuer));
        }
    };
    if reqwest_error.is_redirect() {
        return unreachable(t!("identityProviderRedirected", issuer = issuer));
    }
    unreachable(match crate::outbound::classify(reqwest_error) {
        Failure::NotFound => t!("identityProviderNotFound", issuer = issuer),
        Failure::NoPublicAddress => t!("identityProviderPrivateAddress", issuer = issuer),
        Failure::Certificate(Certificate::Expired) => {
            t!("identityProviderCertificateExpired", issuer = issuer)
        }
        Failure::Certificate(Certificate::Untrusted) => {
            t!("identityProviderCertificateUntrusted", issuer = issuer)
        }
        Failure::Certificate(
            Certificate::NotYetValid | Certificate::WrongName | Certificate::Invalid,
        )
        | Failure::Tls => t!("identityProviderCertificateInvalid", issuer = issuer),
        Failure::ConnectionRefused => t!("identityProviderConnectionRefused", issuer = issuer),
        Failure::TimedOut => t!(
            "identityProviderTimedOut",
            issuer = issuer,
            seconds = TIMEOUT.as_secs()
        ),
        Failure::NoAnswer => t!("identityProviderNoAnswer", issuer = issuer),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(url: &str, allow_private: bool) -> bool {
        refuses_address(&url::Url::parse(url).unwrap(), allow_private)
    }

    #[test]
    fn addresses_named_outright_are_checked() {
        for url in [
            "https://127.0.0.1:6443/keys",
            "https://[::1]/keys",
            "https://[::ffff:127.0.0.1]/keys",
            "https://0.0.0.0/keys",
            "https://[::]/keys",
        ] {
            assert!(refused(url, false), "{url}");
            assert!(
                refused(url, true),
                "{url} is this machine whatever the settings"
            );
        }
        for url in [
            "https://10.0.0.5:9000/token",
            "https://169.254.169.254/latest",
            "https://[::ffff:169.254.169.254]/",
            "https://[fd00::1]/",
        ] {
            assert!(refused(url, false), "{url}");
            assert!(
                !refused(url, true),
                "{url} is allowed with private addresses"
            );
        }
        assert!(!refused("https://93.184.215.14/keys", false));
        assert!(!refused("https://id.example.org/keys", false));
        assert!(!refused("https://localhost:8443/keys", false));
    }
}
