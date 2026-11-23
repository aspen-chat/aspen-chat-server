//! The files of a deployment whose media store is a directory (`app::media_store::Directory`),
//! served by this server at their own origin, `[media.directory] public_base_url`, as an S3
//! bucket's read path and API would serve them: what the public prefixes hold to anyone
//! (`GET` and `HEAD /{prefix}/{id}`), evidence through the signed URLs reviewers are given
//! (`GET /signed/{key}`), and uploads through the signed URLs clients are given
//! (`PUT /upload/{key}`). A request for this origin reaches only these routes, never the API or
//! the web client ([`by_host`]), and nothing here is answered at `public_url`.
//!
//! Every answer carries `nosniff` and a CSP that sandboxes it, so whatever a file holds, a
//! browser opening it runs nothing as this origin, which in any case holds no sign-in.

use std::convert::Infallible;

use aspen_app as app;
use aspen_app::media_store::PublicPrefix;
use aspen_app::media_store::directory::{Directory, Record, Refusal};
use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE,
    ETAG, HOST, HeaderValue, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{Method, StatusCode, uri::Authority};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use futures_util::TryStreamExt;
use serde::Deserialize;
use tower::ServiceExt;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeFile;

/// How long a public object may be kept by whoever fetched it: a key names one file for good
/// (each holds a new id), so as long as caches keep anything.
const PUBLIC_CACHE: &str = "public, max-age=31536000, immutable";

#[derive(Deserialize)]
pub struct Signed {
    expires: u64,
    signature: String,
}

/// The routes of the media origin.
pub fn router(store: Directory) -> Router {
    Router::new()
        .route("/{prefix}/{id}", get(public))
        .route("/signed/{*key}", get(signed))
        .route("/upload/{*key}", put(upload))
        .with_state(store)
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::HEAD, Method::PUT])
                .allow_headers(Any)
                .expose_headers([ETAG])
                .max_age(std::time::Duration::from_secs(3600)),
        )
        .layer(axum::middleware::map_response(sandboxed))
}

async fn sandboxed(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; sandbox"),
    );
    response
}

fn refused(refusal: Refusal) -> Response {
    match refusal {
        Refusal::Unsigned | Refusal::Expired => StatusCode::FORBIDDEN.into_response(),
        Refusal::BadKey => StatusCode::NOT_FOUND.into_response(),
        Refusal::NotAsSigned => StatusCode::BAD_REQUEST.into_response(),
    }
}

fn failed(e: app::Error) -> Response {
    tracing::error!(error = %e, "the media store failed");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

/// `key`'s file, with the type and disposition it was stored with and `cache_control`, ranges
/// and conditional requests answered as `ServeFile` answers them.
async fn serve(
    store: &Directory,
    key: &str,
    request: Request,
    cache_control: &'static str,
) -> Response {
    let (path, record) = match store.object(key).await {
        Ok(Some(found)) => found,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return failed(e),
    };
    let Record {
        content_type,
        disposition,
        e_tag,
    } = record;
    let mut response = match ServeFile::new(path).oneshot(request).await {
        Ok(response) => response.map(Body::new),
        Err(infallible) => match infallible {},
    };
    let headers = response.headers_mut();
    match HeaderValue::from_str(&content_type) {
        Ok(value) => headers.insert(CONTENT_TYPE, value),
        Err(_) => headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/octet-stream"),
        ),
    };
    if let Some(value) = disposition.and_then(|d| HeaderValue::from_str(d.as_str()).ok()) {
        headers.insert(CONTENT_DISPOSITION, value);
    }
    if let Ok(value) = HeaderValue::from_str(&format!("\"{e_tag}\"")) {
        headers.insert(ETAG, value);
    }
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(cache_control));
    response
}

async fn public(
    State(store): State<Directory>,
    Path((prefix, id)): Path<(String, String)>,
    request: Request,
) -> Response {
    let Some(prefix) = PublicPrefix::named(&prefix) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(id) = id.parse::<uuid::Uuid>() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    serve(&store, &prefix.key(id), request, PUBLIC_CACHE).await
}

