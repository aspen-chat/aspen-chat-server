//! What follows from someone's presence keys being set (`app::user_status`), done for many
//! people at once by one task per server rather than by a task of its own for each:
//!
//! - **Arriving.** When their `online` key was not set, they are coming online. Their
//!   `last_seen_at` is written on their account, and their presence override, read back by the
//!   same statement, is copied to Valkey; only then is their `online` key set, and those watching
//!   them told (`app::presence_feed`). Whenever the key exists, so does the copy of what they
//!   chose, even after Valkey lost its keys, so someone who chose invisible is never shown online
//!   in between: until their arrival is done, they show as offline to everyone. Their
//!   `last_seen_at` is then written on each membership, which the member samples are read by.
//! - **Listing.** Each community they belong to lists them as someone who may be online, renewed
//!   once per margin however often their keys are set (`user:{uuid}:listed`).
//!
//! Done one person at a time, each would take a database connection and a statement of its own,
//! and a crowd connecting at once (an ISP's customers coming back from an outage, a server
//! restarting) would make as many tasks, each queueing for the pool ahead of the requests of
//! those already connected. Here at most [`BATCH`] people share a statement, the task holds one
//! connection at a time, and a crowd makes the work wait rather than the requests.
//!
//! Each person is waiting at most once, so what waits is bounded by the people this server
//! marked online, and so by its connections and the users of its requests. Work that fails is
//! logged and not tried again here. An arrival that fails before its copy leaves the `online` key
//! unset, so the next mark, at most `user_status::MARK_EVERY` later while they stay connected,
//! is an arrival again; while the database cannot be read, those coming online show as offline.
//! A listing that fails is made at the next mark too.

use crate::presence_feed::PresenceFeed;
use crate::presence_override::PresenceOverride;
use crate::{CommunityId, UserId};
use aspen_schema::community_user;
use diesel::prelude::*;
use diesel_async::pooled_connection::deadpool::Pool;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use fred::interfaces::{KeysInterface, SortedSetsInterface};
use fred::types::{Expiration, SetOptions};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

/// The most people one statement writes the arrival or reads the memberships of. A person
/// belongs to at most `[limits] max_communities_per_user` communities, so this bounds the
/// memberships one statement touches too.
const BATCH: usize = 100;

/// How long listings last, from the configuration.
#[derive(Clone, Copy)]
pub struct ListingTimes {
    /// How long a listing covers before its margin, in seconds.
    pub ttl: i64,
    /// How long a listing serves before the next renewal, in seconds.
    pub margin: i64,
    /// How long a community's set lives after its last listing, in seconds.
    pub set_lifetime: i64,
}

/// One server's batched presence upkeep; cheap to clone.
#[derive(Clone)]
pub struct PresenceUpkeep {
    waiting: Arc<Mutex<Waiting>>,
    wake: Arc<Notify>,
}

#[derive(Default)]
struct Waiting {
    /// Those coming online, and whether each is a bot.
    arriving: HashMap<UserId, bool>,
    to_list: HashSet<UserId>,
}

impl Waiting {
    fn take(&mut self) -> (HashMap<UserId, bool>, Vec<UserId>) {
        let arriving: HashMap<UserId, bool> = self
            .arriving
            .iter()
            .take(BATCH)
            .map(|(user, bot)| (*user, *bot))
            .collect();
        for user in arriving.keys() {
            self.arriving.remove(user);
        }
        let to_list: Vec<UserId> = self.to_list.iter().take(BATCH).copied().collect();
        for user in &to_list {
            self.to_list.remove(user);
        }
        (arriving, to_list)
    }
}

/// A user's row as their arrival wrote it.
#[derive(QueryableByName)]
struct Arrived {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    id: uuid::Uuid,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    presence_override: Option<PresenceOverride>,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Timestamptz>)]
    presence_override_until: Option<chrono::DateTime<chrono::Utc>>,
}

/// What the task works with.
struct Upkeep {
    pool: Pool<AsyncPgConnection>,
    valkey: fred::clients::Client,
    feed: PresenceFeed,
    times: ListingTimes,
}

impl PresenceUpkeep {
    /// Starts the task that does the upkeep.
    pub fn start(
        pool: Pool<AsyncPgConnection>,
        valkey: fred::clients::Client,
        feed: PresenceFeed,
        times: ListingTimes,
    ) -> Self {
        let upkeep = Self {
            waiting: Arc::default(),
            wake: Arc::new(Notify::new()),
        };
        tokio::spawn(upkeep.clone().run(Upkeep {
            pool,
            valkey,
            feed,
            times,
        }));
        upkeep
    }

    /// `user` is coming online: their `online` key was not set, and is set once their arrival
    /// is recorded. A bot's `active` key is set with it.
    pub fn arriving(&self, user: UserId, bot: bool) {
        self.add(|waiting| waiting.arriving.insert(user, bot).is_none());
    }

    /// One of `user`'s keys was set: they are listed in their communities unless a listing made
    /// within the margin covers them.
    pub fn list(&self, user: UserId) {
        self.add(|waiting| waiting.to_list.insert(user));
    }

    fn add(&self, insert: impl FnOnce(&mut Waiting) -> bool) {
        let added = insert(&mut self.waiting.lock().unwrap_or_else(|e| e.into_inner()));
        if added {
            self.wake.notify_one();
        }
    }

    async fn run(self, upkeep: Upkeep) {
        loop {
            self.wake.notified().await;
            loop {
                let (arriving, to_list) = self
                    .waiting
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                if arriving.is_empty() && to_list.is_empty() {
                    break;
                }
                if !arriving.is_empty() {
                    upkeep.arrive(&arriving).await;
                }
                if !to_list.is_empty() {
                    upkeep.list(&to_list).await;
                }
            }
        }
    }
}

