//! Device credentials (`app::device_credential`) for the simulated users, who present one on
//! every request they make before signing in. One is earned for each of the profile's source
//! addresses, or one in all without them, and the users connecting from an address share its
//! credential: a run limits its users as one device per address. A run that suspends the limits
//! (`suspension`) earns none, since a suspension also lets requests from its networks go
//! without one.

use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use std::net::IpAddr;
use std::time::Duration;

/// Earns a credential from `api` for requests sent from `source`.
pub async fn earn(api: &str, source: Option<IpAddr>) -> Result<String, String> {
    let http = reqwest::Client::builder()
        .local_address(source)
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let post = |path: &str, body: Value| {
        http.post(format!("{api}/api/v1/auth/{path}"))
            .json(&body)
            .send()
    };
    let challenge: Value = post("device-challenges", json!({}))
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("asking for a device challenge: {e}"))?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let token = challenge["challenge"]
        .as_str()
        .ok_or("no challenge given")?;
    let bits = challenge["difficulty"]
        .as_u64()
        .and_then(|bits| u8::try_from(bits).ok())
        .ok_or("no difficulty given")?;
    let sealed = BASE64_URL_SAFE_NO_PAD
        .decode(token)
        .map_err(|e| e.to_string())?;
    let nonce = tokio::task::spawn_blocking(move || aspen_limits::puzzle::solve(&sealed, bits))
        .await
        .map_err(|e| e.to_string())?;
    let earned: Value = post(
        "device-credentials",
        json!({ "challenge": token, "nonce": nonce }),
    )
    .await
    .and_then(reqwest::Response::error_for_status)
    .map_err(|e| format!("redeeming a device challenge: {e}"))?
    .json()
    .await
    .map_err(|e| e.to_string())?;
    earned["credential"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| "no credential given".to_string())
}
