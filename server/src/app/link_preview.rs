//! Server-side link preview generation.
//!
//! Aspen is the authoritative source of link-preview metadata and thumbnails
//! for every message body: URL extraction, outbound HTTP, HTML parsing, and
//! image storage all happen here so the user's IP is never exposed to a
//! third-party origin just by rendering a chat message, and so every
//! connected client (across platforms) paints the same card from the same
//! metadata.
//!
//! 1. [`extract_preview_urls`] walks a markdown body with `pulldown-cmark`,
//!    collecting explicit link destinations plus bare `http(s)://` autolinks
//!    from text events (skipping anything inside code spans / code blocks),
//!    dedupes, and caps at [`MAX_LINK_PREVIEWS_PER_MESSAGE`].
//! 2. [`spawn_preview_fetch`] fires off a `tokio::spawn` background task that
//!    (a) fetches each URL's HTML with `reqwest` under strict byte / time
//!    limits, (b) parses Open Graph / Twitter Card / `<title>` / `<meta
//!    name="description">` / `<meta name="theme-color">` via `html5ever`'s
//!    tokenizer, (c) downloads the referenced `og:image` and uploads the
//!    bytes to the S3 media store, (d) writes a fresh set of
//!    `message_link_preview` rows, and (e) broadcasts a
//!    [`ServerEvent::MessageLinkPreviewsReady`] so connected clients can
//!    swap the empty-preview card stack on the message for the populated
//!    one without reloading the channel.
//! 3. [`load_previews`] batches preview rows back out for REST reads,
//!    templating each row's `image_id` into a public download URL via
//!    [`MediaStore::public_url`].
//! 4. [`delete_images_for_message`] tears down the S3 objects for a message
//!    before `delete_message` / the content-edit refetch path lets the row
//!    itself go away, so we don't leak image blobs.
//!
//! Preview thumbnails are downloaded by clients directly from the
//! anonymous-read endpoint behind [`crate::app::media_store::MediaStore::public_url`];
//! the API never serves the bytes itself.
//!
//! The metadata side of (b) goes through a process-local LRU cache keyed by
//! URL so repeated mentions of the same link don't hammer the third-party
//! origin. Image bytes are intentionally *not* cached — every materialised
//! preview row owns a fresh S3 object, which keeps the delete lifecycle
//! trivial (no ref-counting, no orphan sweeps).

use crate::api::GlobalServerContext;
use crate::api::link_preview::{LinkPreview, image_storage_key};
use crate::api::message_enum::server_event::ServerEvent;
use crate::app::media_store::MediaStore;
use crate::app::{self, ChannelId, LinkPreviewImageId, MessageId};
use crate::database::schema::message_link_preview;
use diesel::{ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable};
use diesel_async::AsyncPgConnection;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use futures_util::stream::StreamExt;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use lru::LruCache;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::num::NonZeroUsize;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::net::lookup_host;
use tracing::{info, warn};
use url::Url;

/// Hard ceiling on how many preview cards a single message can carry.
///
/// Three is enough for a message that genuinely shares a handful of related
/// links (a comparison, a list of references) while keeping the per-message
/// outbound-fetch cost bounded. The limit is enforced at extraction time in
/// [`extract_preview_urls`], so downstream code — including the DB insert
/// and the client render path — can assume preview lists are already
/// small and free of duplicates.
pub const MAX_LINK_PREVIEWS_PER_MESSAGE: usize = 3;

/// Maximum number of HTML bytes we'll read from any single URL while looking
/// for metadata. Just enough that almost every real site's `<head>` fits.
const MAX_METADATA_BYTES: usize = 256 * 1024;

/// Maximum number of image bytes we'll read from any single preview image.
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;

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

