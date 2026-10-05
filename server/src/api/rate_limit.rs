//! The HTTP side of rate limiting (`app::rate_limit`): which route a request is for, which
//! address it comes from, and the `429` answer.

use crate::api::API_PREFIX;
use crate::api::error::{ApiError, ApiResult, ProblemCode};
use crate::app::UserId;
use crate::app::context::GlobalServerContext;
use crate::app::rate_limit::{
    Access, Decision, Identity, RateLimiter, Route, SIGN_IN_ROUTE, Stage,
};
use axum::extract::{FromRequestParts, MatchedPath, RawPathParams, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::net::{IpAddr, SocketAddr};
use utoipa::openapi::path::Operation;
use utoipa::openapi::{ContentBuilder, OpenApi, Ref, RefOr, ResponseBuilder};

/// The TCP peer of the connection a request arrived on, put on every request by the
/// connection loop in `main.rs`.
#[derive(Clone, Copy, Debug)]
pub struct PeerAddr(pub SocketAddr);

/// The client address the limits count by, once resolved through any trusted proxies.
#[derive(Clone, Copy, Debug)]
pub struct ClientIp(pub Option<IpAddr>);

/// The route of the event stream, which is not in the OpenAPI document.
const EVENT_STREAM_ROUTE: (&str, &str) = ("GET", "/events");
/// The passkey handoff page, served outside `API_PREFIX` and not in the OpenAPI document. Its
/// route key is its path as served.
pub const PASSKEY_PAGE: (&str, &str) = ("GET", "/auth/passkey");
/// This deployment's federation document, served outside `API_PREFIX` at the path every
/// deployment uses, and not in the OpenAPI document.
pub const WELL_KNOWN: (&str, &str) = ("GET", "/.well-known/aspen");
/// The page unsubscribe links in mail open, served outside `API_PREFIX` and not in the OpenAPI
/// document; posting to it unsubscribes, under the same limits.
pub const UNSUBSCRIBE_PAGE: (&str, &str) = ("GET", "/email/unsubscribe");

/// Plugins' own routes (`api::plugin::route`), which are not in the OpenAPI document.
pub const PLUGIN_ROUTE: &str = "/plugins/{plugin}/routes/{*path}";
/// The files of plugins' views (`api::plugin::asset`), served to frames that hold no session.
pub const PLUGIN_ASSET: &str = "/plugins/{plugin}/assets/{*path}";
/// People's private URLs to plugins' routes (`api::plugin::capability`).
pub const PLUGIN_CAPABILITY: &str = "/plugins/{plugin}/capabilities/{secret}";
/// The methods plugins' routes answer.
const PLUGIN_ROUTE_METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];

/// Every route the limits can name: the OpenAPI document's operations, the event stream, the
/// passkey page, the federation document, and plugins' routes.
pub fn routes() -> Vec<Route> {
    let openapi = crate::api::openapi();
    let mut routes = Vec::new();
    for (path, item) in &openapi.paths.paths {
        let template = path.strip_prefix(API_PREFIX).unwrap_or(path);
        for (method, operation) in operations(item) {
            routes.push(Route::new(method, template, access(operation)));
        }
    }
    for (method, template) in [
        EVENT_STREAM_ROUTE,
        PASSKEY_PAGE,
        WELL_KNOWN,
        UNSUBSCRIBE_PAGE,
        ("POST", UNSUBSCRIBE_PAGE.1),
    ] {
        routes.push(Route::new(method, template, Access::Anonymous));
    }
    for method in PLUGIN_ROUTE_METHODS {
        routes.push(Route::new(method, PLUGIN_ROUTE, Access::Authenticated));
    }
    routes.push(Route::new("GET", PLUGIN_ASSET, Access::Anonymous));
    routes.push(Route::new("GET", PLUGIN_CAPABILITY, Access::Anonymous));
    routes
}

