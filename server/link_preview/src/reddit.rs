//! Reddit posts. Reddit answers a crawler it does not recognise with a script challenge in
//! place of the post, and the crawlers it does recognise with a card whose picture is a
//! composite of the post with its vote counts drawn in. Its links are previewed instead from
//! the two surfaces Reddit offers sites that show its posts:
//!
//! 1. Its oEmbed endpoint, for the title, the author, and the post's subreddit. It takes only
//!    `/r/{subreddit}/comments/{id}` URLs but finds the post by its id alone, so a link that
//!    does not name the subreddit is asked about under a placeholder.
//! 2. Its embed page (`embed.reddit.com`), served under the post's own subreddit, for the
//!    picture, the text of a text post, and whether the post is marked NSFW or as a spoiler.
//!    The page serves such posts' media too, so the picture and text are kept only when the
//!    page says the post is neither.
//!
//! The embed page is rate limited by address. When Reddit refuses it or says the allowance is
//! spent, previews go without the page until the allowance resets, and such a preview is not
//! cached, so a later mention of the link gets the picture.

use crate::fetch::{Lookup, http_client, read_capped};
use crate::html_meta::ParsedMetadata;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use reqwest::StatusCode;
use std::cell::RefCell;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tracing::debug;
use url::Url;

/// Hosts whose pages are Reddit's and get the challenge.
const PAGE_HOSTS: &[&str] = &[
    "reddit.com",
    "www.reddit.com",
    "old.reddit.com",
    "new.reddit.com",
    "np.reddit.com",
    "m.reddit.com",
    "sh.reddit.com",
    "redd.it",
    "v.redd.it",
];

/// Hosts the embed page's pictures are taken from: the post's own media, never the
/// subreddit's icon, an avatar, or a tracker.
const PICTURE_HOSTS: &[&str] = &["preview.redd.it", "external-preview.redd.it", "i.redd.it"];

const OEMBED_ENDPOINT: &str = "https://www.reddit.com/oembed";

/// The subreddit named when asking oEmbed about a link that names none.
const PLACEHOLDER_SUBREDDIT: &str = "all";

/// Reddit's OrangeRed, the card's accent.
const THEME_COLOR: &str = "#FF4500";

/// The embed page runs to about 340 KiB, nearly all of it scripts and styles ahead of the post.
const MAX_EMBED_PAGE_BYTES: usize = 1024 * 1024;

/// The most of a text post's text a card carries; clients show two lines of it.
const MAX_DESCRIPTION_CHARS: usize = 500;

/// How long embed pages are left alone after a refusal that does not say when to come back.
const DEFAULT_PAUSE: Duration = Duration::from_secs(60);

/// The longest a refusal may keep embed pages paused, whatever it says.
const MAX_PAUSE: Duration = Duration::from_secs(15 * 60);

/// Whether `url` is a page on Reddit, which only this module previews.
pub fn is_reddit(url: &Url) -> bool {
    url.host_str()
        .is_some_and(|host| PAGE_HOSTS.contains(&host.to_ascii_lowercase().as_str()))
}

/// A post as a link names it: its id, and its subreddit when the link says.
#[derive(Debug, PartialEq, Eq)]
struct PostRef {
    subreddit: Option<String>,
    id: String,
}

/// What a Reddit link leads to.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    Post(PostRef),
    /// A link that redirects to a post: a share link, or a video's own address.
    Redirect,
    /// A subreddit, a profile, a search, or anything else that is not a post.
    Other,
}

/// Post ids are base 36.
fn is_post_id(value: &str) -> bool {
    (1..=13).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_lowercase())
}

