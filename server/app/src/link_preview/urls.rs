//! Which links in a message body get previews: explicit markdown links, bare `http(s)://` URLs,
//! and bare domains whose TLD IANA delegates (`tlds.txt`).

use super::MAX_LINK_PREVIEWS_PER_MESSAGE;
use pulldown_cmark::{Event, Tag, TagEnd};
use std::collections::HashSet;
use std::sync::LazyLock;
use url::Url;

/// Extract up to [`MAX_LINK_PREVIEWS_PER_MESSAGE`] unique preview-worthy
/// http(s) URLs from a markdown body. Links to messages of an Aspen
/// deployment are left out: they are shown as the messages themselves
/// (`app::message_link`), never as a page fetched from the web client.
pub fn extract_preview_urls(content: &str) -> Vec<Url> {
    extract_urls(content, MAX_LINK_PREVIEWS_PER_MESSAGE, |url| {
        crate::message_link::message_of(url).is_none()
    })
}

/// Extract up to `limit` unique http(s) URLs from a markdown body that `keep`
/// accepts, in the order the text names them.
///
/// Walks the `pulldown-cmark` event stream:
/// - explicit links (`[text](url)`, `<url>` autolinks) contribute their
///   destination;
/// - bare URLs inside `Event::Text` are scanned for a leading `http://` /
///   `https://` prefix so a user who types a raw link without bracketing
///   it still gets one, matching how GitHub-flavoured markdown autolinks
///   such text at render time;
/// - bare domains inside `Event::Text` whose last label is a TLD IANA
///   delegates (`github.io/pages`) are taken over `https`, under the
///   same rule the client uses to render them as links;
/// - URLs inside `Event::Code` or fenced code blocks are skipped so pasted
///   example snippets don't count.
pub fn extract_urls(content: &str, limit: usize, mut keep: impl FnMut(&Url) -> bool) -> Vec<Url> {
    let mut urls: Vec<Url> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut code_block_depth: u32 = 0;
    let mut push = |urls: &mut Vec<Url>, raw: &str| {
        push_url(urls, &mut seen, raw, &mut keep) && urls.len() >= limit
    };

    for event in crate::markdown::parser(content) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => code_block_depth += 1,
            Event::End(TagEnd::CodeBlock) => {
                code_block_depth = code_block_depth.saturating_sub(1);
            }
            Event::Code(_) => {
                // Explicitly skip inline code so "`curl https://…`" doesn't
                // count.
            }
            _ if code_block_depth > 0 => {}
            Event::Start(Tag::Link { dest_url, .. }) if push(&mut urls, dest_url.as_ref()) => {
                return urls;
            }
            Event::Text(text) => {
                for candidate in scan_bare_urls(text.as_ref()) {
                    if push(&mut urls, candidate) {
                        return urls;
                    }
                }
                for candidate in scan_bare_domains(text.as_ref()) {
                    if push(&mut urls, &candidate) {
                        return urls;
                    }
                }
            }
            _ => {}
        }
    }
    urls
}

fn push_url(
    urls: &mut Vec<Url>,
    seen: &mut HashSet<String>,
    raw: &str,
    keep: &mut impl FnMut(&Url) -> bool,
) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https") || !keep(&url) {
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

/// Every TLD IANA delegates, lower case. The client links bare domains against the same list.
static KNOWN_TLDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    include_str!("tlds.txt")
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect()
});

/// TLDs that are far more often file extensions in chat about software (`main.rs`, `setup.py`)
/// than domains. A bare two-label token with one of these is not a link; a port, a path, or a
/// `www.` prefix makes the intent clear and links it anyway. The client keeps the same set.
const EXTENSION_LIKE_TLDS: &[&str] = &["rs", "py", "sh", "md", "pl", "ps"];

/// Characters that may open a token without being part of it.
const BARE_DOMAIN_LEADING: &[char] = &['(', '[', '{', '<', '"', '\'', '`'];