// ---------------------------------------------------------------------------
// DB row types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = message_link_preview)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct LinkPreviewRow {
    pub message_id: MessageId,
    #[allow(dead_code)] // used only for the ORDER BY column selection
    pub position: i32,
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub image_id: Option<LinkPreviewImageId>,
    // Stored on the row so an operator inspecting the database can match a
    // thumbnail back to its origin content type without round-tripping S3
    // metadata. Not surfaced on the wire DTO: clients learn the type from
    // the `Content-Type` header on the public download.
    #[allow(dead_code)]
    pub image_mime_type: Option<String>,
    pub theme_color: Option<String>,
}

impl LinkPreviewRow {
    fn into_wire(self, media_store: &MediaStore) -> LinkPreview {
        let image_url = self
            .image_id
            .map(|id| media_store.public_url(&image_storage_key(id)));
        LinkPreview {
            url: self.url,
            title: self.title,
            description: self.description,
            site_name: self.site_name,
            image_url,
            theme_color: self.theme_color,
        }
    }
}

/// In-memory representation of a single fully-fetched preview, used to hand
/// the metadata + image-id pair from the concurrent fetchers into the DB
/// commit step.
struct Materialised {
    url: String,
    metadata: ParsedMetadata,
    image: Option<(LinkPreviewImageId, String)>,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = message_link_preview)]
struct NewLinkPreviewRow<'a> {
    message_id: MessageId,
    position: i32,
    url: &'a str,
    title: Option<&'a str>,
    description: Option<&'a str>,
    site_name: Option<&'a str>,
    image_id: Option<LinkPreviewImageId>,
    image_mime_type: Option<&'a str>,
    theme_color: Option<&'a str>,
}

// ---------------------------------------------------------------------------
// URL extraction
// ---------------------------------------------------------------------------

/// Extract up to [`MAX_LINK_PREVIEWS_PER_MESSAGE`] unique preview-worthy
/// http(s) URLs from a markdown body.
///
/// Walks the `pulldown-cmark` event stream:
/// - explicit links (`[text](url)`, `<url>` autolinks) contribute their
///   destination;
/// - bare URLs inside `Event::Text` are scanned for a leading `http://` /
///   `https://` prefix so a user who types a raw link without bracketing
///   it still gets a preview, matching how GitHub-flavoured markdown
///   autolinks such text at render time;
/// - URLs inside `Event::Code` or fenced code blocks are skipped so pasted
///   example snippets don't generate spurious cards.
pub fn extract_preview_urls(content: &str) -> Vec<Url> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_GFM);

    let mut urls: Vec<Url> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut code_block_depth: u32 = 0;

    for event in Parser::new_ext(content, options) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_block_depth += 1,
            Event::End(TagEnd::CodeBlock) => {
                code_block_depth = code_block_depth.saturating_sub(1);
            }
            Event::Code(_) => {
                // Explicitly skip inline code so "`curl https://…`" doesn't
                // try to preview.
            }
            _ if code_block_depth > 0 => {}
            Event::Start(Tag::Link { dest_url, .. }) => {
                if push_url(&mut urls, &mut seen, dest_url.as_ref())
                    && urls.len() >= MAX_LINK_PREVIEWS_PER_MESSAGE
                {
                    return urls;
                }
            }
            Event::Text(text) => {
                for candidate in scan_bare_urls(text.as_ref()) {
                    if push_url(&mut urls, &mut seen, candidate)
                        && urls.len() >= MAX_LINK_PREVIEWS_PER_MESSAGE
                    {
                        return urls;
                    }
                }
            }
            _ => {}
        }
    }
    urls
}

fn push_url(urls: &mut Vec<Url>, seen: &mut HashSet<String>, raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }
    if !seen.insert(url.as_str().to_string()) {
        return false;
    }
    urls.push(url);
    true
}

/// Trailing characters we trim off a naked URL match. Lifted from the set of
/// punctuation GFM autolink usually ignores at the end of the match.
const BARE_URL_TRAILING: &[char] = &['.', ',', ';', ':', '!', '?', ')', ']', '"', '\'', '>', '`'];