/// Subreddit names, and the `u_{name}` subreddits of profiles.
fn is_subreddit(value: &str) -> bool {
    (1..=24).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

fn post(subreddit: Option<&str>, id: &str) -> Target {
    if !is_post_id(id) {
        return Target::Other;
    }
    let subreddit = subreddit.filter(|s| is_subreddit(s)).map(str::to_owned);
    Target::Post(PostRef {
        subreddit,
        id: id.to_owned(),
    })
}

fn target_of(url: &Url) -> Target {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let segments: Vec<&str> = url
        .path_segments()
        .map(|s| s.filter(|seg| !seg.is_empty()).collect())
        .unwrap_or_default();
    match (host.as_str(), segments.as_slice()) {
        ("redd.it", [id]) => post(None, id),
        ("v.redd.it", [_]) => Target::Redirect,
        (_, ["r", subreddit, "comments", id, ..]) => post(Some(subreddit), id),
        (_, ["user" | "u", _, "comments", id, ..] | ["comments" | "gallery", id, ..]) => {
            post(None, id)
        }
        (_, ["r", _, "s", _] | ["video", _]) => Target::Redirect,
        _ => Target::Other,
    }
}

/// The post a share link or a video's address redirects to. Reddit sends the redirect before
/// any challenge, so only the final address is read.
async fn follow_to_post(url: &Url) -> Option<PostRef> {
    let response = match http_client().get(url.as_str()).send().await {
        Ok(r) => r,
        Err(e) => {
            debug!(
                url = url.as_str(),
                error = e.to_string(),
                "Reddit redirect fetch failed"
            );
            return None;
        }
    };
    let landed = response.url().clone();
    drop(response);
    if !is_reddit(&landed) {
        return None;
    }
    match target_of(&landed) {
        Target::Post(post) => Some(post),
        Target::Redirect | Target::Other => None,
    }
}

pub async fn fetch_metadata(url: &Url) -> Lookup {
    let post = match target_of(url) {
        Target::Post(post) => Some(post),
        Target::Redirect => follow_to_post(url).await,
        Target::Other => None,
    };
    let Some(post) = post else {
        // Reddit gives anything else the challenge, which reads as a card titled "Reddit".
        return Lookup::lasting(None);
    };
    let oembed = match fetch_oembed(&post).await {
        Ok(oembed) => oembed,
        Err(lasting) => {
            return Lookup {
                metadata: None,
                lasting,
            };
        }
    };
    let subreddit = post
        .subreddit
        .clone()
        .or_else(|| oembed.html.as_deref().and_then(subreddit_from_html));
    let (embed, lasting) = match &subreddit {
        Some(subreddit) => fetch_embed_page(subreddit, &post.id).await,
        None => (None, true),
    };
    Lookup {
        metadata: Some(card(&oembed, subreddit.as_deref(), embed)),
        lasting,
    }
}

/// The parts of Reddit's oEmbed answer a card uses.
#[derive(Debug, Default, serde::Deserialize)]
struct OembedResponse {
    title: Option<String>,
    author_name: Option<String>,
    /// A blockquote linking the post at its canonical address, which names its subreddit.
    html: Option<String>,
}

/// Reddit's oEmbed answer for the post. `Err` says whether its absence may be cached: a post
/// Reddit does not have stays missing, a refusal or failure does not.
async fn fetch_oembed(post: &PostRef) -> Result<OembedResponse, bool> {
    let subreddit = post.subreddit.as_deref().unwrap_or(PLACEHOLDER_SUBREDDIT);
    let post_url = format!("https://www.reddit.com/r/{subreddit}/comments/{}/", post.id);
    let endpoint =
        Url::parse_with_params(OEMBED_ENDPOINT, [("url", post_url.as_str())]).map_err(|_| true)?;
    let response = match http_client().get(endpoint.as_str()).send().await {
        Ok(r) => r,
        Err(e) => {
            debug!(
                url = post_url,
                error = e.to_string(),
                "Reddit oEmbed fetch failed"
            );
            return Err(false);
        }
    };
    let status = response.status();
    if !status.is_success() {
        return Err(status.is_client_error() && status != StatusCode::TOO_MANY_REQUESTS);
    }
    let bytes = read_capped(response, crate::fetch::MAX_METADATA_BYTES)
        .await
        .ok_or(false)?;
    let oembed: OembedResponse = serde_json::from_slice(&bytes).map_err(|_| true)?;
    if oembed.title.as_deref().is_none_or(|t| t.trim().is_empty()) {
        return Err(true);
    }
    Ok(oembed)
}

/// The subreddit in the first `https://www.reddit.com/r/{subreddit}/` link of an oEmbed
/// answer's `html`.
fn subreddit_from_html(html: &str) -> Option<String> {
    const PREFIX: &str = "https://www.reddit.com/r/";
    let start = html.find(PREFIX)? + PREFIX.len();
    let rest = &html[start..];
    let name = &rest[..rest.find('/')?];
    is_subreddit(name).then(|| name.to_owned())
}

// ---------------------------------------------------------------------------
// The embed page
// ---------------------------------------------------------------------------

/// When embed pages may next be asked for, while Reddit's allowance for this server is spent.
static EMBED_RESUMES_AT: Mutex<Option<Instant>> = Mutex::new(None);

fn embed_paused() -> bool {
    let Ok(mut resumes_at) = EMBED_RESUMES_AT.lock() else {
        return false;
    };
    match *resumes_at {
        Some(at) if Instant::now() < at => true,
        Some(_) => {
            *resumes_at = None;
            false
        }
        None => false,
    }
}

fn pause_embeds(for_: Duration) {
    if let Ok(mut resumes_at) = EMBED_RESUMES_AT.lock() {
        let at = Instant::now() + for_.min(MAX_PAUSE);
        *resumes_at = Some(resumes_at.map_or(at, |current| current.max(at)));
    }
}

/// How long Reddit says its allowance takes to reset, from `x-ratelimit-reset`.
fn reset_after(headers: &reqwest::header::HeaderMap) -> Duration {
    headers
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
        .map_or(DEFAULT_PAUSE, |seconds| {
            Duration::from_secs_f64(seconds.min(MAX_PAUSE.as_secs_f64()))
        })
}

/// Whether `x-ratelimit-remaining` says the allowance is spent.
fn allowance_spent(headers: &reqwest::header::HeaderMap) -> bool {
    headers
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse::<f64>().ok())
        .is_some_and(|remaining| remaining < 1.0)
}

