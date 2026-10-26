//! Reading a page's preview metadata (Open Graph, Twitter Card, `<title>`, description, and
//! `theme-color`) out of its `<head>` with `html5ever`'s tokenizer.
//!
//! A page whose head gives no description (Wikipedia's articles give none) is described by its
//! lead paragraph instead: the first `<p>` of its main content, `<main>`, `<article>`, or the
//! element whose `role` is `main`, that has at least [`MIN_LEAD_CHARS`] characters of shown text.
//! Text inside elements a reader does not take as the paragraph's (footnote markers in `<sup>`,
//! tables, figures and their captions, asides, navigation, scripts, and anything `hidden`) is left
//! out. Only the first such container is read, so a page listing many articles is not described
//! by whichever comes first in the list, and nothing outside one is read, so a cookie banner or a
//! site's navigation never becomes the description.

use aspen_wire::link_preview::VideoEmbed;
use html5ever::Attribute;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::states::RawKind;
use html5ever::tokenizer::{
    BufferQueue, Tag, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
};
use std::cell::RefCell;

#[derive(Debug, Clone, Default)]
pub struct ParsedMetadata {
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub image_url: Option<String>,
    pub theme_color: Option<String>,
    /// The URL itself served an image rather than a page. `image_url` is that URL, and the
    /// preview has no text: clients show the picture inline, as they would an attachment.
    pub direct_image: bool,
    /// A player for the link, when it is a video on an allowlisted provider.
    pub video: Option<VideoEmbed>,
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
    /// anyone's guess. A link that is itself an image is the exception: the
    /// picture is the whole point, and clients show it inline.
    pub fn has_content(&self) -> bool {
        self.direct_image
            || self.video.is_some()
            || self.title.is_some()
            || self.description.is_some()
    }
}

/// The `<meta>` keys [`MetaState::into_metadata`] reads; every other is dropped as it is met,
/// so a page of many thousands of tags holds no more than these.
const KNOWN_KEYS: [&str; 11] = [
    "og:title",
    "twitter:title",
    "og:description",
    "twitter:description",
    "description",
    "og:site_name",
    "og:image",
    "og:image:url",
    "og:image:secure_url",
    "twitter:image",
    "twitter:image:src",
];

/// The most `theme-color` tags kept; the choice among them needs only the first of each kind.
const MAX_THEME_COLORS: usize = 8;

/// The most bytes of `<title>` text kept, far more than a card shows.
const MAX_TITLE_BYTES: usize = 4096;

/// The most characters of a preview's title, description, and site name a card is given; the
/// rest is cut, with an ellipsis.
pub const MAX_TITLE_CHARS: usize = 300;
pub const MAX_DESCRIPTION_CHARS: usize = 1000;
pub const MAX_SITE_NAME_CHARS: usize = 100;

/// The longest `theme-color` kept, in bytes.
const MAX_THEME_COLOR_BYTES: usize = 64;

/// The CSS colour functions a `theme-color` may be written with.
const COLOR_FUNCTIONS: [&str; 9] = [
    "rgb", "rgba", "hsl", "hsla", "hwb", "lab", "lch", "oklab", "oklch",
];

/// Elements with no end tag, which open nothing.
pub(crate) const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// The fewest characters a lead paragraph has, so a byline, a date, or a one-line caption written
/// as a `<p>` is passed over for the paragraph after it.
const MIN_LEAD_CHARS: usize = 50;

/// The bytes of a lead paragraph's text past which no more is kept, ample for
/// [`MAX_DESCRIPTION_CHARS`].
const MAX_LEAD_BYTES: usize = 16 * 1024;

/// The most elements open at once inside the main content that the search for its lead paragraph
/// follows; a page nested deeper is given no lead paragraph.
const MAX_OPEN_ELEMENTS: usize = 256;

/// Elements whose text is not part of a paragraph around them, as a reader takes it.
const HIDING_ELEMENTS: &[&str] = &[
    "aside", "audio", "button", "canvas", "dialog", "figure", "footer", "form", "header", "iframe",
    "math", "nav", "noscript", "object", "script", "select", "style", "sup", "svg", "table",
    "template", "textarea", "video",
];

/// Elements whose start ends an open `<p>`, as HTML's parsing rules have it.
const PARAGRAPH_ENDERS: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "li",
    "listing",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "search",
    "section",
    "summary",
    "table",
    "ul",
    "xmp",
];

