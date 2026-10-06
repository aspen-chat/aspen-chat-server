//! The web client, which this server serves at `public_url` beside the API: its files from
//! `[web_client] dir` as they are asked for (`files`), and every other path the API does not
//! own answered with its `index.html`, given Open Graph tags (and a title and description) that
//! say what the link opens (`app::open_graph`), so a link pasted into a chat, a post, or a
//! message previews as the deployment, or as the community an invite leads to. Crawlers read
//! only the HTML, never running the web client's script, so the tags have to be in the page as
//! served.
//!
//! Everything is read from the directory on each request, so a new release of the web client is
//! served as soon as it is in place. The server does not start without it (`check`).
//!
//! Every file and page goes out with the headers that keep the page to itself
//! (`security_headers`): a Content Security Policy that runs only the web client's own scripts,
//! keeps any other page from framing it, and lets it reach only what it uses, and `nosniff`, no
//! referrer, and, over `https`, HSTS.

use crate::api::error::{ApiError, ApiResult, ProblemCode};
use crate::api::extract::Path;
use crate::api::rate_limit::{self, PeerAddr};
use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::icon::Icon;
use crate::app::open_graph::Preview;
use crate::aspen_config::{AspenConfig, MediaS3Config};
use crate::t;
use askama::Template;
use axum::Extension;
use axum::extract::Request;
use axum::extract::State;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_SECURITY_POLICY, CONTENT_TYPE, REFERRER_POLICY,
    STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS, X_FRAME_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::convert::Infallible;
use tower::Layer;
use tower_http::services::ServeDir;

/// Where the build puts the files named by their contents.
const ASSETS: &str = "/assets/";
/// The page every path without a file of its own is answered with.
const INDEX: &str = "index.html";
/// The Aspen mark on its tile, which the web client ships beside `index.html`
/// (`client/scripts/export_icons.py`): the picture of a deployment that has no icon.
const ASPEN_IMAGE: &str = "open-graph.png";
const ASPEN_IMAGE_SIZE: u32 = 512;
/// The name a deployment that has not given one goes by.
const ASPEN: &str = "Aspen";

/// What the page says about itself, written into its head by `open_graph.html`.
struct Tags {
    site_name: String,
    title: String,
    description: String,
    /// Where the page is, in full.
    url: String,
    image: Image,
}

struct Image {
    url: String,
    mime_type: String,
    alt: String,
    /// Its width and height, when known.
    size: Option<u32>,
}

#[derive(Template)]
#[template(path = "web_client/open_graph.html")]
struct HeadTemplate<'a> {
    tags: &'a Tags,
}

/// Refuses to start without the web client: `[web_client] dir` must hold an `index.html` with a
/// `</head>` for the tags to go before, and the picture they name for a deployment without an
/// icon.
pub fn check(config: &AspenConfig) -> app::Result<()> {
    let dir = &config.web_client.dir;
    let missing = |what: String| {
        Err(app::Error::Config(config::ConfigError::Message(format!(
            "web_client.dir {} {what}. Build the web client (`pnpm build` in client/) and set \
             web_client.dir to its dist directory: this server serves it.",
            dir.display()
        ))))
    };
    let index = match std::fs::read_to_string(dir.join(INDEX)) {
        Ok(index) => index,
        Err(error) => return missing(format!("holds no readable {INDEX} ({error})")),
    };
    if !index.contains("</head>") {
        return missing(format!("holds an {INDEX} with no </head>"));
    }
    if !dir.join(ASPEN_IMAGE).is_file() {
        return missing(format!("holds no {ASPEN_IMAGE}"));
    }
    Ok(())
}

/// The web client's files, and its page wherever there is no file: what this server answers a
/// path no route has with. A directory, the root included, is no file, so `/` is the page too.
/// Files under `assets/` are named by their contents and cached for good, and one missing there
/// is not found rather than the page; everything else is revalidated on every load, so a new
/// release reaches browsers at once.
pub fn files(
    state: GlobalServerContext,
) -> impl tower::Service<Request, Response = Response, Error = Infallible, Future = impl Send + 'static>
+ Clone
+ Send
+ Sync
+ 'static {
    let dir = state.config.web_client.dir.clone();
    let config = state.config.clone();
    let page = axum::routing::get(page).with_state(state);
    let served = ServeDir::new(dir)
        .append_index_html_on_directories(false)
        .fallback(page);
    axum::middleware::from_fn_with_state(config, file_headers).layer(served)
}

/// Each file's caching, and the headers every response of the web client carries.
async fn file_headers(
    State(config): State<std::sync::Arc<AspenConfig>>,
    request: Request,
    next: Next,
) -> Response {
    let immutable = request.uri().path().starts_with(ASSETS);
    let mut response = next.run(request).await;
    security_headers(&config, response.headers_mut());
    if response.status().is_success() || response.status().is_redirection() {
        let value = if immutable {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static(value));
    }
    response
}

