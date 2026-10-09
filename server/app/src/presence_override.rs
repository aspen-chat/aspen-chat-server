//! What a user chooses to show of their presence in place of what their connections say:
//! invisible (offline to everyone else), away, or do not disturb, for a while or until they
//! change it. It is the user's own, on every device: a change is published to their own subject
//! as `presenceOverrideChanged`, and one that runs out ends on each device by its own clock,
//! with no event.
//!
//! The `user` row holds it (`presence_override`, `presence_override_until`). Presence is read
//! from Valkey (`app::user_status`), so the row is copied to `user:{uuid}:override`, set to
//! expire when the override does. The copy is written while the row is locked in the
//! transaction that changes it, so concurrent changes reach Valkey in the order they commit,
//! and again whenever the user comes online, so a Valkey that lost it has it back by the time
//! anyone could be shown it.
//!
//! Every change, and the end of a timed one, is told to those watching the user's presence
//! (`app::presence_feed`).
//!
//! Do not disturb also holds whether or not the user is connected: no phone is woken for them
//! (`app::push`) and no DM call rings them (`app::voice::ring`), both decided from the row.

use crate::context::GlobalServerContext;
use crate::presence_feed::PresenceFeed;
use crate::t;
use crate::{EventScope, UserId, publish_event};
use aspen_schema::user;
use aspen_wire::message_enum::server_event::ServerEvent;
pub use aspen_wire::user::PresenceOverride;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use fred::interfaces::KeysInterface;
use fred::types::Expiration;

/// The longest a timed override may last: thirty days. Longer, it lasts until it is changed.
pub const MAX_OVERRIDE_SECONDS: u32 = 30 * 24 * 60 * 60;

/// An override in force, and until when; `None` until the user changes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChosenPresence {
    pub presence_override: PresenceOverride,
    pub until: Option<DateTime<Utc>>,
}

impl ChosenPresence {
    /// The override held by a row's two columns, if it is still in force at `now`.
    fn in_force(
        presence_override: Option<PresenceOverride>,
        until: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) -> Option<Self> {
        let presence_override = presence_override?;
        until.is_none_or(|until| until > now).then_some(Self {
            presence_override,
            until,
        })
    }
}

/// The SQL condition that `"user"` row `$alias` is in do not disturb now.
macro_rules! do_not_disturb_sql {
    ($alias:literal) => {
        concat!(
            $alias,
            ".presence_override = 'doNotDisturb' AND (",
            $alias,
            ".presence_override_until IS NULL OR ",
            $alias,
            ".presence_override_until > now())"
        )
    };
}
pub(crate) use do_not_disturb_sql;

/// The Valkey key holding the user's override while it is in force: `user:{uuid}:override`.
pub fn override_key(user: UserId) -> String {
    format!("user:{}:override", user.0)
}

/// Sets the user's override, for `duration_seconds` or, with `None`, until they change it,
/// replacing any already in force. Answers it and whether one was in force already.
pub async fn set(
    state: &GlobalServerContext,
    user_id: UserId,
    presence_override: PresenceOverride,
    duration_seconds: Option<u32>,
) -> crate::Result<(ChosenPresence, bool)> {
    if duration_seconds.is_some_and(|s| s == 0 || s > MAX_OVERRIDE_SECONDS) {
        return Err(crate::Error::Validation(t!(
            "presenceOverrideDuration",
            max = MAX_OVERRIDE_SECONDS / (24 * 60 * 60)
        )));
    }
    let now = Utc::now();
    let chosen = ChosenPresence {
        presence_override,
        until: duration_seconds.map(|s| now + chrono::Duration::seconds(i64::from(s))),
    };
    let existed = write(state, user_id, Some(chosen)).await?;
    Ok((chosen, existed))
}

/// Ends the user's override, if one is in force.
pub async fn clear(state: &GlobalServerContext, user_id: UserId) -> crate::Result<()> {
    write(state, user_id, None).await.map(|_| ())
}

