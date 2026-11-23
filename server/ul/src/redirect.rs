//! Plain HTTP, answered only with a permanent redirect to the same path at the deployment's
//! address over HTTPS: to `media_url` for a request naming its host, to `public_url` for any
//! other, so the redirect never leads anywhere the request's `Host` alone chose.

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
        axum::Router::new().fallback(move |headers: HeaderMap, uri: Uri| {
            let location = self.location(&headers, &uri);
            async move { redirect(location) }
        })
    }
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
        let without_media = Redirect::new("https://chat.example.org", None);
        assert_eq!(
            location(&without_media, "media.chat.example.org", "/"),
            "https://chat.example.org/"
        );
    }
}
