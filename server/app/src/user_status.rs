//! Presence. Two Valkey keys per user describe it:
//!
//! - `user:{uuid}:online` exists while the user has a connection: it is set with a short expiry
//!   when they connect the event stream (or make any authenticated request) and refreshed by the
//!   stream's pings, so it expires shortly after their last connection goes; each server sets it
//!   for one user at most every `MARK_EVERY`, a quarter of its life. Finding it not set is their
//!   coming online, which writes their `last_seen_at` and copies their override before setting it
//!   (`app::presence_upkeep`).
//! - `user:{uuid}:active` exists while they are using Aspen: a client sends an `activity` frame
//!   on its event stream while its user interacts with it, and each sets the key to expire after
//!   `[presence] away_after_seconds`.
//!
//! A bot's `active` key is set with its `online` key and lives as long, so a connected bot is
//! online and never away: it uses Aspen through the API, not as a person does.
//!
//! A third, `user:{uuid}:override`, holds what the user chose to show instead, while it is in
//! force (`app::presence_override`): while they are connected, it makes them invisible, away,
//! or do not disturb, whatever the second key says.
//!
//! A user is online while both exist, away while only the first does, and offline otherwise,
//! unless the third says otherwise while they are connected. Someone invisible is offline to
//! everyone but themself, and is neither counted nor listed as online. Because the keys are the user's, not a connection's, any active device keeps them online and
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

