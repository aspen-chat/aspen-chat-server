//! Plain HTTP, answered with a permanent redirect to the same path at the deployment's address
//! over HTTPS: to `media_url` for a request naming its host, to `public_url` for any other, so the
//! redirect never leads anywhere the request's `Host` alone chose.
//!
//! The API's requests and the voice server's are refused instead. A client set up with an
//! `http://` address would follow a redirect and go on working, sending its token in the clear
//! ahead of every request, where a refusal fails the first time it is tried.

use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

#[derive(Clone)]
pub struct Redirect {
    public_url: String,
    media: Option<(String, String)>,
}

impl Redirect {
    /// `media` is the files' host and origin, when the deployment serves them.
    pub fn new(public_url: &str, media: Option<(String, String)>) -> Self {
        Self {
            public_url: public_url.trim_end_matches('/').to_string(),
            media,
        }
    }

    fn location(&self, headers: &HeaderMap, uri: &Uri) -> String {
        let host = headers
            .get(header::HOST)
            .and_then(|host| host.to_str().ok())
            .and_then(|host| host.parse::<axum::http::uri::Authority>().ok());
        let origin = match (&self.media, host) {
            (Some((media_host, media_origin)), Some(host))
                if host.host().eq_ignore_ascii_case(media_host) =>
            {
                media_origin
            }
            (Some(_) | None, Some(_) | None) => &self.public_url,
        };
        let path = uri.path_and_query().map_or("/", |path| path.as_str());
        format!("{origin}{path}")
    }

    pub fn router(self) -> axum::Router {
        axum::Router::new()
            .fallback(move |headers: HeaderMap, uri: Uri| {
                let refused = refused(&uri).then(|| self.public_url.clone());
                let location = self.location(&headers, &uri);
                async move {
                    match refused {
                        Some(public_url) => aspen_api::error::ApiError::new(
                            aspen_api::error::ProblemCode::Forbidden,
                        )
                        .with_detail(crate::t!("plainHttpRefused", url = public_url))
                        .into_response(),
                        None => redirect(location),
                    }
                }
            })
            .layer(axum::middleware::from_fn(aspen_app::locale::layer))
    }
}

/// Where every version of the API is served (`aspen_api::API_PREFIX` is this one's).
const API: &str = "/api";

/// Whether `uri` is the API's or the voice server's, which carry tokens and so are refused.
fn refused(uri: &Uri) -> bool {
    [API, crate::VOICE_PATH].iter().any(|prefix| {
        uri.path()
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    })
}

fn redirect(location: String) -> Response {
    (
        StatusCode::PERMANENT_REDIRECT,
        [(header::LOCATION, location)],
    )
        .into_response()
}

/// Serves `redirect` on `listeners` for as long as the process runs.
pub async fn serve(redirect: Redirect, listeners: Vec<tokio::net::TcpListener>) {
    let router = redirect.router();
    for listener in listeners {
        let router = router.clone();
        tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, router).await {
                tracing::warn!("plain HTTP stopped being answered: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(redirect: &Redirect, host: &str, path: &str) -> String {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, host.parse().unwrap());
        redirect.location(&headers, &path.parse().unwrap())
    }

    #[test]
    fn a_request_goes_to_the_same_path_at_the_deployment() {
        let redirect = Redirect::new(
            "https://chat.example.org/",
            Some((
                "media.chat.example.org".into(),
                "https://media.chat.example.org".into(),
            )),
        );
        assert_eq!(
            location(&redirect, "chat.example.org", "/invite/abc?x=1"),
            "https://chat.example.org/invite/abc?x=1"
        );
        assert_eq!(
            location(&redirect, "MEDIA.chat.example.org:80", "/attachments/1"),
            "https://media.chat.example.org/attachments/1"
        );
        // A host the deployment does not answer is sent to the deployment, never to itself.
        assert_eq!(
            location(&redirect, "elsewhere.example", "/x"),
            "https://chat.example.org/x"
        );
        for path in [
            "/api/v1/health",
            "/api/v1/events",
            "/voice-server/ws",
            "/api",
        ] {
            assert!(refused(&path.parse().unwrap()), "{path}");
        }
        for path in ["/apiary", "/", "/invite/x", "/voice-servers"] {
            assert!(!refused(&path.parse().unwrap()), "{path}");
        }
        let without_media = Redirect::new("https://chat.example.org", None);
        assert_eq!(
            location(&without_media, "media.chat.example.org", "/"),
            "https://chat.example.org/"
        );
    }
}
