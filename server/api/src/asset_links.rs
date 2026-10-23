//! This deployment's Digital Asset Links at `/.well-known/assetlinks.json`: the Android apps
//! that may use its passkeys themselves, through Credential Manager (`app::passkey::android_apps`).
//! Android lets an app use a domain's passkeys only once the domain names it here, with the
//! `get_login_creds` relation, and Google reads the file from the domain itself, so it is served
//! at the path every site uses, outside `/api/v1`, and is not in the OpenAPI document.

use aspen_app::context::GlobalServerContext;
use aspen_app::passkey::android_apps;
use axum::Json;
use axum::extract::State;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// How long Google and devices may keep the file before reading it again.
const MAX_AGE_SECONDS: u32 = 3600;

/// The one relation Credential Manager needs to offer a site's passkeys to an app. App Links'
/// `handle_all_urls` is left out: the apps open no web addresses of a deployment themselves.
const GET_LOGIN_CREDS: &str = "delegate_permission/common.get_login_creds";

#[derive(Serialize)]
struct Statement {
    relation: [&'static str; 1],
    target: Target,
}

#[derive(Serialize)]
struct Target {
    namespace: &'static str,
    package_name: String,
    sha256_cert_fingerprints: Vec<String>,
}

/// One statement per package, naming every certificate it may be signed with; an empty list
/// where no app is named, or passkeys are unavailable.
fn statements(config: &aspen_app::aspen_config::AspenConfig) -> Vec<Statement> {
    let mut statements: Vec<Statement> = Vec::new();
    for app in android_apps(config) {
        let fingerprint = app.fingerprint_hex();
        match statements
            .iter_mut()
            .find(|statement| statement.target.package_name == app.package)
        {
            Some(statement) => statement.target.sha256_cert_fingerprints.push(fingerprint),
            None => statements.push(Statement {
                relation: [GET_LOGIN_CREDS],
                target: Target {
                    namespace: "android_app",
                    package_name: app.package,
                    sha256_cert_fingerprints: vec![fingerprint],
                },
            }),
        }
    }
    statements
}

pub async fn asset_links(State(state): State<GlobalServerContext>) -> Response {
    let mut response = Json(statements(&state.config)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(&format!("public, max-age={MAX_AGE_SECONDS}"))
            .expect("a valid header value"),
    );
    response
}