/// Writes `chosen` as the user's override, announcing it to their devices and copying it to
/// Valkey under the row's lock, unless it changes nothing that is in force. Answers whether an
/// override was in force before.
async fn write(
    state: &GlobalServerContext,
    user_id: UserId,
    chosen: Option<ChosenPresence>,
) -> crate::Result<bool> {
    let mut conn = state.connection_pool.get().await?;
    let written = conn
        .transaction(|conn| {
            async move {
                let (previous, previous_until): (Option<PresenceOverride>, Option<DateTime<Utc>>) =
                    user::table
                        .select((user::presence_override, user::presence_override_until))
                        .filter(user::id.eq(user_id))
                        .for_update()
                        .first(conn.as_mut())
                        .await?;
                let previous = ChosenPresence::in_force(previous, previous_until, Utc::now());
                if previous.is_none() && chosen.is_none() {
                    return Ok(false);
                }
                diesel::update(user::table.filter(user::id.eq(user_id)))
                    .set((
                        user::presence_override.eq(chosen.map(|c| c.presence_override)),
                        user::presence_override_until.eq(chosen.and_then(|c| c.until)),
                    ))
                    .execute(conn.as_mut())
                    .await?;
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::User(user_id),
                    &ServerEvent::PresenceOverrideChanged {
                        presence_override: chosen.map(|c| c.presence_override),
                        until: chosen.and_then(|c| c.until),
                    },
                )
                .await?;
                copy_to_valkey(state, user_id, chosen).await?;
                Ok::<_, crate::Error>(previous.is_some())
            }
            .scope_boxed()
        })
        .await;
    match &written {
        // Those watching the user are told of what it makes them, once it is committed.
        Ok(_) => state.presence_feed.changed(user_id),
        Err(_) => {
            // The copy may have been written before the commit failed; put back what is
            // committed.
            drop(conn);
            if let Err(e) = restore(state, user_id).await {
                tracing::warn!(error = %e, "failed to restore a presence override after a rollback");
            }
        }
    }
    written
}

/// The user's override in force, if any.
pub async fn read(
    state: &GlobalServerContext,
    user_id: UserId,
) -> crate::Result<Option<ChosenPresence>> {
    read_with(state.connection_pool.get().await?.as_mut(), user_id).await
}

/// As [`read`], on a connection already held.
async fn read_with(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
) -> crate::Result<Option<ChosenPresence>> {
    let (presence_override, until): (Option<PresenceOverride>, Option<DateTime<Utc>>) = user::table
        .select((user::presence_override, user::presence_override_until))
        .filter(user::id.eq(user_id))
        .first(conn)
        .await?;
    Ok(ChosenPresence::in_force(
        presence_override,
        until,
        Utc::now(),
    ))
}

/// Copies the user's committed override to Valkey afresh.
async fn restore(state: &GlobalServerContext, user_id: UserId) -> crate::Result<()> {
    let chosen = read(state, user_id).await?;
    copy_to_valkey(state, user_id, chosen).await
}

/// Copies the override held by a `user` row's two columns, as they were read when the user came
/// online, to Valkey.
pub(crate) async fn copy_row_to_valkey(
    valkey: &fred::clients::Client,
    feed: &PresenceFeed,
    user_id: UserId,
    presence_override: Option<PresenceOverride>,
    until: Option<DateTime<Utc>>,
) -> crate::Result<()> {
    let chosen = ChosenPresence::in_force(presence_override, until, Utc::now());
    copy_with(valkey, feed, user_id, chosen).await
}

/// Sets `user:{uuid}:override` to `chosen`, expiring when it does, or removes it for `None`. A
/// timed one's end is told to those watching the user when it comes (`app::presence_feed`).
async fn copy_to_valkey(
    state: &GlobalServerContext,
    user_id: UserId,
    chosen: Option<ChosenPresence>,
) -> crate::Result<()> {
    copy_with(&state.valkey, &state.presence_feed, user_id, chosen).await
}

async fn copy_with(
    valkey: &fred::clients::Client,
    feed: &PresenceFeed,
    user_id: UserId,
    chosen: Option<ChosenPresence>,
) -> crate::Result<()> {
    let key = override_key(user_id);
    match chosen {
        Some(chosen) => {
            let expiry = chosen
                .until
                .map(|until| Expiration::PXAT(until.timestamp_millis()));
            let () = valkey
                .set(
                    key,
                    chosen.presence_override.to_string(),
                    expiry,
                    None,
                    false,
                )
                .await?;
            if let Some(until) = chosen.until {
                feed.expires(
                    user_id,
                    crate::presence_feed::Expiry::Chosen,
                    (until - Utc::now()).to_std().unwrap_or_default(),
                );
            }
        }
        None => {
            let () = valkey.del(key).await?;
        }
    }
    Ok(())
}

/// Whether `user_id` is in do not disturb now, from their row.
pub async fn do_not_disturb(conn: &mut AsyncPgConnection, user_id: UserId) -> crate::Result<bool> {
    Ok(read_with(conn, user_id)
        .await?
        .is_some_and(|c| c.presence_override == PresenceOverride::DoNotDisturb))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_override_lasts_until_its_end_or_for_good() {
        let now = Utc::now();
        let dnd = Some(PresenceOverride::DoNotDisturb);
        assert!(ChosenPresence::in_force(dnd, None, now).is_some());
        assert!(
            ChosenPresence::in_force(dnd, Some(now + chrono::Duration::seconds(1)), now).is_some()
        );
        assert!(ChosenPresence::in_force(dnd, Some(now), now).is_none());
        assert!(ChosenPresence::in_force(None, None, now).is_none());
    }

    #[test]
    fn the_do_not_disturb_condition_names_its_row() {
        assert_eq!(
            do_not_disturb_sql!("u"),
            "u.presence_override = 'doNotDisturb' AND (u.presence_override_until IS NULL OR \
             u.presence_override_until > now())"
        );
    }
}
