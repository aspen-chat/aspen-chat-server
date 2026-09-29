//! The language each request is answered in. A request names the languages its reader prefers
//! in `Accept-Language` (which the client sets from its own language setting); the first that
//! `locales/` has, or has the language of, is the request's locale for as long as it is handled,
//! and every `t!` reads it. Anything done outside a request speaks [`DEFAULT`].
//!
//! Two pseudo-locales are made from English, so that anyone can see which text has not been
//! localized and how the interface takes longer text and right-to-left layout: [`ACCENTED`]
//! (`en-XA`) accents every letter, lengthens each word, and brackets the whole, and
//! [`MIRRORED`] (`ar-XB`) sets each word right to left. `spec/pseudo_locale_vectors.json` holds
//! cases the client's pseudo-locales pass too.

use axum::extract::Request;
use axum::http::HeaderValue;
use axum::http::header::{ACCEPT_LANGUAGE, CONTENT_LANGUAGE, VARY};
use axum::middleware::Next;
use axum::response::Response;
use std::collections::HashMap;
use std::future::Future;
use std::sync::{LazyLock, RwLock};

/// The locale of anything not answering a request, and of a request naming none this server has.
pub const DEFAULT: &str = "en";
/// English with every letter accented and every word lengthened, in brackets.
pub const ACCENTED: &str = "en-XA";
/// English with every word set right to left.
pub const MIRRORED: &str = "ar-XB";

/// How many languages of an `Accept-Language` are considered; a longer list is cut short.
const MAX_PREFERENCES: usize = 16;

tokio::task_local! {
    static LOCALE: &'static str;
}

/// The locale of the request being handled, or [`DEFAULT`] outside one.
pub fn current() -> &'static str {
    LOCALE.try_with(|locale| *locale).unwrap_or(DEFAULT)
}

/// Runs `future` speaking `locale`.
pub async fn scope<F: Future>(locale: &'static str, future: F) -> F::Output {
    LOCALE.scope(locale, future).await
}

/// Every locale this server speaks.
fn available() -> &'static [&'static str] {
    static AVAILABLE: LazyLock<Vec<&'static str>> =
        LazyLock::new(crate::_rust_i18n_available_locales);
    &AVAILABLE
}

/// The locale to answer `accept_language` in: its languages in order of preference, each
/// matched whole and then with its subtags taken off from the end (`en-GB` finds `en`), and
/// [`DEFAULT`] when none matches.
pub fn negotiate(accept_language: &str) -> &'static str {
    negotiate_among(accept_language, available())
}

