//! Presence. Two Valkey keys per user describe it:
//!
//! - `user:{uuid}:online` exists while the user has a connection: it is set with a short expiry
//!   when they connect the event stream (or make any authenticated request) and refreshed by the
//!   stream's pings, so it expires shortly after their last connection goes. Setting it when it
//!   was not set is their coming online, which writes their `last_seen_at`.
//! - `user:{uuid}:active` exists while they are using Aspen: a client sends an `activity` frame
//!   on its event stream while its user interacts with it, and each sets the key to expire after
//!   `[presence] away_after_seconds`.
//!
//! A bot's `active` key is set with its `online` key and lives as long, so a connected bot is
//! online and never away: it uses Aspen through the API, not as a person does.
//!
//! A user is online while both exist, away while only the first does, and offline otherwise.
//! Because both keys are the user's, not a connection's, any active device keeps them online and
//! any connected one keeps them from going offline, whichever API server each device talks to.
//! Nothing announces a change: clients ask for the status of the users they show
//! (`GET /users/statuses`) when they need it.
//!
//! To find who is online among a community's members without reading every member's keys, each
//! community has a sorted set, `community:{uuid}:online`, of the members who may be online, each
//! scored with the Unix time their listing runs out. Setting either key lists them in all their
//! communities for the longer of the two keys' lives and half as long again, and
//! `user:{uuid}:listed`, which lives that half, keeps the fan-out to once per margin however
//! often the keys are set: whenever a key is alive, so is a listing made since it was last set.
//! Joining a community lists the joiner there too. A listing says only that someone may be
//! online; the channel counts (`app::channel_presence`) and the member sample
//! (`connected_members`) confirm each one by their keys.

use crate::api::user::UserOnlineStatus;
use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::user::UserPg;
use crate::app::{CommunityId, UserId};
use crate::database::schema::community_user;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use fred::interfaces::{KeysInterface, SortedSetsInterface};
use fred::types::{Expiration, SetOptions};
use std::collections::HashSet;
use std::sync::Arc;

const KEY_PREFIX: &str = "user:";
const ONLINE_KEY_SUFFIX: &str = ":online";
const ACTIVE_KEY_SUFFIX: &str = ":active";
const LISTED_KEY_SUFFIX: &str = ":listed";
const COMMUNITY_KEY_PREFIX: &str = "community:";

/// The Valkey key whose presence means the user has a connection: `user:{uuid}:online`.
pub fn online_key(user_id: UserId) -> String {
    format!("{KEY_PREFIX}{}{ONLINE_KEY_SUFFIX}", user_id.0)
}

/// The Valkey key whose presence means the user has recently used Aspen: `user:{uuid}:active`.
pub fn active_key(user_id: UserId) -> String {
    format!("{KEY_PREFIX}{}{ACTIVE_KEY_SUFFIX}", user_id.0)
}

/// The Valkey key present while the user's community listings run far enough ahead of their
/// `active` key that setting it again need not renew them: `user:{uuid}:listed`.
fn listed_key(user_id: UserId) -> String {
    format!("{KEY_PREFIX}{}{LISTED_KEY_SUFFIX}", user_id.0)
}

/// The Valkey sorted set of `community`'s members who may be online: `community:{uuid}:online`.
pub fn community_online_key(community: CommunityId) -> String {
    format!("{COMMUNITY_KEY_PREFIX}{}{ONLINE_KEY_SUFFIX}", community.0)
}

/// How far past a key set to live `ttl` seconds a listing made with it runs, and so how long one
/// listing serves before the next renewal.
fn listing_margin(ttl: i64) -> i64 {
    ttl / 2
}

/// How long a listing covers before its margin: the longer of the two keys' lives, so a
/// listing outlives either key set when it was made.
fn listing_ttl(state: &GlobalServerContext) -> i64 {
    let away = i64::try_from(state.config.presence.away_after_seconds).unwrap_or(i64::MAX / 2);
    away.max(ONLINE_TTL_SECONDS)
}

/// How long a community's set lives after its last listing: as long as a listing, so it lasts
/// while any listing in it does and goes once nobody is listed.
fn community_set_lifetime(state: &GlobalServerContext) -> i64 {
    let ttl = listing_ttl(state);
    ttl.saturating_add(listing_margin(ttl))
}

/// A user's status from the values of their two keys.
pub fn status(online: Option<i64>, active: Option<i64>) -> UserOnlineStatus {
    match (online, active) {
        (Some(_), Some(_)) => UserOnlineStatus::Online,
        (Some(_), None) => UserOnlineStatus::Away,
        (None, _) => UserOnlineStatus::Offline,
    }
}