fn operations(item: &utoipa::openapi::PathItem) -> Vec<(&'static str, &Operation)> {
    [
        ("GET", &item.get),
        ("POST", &item.post),
        ("PUT", &item.put),
        ("PATCH", &item.patch),
        ("DELETE", &item.delete),
    ]
    .into_iter()
    .filter_map(|(method, operation)| operation.as_ref().map(|op| (method, op)))
    .collect()
}

fn operations_mut(item: &mut utoipa::openapi::PathItem) -> Vec<&mut Operation> {
    [
        &mut item.get,
        &mut item.post,
        &mut item.put,
        &mut item.patch,
        &mut item.delete,
    ]
    .into_iter()
    .filter_map(Option::as_mut)
    .collect()
}

/// An operation's `security`: none, a list including the empty requirement (optional), or
/// bearer only.
fn access(operation: &Operation) -> Access {
    match &operation.security {
        None => Access::Anonymous,
        Some(requirements) if requirements.is_empty() => Access::Anonymous,
        Some(requirements) => {
            let optional = requirements.iter().any(|requirement| {
                serde_json::to_value(requirement)
                    .is_ok_and(|value| value.as_object().is_some_and(|o| o.is_empty()))
            });
            if optional {
                Access::Optional
            } else {
                Access::Authenticated
            }
        }
    }
}

/// Every operation can be refused for going too fast; the document says so on each.
pub fn document_rate_limits(openapi: &mut OpenApi) {
    for item in openapi.paths.paths.values_mut() {
        for operation in operations_mut(item) {
            let responses = &mut operation.responses.responses;
            match responses.get_mut("429") {
                Some(RefOr::T(existing)) => {
                    existing.description = format!("{} or `rateLimited`", existing.description);
                }
                Some(RefOr::Ref(_)) => {}
                None => {
                    // Declared as every handler declares a Problem answer (`body = Problem`).
                    let limited = ResponseBuilder::new()
                        .description(
                            "`rateLimited`: too many requests; retry after `Retry-After` seconds",
                        )
                        .content(
                            "application/json",
                            ContentBuilder::new()
                                .schema(Some(Ref::from_schema_name("Problem")))
                                .build(),
                        )
                        .build();
                    responses.insert("429".into(), RefOr::T(limited));
                }
            }
        }
    }
}

/// The route key of a request (`"POST /channels/{channel}/messages"`).
fn route_of(method: &Method, matched: &MatchedPath) -> String {
    let path = matched.as_str();
    crate::app::rate_limit::route_key(
        method.as_str(),
        path.strip_prefix(API_PREFIX).unwrap_or(path),
    )
}

/// The client's address: the peer's, or behind a trusted proxy the address it forwarded
/// (`aspen_limits::ClientAddresses::client`).
fn client_ip(limiter: &RateLimiter, peer: Option<IpAddr>, headers: &HeaderMap) -> Option<IpAddr> {
    let forwarded = headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|value| value.to_str().ok());
    peer.map(|peer| limiter.addresses().client(peer, forwarded))
}

fn params(raw: &RawPathParams) -> Vec<(&str, &str)> {
    raw.iter().collect()
}

fn refuse(decision: Decision) -> ApiResult<()> {
    match decision {
        Decision::Allowed => Ok(()),
        Decision::Limited { retry_after } => Err(ApiError::rate_limited(retry_after)),
    }
}

/// Middleware on every API route: resolves the client address, then checks the rules that
/// need no session.
pub async fn limit_requests(
    State(state): State<GlobalServerContext>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let peer = parts.extensions.get::<PeerAddr>().map(|peer| peer.0.ip());
    let ip = client_ip(&state.rate_limiter, peer, &parts.headers);
    parts.extensions.insert(ClientIp(ip));
    if let Some(matched) = parts.extensions.get::<MatchedPath>().cloned() {
        let route = route_of(&parts.method, &matched);
        let raw = RawPathParams::from_request_parts(&mut parts, &state)
            .await
            .ok();
        let identity = Identity {
            ip,
            params: raw.as_ref().map(params).unwrap_or_default(),
            ..Identity::default()
        };
        let decision = state
            .rate_limiter
            .check(&state.valkey, &route, Stage::Request, &identity)
            .await;
        if let Err(refused) = refuse(decision) {
            return refused.into_response();
        }
    }
    next.run(Request::from_parts(parts, body)).await
}