fn negotiate_among(accept_language: &str, available: &[&'static str]) -> &'static str {
    let mut preferences: Vec<(&str, f32)> = accept_language
        .split(',')
        .take(MAX_PREFERENCES)
        .filter_map(|entry| {
            let mut parts = entry.split(';');
            let tag = parts.next()?.trim();
            let quality = parts
                .find_map(|part| part.trim().strip_prefix("q="))
                .map_or(Some(1.0), |q| q.trim().parse::<f32>().ok())?;
            (!tag.is_empty() && tag != "*" && quality > 0.0).then_some((tag, quality))
        })
        .collect();
    // Stable, so languages of equal quality keep the order they were given in.
    preferences.sort_by(|a, b| b.1.total_cmp(&a.1));
    preferences
        .into_iter()
        .find_map(|(tag, _)| {
            let mut candidate = tag;
            loop {
                if let Some(found) = available
                    .iter()
                    .find(|locale| locale.eq_ignore_ascii_case(candidate))
                {
                    return Some(*found);
                }
                candidate = &candidate[..candidate.rfind('-')?];
            }
        })
        .unwrap_or(DEFAULT)
}

/// Answers each request in the locale its `Accept-Language` negotiates, saying which in
/// `Content-Language`.
pub async fn layer(request: Request, next: Next) -> Response {
    let locale = request
        .headers()
        .get(ACCEPT_LANGUAGE)
        .and_then(|value| value.to_str().ok())
        .map_or(DEFAULT, negotiate);
    let mut response = scope(locale, next.run(request)).await;
    let headers = response.headers_mut();
    headers.insert(CONTENT_LANGUAGE, HeaderValue::from_static(locale));
    headers.append(VARY, HeaderValue::from_static("accept-language"));
    response
}

/// The pseudo-locales, as a translation backend laid over the ones `locales/` holds. Each
/// string is made from English the first time it is asked for and kept, so what is kept is at
/// most two copies of the English catalogue.
#[derive(Default)]
pub struct PseudoLocales {
    made: RwLock<HashMap<(&'static str, String), &'static str>>,
}

impl rust_i18n::Backend for PseudoLocales {
    fn available_locales(&self) -> Vec<&str> {
        vec![ACCENTED, MIRRORED]
    }

    fn translate(&self, locale: &str, key: &str) -> Option<&str> {
        let (locale, make): (&'static str, fn(&str) -> String) = match locale {
            ACCENTED => (ACCENTED, accented),
            MIRRORED => (MIRRORED, mirrored),
            _ => return None,
        };
        if let Some(made) = self
            .made
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&(locale, key.to_owned()))
        {
            return Some(made);
        }
        let english = crate::_rust_i18n_try_translate(DEFAULT, key)?;
        let mut made = self
            .made
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Some(
            made.entry((locale, key.to_owned()))
                .or_insert_with(|| Box::leak(make(&english).into_boxed_str())),
        )
    }
}

/// A translation's pieces: text, and the `%{name}` placeholders filled in after it is chosen,
/// which a pseudo-locale leaves alone.
enum Piece<'a> {
    Text(&'a str),
    Placeholder(&'a str),
}

fn pieces(template: &str) -> Vec<Piece<'_>> {
    let mut pieces = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("%{") {
        let Some(length) = rest[start..].find('}') else {
            break;
        };
        if start > 0 {
            pieces.push(Piece::Text(&rest[..start]));
        }
        pieces.push(Piece::Placeholder(&rest[start..=start + length]));
        rest = &rest[start + length + 1..];
    }
    if !rest.is_empty() {
        pieces.push(Piece::Text(rest));
    }
    pieces
}

/// `template` in [`ACCENTED`]: every ASCII letter accented, every vowel doubled, in brackets.
pub fn accented(template: &str) -> String {
    let mut out = String::from("[");
    for piece in pieces(template) {
        match piece {
            Piece::Placeholder(placeholder) => out.push_str(placeholder),
            Piece::Text(text) => {
                for c in text.chars() {
                    let accented = accent(c);
                    out.push(accented);
                    if "aeiouAEIOU".contains(c) {
                        out.push(accented);
                    }
                }
            }
        }
    }
    out.push(']');
    out
}

/// `template` in [`MIRRORED`]: every word set right to left, between a right-to-left override
/// and a pop of it.
pub fn mirrored(template: &str) -> String {
    let mut out = String::new();
    for piece in pieces(template) {
        match piece {
            Piece::Placeholder(placeholder) => out.push_str(placeholder),
            Piece::Text(text) => {
                let mut in_word = false;
                for c in text.chars() {
                    if c.is_whitespace() == in_word {
                        out.push(if in_word { '\u{202C}' } else { '\u{202E}' });
                        in_word = !in_word;
                    }
                    out.push(c);
                }
                if in_word {
                    out.push('\u{202C}');
                }
            }
        }
    }
    out
}

fn accent(c: char) -> char {
    const PLAIN: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const ACCENTED: &str = "áƀçðéƒĝĥîĵķļɱñöþǫŕšţûṽŵẋýžÅƁÇÐÉƑĜĤÎĴĶĻṀÑÖÞǪŔŠŢÛṼŴẊÝŽ";
    PLAIN
        .chars()
        .position(|plain| plain == c)
        .and_then(|index| ACCENTED.chars().nth(index))
        .unwrap_or(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Vectors {
        accented: Vec<Case>,
        mirrored: Vec<Case>,
    }

    #[derive(Deserialize)]
    struct Case {
        input: String,
        output: String,
    }

    #[test]
    fn pseudo_locales_pass_the_shared_vectors() {
        let vectors: Vectors =
            serde_json::from_str(include_str!("../../../spec/pseudo_locale_vectors.json")).unwrap();
        for case in vectors.accented {
            assert_eq!(accented(&case.input), case.output, "{}", case.input);
        }
        for case in vectors.mirrored {
            assert_eq!(mirrored(&case.input), case.output, "{}", case.input);
        }
    }

    #[test]
    fn negotiation_prefers_the_best_language_this_server_has() {
        let available = ["ar-XB", "de", "en", "en-XA"];
        let pick = |header| negotiate_among(header, &available);
        assert_eq!(pick("en-XA"), "en-XA");
        assert_eq!(pick("en-GB,en;q=0.9"), "en");
        assert_eq!(pick("fr-CA, de;q=0.8, en;q=0.5"), "de");
        assert_eq!(pick("en;q=0.4, de-AT;q=0.9"), "de");
        assert_eq!(pick("DE"), "de");
        assert_eq!(pick("fr, *"), "en");
        assert_eq!(pick("de;q=0, en-US"), "en");
        assert_eq!(pick("zh-Hant-TW"), "en");
        assert_eq!(pick(""), "en");
        assert_eq!(pick("de;q=nonsense"), "en");
    }

    #[test]
    fn a_request_speaks_its_locale_and_nothing_else_does() {
        assert_eq!(current(), DEFAULT);
        let inside = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(scope(ACCENTED, async { current() }));
        assert_eq!(inside, ACCENTED);
    }
}