async fn signed(
    State(store): State<Directory>,
    Path(key): Path<String>,
    Query(Signed { expires, signature }): Query<Signed>,
    request: Request,
) -> Response {
    if let Err(refusal) = store.verify_read(&key, expires, &signature) {
        return refused(refusal);
    }
    serve(&store, &key, request, "private, no-store").await
}

async fn upload(
    State(store): State<Directory>,
    Path(key): Path<String>,
    Query(Signed { expires, signature }): Query<Signed>,
    request: Request,
) -> Response {
    let headers = request.headers();
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let Some(length) = headers
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
    else {
        return StatusCode::LENGTH_REQUIRED.into_response();
    };
    let body = request
        .into_body()
        .into_data_stream()
        .map_err(std::io::Error::other);
    match store
        .receive_upload(&key, expires, &signature, &content_type, length, body)
        .await
    {
        Ok(Ok(())) => match store.record(&key).await {
            Ok(Some(record)) => match HeaderValue::from_str(&format!("\"{}\"", record.e_tag)) {
                Ok(e_tag) => (StatusCode::OK, [(ETAG, e_tag)]).into_response(),
                Err(_) => StatusCode::OK.into_response(),
            },
            Ok(None) => StatusCode::OK.into_response(),
            Err(e) => failed(e),
        },
        Ok(Err(refusal)) => refused(refusal),
        Err(e) => failed(e),
    }
}

/// The host and port a request was sent to, from HTTP/2's `:authority` or HTTP/1.1's `Host`.
fn authority_of(request: &Request) -> Option<Authority> {
    request.uri().authority().cloned().or_else(|| {
        request
            .headers()
            .get(HOST)
            .and_then(|host| host.to_str().ok())
            .and_then(|host| host.parse().ok())
    })
}

/// Whether `asked`, a request's authority, names the origin `base` (`public_base_url`): the same
/// host, ignoring case, and the same port, a port left out being the scheme's own.
fn names(base: &url::Url, asked: &Authority) -> bool {
    let default = match base.scheme() {
        "https" => 443,
        _ => 80,
    };
    base.host_str()
        .is_some_and(|host| host.eq_ignore_ascii_case(asked.host()))
        && base.port().unwrap_or(default) == asked.port_u16().unwrap_or(default)
}

/// `web`, with every request for `store`'s origin taken by [`router`] instead.
pub fn by_host(web: Router, store: &Directory) -> Result<Router, app::Error> {
    let base = url::Url::parse(store.public_base_url()).map_err(|e| {
        app::Error::Config(config::ConfigError::Message(format!(
            "[media.directory] public_base_url: {e}"
        )))
    })?;
    let media = router(store.clone());
    Ok(
        Router::new().fallback_service(tower::service_fn(move |request: Request| {
            let to_media = authority_of(&request).is_some_and(|asked| names(&base, &asked));
            let (web, media) = (web.clone(), media.clone());
            async move {
                let response = if to_media {
                    media.oneshot(request).await
                } else {
                    web.oneshot(request).await
                };
                Ok::<_, Infallible>(response.unwrap_or_else(|infallible| match infallible {}))
            }
        })),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_names_the_media_origin_by_host_and_port() {
        let base = url::Url::parse("https://media.example.org").unwrap();
        for (asked, expected) in [
            ("media.example.org", true),
            ("MEDIA.example.org", true),
            ("media.example.org:443", true),
            ("media.example.org:8443", false),
            ("chat.example.org", false),
            ("media.example.org.evil", false),
        ] {
            assert_eq!(names(&base, &asked.parse().unwrap()), expected, "{asked}");
        }
        let local = url::Url::parse("http://media.localhost:5174").unwrap();
        assert!(names(&local, &"media.localhost:5174".parse().unwrap()));
        assert!(!names(&local, &"media.localhost".parse().unwrap()));
    }
}
