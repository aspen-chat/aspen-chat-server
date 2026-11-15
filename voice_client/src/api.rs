//! The API server's part in a call: the join offer that names the voice servers to try and the
//! token to present to them, the failure a client reports when one of them does not take the
//! call, and each candidate's health check. The request and answer types are written from
//! `openapi.yaml`, since the server's own (`aspen_api::voice`) live over the database layer; a
//! field the server adds is ignored here, and one it has not got yet takes its default, as the
//! API changes only by addition.

use crate::transport::{Http, HttpRequest, TransportError};
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Where every REST route lives, as `api::API_PREFIX` has it.
pub const API_PREFIX: &str = "/api/v1";

/// The session the call acts as. The app layer signs in, refreshes, and keeps the token; the
/// engine asks for it before each request, so a refreshed token is used from the next one.
/// The apps implement it directly (`bindings`).
#[uniffi::export(with_foreign)]
#[async_trait]
pub trait Credentials: Send + Sync {
    /// The session token to send as `Authorization: Bearer`, or `None` when signed out.
    async fn session_token(&self) -> Option<String>;
}

/// A voice server a join offer names.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: Uuid,
    pub name: String,
    /// Its HTTP address: `/health` is read under it and `/ws` opened under it.
    pub url: String,
}

/// The call already on the channel, when there is one.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionRef {
    pub id: Uuid,
    pub channel: Uuid,
    pub voice_server: Uuid,
}

/// `POST /channels/{channel}/voice/join`'s answer.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct JoinOffer {
    pub channel_id: Uuid,
    #[serde(default)]
    pub session: Option<SessionRef>,
    /// At most ten, in no particular order; only the call's own server once a call is on.
    pub candidates: Vec<Candidate>,
    /// Presented to the voice server as `identify`; good for every candidate until `expires_at`.
    pub token: String,
    /// RFC 3339, on the API server's clock; kept as text, since the engine never compares it.
    pub expires_at: String,
    pub speak: bool,
    pub share_screen: bool,
    pub transfer_files: bool,
    pub use_camera: bool,
    /// Whether a moderator's mute of the user stands in the channel's community.
    #[serde(default)]
    pub server_muted: bool,
}

/// `POST /voice-servers/{server}/failures`'s answer.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FailureOutcome {
    pub failures: u32,
    pub disabled: bool,
    pub counted: bool,
}

/// An RFC 9457 Problem, as every refusal under `/api/v1` is written.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// The stable code to branch on (`forbidden`, `blocked`, `notFound`, `rateLimited`, ...);
    /// text rather than an enum, since a newer server may answer one this build does not know.
    pub code: String,
    /// Localized, for showing.
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub status: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    Transport(#[from] TransportError),
    /// The server refused, and said why.
    #[error("{}", .0.detail.as_deref().unwrap_or(&.0.title))]
    Problem(Problem),
    /// Nobody is signed in: `Credentials` gave no token.
    #[error("not signed in")]
    SignedOut,
    /// An answer that is neither what the route gives nor a Problem: a proxy's error page.
    #[error("the API server answered {status} with something other than a Problem")]
    Unreadable { status: u16 },
}

/// The REST calls a call makes, against one deployment.
pub struct Api {
    http: Arc<dyn Http>,
    /// The deployment's `public_url`, an origin alone, with no trailing slash.
    base_url: String,
    credentials: Arc<dyn Credentials>,
}

