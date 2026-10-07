//! The public half of the key the API servers sign join tokens with, which this server checks
//! tokens against and could never sign with. It asks the API servers for it over NATS
//! (`voice_protocol::control::TOKEN_KEY_SUBJECT`): once at startup, retrying until one answers,
//! and again whenever a token names a key it does not hold, at most once per
//! `MIN_FETCH_INTERVAL`, so tokens naming made-up keys cannot make it ask over and over. Only
//! the API servers may publish to this server's inbox, so the answer is theirs.

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing::{info, warn};
use voice_protocol::control::{TOKEN_KEY_SUBJECT, TokenKey};
use voice_protocol::token::key_id;

/// How long the API servers have to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(3);
/// The least time between two requests for the key.
const MIN_FETCH_INTERVAL: Duration = Duration::from_secs(5);

pub struct TokenKeys {
    client: async_nats::Client,
    /// Public keys by `key_id`.
    known: Mutex<HashMap<String, Vec<u8>>>,
    /// When the key was last asked for; held while asking, so asks never overlap.
    asked: tokio::sync::Mutex<Option<Instant>>,
}

impl TokenKeys {
    /// Starts asking for the key until the API servers answer.
    pub fn start(client: async_nats::Client) -> Arc<Self> {
        let keys = Arc::new(Self {
            client,
            known: Mutex::default(),
            asked: tokio::sync::Mutex::default(),
        });
        let asking = Arc::clone(&keys);
        tokio::spawn(async move {
            loop {
                if asking.fetch().await {
                    break;
                }
                warn!("no API server answered for the join token key; joins wait until one does");
                tokio::time::sleep(MIN_FETCH_INTERVAL).await;
            }
        });
        keys
    }

    /// The public key `key` names, asking the API servers when it is not held and the last ask
    /// was long enough ago; `None` when they do not know it either.
    pub async fn public_key(&self, key: &str) -> Option<Vec<u8>> {
        if let Some(found) = self.held(key) {
            return Some(found);
        }
        let mut asked = self.asked.lock().await;
        // Another join may have asked while this one waited.
        if let Some(found) = self.held(key) {
            return Some(found);
        }
        if asked.is_some_and(|at| at.elapsed() < MIN_FETCH_INTERVAL) {
            return None;
        }
        *asked = Some(Instant::now());
        drop(asked);
        self.fetch().await;
        self.held(key)
    }

    fn held(&self, key: &str) -> Option<Vec<u8>> {
        self.known
            .lock()
            .expect("token keys lock")
            .get(key)
            .cloned()
    }

    /// Asks the API servers for the key and keeps it; false when none answered with one.
    async fn fetch(&self) -> bool {
        let answer = tokio::time::timeout(
            ANSWER_TIMEOUT,
            self.client.request(TOKEN_KEY_SUBJECT, Vec::new().into()),
        )
        .await;
        let message = match answer {
            Ok(Ok(message)) => message,
            Ok(Err(e)) => {
                warn!(error = %e, "could not ask for the join token key");
                return false;
            }
            Err(_) => return false,
        };
        let Ok(key) = serde_json::from_slice::<TokenKey>(&message.payload) else {
            warn!("the API servers answered for the join token key with something unreadable");
            return false;
        };
        let Ok(public) = URL_SAFE_NO_PAD.decode(&key.public_key) else {
            warn!("the API servers answered for the join token key with something unreadable");
            return false;
        };
        if key_id(&public) != key.key_id {
            warn!("the join token key the API servers gave does not match its id");
            return false;
        }
        info!(key = key.key_id, "holding the join token key");
        self.known
            .lock()
            .expect("token keys lock")
            .insert(key.key_id, public);
        true
    }
}