/// Records that the user is using Aspen, for `[presence] away_after_seconds`. Fire and forget:
/// presence is best effort.
pub fn mark_active(state: &GlobalServerContext, user: UserId) {
    let valkey = state.valkey.clone();
    let key = active_key(user);
    let ttl = i64::try_from(state.config.presence.away_after_seconds).unwrap_or(i64::MAX);
    tokio::spawn(async move {
        if let Err(e) = valkey
            .set::<(), _, i64>(key, 1, Some(Expiration::EX(ttl)), None, false)
            .await
        {
            tracing::warn!(error = %e, "failed to record the user as active");
        }
    });
    list_in_communities(state, user, listing_ttl(state));
}

/// Lists `user` in each of their communities' sets as someone who may be online for as long as
/// a key just set to live `ttl` seconds can, unless a listing made within the margin
/// already covers it. Fire and forget, like the keys.
fn list_in_communities(state: &GlobalServerContext, user: UserId, ttl: i64) {
    let state = state.clone();
    tokio::spawn(async move {
        let margin = listing_margin(ttl);
        let fresh: Option<String> = match state
            .valkey
            .set(
                listed_key(user),
                1,
                Some(Expiration::EX(margin.max(1))),
                Some(SetOptions::NX),
                false,
            )
            .await
        {
            Ok(fresh) => fresh,
            Err(e) => {
                tracing::warn!(error = %e, "failed to renew the user's community listings");
                return;
            }
        };
        if fresh.is_none() {
            return;
        }
        if let Err(e) = renew_listings(&state, user, ttl.saturating_add(margin)).await {
            tracing::warn!(error = %e, "failed to renew the user's community listings");
            // The next time the key is set tries again rather than waiting out the margin.
            let _ = state.valkey.del::<(), _>(listed_key(user)).await;
        }
    });
}

/// Lists `user` in every community they belong to for `seconds` from now, in one round trip.
async fn renew_listings(
    state: &GlobalServerContext,
    user: UserId,
    seconds: i64,
) -> app::Result<()> {
    let communities: Vec<CommunityId> = community_user::table
        .select(community_user::community)
        .filter(community_user::user.eq(user))
        .load(state.connection_pool.get().await?.as_mut())
        .await?;
    if communities.is_empty() {
        return Ok(());
    }
    let until = chrono::Utc::now().timestamp().saturating_add(seconds) as f64;
    let lifetime = community_set_lifetime(state);
    let pipeline = state.valkey.pipeline();
    for community in communities {
        let key = community_online_key(community);
        let () = pipeline
            .zadd(&key, None, None, false, false, (until, user.0.to_string()))
            .await?;
        let () = pipeline.expire(&key, lifetime, None).await?;
    }
    let _: Vec<fred::types::Value> = pipeline.all().await?;
    Ok(())
}

/// Lists `user`, who just joined `community`, in its set for as long as an `active` key of theirs
/// can last, since their listings elsewhere may not be due for renewal. Fire and forget.
pub fn list_in_community(state: &GlobalServerContext, user: UserId, community: CommunityId) {
    let valkey = state.valkey.clone();
    let lifetime = community_set_lifetime(state);
    tokio::spawn(async move {
        let key = community_online_key(community);
        let until = chrono::Utc::now().timestamp().saturating_add(lifetime) as f64;
        let pipeline = valkey.pipeline();
        let listed: Result<Vec<fred::types::Value>, fred::error::Error> = async {
            let () = pipeline
                .zadd(&key, None, None, false, false, (until, user.0.to_string()))
                .await?;
            let () = pipeline.expire(&key, lifetime, None).await?;
            pipeline.all().await
        }
        .await;
        if let Err(e) = listed {
            tracing::warn!(error = %e, "failed to list a new member as possibly online");
        }
    });
}

/// The members of `community` who may be online now: everyone whose listing has not run out,
/// a superset of those whose keys say they are. Listings that have run out are dropped on the
/// way.
pub async fn online_candidates(
    state: &GlobalServerContext,
    community: CommunityId,
) -> app::Result<Vec<UserId>> {
    let key = community_online_key(community);
    // Scores are bounded as floats: fred reads an integer bound as a rank.
    let now = chrono::Utc::now().timestamp() as f64;
    let () = state
        .valkey
        .zremrangebyscore(&key, f64::NEG_INFINITY, now - 1.0)
        .await?;
    let listed: Vec<String> = state
        .valkey
        .zrangebyscore(&key, now, f64::INFINITY, false, None)
        .await?;
    Ok(listed
        .iter()
        .filter_map(|id| uuid::Uuid::parse_str(id).ok().map(UserId))
        .collect())
}

/// How many people's presence keys one read asks for.
const STATUS_BATCH: usize = 1_000;

/// Which of `users` have a status `keep` accepts, by their presence keys, a batch at a time.
async fn having_status(
    state: &GlobalServerContext,
    users: Vec<UserId>,
    keep: impl Fn(UserOnlineStatus) -> bool,
) -> app::Result<HashSet<UserId>> {
    let mut kept = HashSet::new();
    for batch in users.chunks(STATUS_BATCH) {
        kept.extend(
            users_online_status(state, batch.to_vec())
                .await?
                .into_iter()
                .filter(|(_, status)| keep(*status))
                .map(|(user, _)| user),
        );
    }
    Ok(kept)
}

