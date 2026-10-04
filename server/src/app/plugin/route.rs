//! Plugins' routes, `/api/v1/plugins/{id}/routes/{path}`: a plugin holding `routes` answers
//! requests there, authenticated and rate limited as every endpoint is, and every read it makes
//! while answering is made as the caller (`host::Phase::Route`).
//!
//! An answer is served from the API's origin, which may be the web client's, so it is never a
//! page: a content type a browser would render as a document is replaced, and every answer is
//! sandboxed (`Content-Security-Policy: sandbox`) and never sniffed. A plugin's own pages are
//! views (phase 3), served from an origin of their own.

use super::PluginPermission;
use super::host::{self, Instance, Phase, wit};
use crate::app::context::GlobalServerContext;
use crate::app::outbound::PublicResolver;
use crate::app::{self, UserId};
use crate::t;
use std::sync::OnceLock;
use std::time::Duration;

/// The largest body a route is handed, in bytes.
pub const MAX_BODY: usize = 64 << 10;
/// The largest body a route may answer with, in bytes.
pub const MAX_ANSWER: usize = 1 << 20;

/// What every plugin's call to another host is made with: public addresses only, no
/// redirects, HTTPS only, and a bounded time.
pub(super) fn outbound_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .dns_resolver(std::sync::Arc::new(PublicResolver {
                allow_private: false,
            }))
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("Aspen/", env!("CARGO_PKG_VERSION"), " (plugin)"))
            .build()
            .expect("the plugins' outbound client builds")
    })
}

/// A request to a plugin's route.
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: String,
    pub content_type: Option<String>,
    pub body: Vec<u8>,
}

/// A plugin's answer, made safe to serve.
#[derive(Debug)]
pub struct Answer {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

/// Whether a browser would render `content_type` as a document or run it as script.
fn renders(content_type: &str) -> bool {
    let essence = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    essence.contains("html")
        || essence.contains("xml")
        || essence.contains("javascript")
        || essence.contains("ecmascript")
        || essence == "image/svg+xml"
        || essence == "text/css"
        || essence.is_empty()
}

/// The prefix of the routes only the host calls: a card's button pressed, a capability URL
/// followed.
pub const HOST_PREFIX: &str = "aspen/";

/// `plugin_id`'s answer to `request` from `caller`, who asked for it themself; the host's own
/// routes (`HOST_PREFIX`) are not theirs to reach.
pub async fn answer(
    state: &GlobalServerContext,
    plugin_id: &str,
    caller: UserId,
    request: Request,
) -> app::Result<Answer> {
    if request.path.starts_with(HOST_PREFIX) {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    }
    answer_host(state, plugin_id, caller, request).await
}

/// `plugin_id`'s answer to `request`, as `caller`, whichever route it names.
pub async fn answer_host(
    state: &GlobalServerContext,
    plugin_id: &str,
    caller: UserId,
    request: Request,
) -> app::Result<Answer> {
    let not_found = || app::Error::Diesel(diesel::result::Error::NotFound);
    let plugin = state.plugins.get(plugin_id).ok_or_else(not_found)?;
    if !plugin.holds(PluginPermission::Routes) {
        return Err(not_found());
    }
    let locale = app::locale::current();
    let budget = Duration::from_millis(state.config.plugins.route_millis);
    let input = wit::Request {
        method: request.method,
        path: request.path,
        query: request.query,
        content_type: request.content_type,
        body: request.body,
        caller: caller.0.to_string(),
    };
    let context = wit::Context {
        community: None,
        community_settings: None,
        locale: Some(locale.to_string()),
        event_id: None,
    };
    let unavailable = || {
        app::Error::PluginUnavailable(t!("pluginRouteUnavailable", plugin = plugin.name(locale)))
    };
    let mut instance = Instance::new(
        state,
        plugin.clone(),
        Phase::Route { caller },
        Default::default(),
        budget,
    )
    .await
    .map_err(|e| {
        tracing::warn!(plugin = plugin.id, "answering a route failed: {e}");
        unavailable()
    })?;
    let deadline = instance.deadline;
    let response = host::within(
        deadline,
        instance
            .bindings
            .aspen_plugin_hooks()
            .call_route(&mut instance.store, &input, &context),
    )
    .await
    .map_err(|e| {
        tracing::warn!(plugin = plugin.id, "answering a route failed: {e}");
        unavailable()
    })?;
    if !(200..=599).contains(&response.status) || response.body.len() > MAX_ANSWER {
        tracing::warn!(
            plugin = plugin.id,
            status = response.status,
            "a route answered with a status outside 200 to 599, or too large a body"
        );
        return Err(unavailable());
    }
    let content_type = response
        .content_type
        .filter(|c| !renders(c))
        .unwrap_or_else(|| "application/octet-stream".to_string());
    Ok(Answer {
        status: response.status,
        content_type,
        body: response.body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_and_scripts_are_never_served_as_such() {
        for kind in [
            "text/html",
            "text/html; charset=utf-8",
            "application/xhtml+xml",
            "image/svg+xml",
            "text/javascript",
            "",
        ] {
            assert!(renders(kind), "{kind}");
        }
        for kind in ["application/json", "text/plain", "image/png", "text/csv"] {
            assert!(!renders(kind), "{kind}");
        }
    }
}