use crate::aspen_config::PresenceConfig;
use crate::context::GlobalServerContext;
use crate::presence_feed::Expiry;
use crate::presence_override::PresenceOverride;
use crate::presence_upkeep::ListingTimes;
use crate::user::UserPg;
use crate::{CommunityId, UserId};
use aspen_wire::user::UserOnlineStatus;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use fred::interfaces::{KeysInterface, SortedSetsInterface};
use fred::types::{Expiration, SetOptions};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

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
pub(crate) fn listed_key(user_id: UserId) -> String {
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
fn listing_ttl(presence: &PresenceConfig) -> i64 {
    let away = i64::try_from(presence.away_after_seconds).unwrap_or(i64::MAX / 2);
    away.max(ONLINE_TTL_SECONDS)
}

/// How long a community's set lives after its last listing: as long as a listing, so it lasts
/// while any listing in it does and goes once nobody is listed.
fn community_set_lifetime(state: &GlobalServerContext) -> i64 {
    listing_times(&state.config.presence).set_lifetime
}

/// How long listings last under `presence`, for the upkeep that makes them
/// (`app::presence_upkeep`).
pub fn listing_times(presence: &PresenceConfig) -> ListingTimes {
    let ttl = listing_ttl(presence);
    let margin = listing_margin(ttl);
    ListingTimes {
        ttl,
        margin,
        set_lifetime: ttl.saturating_add(margin),
    }
}

/// A user's status from the values of their three keys, as they themself are told it: an
/// invisible user is `Invisible` here, and `Offline` to everyone else ([`seen_by_others`]).
pub fn status(
    online: Option<&str>,
    active: Option<&str>,
    chosen: Option<&str>,
) -> UserOnlineStatus {
    if online.is_none() {
        return UserOnlineStatus::Offline;
    }
    match chosen.and_then(|chosen| chosen.parse().ok()) {
        Some(PresenceOverride::Invisible) => UserOnlineStatus::Invisible,
        Some(PresenceOverride::Away) => UserOnlineStatus::Away,
        Some(PresenceOverride::DoNotDisturb) => UserOnlineStatus::DoNotDisturb,
        None if active.is_some() => UserOnlineStatus::Online,
        None => UserOnlineStatus::Away,
    }
}

/// A status as anyone but the user it belongs to is told it: invisible is offline.
pub fn seen_by_others(status: UserOnlineStatus) -> UserOnlineStatus {
    match status {
        UserOnlineStatus::Invisible => UserOnlineStatus::Offline,
        status => status,
    }
}

/// Records that the user is using Aspen, for `[presence] away_after_seconds`. Fire and forget:
/// presence is best effort. Their becoming active again, and their going away once it runs out,
/// are told to those watching them (`app::presence_feed`).
pub fn mark_active(state: &GlobalServerContext, user: UserId) {
    let feed = state.presence_feed.clone();
    let valkey = state.valkey.clone();
    let key = active_key(user);
    let seconds = state.config.presence.away_after_seconds;
    let ttl = i64::try_from(seconds).unwrap_or(i64::MAX);
    tokio::spawn(async move {
        // Setting the key answers its previous value, which says whether they were away.
        match valkey
            .set::<Option<i64>, _, i64>(key, 1, Some(Expiration::EX(ttl)), None, true)
            .await
        {
            Ok(previous) => {
                if previous.is_none() {
                    feed.changed(user);
                }
                feed.expires(user, Expiry::Activity, Duration::from_secs(seconds));
            }
            Err(e) => tracing::warn!(error = %e, "failed to record the user as active"),
        }
    });
    state.presence_upkeep.list(user);
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
) -> crate::Result<Vec<UserId>> {
    let key = community_online_key(community);
    // Scores are bounded as floats: fred reads an integer bound as a rank.
    let now = chrono::Utc::now().timestamp() as f64;
    // Pruning and reading go in one round trip.
    let pipeline = state.valkey.pipeline();
    let () = pipeline
        .zremrangebyscore(&key, f64::NEG_INFINITY, now - 1.0)
        .await?;
    let () = pipeline
        .zrangebyscore(&key, now, f64::INFINITY, false, None)
        .await?;
    let (_, listed): (i64, Vec<String>) = pipeline.all().await?;
    Ok(listed
        .iter()
        .filter_map(|id| uuid::Uuid::parse_str(id).ok().map(UserId))
        .collect())
}

/// How many people's presence keys one read asks for.
const STATUS_BATCH: usize = 1_000;

/// Which of `users` have a status `keep` accepts, as others see it, by their presence keys, in batches read at
/// once rather than one after another.
async fn having_status(
    state: &GlobalServerContext,
    users: Vec<UserId>,
    keep: impl Fn(UserOnlineStatus) -> bool,
) -> crate::Result<HashSet<UserId>> {
    let batches = futures_util::future::try_join_all(
        users
            .chunks(STATUS_BATCH)
            .map(|batch| users_online_status(&state.valkey, batch.to_vec())),
    )
    .await?;
    Ok(batches
        .into_iter()
        .flatten()
        .filter(|(_, status)| keep(seen_by_others(*status)))
        .map(|(user, _)| user)
        .collect())
}

/// Which of `users` are online, not away: those online or in do not disturb.
pub async fn online_among(
    state: &GlobalServerContext,
    users: Vec<UserId>,
) -> crate::Result<HashSet<UserId>> {
    having_status(state, users, |status| {
        matches!(
            status,
            UserOnlineStatus::Online | UserOnlineStatus::DoNotDisturb
        )
    })
    .await
}

/// The members of `community` who are online, away, or in do not disturb: everyone with a
/// connection who is not invisible. Each server
/// reuses a recent answer (`app::recent`), since a community's member sample asks on every read.
pub async fn connected_members(
    state: &GlobalServerContext,
    community: CommunityId,
) -> crate::Result<Arc<HashSet<UserId>>> {
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

/// Which of `users` `viewer` may learn the presence of: themself, and anyone who shares a
/// community (not a deleted one) or a DM with them, or is a bot they own, unless that person
/// has blocked them. Of anyone else, presence would tell a stranger when someone is about, so
/// they learn nothing.
pub async fn presence_visible(
    conn: &mut AsyncPgConnection,
    viewer: UserId,
    users: &[UserId],
) -> crate::Result<HashSet<UserId>> {
    let pairs: Vec<(UserId, UserId)> = users.iter().map(|user| (viewer, *user)).collect();
    Ok(presence_visible_pairs(conn, &pairs)
        .await?
        .into_iter()
        .map(|(_, user)| user)
        .collect())
}

/// Which of `pairs`, each a viewer and someone whose presence they ask for, the viewer may learn
/// it of, as [`presence_visible`] decides, in one query however many viewers there are.
pub async fn presence_visible_pairs(
    conn: &mut AsyncPgConnection,
    pairs: &[(UserId, UserId)],
) -> crate::Result<HashSet<(UserId, UserId)>> {
    use diesel::sql_types::{Array, Uuid};
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = Uuid)]
        viewer: uuid::Uuid,
        #[diesel(sql_type = Uuid)]
        asked: uuid::Uuid,
    }
    if pairs.is_empty() {
        return Ok(HashSet::new());
    }
    let rows: Vec<Row> = diesel::sql_query(
        r#"
        SELECT p.viewer, p.asked
        FROM unnest($1::uuid[], $2::uuid[]) AS p(viewer, asked)
        WHERE p.asked = p.viewer
           OR ((EXISTS (
                    SELECT 1 FROM community_user mine
                    JOIN community_user theirs ON theirs.community = mine.community
                    JOIN community shared ON shared.id = mine.community
                    WHERE mine."user" = p.viewer AND theirs."user" = p.asked
                      AND shared.deleted_at IS NULL
                )
                OR EXISTS (
                    SELECT 1 FROM dm_recipient mine
                    JOIN dm_recipient theirs ON theirs.channel = mine.channel
                    WHERE mine."user" = p.viewer AND theirs."user" = p.asked
                )
                OR EXISTS (
                    SELECT 1 FROM "user" bot WHERE bot.id = p.asked AND bot.bot_owner = p.viewer
                ))
               AND NOT EXISTS (
                    SELECT 1 FROM user_block WHERE blocker = p.asked AND blocked = p.viewer
               ))
        "#,
    )
    .bind::<Array<Uuid>, _>(pairs.iter().map(|(viewer, _)| viewer.0).collect::<Vec<_>>())
    .bind::<Array<Uuid>, _>(pairs.iter().map(|(_, asked)| asked.0).collect::<Vec<_>>())
    .load(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| (UserId(row.viewer), UserId(row.asked)))
        .collect())
}