/// The post as its embed page shows it, and whether a card made without it may be cached.
async fn fetch_embed_page(subreddit: &str, id: &str) -> (Option<EmbedPost>, bool) {
    if embed_paused() {
        return (None, false);
    }
    let url = format!("https://embed.reddit.com/r/{subreddit}/comments/{id}/?embed=true");
    let response = match http_client().get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            debug!(url, error = e.to_string(), "Reddit embed page fetch failed");
            return (None, false);
        }
    };
    let status = response.status();
    if status == StatusCode::TOO_MANY_REQUESTS {
        pause_embeds(reset_after(response.headers()));
        return (None, false);
    }
    if allowance_spent(response.headers()) {
        pause_embeds(reset_after(response.headers()));
    }
    if !status.is_success() {
        return (None, !status.is_server_error());
    }
    let Some(bytes) = read_capped(response, MAX_EMBED_PAGE_BYTES).await else {
        return (None, false);
    };
    // Tokenizing the page is work for a blocking thread, not the runtime's.
    let id = id.to_owned();
    let post = tokio::task::spawn_blocking(move || {
        parse_embed_page(String::from_utf8_lossy(&bytes).as_ref(), &id)
    })
    .await
    .ok()
    .flatten();
    (post, true)
}

/// What a card takes from the embed page.
#[derive(Debug, Default, PartialEq, Eq)]
struct EmbedPost {
    /// The post's picture: a picture post's, a gallery's first, a video's poster, or a link's
    /// thumbnail. `None` for an NSFW or spoiler post.
    image_url: Option<String>,
    /// A text post's text on one line. `None` for an NSFW or spoiler post.
    text: Option<String>,
}

/// The JSON Reddit's analytics attributes carry, which states the post's markings.
#[derive(Debug, serde::Deserialize)]
struct TrackingContext {
    post: Option<TrackedPost>,
}

