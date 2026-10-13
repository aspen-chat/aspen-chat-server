//! Whether this server can do its work, for `GET /health/ready`: it reaches its database, NATS,
//! and Valkey. Whether it is running at all (`GET /health`) needs nothing of this.
//!
//! Anyone may ask, as often as they like, so the services are asked at most once in [`FRESH`]
//! however many requests arrive: each of them takes the one answer.

use crate::context::GlobalServerContext;
use async_nats::connection::State;
use diesel_async::RunQueryDsl;
use fred::interfaces::ClientLike;
use std::time::{Duration, Instant};
use tokio::time::timeout;

/// How long one answer stands for everyone who asks.
const FRESH: Duration = Duration::from_secs(1);
/// How long the database and Valkey have to answer before they count as unreachable. Short of
/// what the things that ask (load balancers, proxies) wait themselves, so they get an answer
/// that says what is wrong rather than none.
const ANSWER_WITHIN: Duration = Duration::from_secs(2);

/// Which of the services this server needs answered it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Readiness {
    pub database: bool,
    pub nats: bool,
    pub valkey: bool,
}

impl Readiness {
    pub fn is_ready(self) -> bool {
        self.database && self.nats && self.valkey
    }

    /// The services that did not answer, by the names their operators know them by.
    pub fn unreachable(self) -> Vec<&'static str> {
        [
            (self.database, "PostgreSQL"),
            (self.nats, "NATS"),
            (self.valkey, "Valkey"),
        ]
        .into_iter()
        .filter_map(|(reached, name)| (!reached).then_some(name))
        .collect()
    }
}

/// The last answer and when it was found, which [`readiness`] gives again while it is fresh.
#[derive(Default)]
pub struct LastChecked(tokio::sync::Mutex<Option<(Instant, Readiness)>>);

/// Whether this server reaches what it needs, as of at most [`FRESH`] ago. Requests that arrive
/// while the services are being asked wait for that answer rather than asking again.
pub async fn readiness(state: &GlobalServerContext) -> Readiness {
    let mut last = state.readiness.0.lock().await;
    if let Some((at, answer)) = *last
        && at.elapsed() < FRESH
    {
        return answer;
    }
    let answer = check(state).await;
    let before = last.map(|(_, before)| before);
    // Said once when it changes, not each time something asks.
    if before != Some(answer) {
        if !answer.is_ready() {
            tracing::warn!(
                unreachable = ?answer.unreachable(),
                "this server is not ready: it cannot reach every service it needs"
            );
        } else if before.is_some() {
            tracing::info!("this server is ready again");
        }
    }
    *last = Some((Instant::now(), answer));
    answer
}

async fn check(state: &GlobalServerContext) -> Readiness {
    let database = async {
        let mut conn = state.connection_pool.get().await.ok()?;
        diesel::sql_query("SELECT 1")
            .execute(conn.as_mut())
            .await
            .ok()
    };
    let (database, valkey) = tokio::join!(
        timeout(ANSWER_WITHIN, database),
        timeout(ANSWER_WITHIN, state.valkey.ping::<String>(None)),
    );
    Readiness {
        database: matches!(database, Ok(Some(_))),
        // The client's own account of its connection, which it keeps with pings of its own:
        // nothing is sent here.
        nats: state.nats_context.client().connection_state() == State::Connected,
        valkey: matches!(valkey, Ok(Ok(_))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_what_did_not_answer() {
        let all = Readiness {
            database: true,
            nats: true,
            valkey: true,
        };
        assert!(all.is_ready());
        assert!(all.unreachable().is_empty());
        let some = Readiness {
            database: false,
            valkey: false,
            ..all
        };
        assert!(!some.is_ready());
        assert_eq!(some.unreachable(), ["PostgreSQL", "Valkey"]);
    }
}