/// Find runs that look like `http://…` or `https://…` in a plain-text span.
///
/// This is deliberately lightweight: it's only used as a supplement to the
/// markdown-level link extraction, so a false positive is capped by the
/// overall `MAX_LINK_PREVIEWS_PER_MESSAGE` budget and a false negative just
/// means we don't unfurl a particular paste.
fn scan_bare_urls(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let Some(rel) = text[cursor..].find("http") else {
            break;
        };
        let start = cursor + rel;
        let rest = &text[start..];
        let prefix_len = if rest.starts_with("https://") {
            8
        } else if rest.starts_with("http://") {
            7
        } else {
            cursor = start + 4;
            continue;
        };
        if start > 0 {
            // Reject matches that are part of a longer token (e.g. "xhttps://"
            // appearing inside a file path).
            let prev = text[..start].chars().next_back();
            if let Some(c) = prev
                && (c.is_alphanumeric() || c == '+' || c == '-' || c == '.')
            {
                cursor = start + prefix_len;
                continue;
            }
        }
        let mut end = start;
        for (off, c) in rest.char_indices() {
            if c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`') {
                break;
            }
            end = start + off + c.len_utf8();
        }
        while end > start {
            let slice = &text[start..end];
            let Some(last) = slice.chars().next_back() else {
                break;
            };
            if BARE_URL_TRAILING.contains(&last) {
                end -= last.len_utf8();
            } else {
                break;
            }
        }
        if end - start > prefix_len {
            out.push(&text[start..end]);
        }
        cursor = end.max(start + prefix_len);
    }
    out
}

// ---------------------------------------------------------------------------
// HTML parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct ParsedMetadata {
    title: Option<String>,
    description: Option<String>,
    site_name: Option<String>,
    image_url: Option<String>,
    theme_color: Option<String>,
}

impl ParsedMetadata {
    /// True if there's enough here to make a preview card worth showing.
    ///
    /// A card with only a URL and nothing else (no title, no description)
    /// is visually indistinguishable from the raw link the markdown
    /// renderer already shows inline, so publishing it would just double
    /// the row height for no informational gain. A theme colour or an
    /// `og:image` on their own are also insufficient: without at least
    /// one text field the card is a coloured rectangle whose subject is
    /// anyone's guess.
    fn has_content(&self) -> bool {
        self.title.is_some() || self.description.is_some()
    }
}

#[derive(Default)]
struct MetaState {
    /// `(key_lower, value)` pairs, first writer wins so the HTML's first
    /// occurrence of each canonical metadata key beats any fallbacks.
    meta: Vec<(String, String)>,
    /// `(media_attr, value)` pairs for `<meta name="theme-color">` tags.
    /// `None` for the `media` field means the tag applied unconditionally.
    theme_colors: Vec<(Option<String>, String)>,
    title_chunks: Vec<String>,
    in_head: bool,
    in_title: bool,
    done: bool,
}

impl MetaState {
    fn new() -> Self {
        Self {
            in_head: true,
            ..Self::default()
        }
    }

    fn lookup(&self, key: &str) -> Option<&str> {
        self.meta
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    fn insert_meta(&mut self, key: String, value: String) {
        if !self.meta.iter().any(|(k, _)| k == &key) {
            self.meta.push((key, value));
        }
    }

    /// Pick the most-appropriate `theme-color` entry.
    ///
    /// Priority is "prefer-dark, fall back to the unqualified tag, then
    /// take anything left": Aspen's UI is dark-themed everywhere, so a
    /// `media="(prefers-color-scheme: dark)"` variant will always look
    /// better against the card's background than a light-scheme one; an
    /// unqualified `<meta name="theme-color">` is the author's default
    /// answer and the next-best choice; any other media-qualified value
    /// is a last-resort fallback for sites that only shipped a
    /// light-scheme colour.
    fn theme_color(&self) -> Option<String> {
        let mut dark: Option<&str> = None;
        let mut unqualified: Option<&str> = None;
        let mut any: Option<&str> = None;
        for (media, value) in &self.theme_colors {
            match media {
                Some(m) if m.to_ascii_lowercase().contains("dark") => {
                    if dark.is_none() {
                        dark = Some(value.as_str());
                    }
                }
                None => {
                    if unqualified.is_none() {
                        unqualified = Some(value.as_str());
                    }
                }
                Some(_) => {
                    if any.is_none() {
                        any = Some(value.as_str());
                    }
                }
            }
        }
        dark.or(unqualified).or(any).map(ToOwned::to_owned)
    }

    fn into_metadata(self) -> ParsedMetadata {
        let title = self
            .lookup("og:title")
            .or_else(|| self.lookup("twitter:title"))
            .map(ToOwned::to_owned)
            .or_else(|| {
                let assembled: String = self.title_chunks.join("");
                let trimmed = assembled.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_owned())
                }
            });
        let description = self
            .lookup("og:description")
            .or_else(|| self.lookup("twitter:description"))
            .or_else(|| self.lookup("description"))
            .map(ToOwned::to_owned);
        let site_name = self.lookup("og:site_name").map(ToOwned::to_owned);
        let image_url = self
            .lookup("og:image")
            .or_else(|| self.lookup("og:image:url"))
            .or_else(|| self.lookup("og:image:secure_url"))
            .or_else(|| self.lookup("twitter:image"))
            .or_else(|| self.lookup("twitter:image:src"))
            .map(ToOwned::to_owned);
        let theme_color = self.theme_color();
        ParsedMetadata {
            title,
            description,
            site_name,
            image_url,
            theme_color,
        }
    }
}

struct MetaSink {
    state: RefCell<MetaState>,
}

impl TokenSink for MetaSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<Self::Handle> {
        let mut state = self.state.borrow_mut();
        if state.done {
            return TokenSinkResult::Continue;
        }
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                match (tag.kind, name) {
                    (TagKind::StartTag, "head") => {
                        state.in_head = true;
                    }
                    (TagKind::EndTag, "head") => {
                        state.in_head = false;
                    }
                    (TagKind::StartTag, "body") => {
                        state.in_head = false;
                        state.done = true;
                    }
                    (TagKind::StartTag, "title") => {
                        state.in_title = true;
                    }
                    (TagKind::EndTag, "title") => {
                        state.in_title = false;
                    }
                    (TagKind::StartTag, "meta") => {
                        let mut name_attr = String::new();
                        let mut property_attr = String::new();
                        let mut content_attr: Option<String> = None;
                        let mut media_attr: Option<String> = None;
                        for attr in &tag.attrs {
                            let key = attr.name.local.as_ref().to_ascii_lowercase();
                            let value = attr.value.as_ref();
                            match key.as_str() {
                                "name" => name_attr = value.to_ascii_lowercase(),
                                "property" => property_attr = value.to_ascii_lowercase(),
                                "content" => content_attr = Some(value.to_string()),
                                "media" => media_attr = Some(value.to_string()),
                                _ => {}
                            }
                        }
                        let Some(content) = content_attr else {
                            return TokenSinkResult::Continue;
                        };
                        let key = if !property_attr.is_empty() {
                            property_attr
                        } else {
                            name_attr
                        };
                        if key.is_empty() {
                            return TokenSinkResult::Continue;
                        }
                        if key == "theme-color" {
                            state.theme_colors.push((media_attr, content));
                        } else {
                            state.insert_meta(key, content);
                        }
                    }
                    _ => {}
                }
            }
            Token::CharacterTokens(data) => {
                if state.in_title {
                    state.title_chunks.push(data.as_ref().to_owned());
                }
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn parse_html_metadata(body: &str) -> ParsedMetadata {
    let sink = MetaSink {
        state: RefCell::new(MetaState::new()),
    };
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    let input = BufferQueue::default();
    // `StrTendril` requires <= 4 GiB. We're already capped at
    // MAX_METADATA_BYTES so this is infallible in practice.
    input.push_back(StrTendril::from(body));
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let state = tokenizer.sink.state.into_inner();
    state.into_metadata()
}

// ---------------------------------------------------------------------------
// HTTP client + caches
// ---------------------------------------------------------------------------

fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
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
// Metadata + image fetching
// ---------------------------------------------------------------------------

/// Fetch + parse metadata for `url`, going through the process-local cache.
///
/// Returns `None` if the fetch failed or the response wasn't preview-worthy.
/// The intermediate byte buffer is capped at [`MAX_METADATA_BYTES`] and we
/// only decode bodies whose `Content-Type` starts with `text/`.
async fn fetch_metadata(url: &Url) -> Option<ParsedMetadata> {
    let cache_key = url.as_str().to_owned();
    if let Some(cached) = cache_get(&cache_key) {
        return cached;
    }
    let parsed = fetch_metadata_uncached(url).await;
    cache_put(cache_key, parsed.clone());
    parsed
}

async fn fetch_metadata_uncached(url: &Url) -> Option<ParsedMetadata> {
    // Block all IP address classes which could unintentionally leak internal information.
    let domain = url.domain()?;
    let port = url.port().or_else(|| match url.scheme() {
        "https" => Some(443),
        "http" => Some(80),
        _ => None,
    })?;
    let socket_addrs = match lookup_host(format!("{domain}:{port}")).await {
        Ok(iter) => iter,
        Err(e) => {
            info!("preview generation: lookup domain {domain} failed {e}");
            return None;
        }
    };
    for socket_addr in socket_addrs {
        let ip = socket_addr.ip().to_canonical();
        if ip.is_loopback() || ip.is_multicast() || ip.is_unspecified() {
            info!("preview generation: IP address {ip} blocked");
            return None;
        }
        match ip {
            IpAddr::V4(v4) => {
                if v4.is_broadcast()
                    || v4.is_documentation()
                    || v4.is_link_local()
                    || v4.is_private()
                {
                    info!("preview generation: IPv4 address {v4} blocked");
                    return None;
                }
            }
            IpAddr::V6(v6) => {
                if v6.is_unicast_link_local() || v6.is_unique_local() {
                    info!("preview generation: IPv6 address {v6} blocked");
                    return None;
                }
            }
        }
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
    if !content_type.starts_with("text/") && !content_type.contains("html") {
        return None;
    }
    let final_url = response.url().clone();
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return None;
        };
        let room = MAX_METADATA_BYTES.saturating_sub(bytes.len());
        if room == 0 {
            break;
        }
        let take = room.min(chunk.len());
        bytes.extend_from_slice(&chunk[..take]);
        if take < chunk.len() {
            break;
        }
    }
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

/// Download a preview image and push it through the media store.
///
/// Failures are intentionally swallowed (logged at `warn`): the text preview
/// is still worth showing without a thumbnail, and the cost of aborting the
/// whole preview because one image didn't come back in time would be strictly
/// worse.
async fn fetch_and_store_image(
    state: &GlobalServerContext,
    image_url: &str,
) -> Option<(LinkPreviewImageId, String)> {
    let response = match http_client().get(image_url).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!(
                url = image_url,
                error = e.to_string(),
                "preview image fetch failed"
            );
            return None;
        }
    };
    if !response.status().is_success() {
        return None;
    }
    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !mime_type.starts_with("image/") {
        return None;
    }
    // Store just the `image/foo` portion — strip any `; charset=...` tail.
    let mime_type = mime_type
        .split(';')
        .next()
        .unwrap_or(&mime_type)
        .trim()
        .to_owned();
    let mut bytes: Vec<u8> = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let Ok(chunk) = chunk else {
            return None;
        };
        if bytes.len() + chunk.len() > MAX_IMAGE_BYTES {
            // Image too big; don't bother uploading a partial.
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return None;
    }
    let id = LinkPreviewImageId::new();
    let key = image_storage_key(id);
    if let Err(e) = state.media_store.put_bytes(&key, bytes, &mime_type).await {
        warn!(
            error = e.to_string(),
            url = image_url,
            "failed to upload preview image to media store"
        );
        return None;
    }
    Some((id, mime_type))
}

// ---------------------------------------------------------------------------
// Entry points for the rest of the app
// ---------------------------------------------------------------------------

/// Spawn a background task to materialise previews for a message.
///
/// This is fire-and-forget: callers publish the `Create` event with an empty
/// `link_previews` list and immediately return to the client. When the task
/// finishes it (atomically) replaces the existing preview rows and publishes
/// a [`ServerEvent::MessageLinkPreviewsReady`] so subscribers can update
/// their rendered messages in place.
pub fn spawn_preview_fetch(
    state: GlobalServerContext,
    message_id: MessageId,
    channel_id: ChannelId,
    content: String,
) {
    tokio::spawn(async move {
        if let Err(e) = run_preview_fetch(&state, message_id, channel_id, &content).await {
            warn!(
                message_id = message_id.0.to_string(),
                error = e.to_string(),
                "link preview background task failed"
            );
        }
    });
}

async fn run_preview_fetch(
    state: &GlobalServerContext,
    message_id: MessageId,
    channel_id: ChannelId,
    content: &str,
) -> app::Result<()> {
    let urls = extract_preview_urls(content);
    if urls.is_empty() {
        // Nothing to do; any previously-attached previews (e.g. from a prior
        // version of this message) have already been cleared by the edit
        // path, and the caller has already published the "empty previews"
        // event on the transition that got us here.
        return Ok(());
    }

    // Fetch metadata for each URL concurrently.
    let metadata_results: Vec<(Url, Option<ParsedMetadata>)> =
        futures_util::future::join_all(urls.into_iter().map(|url| async move {
            let meta = fetch_metadata(&url).await;
            (url, meta)
        }))
        .await;

    // Download images in parallel too. Each preview owns its image, so we
    // mint a fresh `LinkPreviewImageId` per row.
    let with_images = futures_util::future::join_all(metadata_results.into_iter().map(
        |(url, metadata)| async move {
            let metadata = metadata?;
            let image = match &metadata.image_url {
                Some(image_url) => fetch_and_store_image(state, image_url).await,
                None => None,
            };
            Some(Materialised {
                url: url.to_string(),
                metadata,
                image,
            })
        },
    ))
    .await;
    let materialised: Vec<Materialised> = with_images.into_iter().flatten().collect();
    let new_image_ids: Vec<LinkPreviewImageId> = materialised
        .iter()
        .filter_map(|m| m.image.as_ref().map(|(id, _)| *id))
        .collect();

    let media_store = state.media_store.as_ref();
    let wire_previews: Vec<LinkPreview> = materialised
        .iter()
        .map(|m| LinkPreview {
            url: m.url.clone(),
            title: m.metadata.title.clone(),
            description: m.metadata.description.clone(),
            site_name: m.metadata.site_name.clone(),
            image_url: m
                .image
                .as_ref()
                .map(|(id, _)| media_store.public_url(&image_storage_key(*id))),
            theme_color: m.metadata.theme_color.clone(),
        })
        .collect();

    let mut conn = state.connection_pool.get().await?;
    let txn_result: app::Result<Vec<LinkPreviewImageId>> = conn
        .transaction::<_, app::Error, _>(|conn| {
            let wire_previews = wire_previews.clone();
            let materialised_ref = &materialised;
            async move {
                // Collect the image ids currently attached to this message so
                // we can drop them from S3 after the new generation commits.
                let stale_ids: Vec<Option<LinkPreviewImageId>> = message_link_preview::table
                    .select(message_link_preview::image_id)
                    .filter(message_link_preview::message_id.eq(message_id))
                    .load(conn.as_mut())
                    .await?;
                diesel::delete(message_link_preview::table)
                    .filter(message_link_preview::message_id.eq(message_id))
                    .execute(conn.as_mut())
                    .await?;
                for (i, m) in materialised_ref.iter().enumerate() {
                    let (image_id, image_mime_type) = match &m.image {
                        Some((id, mime)) => (Some(*id), Some(mime.as_str())),
                        None => (None, None),
                    };
                    let row = NewLinkPreviewRow {
                        message_id,
                        position: i as i32,
                        url: m.url.as_str(),
                        title: m.metadata.title.as_deref(),
                        description: m.metadata.description.as_deref(),
                        site_name: m.metadata.site_name.as_deref(),
                        image_id,
                        image_mime_type,
                        theme_color: m.metadata.theme_color.as_deref(),
                    };
                    diesel::insert_into(message_link_preview::table)
                        .values(&row)
                        .execute(conn.as_mut())
                        .await?;
                }
                // NATS-then-commit: publish before the transaction commits so
                // a transient NATS failure rolls back the row writes too.
                // This mirrors the ordering rule in AGENTS.md.
                let event = ServerEvent::MessageLinkPreviewsReady {
                    message_id,
                    channel_id,
                    previews: wire_previews,
                };
                app::publish_event(state, &event).await?;
                Ok(stale_ids.into_iter().flatten().collect::<Vec<_>>())
            }
            .scope_boxed()
        })
        .await;

    match txn_result {
        Ok(stale_image_ids) => {
            // Best-effort delete of the old generation's S3 objects. If the
            // deletion fails we leak an orphan; a future sweep job can clean
            // them up — same trade-off as `attachment::delete_attachment`.
            for id in stale_image_ids {
                if let Err(e) = state.media_store.delete(&image_storage_key(id)).await {
                    warn!(
                        error = e.to_string(),
                        id = id.0.to_string(),
                        "failed to delete stale preview image from media store"
                    );
                }
            }
            Ok(())
        }
        Err(e) => {
            // Transaction failed (message was hard-deleted between the send
            // and now, NATS publish failed, etc). Drop any S3 uploads we
            // made so we don't leak orphaned preview bytes.
            for id in new_image_ids {
                if let Err(del_err) = state.media_store.delete(&image_storage_key(id)).await {
                    warn!(
                        error = del_err.to_string(),
                        id = id.0.to_string(),
                        "failed to clean up orphaned preview image after commit failure"
                    );
                }
            }
            Err(e)
        }
    }
}

/// Load all previews for `message_ids` in a single query and bucket them by
/// message id preserving the on-disk `position` order.
///
/// The `media_store` argument is the same one threaded through the rest of
/// the app; it's used here to template each row's `image_id` into a public
/// download URL so the wire DTO is what clients actually paint.
pub async fn load_previews(
    conn: &mut AsyncPgConnection,
    media_store: &MediaStore,
    message_ids: &[MessageId],
) -> app::Result<HashMap<MessageId, Vec<LinkPreview>>> {
    if message_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let rows: Vec<LinkPreviewRow> = message_link_preview::table
        .select(<LinkPreviewRow as diesel::SelectableHelper<
            diesel::pg::Pg,
        >>::as_select())
        .filter(message_link_preview::message_id.eq_any(message_ids))
        .order_by((
            message_link_preview::message_id.asc(),
            message_link_preview::position.asc(),
        ))
        .load(conn)
        .await?;
    let mut out: HashMap<MessageId, Vec<LinkPreview>> = HashMap::new();
    for row in rows {
        out.entry(row.message_id)
            .or_default()
            .push(row.into_wire(media_store));
    }
    Ok(out)
}

/// Delete S3 objects for all previews attached to a message, then remove the
/// rows themselves.
///
/// Invoked by `delete_message` (before the soft-delete so the FK cascade
/// doesn't race us to the rows) and by the content-edit refetch path. The
/// media-store delete is best-effort; a transient S3 failure is logged and
/// swallowed, matching the pattern used in `attachment::delete_attachment`.
pub async fn delete_images_for_message(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    message_id: MessageId,
) -> app::Result<()> {
    let image_ids: Vec<Option<LinkPreviewImageId>> = message_link_preview::table
        .select(message_link_preview::image_id)
        .filter(message_link_preview::message_id.eq(message_id))
        .load(conn)
        .await?;
    diesel::delete(message_link_preview::table)
        .filter(message_link_preview::message_id.eq(message_id))
        .execute(conn)
        .await?;
    for id in image_ids.into_iter().flatten() {
        if let Err(e) = state.media_store.delete(&image_storage_key(id)).await {
            warn!(
                error = e.to_string(),
                id = id.0.to_string(),
                "failed to delete link preview image from media store"
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn urls_from(content: &str) -> Vec<String> {
        extract_preview_urls(content)
            .into_iter()
            .map(|u| u.as_str().to_owned())
            .collect()
    }

    #[test]
    fn extracts_inline_markdown_links() {
        assert_eq!(
            urls_from("See [the docs](https://example.com/docs) for details."),
            vec!["https://example.com/docs".to_string()],
        );
    }

    #[test]
    fn extracts_autolinks_and_bare_urls() {
        assert_eq!(
            urls_from("<https://angle.example.com/> and naked https://bare.example.com/path!"),
            vec![
                "https://angle.example.com/".to_string(),
                "https://bare.example.com/path".to_string(),
            ],
        );
    }

    #[test]
    fn extracts_reference_links() {
        let md = "See [the ref][foo] page.\n\n[foo]: https://ref.example.com/";
        assert_eq!(urls_from(md), vec!["https://ref.example.com/".to_string()],);
    }

    #[test]
    fn skips_non_http_schemes() {
        assert!(urls_from("[local](file:///etc/passwd) and [mail](mailto:a@b)").is_empty());
    }

    #[test]
    fn dedupes_and_caps_at_three() {
        let md = "\
https://a.example.com \
https://b.example.com \
https://a.example.com \
https://c.example.com \
https://d.example.com";
        assert_eq!(
            urls_from(md),
            vec![
                "https://a.example.com/".to_string(),
                "https://b.example.com/".to_string(),
                "https://c.example.com/".to_string(),
            ],
        );
    }

    #[test]
    fn skips_urls_inside_code() {
        let md = "Inline `https://inline.example.com` and\n```\nhttps://fenced.example.com\n```";
        assert!(urls_from(md).is_empty());
    }

    #[test]
    fn parses_open_graph_and_falls_back_to_title() {
        let html = r##"
<html><head>
  <title>Fallback Title</title>
  <meta property="og:title" content="Great Page">
  <meta name="description" content="ignored because og:description beats it">
  <meta property="og:description" content="OG desc">
  <meta property="og:site_name" content="Example">
  <meta property="og:image" content="/img.png">
  <meta name="theme-color" content="#ffffff">
  <meta name="theme-color" media="(prefers-color-scheme: dark)" content="#000000">
</head><body>nope</body></html>
"##;
        let meta = parse_html_metadata(html);
        assert_eq!(meta.title.as_deref(), Some("Great Page"));
        assert_eq!(meta.description.as_deref(), Some("OG desc"));
        assert_eq!(meta.site_name.as_deref(), Some("Example"));
        assert_eq!(meta.image_url.as_deref(), Some("/img.png"));
        // Prefer dark when available.
        assert_eq!(meta.theme_color.as_deref(), Some("#000000"));
        assert!(meta.has_content());
    }

    #[test]
    fn theme_color_falls_back_to_unqualified() {
        let html = r##"<head><meta name="theme-color" content="#abcdef"></head>"##;
        let meta = parse_html_metadata(html);
        assert_eq!(meta.theme_color.as_deref(), Some("#abcdef"));
    }

    #[test]
    fn has_content_gates_empty_responses() {
        let html = "<html><head></head><body>hi</body></html>";
        let meta = parse_html_metadata(html);
        assert!(!meta.has_content());
    }

    #[test]
    fn has_content_true_when_only_title() {
        let html = "<html><head><title>Hi</title></head></html>";
        let meta = parse_html_metadata(html);
        assert!(meta.has_content());
        assert_eq!(meta.title.as_deref(), Some("Hi"));
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
