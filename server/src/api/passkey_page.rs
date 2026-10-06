//! The page at `/auth/passkey` that runs a passkey ceremony in the system browser for the
//! desktop and mobile apps, which cannot run one in their own pages (see `app::passkey`). It is
//! opened as `/auth/passkey#ceremony=<id>`, runs a handed-off ceremony through the public
//! `/auth/passkey-ceremonies` endpoints, and sends the browser back to the app with the return
//! code the app's claim needs. Since its link can be sent to anyone, it tells whoever opens it to
//! continue only for a request they made themselves, though a ceremony someone else started is
//! never claimed by them.
//!
//! The page is static apart from its localized strings, which are inserted as a JSON object.
//! The web client runs the same ceremonies in its own page when its origin is under the relying
//! party's domain, with the same base64url conversions in `@aspen/protocol`'s `passkeys.ts`.

use crate::t;
use axum::http::header::{CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY};
use axum::response::IntoResponse;
use std::borrow::Cow;

const TEMPLATE: &str = include_str!("passkey_page/page.html");
const STRINGS_MARKER: &str = "/*STRINGS*/{}";

/// Inline script and style only, images only from inside the page (its favicon), talking to this
/// origin only, never framed.
const POLICY: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
     img-src data:; \
     connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

fn strings() -> serde_json::Map<String, serde_json::Value> {
    let entries: [(&str, Cow<'static, str>); 15] = [
        ("passkeyPageTitle", t!("passkeyPageTitle")),
        ("passkeyPageSignIn", t!("passkeyPageSignIn")),
        ("passkeyPageRegister", t!("passkeyPageRegister")),
        ("passkeyPageReauthenticate", t!("passkeyPageReauthenticate")),
        ("passkeyPageWorking", t!("passkeyPageWorking")),
        ("passkeyPageContinue", t!("passkeyPageContinue")),
        ("passkeyPageCancel", t!("passkeyPageCancel")),
        ("passkeyPageReturning", t!("passkeyPageReturning")),
        ("passkeyPageReturnHint", t!("passkeyPageReturnHint")),
        ("passkeyPageOnlyYours", t!("passkeyPageOnlyYours")),
        ("passkeyPageFailed", t!("passkeyPageFailed")),
        ("passkeyPageExpired", t!("passkeyPageExpired")),
        ("passkeyPageUnsupported", t!("passkeyPageUnsupported")),
        ("passkeyPageCancelled", t!("passkeyPageCancelled")),
        ("passkeyPageFinished", t!("passkeyPageFinished")),
    ];
    entries
        .into_iter()
        .map(|(key, value)| {
            (
                key.to_string(),
                serde_json::Value::String(value.into_owned()),
            )
        })
        .collect()
}

/// Renders the page. `</` is escaped in the inserted JSON so no string can close the script.
fn render() -> String {
    let json = serde_json::Value::Object(strings())
        .to_string()
        .replace("</", "<\\/");
    TEMPLATE.replacen(STRINGS_MARKER, &json, 1)
}

pub async fn page() -> impl IntoResponse {
    (
        [
            (CONTENT_TYPE, "text/html; charset=utf-8"),
            (CONTENT_SECURITY_POLICY, POLICY),
            (REFERRER_POLICY, "no-referrer"),
            (CACHE_CONTROL, "no-store"),
        ],
        render(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_string_the_page_uses_is_supplied() {
        let rendered = render();
        assert!(!rendered.contains(STRINGS_MARKER));
        let supplied = strings();
        for key in TEMPLATE
            .match_indices("T.")
            .map(|(at, _)| &TEMPLATE[at + 2..])
            .map(|rest| {
                rest.split(|c: char| !c.is_ascii_alphanumeric())
                    .next()
                    .unwrap_or("")
            })
            .chain(
                TEMPLATE
                    .match_indices("data-text=\"")
                    .map(|(at, m)| &TEMPLATE[at + m.len()..])
                    .map(|rest| rest.split('"').next().unwrap_or("")),
            )
        {
            assert!(supplied.contains_key(key), "missing {key}");
        }
    }
}