#[derive(Debug, serde::Deserialize)]
struct TrackedPost {
    id: Option<String>,
    nsfw: Option<bool>,
}

#[derive(Default)]
struct EmbedState {
    /// `t3_{id}`, the post's full name.
    fullname: String,
    /// The `id` of the element holding a text post's text.
    text_element_id: String,
    /// The post's NSFW marking, once a tracking context naming the post has been read.
    nsfw: Option<bool>,
    spoiler: bool,
    image_url: Option<String>,
    /// Open elements inside the text element, while reading it.
    text_depth: Option<usize>,
    text_read: bool,
    text: String,
}

/// Elements with no end tag, which must not count towards `text_depth`.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Elements whose end starts a new line of text.
const BLOCK_ELEMENTS: &[&str] = &[
    "p",
    "div",
    "li",
    "blockquote",
    "pre",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "tr",
    "table",
    "ul",
    "ol",
];

impl EmbedState {
    fn start_tag(&mut self, name: &str, attrs: &[html5ever::Attribute], self_closing: bool) {
        if name == "shreddit-embed-spoiler-button" {
            self.spoiler = true;
        }
        let attr = |key: &str| {
            attrs
                .iter()
                .find(|a| a.name.local.as_ref() == key)
                .map(|a| a.value.as_ref())
        };
        if self.nsfw.is_none()
            && let Some(context) = attr("data-faceplate-tracking-context")
            && let Ok(context) = serde_json::from_str::<TrackingContext>(context)
            && let Some(post) = context.post
            && post.id.as_deref() == Some(self.fullname.as_str())
        {
            // A context without the marking counts as marked.
            self.nsfw = Some(post.nsfw.unwrap_or(true));
        }
        // The post's media follows the context in the page; what precedes it is the
        // subreddit's header.
        if self.image_url.is_none()
            && self.nsfw.is_some()
            && matches!(name, "img" | "faceplate-img")
            && let Some(src) = attr("src")
            && let Ok(src) = Url::parse(src)
            && src.scheme() == "https"
            && src
                .host_str()
                .is_some_and(|host| PICTURE_HOSTS.contains(&host))
        {
            self.image_url = Some(src.to_string());
        }
        let void = self_closing || VOID_ELEMENTS.contains(&name);
        match self.text_depth.as_mut() {
            Some(depth) if !void => *depth += 1,
            Some(_) if name == "br" => self.text.push('\n'),
            Some(_) => {}
            None if !self.text_read
                && !void
                && attr("id") == Some(self.text_element_id.as_str()) =>
            {
                self.text_depth = Some(1);
                self.text_read = true;
            }
            None => {}
        }
    }

    fn end_tag(&mut self, name: &str) {
        let Some(depth) = self.text_depth.as_mut() else {
            return;
        };
        if VOID_ELEMENTS.contains(&name) {
            return;
        }
        if BLOCK_ELEMENTS.contains(&name) {
            self.text.push('\n');
        }
        *depth -= 1;
        if *depth == 0 {
            self.text_depth = None;
        }
    }

    fn into_post(self) -> EmbedPost {
        if self.nsfw != Some(false) || self.spoiler {
            return EmbedPost::default();
        }
        EmbedPost {
            image_url: self.image_url,
            text: one_line(&self.text, MAX_DESCRIPTION_CHARS),
        }
    }
}