/// The presence of each of `users` as `viewer` may learn it (`presence_visible`), in the order
/// given: `offline` for anyone whose presence is not theirs to learn, and for anyone invisible
/// but the viewer themself.
pub async fn statuses_for(
    state: &GlobalServerContext,
    viewer: UserId,
    users: Vec<UserId>,
) -> crate::Result<Vec<(UserId, UserOnlineStatus)>> {
    if users.is_empty() {
        return Ok(Vec::new());
    }
    let visible = {
        let mut conn = state.connection_pool.get().await?;
        presence_visible(conn.as_mut(), viewer, &users).await?
    };
    let asked: Vec<UserId> = users
        .iter()
        .copied()
        .filter(|user| visible.contains(user))
        .collect();
    let known = raw_statuses(&state.valkey, asked).await?;
    Ok(users
        .into_iter()
        .map(|user| {
            let status = known
                .get(&user)
                .copied()
                .unwrap_or(UserOnlineStatus::Offline);
            let status = if user == viewer {
                status
            } else {
                seen_by_others(status)
            };
            (user, status)
        })
        .collect())
}

/// The presence of each of `users` as they themself are told it, whoever asks, read
/// [`STATUS_BATCH`] at a time, the batches at once. What a person is told of someone else's
/// goes through [`seen_by_others`] and a check that it is theirs to learn.
pub async fn raw_statuses(
    valkey: &fred::clients::Client,
    users: Vec<UserId>,
) -> crate::Result<HashMap<UserId, UserOnlineStatus>> {
    let batches = futures_util::future::try_join_all(
        users
            .chunks(STATUS_BATCH)
            .map(|batch| users_online_status(valkey, batch.to_vec())),
    )
    .await?;
    Ok(batches.into_iter().flatten().collect())
}