/// How the tokenizer reads what follows an element's start tag, for the elements whose content
/// is text rather than markup: a script's `"<p>"` in a string is not a paragraph.
fn raw_kind(name: &str) -> Option<RawKind> {
    match name {
        "script" => Some(RawKind::ScriptData),
        "style" | "xmp" | "iframe" | "noembed" | "noframes" | "noscript" => Some(RawKind::Rawtext),
        "title" | "textarea" => Some(RawKind::Rcdata),
        _ => None,
    }
}

/// `text`'s words on one line, cut to `max_chars` with an ellipsis; `None` when it has none.
pub(crate) fn one_line(text: &str, max_chars: usize) -> Option<String> {
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

/// `text` cut to `max_chars` characters, with an ellipsis when anything was cut.
fn cut(text: String, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((end, _)) => {
            let mut cut = text[..end].trim_end().to_owned();
            cut.push('…');
            cut
        }
        None => text,
    }
}

/// `value` when it is a CSS colour as a page's `theme-color` gives one: a hex colour of 3, 4, 6,
/// or 8 digits, a colour keyword (letters alone), or one of [`COLOR_FUNCTIONS`] of numbers,
/// percentages, angles, and separators. Clients set it as a style, so nothing else is passed on.
fn css_color(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_THEME_COLOR_BYTES {
        return None;
    }
    let valid = if let Some(hex) = value.strip_prefix('#') {
        matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
    } else if let Some((function, rest)) = value.split_once('(') {
        COLOR_FUNCTIONS.contains(&function.to_ascii_lowercase().as_str())
            && rest.strip_suffix(')').is_some_and(|args| {
                args.bytes().all(|b| {
                    b.is_ascii_alphanumeric()
                        || matches!(b, b'.' | b',' | b'%' | b'/' | b' ' | b'-' | b'+')
                })
            })
    } else {
        value.bytes().all(|b| b.is_ascii_alphabetic())
    };
    valid.then(|| value.to_owned())
}

impl ParsedMetadata {
    /// The metadata as a card is given it: its texts cut to [`MAX_TITLE_CHARS`],
    /// [`MAX_DESCRIPTION_CHARS`], and [`MAX_SITE_NAME_CHARS`], and its theme colour kept only
    /// when it is a CSS colour.
    pub fn bounded(self) -> Self {
        Self {
            title: self.title.map(|t| cut(t, MAX_TITLE_CHARS)),
            description: self.description.map(|d| cut(d, MAX_DESCRIPTION_CHARS)),
            site_name: self.site_name.map(|s| cut(s, MAX_SITE_NAME_CHARS)),
            theme_color: self.theme_color.as_deref().and_then(css_color),
            ..self
        }
    }
}

/// An element open inside the main content.
struct OpenElement {
    name: String,
    /// Whether its text, and its descendants', is left out of paragraphs.
    hides: bool,
}

/// The search for a page's lead paragraph (see the module's documentation).
#[derive(Default)]
struct LeadSearch {
    /// The elements open inside the main content, its container first; empty until the
    /// container starts.
    open: Vec<OpenElement>,
    /// How many of `open` hide their text.
    hiding: usize,
    /// The text read so far of the open paragraph, when it is shown.
    paragraph: Option<String>,
    found: Option<String>,
    /// A paragraph was found, the container ended, or the search was given up.
    finished: bool,
}

impl LeadSearch {
    fn is_main_content(name: &str, attrs: &[Attribute]) -> bool {
        matches!(name, "main" | "article")
            || attr(attrs, "role").is_some_and(|role| {
                role.split_ascii_whitespace()
                    .any(|role| role.eq_ignore_ascii_case("main"))
            })
    }

    fn hides(name: &str, attrs: &[Attribute]) -> bool {
        HIDING_ELEMENTS.contains(&name)
            || attr(attrs, "hidden").is_some()
            || attr(attrs, "aria-hidden").is_some_and(|value| value.eq_ignore_ascii_case("true"))
    }