/// `text`'s words on one line, cut to `max_chars` with an ellipsis; `None` when it has none.
fn one_line(text: &str, max_chars: usize) -> Option<String> {
    let mut line = String::new();
    let mut chars = 0;
    for word in text.split_whitespace() {
        let needed = word.chars().count() + usize::from(!line.is_empty());
        if chars + needed > max_chars {
            // A first word longer than the limit is cut inside the word.
            if line.is_empty() {
                line.extend(word.chars().take(max_chars));
            }
            line.push('…');
            return Some(line);
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
        chars += needed;
    }
    (!line.is_empty()).then_some(line)
}

struct EmbedSink {
    state: RefCell<EmbedState>,
}

impl TokenSink for EmbedSink {
    type Handle = ();

    fn process_token(&self, token: Token, _line: u64) -> TokenSinkResult<Self::Handle> {
        let mut state = self.state.borrow_mut();
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                match tag.kind {
                    TagKind::StartTag => {
                        state.start_tag(name, &tag.attrs, tag.self_closing);
                        // The page's scripts hold markup in strings; read them as text.
                        match name {
                            "script" => return TokenSinkResult::RawData(RawKind::ScriptData),
                            "style" => return TokenSinkResult::RawData(RawKind::Rawtext),
                            _ => {}
                        }
                    }
                    TagKind::EndTag => state.end_tag(name),
                }
            }
            Token::CharacterTokens(data) if state.text_depth.is_some() => {
                state.text.push_str(data.as_ref());
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

fn parse_embed_page(page: &str, id: &str) -> Option<EmbedPost> {
    let fullname = format!("t3_{id}");
    let sink = EmbedSink {
        state: RefCell::new(EmbedState {
            text_element_id: format!("{fullname}-post-rtjson-content"),
            fullname,
            ..EmbedState::default()
        }),
    };
    let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
    let input = BufferQueue::default();
    input.push_back(StrTendril::from(page));
    let _ = tokenizer.feed(&input);
    tokenizer.end();
    let state = tokenizer.sink.state.into_inner();
    // A page that never names the post is not its embed page.
    state.nsfw.is_some().then(|| state.into_post())
}

// ---------------------------------------------------------------------------
// The card
// ---------------------------------------------------------------------------

/// The card: the post's title, `r/{subreddit} · u/{author}` above it, a text post's text, and
/// the post's picture.
fn card(
    oembed: &OembedResponse,
    subreddit: Option<&str>,
    embed: Option<EmbedPost>,
) -> ParsedMetadata {
    let author = oembed
        .author_name
        .as_deref()
        .map(str::trim)
        // A deleted account is `[deleted]`, which is no one's name.
        .filter(|name| !name.is_empty() && !name.starts_with('['));
    let site_name = match (subreddit, author) {
        (Some(subreddit), Some(author)) => format!("r/{subreddit} · u/{author}"),
        (Some(subreddit), None) => format!("r/{subreddit}"),
        (None, Some(author)) => format!("u/{author}"),
        (None, None) => "Reddit".to_owned(),
    };
    let embed = embed.unwrap_or_default();
    ParsedMetadata {
        title: oembed.title.as_deref().map(|t| t.trim().to_owned()),
        description: embed.text,
        site_name: Some(site_name),
        image_url: embed.image_url,
        theme_color: Some(THEME_COLOR.to_owned()),
        direct_image: false,
        video: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(url: &str) -> Target {
        target_of(&Url::parse(url).unwrap())
    }

    fn post_ref(subreddit: Option<&str>, id: &str) -> Target {
        Target::Post(PostRef {
            subreddit: subreddit.map(str::to_owned),
            id: id.to_owned(),
        })
    }

    #[test]
    fn links_to_posts_name_the_post() {
        assert_eq!(
            target("https://www.reddit.com/r/rust/comments/1wxtufi/made_a_desk_status_monitor/"),
            post_ref(Some("rust"), "1wxtufi")
        );
        assert_eq!(
            target("https://old.reddit.com/r/pics/comments/1wte1my/x/pcufytc/?context=3"),
            post_ref(Some("pics"), "1wte1my")
        );
        assert_eq!(
            target("https://www.reddit.com/r/pics/comments/1wte1my/comment/pcufytc/"),
            post_ref(Some("pics"), "1wte1my")
        );
        assert_eq!(target("https://redd.it/1wxtufi"), post_ref(None, "1wxtufi"));
        assert_eq!(
            target("https://www.reddit.com/gallery/1wte1my"),
            post_ref(None, "1wte1my")
        );
        assert_eq!(
            target("https://reddit.com/comments/1wte1my/"),
            post_ref(None, "1wte1my")
        );
        assert_eq!(
            target("https://www.reddit.com/user/someone/comments/1abc23/title/"),
            post_ref(None, "1abc23")
        );
    }

    #[test]
    fn share_links_and_video_addresses_redirect() {
        assert_eq!(
            target("https://www.reddit.com/r/rust/s/AbCdEf123"),
            Target::Redirect
        );
        assert_eq!(target("https://v.redd.it/c7s1rctplkth1"), Target::Redirect);
        assert_eq!(
            target("https://www.reddit.com/video/c7s1rctplkth1"),
            Target::Redirect
        );
    }

    #[test]
    fn other_pages_are_not_posts() {
        assert_eq!(target("https://www.reddit.com/r/rust/"), Target::Other);
        assert_eq!(
            target("https://www.reddit.com/user/someone/"),
            Target::Other
        );
        assert_eq!(target("https://www.reddit.com/"), Target::Other);
        assert_eq!(
            target("https://www.reddit.com/r/rust/comments/NOT-AN-ID/"),
            Target::Other
        );
    }

    #[test]
    fn a_bad_subreddit_is_dropped_rather_than_sent() {
        assert_eq!(
            target("https://www.reddit.com/r/a%2Fb/comments/1wxtufi/"),
            post_ref(None, "1wxtufi")
        );
    }

    #[test]
    fn only_reddit_pages_are_reddit() {
        assert!(is_reddit(
            &Url::parse("https://www.reddit.com/r/rust/").unwrap()
        ));
        assert!(is_reddit(&Url::parse("https://redd.it/abc").unwrap()));
        assert!(!is_reddit(
            &Url::parse("https://i.redd.it/abc.jpeg").unwrap()
        ));
        assert!(!is_reddit(
            &Url::parse("https://notreddit.com/r/x/").unwrap()
        ));
    }

    #[test]
    fn the_subreddit_comes_from_the_oembed_link() {
        let html = "<blockquote class=\"reddit-embed-bq\">\n<a href=\"https://www.reddit.com/r/rust/comments/1wxtufi/made_a_desk_status_monitor/\">Made a desk status monitor</a><br> by\n<a href=\"https://www.reddit.com/user/Gugu108/\">u/Gugu108</a></blockquote>";
        assert_eq!(subreddit_from_html(html).as_deref(), Some("rust"));
        assert_eq!(
            subreddit_from_html("<a href=\"https://example.com\">"),
            None
        );
    }

    /// The shape of an embed page: the subreddit's header, a tracking context naming the
    /// post, its media, and a script holding markup in a string.
    fn page(context: &str, body: &str) -> String {
        format!(
            r#"<html><head><script>var s = "<img src='https://i.redd.it/fromscript.png'>";</script>
<style>.x{{content:"<div id='t3_abc-post-rtjson-content'>"}}</style></head><body>
<img src="https://styles.redditmedia.com/t5_1/styles/communityIcon.png">
<img src="https://b.thumbs.redditmedia.com/header.png">
<a id="embed-title"><faceplate-tracker data-faceplate-tracking-context="{context}"><h1>Title</h1></faceplate-tracker></a>
{body}
<img src="https://id.rlcdn.com/472486.gif"></body></html>"#
        )
    }

    fn context(nsfw: &str) -> String {
        format!(
            "{{&quot;post&quot;:{{&quot;id&quot;:&quot;t3_abc&quot;,&quot;nsfw&quot;:{nsfw},&quot;type&quot;:&quot;image&quot;}},&quot;subreddit&quot;:{{&quot;name&quot;:&quot;pics&quot;}}}}"
        )
    }

    const PICTURE: &str = r#"<faceplate-img src="https://preview.redd.it/a-v0-abc.jpeg?width=640&amp;crop=smart&amp;auto=webp&amp;s=f33d"></faceplate-img>"#;

    #[test]
    fn a_picture_post_gives_its_picture() {
        let post = parse_embed_page(&page(&context("false"), PICTURE), "abc").unwrap();
        assert_eq!(
            post.image_url.as_deref(),
            Some("https://preview.redd.it/a-v0-abc.jpeg?width=640&crop=smart&auto=webp&s=f33d")
        );
        assert_eq!(post.text, None);
    }

    #[test]
    fn a_text_post_gives_its_text_on_one_line() {
        let body = r#"<div id="t3_abc-post-rtjson-content" dir="auto"><h1>Poll</h1><p> I&#39;ve started <strong>to dip</strong> my toes.</p><p>Second<br>line</p><img src="https://preview.redd.it/inline.png"></div><p>Not the post</p>"#;
        let post = parse_embed_page(&page(&context("false"), body), "abc").unwrap();
        assert_eq!(
            post.text.as_deref(),
            Some("Poll I've started to dip my toes. Second line")
        );
    }

    #[test]
    fn nsfw_and_spoiler_posts_give_nothing_but_their_title() {
        let nsfw = parse_embed_page(&page(&context("true"), PICTURE), "abc").unwrap();
        assert_eq!(nsfw, EmbedPost::default());
        let unmarked = parse_embed_page(&page(&context("null"), PICTURE), "abc").unwrap();
        assert_eq!(unmarked, EmbedPost::default());
        let spoiler = format!(
            r#"{PICTURE}<div class="spoiler-button"><shreddit-embed-spoiler-button></shreddit-embed-spoiler-button></div>"#
        );
        let spoiler = parse_embed_page(&page(&context("false"), &spoiler), "abc").unwrap();
        assert_eq!(spoiler, EmbedPost::default());
    }

    #[test]
    fn a_page_about_another_post_is_not_read() {
        assert_eq!(
            parse_embed_page(&page(&context("false"), PICTURE), "xyz"),
            None
        );
        assert_eq!(
            parse_embed_page("<html><body>gone</body></html>", "abc"),
            None
        );
    }

    #[test]
    fn long_text_is_cut_at_a_word() {
        assert_eq!(
            one_line("  one two\n three ", 20).as_deref(),
            Some("one two three")
        );
        assert_eq!(one_line("one two three", 8).as_deref(), Some("one two…"));
        assert_eq!(one_line("abcdefghij", 4).as_deref(), Some("abcd…"));
        assert_eq!(one_line(" \n ", 4), None);
    }

    #[test]
    fn the_card_names_the_subreddit_and_author() {
        let oembed = OembedResponse {
            title: Some(" Made a desk status monitor ".to_owned()),
            author_name: Some("Gugu108".to_owned()),
            html: None,
        };
        let card = card(&oembed, Some("rust"), None);
        assert_eq!(card.title.as_deref(), Some("Made a desk status monitor"));
        assert_eq!(card.site_name.as_deref(), Some("r/rust · u/Gugu108"));
        assert_eq!(card.theme_color.as_deref(), Some(THEME_COLOR));
        assert_eq!(card.image_url, None);
        let deleted = OembedResponse {
            author_name: Some("[deleted]".to_owned()),
            ..oembed
        };
        assert_eq!(
            super::card(&deleted, Some("rust"), None)
                .site_name
                .as_deref(),
            Some("r/rust")
        );
    }

    #[test]
    fn rate_limit_headers_pause_embed_pages() {
        let mut headers = reqwest::header::HeaderMap::new();
        assert!(!allowance_spent(&headers));
        assert_eq!(reset_after(&headers), DEFAULT_PAUSE);
        headers.insert("x-ratelimit-remaining", "0.0".parse().unwrap());
        headers.insert("x-ratelimit-reset", "24".parse().unwrap());
        assert!(allowance_spent(&headers));
        assert_eq!(reset_after(&headers), Duration::from_secs(24));
        headers.insert("x-ratelimit-reset", "999999".parse().unwrap());
        assert_eq!(reset_after(&headers), MAX_PAUSE);
        headers.insert("x-ratelimit-remaining", "199.0".parse().unwrap());
        assert!(!allowance_spent(&headers));
    }
}