impl Upkeep {
    /// Brings `arriving` online: writes their `last_seen_at`, copies their overrides to Valkey,
    /// sets the `online` key of each whose copy landed, tells those watching them, and writes
    /// `last_seen_at` on their memberships. The rows are locked in key order first: another
    /// server may write some of the same people's at once (their other devices), and rows locked
    /// in any order could deadlock.
    async fn arrive(&self, arriving: &HashMap<UserId, bool>) {
        let ids: Vec<uuid::Uuid> = arriving.keys().map(|user| user.0).collect();
        let rows = match self.record_arrivals(&ids).await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(error = %e, "failed to record when users came online");
                return;
            }
        };
        let online = rows.into_iter().map(|row| async move {
            let user = UserId(row.id);
            let bot = arriving.get(&user).copied().unwrap_or(false);
            let came = async {
                crate::presence_override::copy_row_to_valkey(
                    &self.valkey,
                    &self.feed,
                    user,
                    row.presence_override,
                    row.presence_override_until,
                )
                .await?;
                crate::user_status::set_online_keys(&self.valkey, &self.feed, user, bot).await
            };
            match came.await {
                // After the override is copied, so those told read what the user chose.
                Ok(()) => self.feed.changed(user),
                Err(e) => tracing::warn!(error = %e, "failed to bring a user online"),
            }
        });
        futures_util::future::join_all(online).await;
        if let Err(e) = self.record_memberships_seen(&ids).await {
            tracing::warn!(error = %e, "failed to record when users came online");
        }
    }

    /// Writes `ids`' `last_seen_at` as now and reads back their presence overrides.
    async fn record_arrivals(&self, ids: &[uuid::Uuid]) -> crate::Result<Vec<Arrived>> {
        use diesel::sql_types::{Array, Uuid};
        Ok(diesel::sql_query(
            r#"
            UPDATE "user" u SET last_seen_at = now()
            FROM (SELECT id FROM "user" WHERE id = ANY($1) ORDER BY id FOR UPDATE) locked
            WHERE u.id = locked.id
            RETURNING u.id, u.presence_override, u.presence_override_until
            "#,
        )
        .bind::<Array<Uuid>, _>(ids)
        .load(self.pool.get().await?.as_mut())
        .await?)
    }

    /// Writes `ids`' `last_seen_at` as now on each of their memberships.
    async fn record_memberships_seen(&self, ids: &[uuid::Uuid]) -> crate::Result<()> {
        use diesel::sql_types::{Array, Uuid};
        diesel::sql_query(
            r#"
            UPDATE community_user cu SET last_seen_at = now()
            FROM (
                SELECT "user", community FROM community_user
                WHERE "user" = ANY($1)
                ORDER BY "user", community
                FOR UPDATE
            ) locked
            WHERE cu."user" = locked."user" AND cu.community = locked.community
            "#,
        )
        .bind::<Array<Uuid>, _>(ids)
        .execute(self.pool.get().await?.as_mut())
        .await?;
        Ok(())
    }

    /// Lists those of `users` whose listings are due in each community they belong to.
    async fn list(&self, users: &[UserId]) {
        let due = self.due_for_listing(users).await;
        if due.is_empty() {
            return;
        }
        if let Err(e) = self.renew_listings(&due).await {
            tracing::warn!(error = %e, "failed to renew users' community listings");
            // The next time a key of theirs is set tries again rather than waiting out the
            // margin.
            let dels = due.iter().map(|user| {
                self.valkey
                    .del::<(), _>(crate::user_status::listed_key(*user))
            });
            futures_util::future::join_all(dels).await;
        }
    }

    /// Those of `users` with no listing made within the margin, each now marked as listed.
    async fn due_for_listing(&self, users: &[UserId]) -> Vec<UserId> {
        let marks = users.iter().map(|user| async move {
            let fresh: Result<Option<String>, _> = self
                .valkey
                .set(
                    crate::user_status::listed_key(*user),
                    1,
                    Some(Expiration::EX(self.times.margin.max(1))),
                    Some(SetOptions::NX),
                    false,
                )
                .await;
            match fresh {
                Ok(fresh) => fresh.is_some().then_some(*user),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to renew the user's community listings");
                    None
                }
            }
        });
        futures_util::future::join_all(marks)
            .await
            .into_iter()
            .flatten()
            .collect()
    }

    /// Lists `users` in every community they belong to until a listing's whole life from now,
    /// each community's set written once for all of them, in one round trip.
    async fn renew_listings(&self, users: &[UserId]) -> crate::Result<()> {
        let memberships: Vec<(UserId, CommunityId)> = community_user::table
            .select((community_user::user, community_user::community))
            .filter(community_user::user.eq_any(users))
            .filter(community_user::community.eq_any(crate::community::live()))
            .load(self.pool.get().await?.as_mut())
            .await?;
        if memberships.is_empty() {
            return Ok(());
        }
        let mut by_community: HashMap<CommunityId, Vec<UserId>> = HashMap::new();
        for (user, community) in memberships {
            by_community.entry(community).or_default().push(user);
        }
        let until = chrono::Utc::now()
            .timestamp()
            .saturating_add(self.times.ttl.saturating_add(self.times.margin))
            as f64;
        let pipeline = self.valkey.pipeline();
        for (community, members) in by_community {
            let key = crate::user_status::community_online_key(community);
            let scored: Vec<(f64, String)> = members
                .into_iter()
                .map(|user| (until, user.0.to_string()))
                .collect();
            let () = pipeline
                .zadd(&key, None, None, false, false, scored)
                .await?;
            let () = pipeline.expire(&key, self.times.set_lifetime, None).await?;
        }
        let _: Vec<fred::types::Value> = pipeline.all().await?;
        Ok(())
    }
}