    fn start_tag(&mut self, name: &str, attrs: &[Attribute], self_closing: bool) {
        if self.finished {
            return;
        }
        let void = self_closing || VOID_ELEMENTS.contains(&name);
        if self.open.is_empty() {
            if !void && Self::is_main_content(name, attrs) {
                self.open.push(OpenElement {
                    name: name.to_owned(),
                    hides: false,
                });
            }
            return;
        }
        if PARAGRAPH_ENDERS.contains(&name)
            && let Some(paragraph) = self.open.iter().rposition(|e| e.name == "p")
        {
            self.close_from(paragraph);
            if self.finished {
                return;
            }
        }
        if name == "br" {
            self.text(" ");
        }
        if void {
            return;
        }
        if self.open.len() >= MAX_OPEN_ELEMENTS {
            self.finished = true;
            return;
        }
        let hides = Self::hides(name, attrs);
        self.hiding += usize::from(hides);
        self.open.push(OpenElement {
            name: name.to_owned(),
            hides,
        });
        if name == "p" && self.hiding == 0 {
            self.paragraph = Some(String::new());
        }
    }

    /// An end tag closes the innermost open element of its name and every element opened inside
    /// it; one that matches no open element is ignored, as browsers ignore it.
    fn end_tag(&mut self, name: &str) {
        if self.finished {
            return;
        }
        if let Some(element) = self.open.iter().rposition(|e| e.name == name) {
            self.close_from(element);
        }
    }

    fn close_from(&mut self, index: usize) {
        for element in self.open.split_off(index) {
            self.hiding -= usize::from(element.hides);
            if element.name == "p" {
                self.end_paragraph();
            }
        }
        if self.open.is_empty() {
            self.finished = true;
        }
    }

    fn end_paragraph(&mut self) {
        if let Some(text) = self.paragraph.take()
            && let Some(line) = one_line(&text, MAX_DESCRIPTION_CHARS)
            && line.chars().count() >= MIN_LEAD_CHARS
        {
            self.found = Some(line);
            self.finished = true;
        }
    }

    fn text(&mut self, text: &str) {
        if self.hiding == 0
            && let Some(paragraph) = self.paragraph.as_mut()
            && paragraph.len() < MAX_LEAD_BYTES
        {
            paragraph.push_str(text);
        }
    }
}

fn attr<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|a| a.name.local.as_ref().eq_ignore_ascii_case(name))
        .map(|a| a.value.as_ref())
}

#[derive(Default)]
struct MetaState {
    /// `(key_lower, value)` pairs of [`KNOWN_KEYS`], first writer wins so the HTML's first
    /// occurrence of each canonical metadata key beats any fallbacks.
    meta: Vec<(&'static str, String)>,
    /// `(media_attr, value)` pairs for `<meta name="theme-color">` tags, at most
    /// [`MAX_THEME_COLORS`]. `None` for the `media` field means the tag applied unconditionally.
    theme_colors: Vec<(Option<String>, String)>,
    title_chunks: Vec<String>,
    /// The bytes of `title_chunks`, at most [`MAX_TITLE_BYTES`] (one more once text was dropped).
    title_bytes: usize,
    in_title: bool,
    /// The `<body>` started: the head's tags are all read, and only the lead paragraph is
    /// searched for, when the head gave no description.
    in_body: bool,
    lead: LeadSearch,
}

impl MetaState {
    fn has_description(&self) -> bool {
        ["og:description", "twitter:description", "description"]
            .iter()
            .any(|key| self.lookup(key).is_some())
    }

    fn head_tag(&mut self, tag: &Tag) {
        match (tag.kind, tag.name.as_ref()) {
            (TagKind::StartTag, "body") => {
                self.in_body = true;
                self.in_title = false;
                if self.has_description() {
                    self.lead.finished = true;
                }
            }
            (TagKind::StartTag, "title") => {
                self.in_title = true;
            }
            (TagKind::EndTag, "title") => {
                self.in_title = false;
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
                    return;
                };
                let key = if !property_attr.is_empty() {
                    property_attr
                } else {
                    name_attr
                };
                if key.is_empty() {
                    return;
                }
                if key == "theme-color" {
                    if self.theme_colors.len() < MAX_THEME_COLORS {
                        self.theme_colors.push((media_attr, content));
                    }
                } else {
                    self.insert_meta(&key, &content);
                }
            }
            _ => {}
        }
    }

    fn title_text(&mut self, text: &str) {
        // Text past the cap is dropped, and with it everything after it.
        if self.title_bytes + text.len() <= MAX_TITLE_BYTES {
            self.title_bytes += text.len();
            self.title_chunks.push(text.to_owned());
        } else {
            self.title_bytes = MAX_TITLE_BYTES + 1;
        }
    }

