//! Whether someone is in front of Aspen right now, which decides whether their phones are woken
//! (`app::push`): a connection is attended while the client says so with `attending` frames, until
//! it says `stoppedAttending`, ends, or sends none for [`ATTENDED_FOR`].
//!
//! Each user's attended connections are a Valkey sorted set, [`attended_key`], whose members are
//! connection ids scored by when each stops counting, in Unix milliseconds on Valkey's clock, so
//! the servers that mark and the servers that ask never compare clocks. One connection going idle
//! never answers for another of the same user.

use std::time::Duration;

use fred::prelude::{LuaInterface as _, SortedSetsInterface as _};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::{GlobalServerContext, UserId};

/// How long a connection counts as attended after its last `attending` frame.
pub const ATTENDED_FOR: Duration = Duration::from_secs(60);

/// The least time between two `attending` frames of one connection that are written; clients send
/// them at most this often while their user goes on using them.
pub const ATTENDING_REFRESH: Duration = Duration::from_secs(15);

/// Frames arriving a little early still count, so a client's timer jitter never costs one.
const REFRESH_GRACE: Duration = Duration::from_secs(2);

/// The least time between two writes of one connection, however its frames come, so a client
/// flipping between the two frames costs Valkey at most one command a second. A frame overtaken
/// meanwhile is never written; the latest is.
const WRITE_GAP: Duration = Duration::from_secs(1);

/// How many users one Valkey call asks about.
pub const ASK_BATCH: usize = 1000;

/// The Valkey sorted set of `user`'s attended connections: `user:{uuid}:attended`.
pub fn attended_key(user: UserId) -> String {
    format!("user:{}:attended", user.0)
}

/// Marks `ARGV[2]`, a connection of the user whose set is `KEYS[1]`, attended for `ARGV[1]`
/// milliseconds, forgetting members whose time has passed. The set lives as long as its newest
/// member, which this one is.
const MARK: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local lasting = tonumber(ARGV[1])
redis.call('ZREMRANGEBYSCORE', KEYS[1], '-inf', now)
redis.call('ZADD', KEYS[1], now + lasting, ARGV[2])
redis.call('PEXPIRE', KEYS[1], lasting)
return 1
";

/// For each set of `KEYS`, whether any member's time has not yet passed: 1 or 0.
const ASK: &str = r"
local time = redis.call('TIME')
local now = tonumber(time[1]) * 1000 + math.floor(tonumber(time[2]) / 1000)
local found = {}
for i, key in ipairs(KEYS) do
  found[i] = math.min(redis.call('ZCOUNT', key, '(' .. now, '+inf'), 1)
end
return found
";

/// Which of `users` have a connection attended now, in their order. Asked [`ASK_BATCH`] at a
/// time, the batches at once, so a message to a whole community never sends Valkey one command
/// naming every member.
pub async fn attended(state: &GlobalServerContext, users: &[UserId]) -> crate::Result<Vec<bool>> {
    attended_on(&state.valkey, users).await
}

async fn attended_on(valkey: &fred::clients::Client, users: &[UserId]) -> crate::Result<Vec<bool>> {
    let batches = futures_util::future::try_join_all(users.chunks(ASK_BATCH).map(|batch| {
        let keys: Vec<String> = batch.iter().map(|user| attended_key(*user)).collect();
        async move {
            let found: Vec<i64> = valkey.eval(ASK, keys, Vec::<String>::new()).await?;
            Ok::<_, crate::Error>(found)
        }
    }))
    .await?;
    let found: Vec<bool> = batches.into_iter().flatten().map(|n| n > 0).collect();
    if found.len() != users.len() {
        return Err(fred::error::Error::new(
            fred::error::ErrorKind::Unknown,
            "Valkey answered whether users are attended for a different number of them",
        )
        .into());
    }
    Ok(found)
}

/// One event stream connection's attention, written in the order its frames say it by a task of
/// its own, so Valkey's answer holds up neither the connection's events nor its later frames.
/// Dropping it (the connection ending) ends the connection's attention.
pub struct Attention {
    wanted: watch::Sender<bool>,
}

impl Attention {
    pub fn spawn(state: GlobalServerContext, user: UserId) -> Self {
        let (wanted, received) = watch::channel(false);
        tokio::spawn(run(state.valkey, user, received));
        Attention { wanted }
    }

    /// The user is using the app on this connection.
    pub fn attending(&self) {
        self.wanted.send_replace(true);
    }

    /// The user stopped using the app on this connection: it went to the background, lost focus,
    /// or was left.
    pub fn stopped(&self) {
        self.wanted.send_replace(false);
    }
}

async fn run(valkey: fred::clients::Client, user: UserId, mut wanted: watch::Receiver<bool>) {
    let key = attended_key(user);
    let connection = uuid::Uuid::now_v7().to_string();
    // When this connection was last written attended, while it still is.
    let mut marked: Option<Instant> = None;
    while wanted.changed().await.is_ok() {
        let attending = *wanted.borrow_and_update();
        let now = Instant::now();
        if attending {
            if marked.is_some_and(|at| now.duration_since(at) + REFRESH_GRACE < ATTENDING_REFRESH) {
                continue;
            }
            match mark(&valkey, &key, &connection, ATTENDED_FOR).await {
                Ok(()) => marked = Some(now),
                Err(e) => tracing::warn!(error = %e, "failed to record a connection as attended"),
            }
        } else if marked.take().is_some() {
            unmark(&valkey, &key, &connection).await;
        } else {
            continue;
        }
        tokio::time::sleep(WRITE_GAP).await;
    }
    if marked.is_some() {
        unmark(&valkey, &key, &connection).await;
    }
}