/// Every page of the web client but an invite's. Over its limits it previews as the deployment
/// by name alone, without reading its icon.
async fn page(
    State(state): State<GlobalServerContext>,
    peer: Option<Extension<PeerAddr>>,
    headers: HeaderMap,
    uri: Uri,
) -> ApiResult<Response> {
    // A file of a release no longer served, which a page of that release may still ask for, is
    // missing rather than a page its script cannot run.
    if uri.path().starts_with(ASSETS) {
        return Err(ApiError::new(ProblemCode::NotFound));
    }
    let within = rate_limit::within_limits(
        &state,
        rate_limit::WEB_CLIENT_PAGE,
        peer.map(|Extension(peer)| peer),
        &headers,
        Vec::new(),
    )
    .await;
    let preview = if within {
        app::open_graph::of_deployment(&state).await
    } else {
        Ok(app::open_graph::by_name(&state))
    };
    respond(&state, &uri, preview).await
}

/// An invite's page, `/invite/{code}`, which previews as the invite's community. One whose
/// `?at=` names another deployment is that deployment's invite, and previews as this one, as
/// does one asked for too often from one address: its limits keep codes from being guessed, but
/// whoever opens the link still gets the web client.
pub async fn invite_page(
    State(state): State<GlobalServerContext>,
    Path(code): Path<String>,
    peer: Option<Extension<PeerAddr>>,
    headers: HeaderMap,
    uri: Uri,
) -> ApiResult<Response> {
    let read_invite = names_this_deployment(&state, uri.query())
        && rate_limit::within_limits(
            &state,
            rate_limit::WEB_CLIENT_INVITE,
            peer.map(|Extension(peer)| peer),
            &headers,
            vec![("code", code.as_str())],
        )
        .await;
    let preview = if read_invite {
        app::open_graph::of_invite(&state, &code).await
    } else {
        app::open_graph::of_deployment(&state).await
    };
    respond(&state, &uri, preview).await
}

/// Whether an invite page's query leaves out `at` or names this deployment with it.
fn names_this_deployment(state: &GlobalServerContext, query: Option<&str>) -> bool {
    let at = query.and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "at")
            .map(|(_, value)| value.into_owned())
    });
    match (at, &state.config.federation.domain) {
        (None, _) => true,
        (Some(at), Some(domain)) => at.eq_ignore_ascii_case(domain),
        (Some(_), None) => false,
    }
}

/// The page with `preview`'s tags. A preview that could not be read leaves the page the
/// deployment's by name alone rather than keeping anyone from the web client.
async fn respond(
    state: &GlobalServerContext,
    uri: &Uri,
    preview: app::Result<Preview>,
) -> ApiResult<Response> {
    let base = &state.config.public_url;
    let index_html = state.config.web_client.dir.join(INDEX);
    let index = tokio::fs::read_to_string(&index_html)
        .await
        .map_err(|error| {
            tracing::error!(
                path = %index_html.display(),
                %error,
                "Could not read the web client's index.html (web_client.dir)"
            );
            ApiError::new(ProblemCode::Internal)
        })?;
    let preview = preview.unwrap_or_else(|error| {
        tracing::warn!(%error, "Could not read what a web client page previews as");
        app::open_graph::by_name(state)
    });
    let tags = tags(state, base, uri, preview);
    let head = HeadTemplate { tags: &tags }.render().map_err(|error| {
        tracing::error!(%error, "Could not render the web client's Open Graph tags");
        ApiError::new(ProblemCode::Internal)
    })?;
    let page = with_head(&index, &head).unwrap_or_else(|| {
        tracing::error!(
            path = %index_html.display(),
            "The web client's index.html has no </head>, so it is served without its tags"
        );
        index.clone()
    });
    let mut response = (
        [
            (CONTENT_TYPE, "text/html; charset=utf-8"),
            // Revalidated on every load, like the file it is made from, so a new release and a
            // changed name or icon reach browsers at once.
            (CACHE_CONTROL, "no-cache"),
        ],
        page,
    )
        .into_response();
    security_headers(&state.config, response.headers_mut());
    Ok(response)
}

/// The headers on every file and page of the web client: its Content Security Policy
/// (`content_security_policy`), `nosniff`, so no file is run as anything but its type, no
/// referrer, so the deployment's paths reach no one the page links to or loads from, framing
/// refused (for browsers that predate `frame-ancestors` too), and, over `https`, HSTS, without
/// `includeSubDomains`, since other names under the deployment's may be served otherwise.
fn security_headers(config: &AspenConfig, headers: &mut HeaderMap) {
    let policy = content_security_policy(&config.public_url, &config.media.s3);
    match HeaderValue::from_str(&policy) {
        Ok(value) => {
            headers.insert(CONTENT_SECURITY_POLICY, value);
        }
        Err(error) => {
            tracing::error!(%error, %policy, "The web client's Content Security Policy is no header value");
        }
    }
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if config.public_url.starts_with("https://") {
        headers.insert(
            STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=31536000"),
        );
    }
}