    fn lookup(&self, key: &str) -> Option<&str> {
        self.meta
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, v)| v.as_str())
    }

    /// A blank `value` is no value: a page that writes `<meta property="og:title" content="">`
    /// leaves the title to the next source rather than giving the card an empty one.
    fn insert_meta(&mut self, key: &str, value: &str) {
        let Some(known) = KNOWN_KEYS.iter().find(|known| **known == key) else {
            return;
        };
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        if !self.meta.iter().any(|(k, _)| k == known) {
            self.meta.push((known, value.to_owned()));
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

    fn into_metadata(mut self) -> ParsedMetadata {
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
            .map(ToOwned::to_owned)
            .or(self.lead.found.take());
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
            direct_image: false,
            video: None,
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
        if state.in_body && state.lead.finished {
            return TokenSinkResult::Continue;
        }
        match token {
            Token::TagToken(tag) => {
                let name = tag.name.as_ref();
                match tag.kind {
                    TagKind::StartTag => state.lead.start_tag(name, &tag.attrs, tag.self_closing),
                    TagKind::EndTag => state.lead.end_tag(name),
                }
                if !state.in_body {
                    state.head_tag(&tag);
                }
                if tag.kind == TagKind::StartTag
                    && let Some(kind) = raw_kind(name)
                {
                    return TokenSinkResult::RawData(kind);
                }
            }
            Token::CharacterTokens(data) => {
                if state.in_title {
                    state.title_text(&data);
                }
                state.lead.text(&data);
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

/// The page's metadata. Tokenizing a page of up to `fetch::MAX_METADATA_BYTES` is work for a
/// blocking thread, where `fetch` runs it.
pub fn parse_html_metadata(body: &str) -> ParsedMetadata {
    let sink = MetaSink {
        state: RefCell::new(MetaState::default()),
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

#[cfg(test)]
mod tests {
    use super::*;

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

    /// rust-lang.org's home page has this head: empty card titles over a real `<title>`.
    #[test]
    fn blank_meta_falls_through_to_the_next_source() {
        let html = r#"
<html><head>
  <title>
          Rust Programming Language
      </title>
  <meta name="twitter:title" content="">
  <meta property="og:title" content="" />
  <meta property="og:site_name" content="  ">
  <meta name="description" content="  Plain description ">
  <meta property="og:description" content="">
  <meta property="og:description" content="Later description">
</head></html>
"#;
        let meta = parse_html_metadata(html);
        assert_eq!(meta.title.as_deref(), Some("Rust Programming Language"));
        assert_eq!(meta.site_name, None);
        assert_eq!(meta.description.as_deref(), Some("Later description"));
    }

    #[test]
    fn many_tags_keep_only_what_is_read() {
        let mut html = String::from("<head>");
        for i in 0..10_000 {
            html.push_str(&format!(r#"<meta name="k{i}" content="v">"#));
            html.push_str(r##"<meta name="theme-color" content="#123456">"##);
        }
        html.push_str(r#"<meta property="og:title" content="Late"></head>"#);
        let sink = MetaSink {
            state: RefCell::new(MetaState::default()),
        };
        let tokenizer = Tokenizer::new(sink, TokenizerOpts::default());
        let input = BufferQueue::default();
        input.push_back(StrTendril::from(html.as_str()));
        let _ = tokenizer.feed(&input);
        tokenizer.end();
        let state = tokenizer.sink.state.into_inner();
        assert_eq!(state.meta.len(), 1);
        assert_eq!(state.theme_colors.len(), MAX_THEME_COLORS);
        assert_eq!(state.into_metadata().title.as_deref(), Some("Late"));
    }

    #[test]
    fn bounded_cuts_texts_and_keeps_only_colours() {
        let meta = ParsedMetadata {
            title: Some("t".repeat(MAX_TITLE_CHARS + 5)),
            description: Some("短".repeat(MAX_DESCRIPTION_CHARS)),
            site_name: Some("s".repeat(MAX_SITE_NAME_CHARS + 1)),
            theme_color: Some("red; background: url(x)".to_owned()),
            ..ParsedMetadata::default()
        }
        .bounded();
        assert_eq!(meta.title.unwrap().chars().count(), MAX_TITLE_CHARS + 1);
        assert_eq!(
            meta.description.unwrap().chars().count(),
            MAX_DESCRIPTION_CHARS
        );
        assert!(meta.site_name.unwrap().ends_with('…'));
        assert_eq!(meta.theme_color, None);
        for colour in [
            "#fff",
            "#A1b2C3",
            "#11223344",
            "rebeccapurple",
            "rgb(1, 2, 3)",
            "oklch(70% 0.1 200 / 50%)",
        ] {
            assert_eq!(css_color(colour).as_deref(), Some(colour), "{colour}");
        }
        for not_colour in [
            "#12345",
            "url(x)",
            "rgb(1,2,3",
            "red blue",
            "var(--x)",
            "expression(alert(1))",
        ] {
            assert_eq!(css_color(not_colour), None, "{not_colour}");
        }
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

    /// Wikipedia's article markup, abridged: no description in the head, and ahead of the lead
    /// paragraph a placeholder `<p>`, a maintenance banner in a table, and a captioned picture.
    #[test]
    fn a_page_without_a_description_gives_its_lead_paragraph() {
        let html = r#"
<html><head><title>Tetraphobia - Wikipedia</title>
<meta property="og:title" content="Tetraphobia - Wikipedia"></head>
<body>
<div class="cookie-notice"><p>We use cookies to make this site work, and to understand how it is used.</p></div>
<main id="content"><div id="bodyContent"><section>
<p class="mw-empty-elt">
</p>
<table role="presentation"><tr><td><p>This article needs additional citations for verification. Please help improve this article.</p></td></tr></table>
<figure><img src="lift.jpg"><figcaption><p>An elevator control panel in a residential apartment building in Shanghai</p></figcaption></figure>
<script>document.write("<p>Not a paragraph of the page, but a string of a script</p>")</script>
<p><b>Tetraphobia</b> (<span>from</span>&nbsp;<a href="/wiki/Ancient_Greek">Ancient Greek</a>
 <span lang="grc">τετράς (tetrás)</span>&nbsp;'four') is the practice of avoiding instances
of the digit number 4.<sup class="reference">[1]</sup></p>
<p>The Chinese word for "four" sounds quite similar to the word for "death".</p>
</section></div></main></body></html>
"#;
        let meta = parse_html_metadata(html);
        assert_eq!(meta.title.as_deref(), Some("Tetraphobia - Wikipedia"));
        assert_eq!(
            meta.description.as_deref(),
            Some(
                "Tetraphobia (from Ancient Greek τετράς (tetrás) 'four') is the practice of \
                 avoiding instances of the digit number 4."
            )
        );
    }

    #[test]
    fn a_description_in_the_head_beats_the_lead_paragraph() {
        let html = r#"<head><meta name="description" content="From the head"></head>
<body><main><p>A paragraph long enough to be a lead paragraph, were the head silent.</p></main></body>"#;
        let meta = parse_html_metadata(html);
        assert_eq!(meta.description.as_deref(), Some("From the head"));
    }

    #[test]
    fn only_shown_paragraphs_of_the_main_content_are_read() {
        let outside = r#"<head></head><body>
<p>A paragraph long enough to be a lead paragraph, but outside the main content.</p>
<div><p>Another paragraph long enough to be a lead paragraph, in a plain division.</p></div>
</body>"#;
        assert_eq!(parse_html_metadata(outside).description, None);

        // A byline is too short; `hidden` and `aria-hidden` text is not shown; a `<p>` left open
        // is ended by the next block, as browsers end it.
        let html = r#"<head></head><body><div role="banner navigation"></div>
<div role="main">
<p>By Jane Doe, 3 October 2026
<div hidden><p>A paragraph long enough to be a lead paragraph, but hidden from readers.</p></div>
<p aria-hidden="true">A paragraph long enough to be a lead paragraph, but hidden from readers.
<p>The lead paragraph, <span aria-hidden="TRUE">(hidden) </span>left open<br>across a line break.
<h2>A heading</h2>
</div></body>"#;
        assert_eq!(
            parse_html_metadata(html).description.as_deref(),
            Some("The lead paragraph, left open across a line break.")
        );
    }

    #[test]
    fn only_the_first_main_content_is_read() {
        let html = r#"<head></head><body>
<article><h2>A post</h2><p>Too short to lead.</p></article>
<article><p>The second article's paragraph, long enough to be a lead paragraph.</p></article>
</body>"#;
        assert_eq!(parse_html_metadata(html).description, None);
    }

    #[test]
    fn the_bodys_titles_are_not_the_pages() {
        let html = r#"<head><title>The page</title></head>
<body><svg><title>An icon</title></svg></body>"#;
        assert_eq!(parse_html_metadata(html).title.as_deref(), Some("The page"));
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
}