/// The presence of each user (`app::user_status`) as they themself are told it, read in one
/// round trip, whoever asks: for counting and ordering members (`online_among`,
/// `connected_members`, through [`seen_by_others`]). What a person is told of someone's presence
/// goes through `statuses_for`.
async fn users_online_status(
    valkey: &fred::clients::Client,
    user_ids: Vec<UserId>,
) -> crate::Result<Vec<(UserId, UserOnlineStatus)>> {
    // MGET with no keys is a protocol error, so an empty batch is answered locally.
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = user_ids
        .iter()
        .flat_map(|user_id| {
            [
                online_key(*user_id),
                active_key(*user_id),
                crate::presence_override::override_key(*user_id),
            ]
        })
        .collect();
    let values: Vec<Option<String>> = valkey.mget(keys).await?;
    Ok(user_ids
        .into_iter()
        .zip(
            values
                .as_chunks::<3>()
                .0
                .iter()
                .map(|[online, active, chosen]| {
                    status(online.as_deref(), active.as_deref(), chosen.as_deref())
                }),
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
/// When the `online` key is not set, the user is coming online: their `last_seen_at` is written,
/// it being when they last came online, and their presence override is copied to Valkey
/// (`app::presence_override`), in case Valkey has lost it, before the key is set, so the key
/// never stands without the copy beside it. Their coming online, and their going offline once
/// the key runs out, are told to those watching them (`app::presence_feed`). The writes, the
/// copy, and their listings are batched with others' (`app::presence_upkeep`).
///
/// Each server marks one user at most every [`MARK_EVERY`]: a client making many requests, or
/// many of a user's devices on one server, cost a write each quarter of the key's life rather
/// than one each, and the key, living [`ONLINE_TTL_SECONDS`], is always renewed before it runs
/// out.
pub fn mark_user_online_id(state: &GlobalServerContext, user: UserId, bot: bool) {
    if state.presence_marked.get(&user).is_some() {
        return;
    }
    state.presence_marked.insert(user, ());
    let state_for_task = state.clone();
    tokio::spawn(async move {
        let state = state_for_task;
        // Renewed only if it is set: answering its previous value says whether it was. A user
        // not online yet is brought online by their arrival, which sets the key once their
        // override is copied (`app::presence_upkeep`).
        let renewed: Option<i64> = match state
            .valkey
            .set(
                online_key(user),
                1,
                Some(Expiration::EX(ONLINE_TTL_SECONDS)),
                Some(SetOptions::XX),
                true,
            )
            .await
        {
            Ok(previous) => previous,
            Err(e) => {
                tracing::warn!(error = %e, "failed to record the user as online");
                return;
            }
        };
        if renewed.is_none() {
            state.presence_upkeep.arriving(user, bot);
            return;
        }
        state.presence_feed.expires(
            user,
            Expiry::Connection,
            Duration::from_secs(ONLINE_TTL_SECONDS as u64),
        );
        if bot
            && let Err(e) = state
                .valkey
                .set::<(), _, i64>(
                    active_key(user),
                    1,
                    Some(Expiration::EX(ONLINE_TTL_SECONDS)),
                    None,
                    false,
                )
                .await
        {
            tracing::warn!(error = %e, "failed to record the user as online");
        }
    });
    state.presence_upkeep.list(user);
}

/// Sets `user`'s `online` key, and a bot's `active` key with it, for [`ONLINE_TTL_SECONDS`]: their
/// arrival, once their override is copied (`app::presence_upkeep`).
pub(crate) async fn set_online_keys(
    valkey: &fred::clients::Client,
    feed: &crate::presence_feed::PresenceFeed,
    user: UserId,
    bot: bool,
) -> crate::Result<()> {
    let expiry = Some(Expiration::EX(ONLINE_TTL_SECONDS));
    let () = valkey
        .set(online_key(user), 1, expiry.clone(), None, false)
        .await?;
    feed.expires(
        user,
        Expiry::Connection,
        Duration::from_secs(ONLINE_TTL_SECONDS as u64),
    );
    if bot {
        let () = valkey.set(active_key(user), 1, expiry, None, false).await?;
    }
    Ok(())
}

/// How long a presence key lives; the event stream refreshes it while the user is connected.
pub const ONLINE_TTL_SECONDS: i64 = 60;

/// The least time between two of one server's marks of a user as online
/// (`mark_user_online_id`): a quarter of the key's life.
pub const MARK_EVERY: std::time::Duration =
    std::time::Duration::from_secs(ONLINE_TTL_SECONDS as u64 / 4);

/// The users this server marked online within [`MARK_EVERY`], which it does not mark again.
pub type PresenceMarked = moka::sync::Cache<UserId, ()>;

/// An empty [`PresenceMarked`], forgetting each user [`MARK_EVERY`] after their mark.
pub fn presence_marked() -> PresenceMarked {
    moka::sync::Cache::builder()
        .max_capacity(1_000_000)
        .time_to_live(MARK_EVERY)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_without_recent_activity_is_away() {
        let set = || Some("1");
        assert_eq!(status(set(), set(), None), UserOnlineStatus::Online);
        assert_eq!(status(set(), None, None), UserOnlineStatus::Away);
        assert_eq!(status(None, None, None), UserOnlineStatus::Offline);
        // Activity outliving the last connection does not keep anyone online.
        assert_eq!(status(None, set(), None), UserOnlineStatus::Offline);
    }

    #[test]
    fn a_chosen_presence_holds_only_while_connected() {
        let set = || Some("1");
        assert_eq!(status(set(), set(), Some("away")), UserOnlineStatus::Away);
        assert_eq!(
            status(set(), None, Some("doNotDisturb")),
            UserOnlineStatus::DoNotDisturb
        );
        assert_eq!(
            status(set(), set(), Some("invisible")),
            UserOnlineStatus::Invisible
        );
        assert_eq!(
            status(None, None, Some("doNotDisturb")),
            UserOnlineStatus::Offline
        );
        // A value this version does not know is no override.
        assert_eq!(status(set(), set(), Some("busy")), UserOnlineStatus::Online);
        assert_eq!(
            seen_by_others(UserOnlineStatus::Invisible),
            UserOnlineStatus::Offline
        );
    }
}
