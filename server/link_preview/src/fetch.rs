//! Outbound requests for previews: the shared HTTP client, the process-local metadata cache,
//! and fetching a page's metadata under byte and time limits.

use crate::html_meta::{ParsedMetadata, parse_html_metadata};
use crate::reddit;
use crate::video::{fetch_video_embed, video_provider_for};
use aspen_outbound::{self as outbound, PublicResolver};
use futures_util::stream::StreamExt;
use lru::LruCache;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tracing::{info, warn};
use url::Url;

/// Maximum number of HTML bytes we'll read from any single URL while looking
/// for metadata. Just enough that almost every real site's `<head>` fits.
pub const MAX_METADATA_BYTES: usize = 256 * 1024;

/// End-to-end timeout for a single metadata fetch, including redirect chasing.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a metadata lookup stays cached in memory before we'll re-fetch.
const METADATA_CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// How many URLs to remember in the process-local metadata cache.
const METADATA_CACHE_CAPACITY: usize = 2048;

/// User-Agent header we announce on outbound link-preview requests. Picked so
/// server operators know where the traffic is coming from if they inspect
/// their logs — mimicking common chat-clients that expose an ``/unfurl``
/// equivalent.
const USER_AGENT: &str = concat!(
    "AspenChatServer/",
    env!("CARGO_PKG_VERSION"),
    " (+link-preview)"
);

/// The most redirects one fetch follows.
const MAX_REDIRECTS: usize = 10;

/// Whether a preview may fetch `url`: a public address as [`outbound::may_fetch`] allows, and
/// only at the web's ports, 80 and 443 (a URL naming none takes its scheme's). A host's public
/// address can reach more of it than its web server, this server's own included (its NATS, its
/// database, its metrics), so a message may not name another port.
pub fn may_fetch(url: &Url) -> bool {
    outbound::may_fetch(url) && matches!(url.port_or_known_default(), Some(80 | 443))
}

/// The client every preview fetch is made with: public addresses only, as names resolve and as
/// URLs and redirects name them (`app::outbound`), at the web's ports; a request for a URL must
/// still be checked with [`may_fetch`] before it is made.
pub fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            // A proxy would resolve names itself, past `PublicResolver`.
            .no_proxy()
            .dns_resolver(Arc::new(PublicResolver {
                allow_private: false,
                allow_loopback: false,
            }))
            .redirect(outbound::checked_redirects(MAX_REDIRECTS, may_fetch))
            .user_agent(USER_AGENT)
            .timeout(FETCH_TIMEOUT)
            .connect_timeout(Duration::from_secs(5))
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .expect("failed to build link-preview reqwest client")
    })
}

type MetadataCache = Mutex<LruCache<String, (Instant, Option<ParsedMetadata>)>>;

fn metadata_cache() -> &'static MetadataCache {
    static CACHE: OnceLock<MetadataCache> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(LruCache::new(
            NonZeroUsize::new(METADATA_CACHE_CAPACITY).expect("non-zero capacity"),
        ))
    })
}

fn cache_get(url: &str) -> Option<Option<ParsedMetadata>> {
    let mut guard = metadata_cache().lock().ok()?;
    let (fetched_at, value) = guard.get(url)?;
    if fetched_at.elapsed() >= METADATA_CACHE_TTL {
        return None;
    }
    Some(value.clone())
}

fn cache_put(url: String, value: Option<ParsedMetadata>) {
    if let Ok(mut guard) = metadata_cache().lock() {
        guard.put(url, (Instant::now(), value));
    }
}

// ---------------------------------------------------------------------------
// Metadata fetching
// ---------------------------------------------------------------------------

/// A lookup's metadata, and whether it may be cached. A lookup cut short by the other side's
/// rate limit may not, so a later mention of the link gets the whole preview.
pub struct Lookup {
    pub metadata: Option<ParsedMetadata>,
    pub lasting: bool,
}

impl Lookup {
    pub fn lasting(metadata: Option<ParsedMetadata>) -> Self {
        Self {
            metadata,
            lasting: true,
        }
    }
}

/// Fetch + parse metadata for `url`, going through the process-local cache.
///
/// Returns `None` if the fetch failed or the response wasn't preview-worthy.
/// The intermediate byte buffer is capped at [`MAX_METADATA_BYTES`] and we
/// only decode bodies whose `Content-Type` starts with `text/`.
pub async fn fetch_metadata(url: &Url) -> Option<ParsedMetadata> {
    if !may_fetch(url) {
        info!(url = url.as_str(), "preview generation: URL refused");
        return None;
    }
    let cache_key = url.as_str().to_owned();
    if let Some(cached) = cache_get(&cache_key) {
        return cached;
    }
    let lookup = if reddit::is_reddit(url) {
        reddit::fetch_metadata(url).await
    } else {
        Lookup::lasting(fetch_metadata_uncached(url).await)
    };
    if lookup.lasting {
        cache_put(cache_key, lookup.metadata.clone());
    }
    lookup.metadata
}