/// The web client's Content Security Policy. Scripts are its own files alone (and WebAssembly,
/// which the QR code reader is); styles may be inline, since React Aria and the emoji picker
/// write `<style>` elements and style attributes. Pictures, videos, requests, and frames reach
/// this deployment, its storage (`[media.s3] public_base_url` to read, `public_endpoint` to
/// upload), and any `https:` and `wss:` address: the client holds sessions on other
/// deployments, whose APIs, storage, voice servers, and plugin views are anywhere, and plays
/// videos in the players of the providers the server previews. An `http:` deployment, which is
/// a development one, also reaches `http:` and `ws:` addresses, as its storage and voice servers
/// are. Nothing may frame the page.
fn content_security_policy(public_url: &str, s3: &MediaS3Config) -> String {
    let development = public_url.starts_with("http://");
    let origin = |url: &str| {
        url::Url::parse(url)
            .ok()
            .map(|url| url.origin())
            .filter(url::Origin::is_tuple)
            .map(|origin| origin.ascii_serialization())
    };
    let storage = origin(&s3.public_base_url);
    let uploads = origin(s3.public_endpoint.as_deref().unwrap_or(&s3.endpoint));
    let own_socket = url::Url::parse(public_url).ok().and_then(|mut url| {
        let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme).ok()?;
        origin(url.as_str()).or_else(|| Some(url.as_str().trim_end_matches('/').to_string()))
    });
    let plain = if development { " http:" } else { "" };
    let join = |sources: &[Option<&str>]| {
        sources
            .iter()
            .flatten()
            .copied()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let media = join(&[Some("'self' data: blob: https:"), storage.as_deref()]);
    let connect = join(&[
        Some("'self'"),
        own_socket.as_deref(),
        Some("https: wss:"),
        storage.as_deref(),
        uploads.as_deref(),
        development.then_some("http: ws:"),
    ]);
    format!(
        "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
         style-src 'self' 'unsafe-inline'; img-src {media}{plain}; media-src {media}{plain}; \
         font-src 'self' data:; connect-src {connect}; frame-src 'self' https:{plain}; \
         worker-src 'self' blob:; manifest-src 'self'; object-src 'none'; base-uri 'none'; \
         form-action 'self'; frame-ancestors 'none'"
    )
}

fn tags(state: &GlobalServerContext, base: &str, uri: &Uri, preview: Preview) -> Tags {
    let site_name = preview
        .deployment
        .name
        .clone()
        .unwrap_or_else(|| ASPEN.to_string());
    let url = page_url(base, uri);
    match preview.community {
        Some(community) => {
            let name = community.name.unwrap_or_default();
            let image = image(state, community.icon, &name)
                .or_else(|| image(state, preview.deployment.icon, &site_name))
                .unwrap_or_else(|| aspen_image(base));
            Tags {
                title: name.clone(),
                description: t!(
                    "openGraphInvite",
                    community = name.as_str(),
                    deployment = site_name.as_str()
                )
                .into_owned(),
                site_name,
                url,
                image,
            }
        }
        None => {
            let image = image(state, preview.deployment.icon, &site_name)
                .unwrap_or_else(|| aspen_image(base));
            Tags {
                title: site_name.clone(),
                description: t!("openGraphDeployment", deployment = site_name.as_str())
                    .into_owned(),
                site_name,
                url,
                image,
            }
        }
    }
}

/// The address of the page asked for: the request's path on the web client's origin.
fn page_url(base: &str, uri: &Uri) -> String {
    let path = uri.path_and_query().map_or("/", |path| path.as_str());
    url::Url::parse(base)
        .and_then(|base| base.join(path))
        .map_or_else(|_| format!("{base}{path}"), String::from)
}

/// The picture of something named `name`, when it has an icon every unfurler shows (one of
/// [`app::icon::IMAGE_TYPES`]); an icon of another kind is passed over for the next picture.
fn image(state: &GlobalServerContext, icon: Option<Icon>, name: &str) -> Option<Image> {
    let icon = icon.filter(|icon| app::icon::IMAGE_TYPES.contains(&icon.mime_type.as_str()))?;
    Some(Image {
        url: state.media_store.public_url(&icon.storage_key),
        mime_type: icon.mime_type,
        alt: t!("openGraphIcon", name = name).into_owned(),
        size: None,
    })
}

fn aspen_image(base: &str) -> Image {
    Image {
        url: format!("{base}/{ASPEN_IMAGE}"),
        mime_type: "image/png".to_string(),
        alt: t!("openGraphAspenIcon").into_owned(),
        size: Some(ASPEN_IMAGE_SIZE),
    }
}