async fn mark(
    valkey: &fred::clients::Client,
    key: &str,
    connection: &str,
    lasting: Duration,
) -> Result<(), fred::error::Error> {
    let _: i64 = valkey
        .eval(
            MARK,
            vec![key.to_string()],
            vec![lasting.as_millis().to_string(), connection.to_string()],
        )
        .await?;
    Ok(())
}

async fn unmark(valkey: &fred::clients::Client, key: &str, connection: &str) {
    let removed: Result<i64, fred::error::Error> = valkey.zrem(key, connection).await;
    if let Err(e) = removed {
        // Its member is forgotten [`ATTENDED_FOR`] after it was written instead.
        tracing::warn!(error = %e, "failed to record a connection as no longer attended");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Uses the Valkey of `docker-compose.yaml`, or `VALKEY_URL`.
    async fn test_valkey() -> fred::clients::Client {
        use fred::interfaces::ClientLike;
        let url = std::env::var("VALKEY_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".into());
        let client = fred::clients::Client::new(
            fred::prelude::Config::from_url(&url).unwrap(),
            None,
            None,
            None,
        );
        client
            .init()
            .await
            .expect("Valkey from docker-compose.yaml");
        client
    }

    const SHORT: Duration = Duration::from_millis(300);

    #[tokio::test]
    async fn a_connection_counts_until_it_stops_or_its_time_passes() {
        let valkey = test_valkey().await;
        let user = UserId::new();
        let key = attended_key(user);
        assert_eq!(attended_on(&valkey, &[user]).await.unwrap(), [false]);

        mark(&valkey, &key, "a", SHORT).await.unwrap();
        assert_eq!(attended_on(&valkey, &[user]).await.unwrap(), [true]);
        unmark(&valkey, &key, "a").await;
        assert_eq!(attended_on(&valkey, &[user]).await.unwrap(), [false]);

        mark(&valkey, &key, "a", SHORT).await.unwrap();
        tokio::time::sleep(SHORT + Duration::from_millis(100)).await;
        assert_eq!(attended_on(&valkey, &[user]).await.unwrap(), [false]);
    }

    #[tokio::test]
    async fn one_connection_stopping_leaves_another_attended() {
        let valkey = test_valkey().await;
        let user = UserId::new();
        let key = attended_key(user);
        mark(&valkey, &key, "phone", Duration::from_secs(5))
            .await
            .unwrap();
        mark(&valkey, &key, "desktop", Duration::from_secs(5))
            .await
            .unwrap();
        unmark(&valkey, &key, "phone").await;
        assert_eq!(attended_on(&valkey, &[user]).await.unwrap(), [true]);
        unmark(&valkey, &key, "desktop").await;
        assert_eq!(attended_on(&valkey, &[user]).await.unwrap(), [false]);
    }

    #[tokio::test]
    async fn an_expired_connection_is_not_kept_alive_by_a_newer_one() {
        let valkey = test_valkey().await;
        let user = UserId::new();
        let key = attended_key(user);
        mark(&valkey, &key, "left", SHORT).await.unwrap();
        mark(&valkey, &key, "used", Duration::from_secs(5))
            .await
            .unwrap();
        unmark(&valkey, &key, "used").await;
        tokio::time::sleep(SHORT + Duration::from_millis(100)).await;
        assert_eq!(attended_on(&valkey, &[user]).await.unwrap(), [false]);
    }

    #[tokio::test]
    async fn users_are_answered_in_order_across_batches() {
        let valkey = test_valkey().await;
        let users: Vec<UserId> = (0..ASK_BATCH + 3).map(|_| UserId::new()).collect();
        let chosen = [0, ASK_BATCH - 1, ASK_BATCH, ASK_BATCH + 2];
        for &i in &chosen {
            mark(
                &valkey,
                &attended_key(users[i]),
                "c",
                Duration::from_secs(5),
            )
            .await
            .unwrap();
        }
        let found = attended_on(&valkey, &users).await.unwrap();
        assert_eq!(found.len(), users.len());
        for (i, attended) in found.iter().enumerate() {
            assert_eq!(*attended, chosen.contains(&i), "user {i}");
        }
        for &i in &chosen {
            unmark(&valkey, &attended_key(users[i]), "c").await;
        }
    }

    #[tokio::test]
    async fn the_task_marks_on_attending_and_unmarks_when_dropped() {
        let valkey = test_valkey().await;
        let user = UserId::new();
        let (wanted, received) = watch::channel(false);
        let task = tokio::spawn(run(valkey.clone(), user, received));
        wanted.send_replace(true);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(attended_on(&valkey, &[user]).await.unwrap(), [true]);
        drop(wanted);
        task.await.unwrap();
        assert_eq!(attended_on(&valkey, &[user]).await.unwrap(), [false]);
    }

    #[tokio::test]
    async fn the_task_writes_the_latest_of_frames_closer_than_its_gap() {
        let valkey = test_valkey().await;
        let user = UserId::new();
        let (wanted, received) = watch::channel(false);
        let task = tokio::spawn(run(valkey.clone(), user, received));
        wanted.send_replace(true);
        tokio::time::sleep(Duration::from_millis(200)).await;
        wanted.send_replace(false);
        wanted.send_replace(true);
        wanted.send_replace(false);
        tokio::time::sleep(WRITE_GAP + Duration::from_millis(300)).await;
        assert_eq!(attended_on(&valkey, &[user]).await.unwrap(), [false]);
        drop(wanted);
        task.await.unwrap();
    }
}
