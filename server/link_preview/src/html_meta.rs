//! Reading a page's preview metadata (Open Graph, Twitter Card, `<title>`, description, and
//! `theme-color`) out of its `<head>` with `html5ever`'s tokenizer.

use aspen_wire::link_preview::VideoEmbed;
use html5ever::tendril::StrTendril;
use html5ever::tokenizer::{
    BufferQueue, TagKind, Token, TokenSink, TokenSinkResult, Tokenizer, TokenizerOpts,
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
            Token::CharacterTokens(data) if state.in_title => {
                state.title_chunks.push(data.as_ref().to_owned());
            }
            _ => {}
        }
        TokenSinkResult::Continue
    }
}

pub fn parse_html_metadata(body: &str) -> ParsedMetadata {
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