/// Which of `users` are online, not away.
pub async fn online_among(
    state: &GlobalServerContext,
    users: Vec<UserId>,
) -> app::Result<HashSet<UserId>> {
    having_status(state, users, |status| {
        matches!(status, UserOnlineStatus::Online)
    })
    .await
}

/// The members of `community` who are online or away: everyone with a connection. Each server
/// reuses a recent answer (`app::recent`), since a community's member sample asks on every read.
pub async fn connected_members(
    state: &GlobalServerContext,
    community: CommunityId,
) -> app::Result<Arc<HashSet<UserId>>> {
    state
        .connected_members
        .get_or_work(community, || async {
            let candidates = online_candidates(state, community).await?;
            let connected = having_status(state, candidates, |status| {
                !matches!(status, UserOnlineStatus::Offline)
            })
            .await?;
            Ok(Arc::new(connected))
        })
        .await
}

pub async fn user_online_status(
    state: &GlobalServerContext,
    user_id: UserId,
) -> crate::app::Result<UserOnlineStatus> {
    Ok(users_online_status(state, vec![user_id])
        .await?
        .pop()
        .map_or(UserOnlineStatus::Offline, |(_, status)| status))
}

/// The presence of each user (`app::user_status`), read in one round trip.
pub async fn users_online_status(
    state: &GlobalServerContext,
    user_ids: Vec<UserId>,
) -> crate::app::Result<Vec<(UserId, UserOnlineStatus)>> {
    // MGET with no keys is a protocol error, so an empty batch is answered locally.
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = user_ids
        .iter()
        .flat_map(|user_id| {
            [
                app::user_status::online_key(*user_id),
                app::user_status::active_key(*user_id),
            ]
        })
        .collect();
    let values: Vec<Option<i64>> = state.valkey.mget(keys).await?;
    Ok(user_ids
        .into_iter()
        .zip(
            values
                .chunks(2)
                .map(|pair| app::user_status::status(pair[0], pair.get(1).copied().flatten())),
        )
        .collect())
}

/// Records that the user has a connection for `ONLINE_TTL_SECONDS`; the event stream calls this
/// when they connect and while they stay, and so does every authenticated request. Fire and
/// forget: presence is best effort.
pub fn mark_user_online(state: &GlobalServerContext, user: &UserPg) {
    mark_user_online_id(state, user.id, user.bot);
}

/// As `mark_user_online`, for a user known by id. A bot is never away: it uses Aspen through
/// the API rather than as a person does, so being connected is being active, and both of its
/// keys are set together.
///
/// When the `online` key was not already set, the user has just come online, and their
/// `last_seen_at` is written: it is when they last came online.
pub fn mark_user_online_id(state: &GlobalServerContext, user: UserId, bot: bool) {
    let state_for_task = state.clone();
    tokio::spawn(async move {
        let state = state_for_task;
        let expiry = Some(Expiration::EX(ONLINE_TTL_SECONDS));
        // Setting the key answers its previous value, which says whether they were online.
        let previous: Option<i64> = match state
            .valkey
            .set(online_key(user), 1, expiry.clone(), None, true)
            .await
        {
            Ok(previous) => previous,
            Err(e) => {
                tracing::warn!(error = %e, "failed to record the user as online");
                return;
            }
        };
        if bot
            && let Err(e) = state
                .valkey
                .set::<(), _, i64>(active_key(user), 1, expiry, None, false)
                .await
        {
            tracing::warn!(error = %e, "failed to record the user as online");
        }
        if previous.is_none()
            && let Err(e) = record_seen(&state, user).await
        {
            tracing::warn!(error = %e, "failed to record when the user came online");
        }
    });
    list_in_communities(state, user, listing_ttl(state));
}

/// Writes `user`'s `last_seen_at` as now.
async fn record_seen(state: &GlobalServerContext, user: UserId) -> app::Result<()> {
    use crate::database::schema::user;
    diesel::update(user::table.filter(user::id.eq(user)))
        .set(user::last_seen_at.eq(diesel::dsl::now))
        .execute(state.connection_pool.get().await?.as_mut())
        .await?;
    Ok(())
}

/// How long a presence key lives; the event stream refreshes it while the user is connected.
pub const ONLINE_TTL_SECONDS: i64 = 60;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_without_recent_activity_is_away() {
        assert!(matches!(status(Some(1), Some(1)), UserOnlineStatus::Online));
        assert!(matches!(status(Some(1), None), UserOnlineStatus::Away));
        assert!(matches!(status(None, None), UserOnlineStatus::Offline));
        // Activity outliving the last connection does not keep anyone online.
        assert!(matches!(status(None, Some(1)), UserOnlineStatus::Offline));
    }
}