impl Api {
    pub fn new(http: Arc<dyn Http>, base_url: &str, credentials: Arc<dyn Credentials>) -> Self {
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            credentials,
        }
    }

    pub async fn join_offer(&self, channel: Uuid) -> Result<JoinOffer, ApiError> {
        self.post(&format!("/channels/{channel}/voice/join")).await
    }

    /// Reports that `server` failed to take the call.
    pub async fn report_failure(&self, server: Uuid) -> Result<FailureOutcome, ApiError> {
        self.post(&format!("/voice-servers/{server}/failures"))
            .await
    }

    /// `GET /health` of the voice server at `url`: how long its answer took, or `None` when it
    /// gave no good one within `timeout`. Unauthenticated, and the only request the engine
    /// makes to a voice server over HTTP.
    pub async fn voice_server_health(&self, url: &str, timeout: Duration) -> Option<Duration> {
        let request = HttpRequest {
            method: "GET".to_string(),
            url: format!("{}/health", url.trim_end_matches('/')),
            headers: Vec::new(),
            body: None,
        };
        let started = tokio::time::Instant::now();
        match tokio::time::timeout(timeout, self.http.request(request)).await {
            Ok(Ok(response)) if (200..300).contains(&response.status) => Some(started.elapsed()),
            _ => None,
        }
    }

    async fn post<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, ApiError> {
        let token = self
            .credentials
            .session_token()
            .await
            .ok_or(ApiError::SignedOut)?;
        let request = HttpRequest {
            method: "POST".to_string(),
            url: format!("{}{API_PREFIX}{path}", self.base_url),
            headers: vec![
                ("Authorization".to_string(), format!("Bearer {token}")),
                ("Accept".to_string(), "application/json".to_string()),
            ],
            body: None,
        };
        let response = self.http.request(request).await?;
        if (200..300).contains(&response.status) {
            return serde_json::from_str(&response.body).map_err(|_| ApiError::Unreadable {
                status: response.status,
            });
        }
        match serde_json::from_str::<Problem>(&response.body) {
            Ok(mut problem) => {
                if problem.status == 0 {
                    problem.status = response.status;
                }
                Err(ApiError::Problem(problem))
            }
            Err(_) => Err(ApiError::Unreadable {
                status: response.status,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::HttpResponse;
    use std::sync::Mutex;

    struct Answering {
        requests: Mutex<Vec<HttpRequest>>,
        answer: HttpResponse,
    }

    #[async_trait]
    impl Http for Answering {
        async fn request(&self, request: HttpRequest) -> Result<HttpResponse, TransportError> {
            self.requests.lock().unwrap().push(request);
            Ok(self.answer.clone())
        }
    }

    struct Token(Option<&'static str>);

    #[async_trait]
    impl Credentials for Token {
        async fn session_token(&self) -> Option<String> {
            self.0.map(str::to_string)
        }
    }

    fn answering(answer: HttpResponse, token: Option<&'static str>) -> (Api, Arc<Answering>) {
        let http = Arc::new(Answering {
            requests: Mutex::new(Vec::new()),
            answer,
        });
        (
            Api::new(
                http.clone(),
                "https://chat.example.org/",
                Arc::new(Token(token)),
            ),
            http,
        )
    }

    #[tokio::test]
    async fn a_join_offer_is_asked_for_with_the_session_and_read_leniently() {
        let channel = Uuid::now_v7();
        let body = format!(
            r#"{{"channelId":"{channel}","candidates":[{{"id":"{}","name":"a","url":"https://a.example.org","later":1}}],"token":"t","expiresAt":"2026-01-01T00:00:00Z","speak":true,"shareScreen":false,"transferFiles":false,"useCamera":true}}"#,
            Uuid::nil()
        );
        let (api, http) = answering(
            HttpResponse {
                status: 200,
                headers: Vec::new(),
                body,
            },
            Some("secret"),
        );
        let offer = api.join_offer(channel).await.unwrap();
        assert_eq!(offer.channel_id, channel);
        assert!(!offer.server_muted);
        assert_eq!(offer.candidates[0].url, "https://a.example.org");
        let requests = http.requests.lock().unwrap();
        assert_eq!(requests[0].method, "POST");
        assert_eq!(
            requests[0].url,
            format!("https://chat.example.org/api/v1/channels/{channel}/voice/join")
        );
        assert!(
            requests[0]
                .headers
                .contains(&("Authorization".to_string(), "Bearer secret".to_string()))
        );
    }

    #[tokio::test]
    async fn a_problem_is_read_and_a_missing_token_asks_nothing() {
        let (api, http) = answering(
            HttpResponse {
                status: 403,
                headers: Vec::new(),
                body: r#"{"code":"blocked","title":"Blocked","detail":"A block stands.","status":403}"#
                    .to_string(),
            },
            Some("secret"),
        );
        match api.join_offer(Uuid::now_v7()).await {
            Err(ApiError::Problem(problem)) => {
                assert_eq!(problem.code, "blocked");
                assert_eq!(problem.detail.as_deref(), Some("A block stands."));
            }
            other => panic!("{other:?}"),
        }
        let (signed_out, http_out) = answering(http.answer.clone(), None);
        assert_eq!(
            signed_out.report_failure(Uuid::now_v7()).await,
            Err(ApiError::SignedOut)
        );
        assert!(http_out.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_answer_that_is_no_problem_is_unreadable() {
        let (api, _) = answering(
            HttpResponse {
                status: 502,
                headers: Vec::new(),
                body: "<html>bad gateway</html>".to_string(),
            },
            Some("secret"),
        );
        assert_eq!(
            api.report_failure(Uuid::now_v7()).await,
            Err(ApiError::Unreadable { status: 502 })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn health_is_timed_and_a_bad_or_late_answer_is_none() {
        let (api, http) = answering(
            HttpResponse {
                status: 200,
                headers: Vec::new(),
                body: String::new(),
            },
            None,
        );
        assert!(
            api.voice_server_health("https://a.example.org/", Duration::from_secs(3))
                .await
                .is_some()
        );
        assert_eq!(
            http.requests.lock().unwrap()[0].url,
            "https://a.example.org/health"
        );
        let (sick, _) = answering(
            HttpResponse {
                status: 503,
                headers: Vec::new(),
                body: String::new(),
            },
            None,
        );
        assert_eq!(
            sick.voice_server_health("https://a.example.org", Duration::from_secs(3))
                .await,
            None
        );

        struct Late;
        #[async_trait]
        impl Http for Late {
            async fn request(&self, _: HttpRequest) -> Result<HttpResponse, TransportError> {
                tokio::time::sleep(Duration::from_secs(10)).await;
                unreachable!("the health check waits no longer than its timeout")
            }
        }
        let late = Api::new(
            Arc::new(Late),
            "https://chat.example.org",
            Arc::new(Token(None)),
        );
        assert_eq!(
            late.voice_server_health("https://a.example.org", Duration::from_secs(3))
                .await,
            None
        );
    }
}