/// Page metadata, plus a player when the link is a video on an allowlisted provider. The
/// provider's oEmbed answer also fills in a title, site name, and thumbnail the page did not
/// give, so a video page that hides its metadata behind a consent wall still gets a card.
async fn fetch_metadata_uncached(url: &Url) -> Option<ParsedMetadata> {
    let mut metadata = fetch_page_metadata(url).await;
    if let Some(provider) = video_provider_for(url)
        && let Some(oembed) = fetch_video_embed(provider, url).await
    {
        let page = metadata.get_or_insert_with(ParsedMetadata::default);
        page.video = Some(oembed.embed);
        if page.title.is_none() {
            page.title = oembed.title;
        }
        if page.site_name.is_none() {
            page.site_name = oembed.provider_name;
        }
        if page.image_url.is_none() {
            page.image_url = oembed.thumbnail_url;
        }
    }
    metadata
}

async fn fetch_page_metadata(url: &Url) -> Option<ParsedMetadata> {
    // Refuse addresses inside a network (`app::outbound`), and ports but the web's.
    if !may_fetch(url) {
        info!(url = url.as_str(), "preview generation: URL refused");
        return None;
    }
    let response = match http_client().get(url.as_str()).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(
                url = url.as_str(),
                error = e.to_string(),
                "link preview fetch failed"
            );
            return None;
        }
    };
    if !response.status().is_success() {
        return None;
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if content_type.starts_with("image/") {
        // A link straight to a picture: no page to read, the picture is the preview.
        return Some(ParsedMetadata {
            image_url: Some(response.url().to_string()),
            direct_image: true,
            ..ParsedMetadata::default()
        });
    }
    if !content_type.starts_with("text/") && !content_type.contains("html") {
        return None;
    }
    let final_url = response.url().clone();
    let bytes = read_capped(response, MAX_METADATA_BYTES).await?;
    if bytes.is_empty() {
        return None;
    }
    let body = String::from_utf8_lossy(&bytes);
    let mut metadata = parse_html_metadata(body.as_ref());
    // Resolve a relative og:image against the final redirected URL so S3
    // object creation has an absolute URL to work from.
    if let Some(raw) = metadata.image_url.take()
        && let Ok(resolved) = final_url.join(&raw)
        && matches!(resolved.scheme(), "http" | "https")
    {
        metadata.image_url = Some(resolved.to_string());
    }
    if !metadata.has_content() {
        return None;
    }
    Some(metadata)
}

/// Reads at most `cap` bytes of a response body, dropping the rest; `None` on a read error.
pub async fn read_capped(response: reqwest::Response, cap: usize) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        let room = cap.saturating_sub(bytes.len());
        if room == 0 {
            break;
        }
        let take = room.min(chunk.len());
        bytes.extend_from_slice(&chunk[..take]);
        if take < chunk.len() {
            break;
        }
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_webs_ports_are_fetched() {
        for allowed in [
            "https://example.com/",
            "http://example.com/",
            "https://example.com:443/",
            "http://example.com:80/",
            "http://example.com:443/",
        ] {
            assert!(may_fetch(&Url::parse(allowed).unwrap()), "{allowed}");
        }
        for refused in [
            "https://example.com:4222/",
            "http://example.com:5432/",
            "https://example.com:9464/metrics",
            "http://127.0.0.1/",
            "ftp://example.com/",
        ] {
            assert!(!may_fetch(&Url::parse(refused).unwrap()), "{refused}");
        }
    }

    #[test]
    fn relative_og_image_resolves_against_final_url() {
        // Matches the post-redirect resolution we apply in
        // `fetch_metadata_uncached`: a relative `og:image` is joined against
        // the final response URL (which reqwest tracks through redirects),
        // not the originally-requested URL.
        let final_url = Url::parse("https://cdn.example.com/posts/42/").unwrap();
        let resolved = final_url.join("/assets/thumb.png").unwrap();
        assert_eq!(
            resolved.as_str(),
            "https://cdn.example.com/assets/thumb.png"
        );
        let resolved = final_url.join("../thumb.png").unwrap();
        assert_eq!(resolved.as_str(), "https://cdn.example.com/posts/thumb.png");
        let resolved = final_url.join("https://img.example.com/a.png").unwrap();
        assert_eq!(resolved.as_str(), "https://img.example.com/a.png");
    }
}