/// Find bare domains such as `github.io/pages` in a plain-text span and return them as
/// `https://` URLs. A token is a whitespace-delimited run; tokens with a scheme belong to
/// [`scan_bare_urls`], and tokens with `@` are email addresses.
fn scan_bare_domains(text: &str) -> Vec<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '`'))
        .filter(|token| !token.contains("://") && !token.contains('@'))
        .filter_map(bare_domain_url)
        .collect()
}

/// The `https://` URL for a token that is a bare domain with an optional port and path, or
/// `None` when the token is anything else.
fn bare_domain_url(token: &str) -> Option<String> {
    let token = token.trim_start_matches(BARE_DOMAIN_LEADING);
    let mut end = token.len();
    // The parentheses left in the token, counted once and kept as its end is trimmed, so a
    // token ending in many of them is read in one pass.
    let opening = token.matches('(').count();
    let mut closing = token.matches(')').count();
    loop {
        let last = token[..end].chars().next_back()?;
        let unbalanced_paren = last == ')' && opening < closing;
        if BARE_URL_TRAILING.contains(&last) && (last != ')' || unbalanced_paren) {
            end -= last.len_utf8();
            if last == ')' {
                closing -= 1;
            }
        } else {
            break;
        }
    }
    let token = &token[..end];
    let (host_port, path) = match token.find('/') {
        Some(i) => (&token[..i], &token[i..]),
        None => (token, ""),
    };
    let (host, port) = match host_port.find(':') {
        Some(i) => (&host_port[..i], Some(&host_port[i + 1..])),
        None => (host_port, None),
    };
    if let Some(port) = port
        && (port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()))
    {
        return None;
    }
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() < 2
        || labels.iter().any(|label| {
            label.is_empty()
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
    {
        return None;
    }
    let tld = labels[labels.len() - 1].to_ascii_lowercase();
    if tld.len() < 2 || !tld.bytes().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    if !KNOWN_TLDS.contains(tld.as_str()) {
        return None;
    }
    if EXTENSION_LIKE_TLDS.contains(&tld.as_str()) {
        let explicit_enough =
            port.is_some() || !path.is_empty() || host.to_ascii_lowercase().starts_with("www.");
        if !explicit_enough {
            return None;
        }
    }
    Some(format!("https://{token}"))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_parentheses_are_trimmed_in_one_pass() {
        assert_eq!(
            bare_domain_url("example.com/a_(b))").as_deref(),
            Some("https://example.com/a_(b)")
        );
        let many = format!("example.com/{}", ")".repeat(200_000));
        assert_eq!(
            bare_domain_url(&many).as_deref(),
            Some("https://example.com/")
        );
    }

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

    /// Mirrors the client's `linkify` cases so the two rule sets stay aligned.
    #[test]
    fn extracts_bare_domains_with_known_tlds() {
        assert_eq!(
            urls_from("try facebook.com or GitHub.io/pages, then docs.rs/serde."),
            vec![
                "https://facebook.com/".to_string(),
                "https://github.io/pages".to_string(),
                "https://docs.rs/serde".to_string(),
            ],
        );
        assert_eq!(
            urls_from("(see example.org/a(b)). localhost:5173/x and www.onet.pl"),
            vec![
                "https://example.org/a(b)".to_string(),
                "https://www.onet.pl/".to_string(),
            ],
        );
    }

    #[test]
    fn leaves_file_names_emails_versions_and_unknown_tlds_alone() {
        for content in [
            "edit main.rs and setup.py",
            "mail me at kate@example.com",
            "bump to 1.2.3 or v2.0",
            "not a site: thing.notatld",
            "e.g. this",
        ] {
            assert!(urls_from(content).is_empty(), "{content}");
        }
    }

    #[test]
    fn does_not_extract_a_domain_from_inside_an_explicit_url() {
        assert_eq!(
            urls_from("https://example.com/see/github.io"),
            vec!["https://example.com/see/github.io".to_string()],
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
}