/// `index` with its `<title>` taken out and `head` put in before its `</head>`; `None` when it
/// has no `</head>`.
fn with_head(index: &str, head: &str) -> Option<String> {
    let end = index.find("</head>")?;
    let (before, after) = index.split_at(end);
    let before = match (before.find("<title>"), before.find("</title>")) {
        (Some(start), Some(stop)) if start < stop => {
            let mut without = String::with_capacity(before.len());
            without.push_str(before[..start].trim_end_matches([' ', '\t']));
            without.push_str(before[stop + "</title>".len()..].trim_start_matches(['\r', '\n']));
            without
        }
        _ => before.to_string(),
    };
    Some(format!("{before}{head}\n  {after}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const INDEX: &str = "<!doctype html>\n<html lang=\"en\">\n  <head>\n    \
        <meta charset=\"UTF-8\" />\n    <title>Aspen</title>\n    \
        <script type=\"module\" src=\"/assets/index-abc.js\"></script>\n  </head>\n  \
        <body><div id=\"root\"></div></body>\n</html>\n";

    fn tags(title: &str) -> Tags {
        Tags {
            site_name: "Example".to_string(),
            title: title.to_string(),
            description: "Come in".to_string(),
            url: "https://chat.example.org/invite/abc".to_string(),
            image: Image {
                url: "https://chat.example.org/open-graph.png".to_string(),
                mime_type: "image/png".to_string(),
                alt: "Aspen".to_string(),
                size: Some(512),
            },
        }
    }

    fn render(tags: &Tags) -> String {
        let head = HeadTemplate { tags }.render().unwrap();
        with_head(INDEX, &head).unwrap()
    }

    #[test]
    fn the_tags_replace_the_title_and_go_before_the_head_ends() {
        let page = render(&tags("Gardeners"));
        assert_eq!(page.matches("<title>").count(), 1);
        assert!(page.contains("<title>Gardeners</title>"));
        assert!(page.contains(r#"<meta property="og:title" content="Gardeners" />"#));
        assert!(page.contains(r#"<meta property="og:image:width" content="512" />"#));
        let tags_at = page.find("og:title").unwrap();
        assert!(page.find("/assets/index-abc.js").unwrap() < tags_at);
        assert!(tags_at < page.find("</head>").unwrap());
        assert!(page.ends_with("<body><div id=\"root\"></div></body>\n</html>\n"));
    }

    #[test]
    fn names_are_escaped() {
        let page = render(&tags(r#""/><script>alert(1)</script>"#));
        assert!(!page.contains("<script>alert"));
        assert!(page.contains("<title>&#34;/&#62;&#60;script&#62;"));
    }

    #[test]
    fn the_policy_runs_only_own_scripts_and_names_storage() {
        let s3 = MediaS3Config {
            public_endpoint: Some("https://s3.example.org/upload-path".to_string()),
            public_base_url: "https://media.example.org/aspen-media".to_string(),
            ..MediaS3Config::default()
        };
        let policy = content_security_policy("https://chat.example.org", &s3);
        assert!(policy.contains("script-src 'self' 'wasm-unsafe-eval';"));
        assert!(policy.contains("frame-ancestors 'none'"));
        assert!(policy.contains("object-src 'none'"));
        assert!(policy.contains("img-src 'self' data: blob: https: https://media.example.org;"));
        assert!(policy.contains(
            "connect-src 'self' wss://chat.example.org https: wss: https://media.example.org \
             https://s3.example.org;"
        ));
        assert!(!policy.contains("http:"));
        assert!(!policy.contains("ws:"));
        assert!(HeaderValue::from_str(&policy).is_ok());
    }

    #[test]
    fn a_development_policy_reaches_plain_http() {
        let policy = content_security_policy("http://localhost:5173", &MediaS3Config::default());
        assert!(policy.contains("http://127.0.0.1:3902"));
        assert!(policy.contains("http://127.0.0.1:3900"));
        assert!(policy.contains("ws://localhost:5173"));
        assert!(policy.contains("http: ws:"));
        assert!(policy.contains("frame-src 'self' https: http:;"));
    }

    #[test]
    fn a_page_without_a_head_is_left_alone() {
        assert!(with_head("<p>hi</p>", "<title>x</title>").is_none());
    }

    #[test]
    fn the_page_url_is_on_the_web_client_origin() {
        let uri: Uri = "/invite/abc?at=chat.example.org".parse().unwrap();
        assert_eq!(
            page_url("https://chat.example.org", &uri),
            "https://chat.example.org/invite/abc?at=chat.example.org"
        );
        let uri: Uri = "/aspen/communities/x".parse().unwrap();
        assert_eq!(
            page_url("https://example.org/aspen", &uri),
            "https://example.org/aspen/communities/x"
        );
    }
}
