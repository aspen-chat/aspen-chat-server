//! Presence. Two Valkey keys per user describe it:
//!
//! - `user:{uuid}:online` exists while the user has a connection: it is set with a short expiry
//!   when they connect the event stream (or make any authenticated request) and refreshed by the
//!   stream's pings, so it expires shortly after their last connection goes.
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

use crate::api::user::UserOnlineStatus;
use crate::app;
use crate::app::UserId;
use crate::app::context::GlobalServerContext;
use crate::app::user::UserPg;
use fred::interfaces::KeysInterface;
use fred::types::Expiration;

const KEY_PREFIX: &str = "user:";
const ONLINE_KEY_SUFFIX: &str = ":online";
const ACTIVE_KEY_SUFFIX: &str = ":active";

/// The Valkey key whose presence means the user has a connection: `user:{uuid}:online`.
pub fn online_key(user_id: UserId) -> String {
    format!("{KEY_PREFIX}{}{ONLINE_KEY_SUFFIX}", user_id.0)
}

/// The Valkey key whose presence means the user has recently used Aspen: `user:{uuid}:active`.
pub fn active_key(user_id: UserId) -> String {
    format!("{KEY_PREFIX}{}{ACTIVE_KEY_SUFFIX}", user_id.0)
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
pub fn mark_user_online_id(state: &GlobalServerContext, user: UserId, bot: bool) {
    use fred::interfaces::KeysInterface;
    let valkey = state.valkey.clone();
    let mut keys = vec![app::user_status::online_key(user)];
    if bot {
        keys.push(app::user_status::active_key(user));
    }
    tokio::spawn(async move {
        for key in keys {
            if let Err(e) = valkey
                .set::<(), _, i64>(
                    key,
                    1,
                    Some(fred::types::Expiration::EX(ONLINE_TTL_SECONDS)),
                    None,
                    false,
                )
                .await
            {
                tracing::warn!(error = %e, "failed to record the user as online");
            }
        }
    });
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