/// Checks the rules that count by user, once the session has said who the caller is.
pub async fn limit_session(
    state: &GlobalServerContext,
    parts: &mut Parts,
    user: UserId,
) -> ApiResult<()> {
    let Some(matched) = parts.extensions.get::<MatchedPath>().cloned() else {
        return Ok(());
    };
    let route = route_of(&parts.method, &matched);
    let ip = parts.extensions.get::<ClientIp>().and_then(|ip| ip.0);
    let raw = RawPathParams::from_request_parts(parts, state).await.ok();
    let identity = Identity {
        ip,
        user: Some(user),
        params: raw.as_ref().map(params).unwrap_or_default(),
        username: None,
    };
    refuse(
        state
            .rate_limiter
            .check(&state.valkey, &route, Stage::Session, &identity)
            .await,
    )
}

/// Checks the rules that count by the username given to `POST /auth/login`.
pub async fn limit_sign_in(state: &GlobalServerContext, username: &str) -> ApiResult<()> {
    let identity = Identity {
        username: Some(username),
        ..Identity::default()
    };
    refuse(
        state
            .rate_limiter
            .check(&state.valkey, SIGN_IN_ROUTE, Stage::Username, &identity)
            .await,
    )
}

impl ApiError {
    pub fn rate_limited(retry_after: std::time::Duration) -> Self {
        Self::new(ProblemCode::RateLimited).with_retry_after(retry_after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_documented_operation_can_be_rate_limited() {
        let openapi = crate::api::openapi();
        for (path, item) in &openapi.paths.paths {
            for (method, operation) in operations(item) {
                let Some(RefOr::T(response)) = operation.responses.responses.get("429") else {
                    panic!("{method} {path} documents no 429");
                };
                assert!(
                    response.description.contains("rateLimited"),
                    "{method} {path}"
                );
            }
        }
    }

    #[test]
    fn the_client_address_reads_forwarded_headers() {
        let config = crate::aspen_config::RateLimitConfig {
            enabled: true,
            trusted_proxies: vec!["10.0.0.0/8".into()],
            ipv6_prefix: 64,
            max_suspension_seconds: 3600,
            ..Default::default()
        };
        let limiter = RateLimiter::compile(&config, &[]).unwrap();
        let mut headers = HeaderMap::new();
        headers.append("x-forwarded-for", "6.6.6.6, 198.51.100.9".parse().unwrap());
        let ip = |text: &str| Some(text.parse::<IpAddr>().unwrap());
        assert_eq!(
            client_ip(&limiter, ip("10.0.0.2"), &headers),
            ip("198.51.100.9")
        );
        assert_eq!(
            client_ip(&limiter, ip("192.0.2.1"), &headers),
            ip("192.0.2.1")
        );
        assert_eq!(client_ip(&limiter, None, &headers), None);
    }

    #[test]
    fn routes_know_who_may_call_them() {
        let routes = routes();
        let find = |key: &str| routes.iter().find(|r| r.key == key).unwrap();
        assert_eq!(find("POST /auth/login").access, Access::Anonymous);
        assert_eq!(
            find("POST /auth/passkey-ceremonies").access,
            Access::Optional
        );
        assert_eq!(
            find("POST /channels/{channel}/messages").access,
            Access::Authenticated
        );
        assert_eq!(
            find("POST /channels/{channel}/messages").params,
            ["channel"]
        );
        assert_eq!(find("GET /events").access, Access::Anonymous);
        assert_eq!(find("GET /auth/passkey").access, Access::Anonymous);
    }
}
