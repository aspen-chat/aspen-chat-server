//! Account-scoped user preferences: a JSON object per user that the client reads at sign-in
//! and patches as settings change, and that every device of the user follows through the
//! `userPreferencesChanged` event. The server stores the object as the client wrote it; keys
//! are the client's, namespaced (`audio.input`), and only the size is policed here, so a new
//! setting needs no server change.

use crate::api::message_enum::server_event::ServerEvent;
use crate::app::{self, EventScope, GlobalServerContext, UserId, publish_event};
use crate::database::schema::user_preferences;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use rust_i18n::t;
use serde_json::{Map, Value};

/// The most the object may take when serialised. Enough for hundreds of settings; a client
/// that stores more than that is storing the wrong thing.
pub const MAX_PREFERENCES_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = user_preferences)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct Row {
    user: UserId,
    values: Value,
    updated_at: DateTime<Utc>,
}

/// A user's preferences as the API returns them.
#[derive(Debug, Clone)]
pub struct Preferences {
    pub values: Map<String, Value>,
    /// When they were last written; `None` until the user has written any.
    pub updated_at: Option<DateTime<Utc>>,
}

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

/// The caller's preferences. Only the user themself may read them.
pub async fn read(
    state: &GlobalServerContext,
    caller: UserId,
    user: UserId,
) -> app::Result<Preferences> {
    if caller != user {
        return Err(app::Error::Unauthorized);
    }
    let mut conn = state.connection_pool.get().await?;
    let row: Option<Row> = user_preferences::table
        .select(Row::as_select())
        .filter(user_preferences::user.eq(user))
        .first(conn.as_mut())
        .await
        .optional()?;
    Ok(match row {
        Some(row) => Preferences {
            values: object(row.values),
            updated_at: Some(row.updated_at),
        },
        None => Preferences {
            values: Map::new(),
            updated_at: None,
        },
    })
}

/// Applies a JSON Merge Patch at the top level: a key present is written, `null` removes it,
/// keys absent are untouched. Only the user themself may write.
pub async fn merge(
    state: &GlobalServerContext,
    caller: UserId,
    user: UserId,
    patch: Map<String, Value>,
) -> app::Result<Preferences> {
    if caller != user {
        return Err(app::Error::Unauthorized);
    }
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let current: Option<Value> = user_preferences::table
                .select(user_preferences::values)
                .filter(user_preferences::user.eq(user))
                .for_update()
                .first(conn.as_mut())
                .await
                .optional()?;
            let mut values = current.map(object).unwrap_or_default();
            for (key, value) in patch {
                if value.is_null() {
                    values.remove(&key);
                } else {
                    values.insert(key, value);
                }
            }
            let serialised = serde_json::to_vec(&values)?;
            if serialised.len() > MAX_PREFERENCES_BYTES {
                return Err(app::Error::Validation(t!(
                    "preferencesTooLarge",
                    max = MAX_PREFERENCES_BYTES
                )));
            }
            let now = Utc::now();
            let row = Row {
                user,
                values: Value::Object(values.clone()),
                updated_at: now,
            };
            diesel::insert_into(user_preferences::table)
                .values(&row)
                .on_conflict(user_preferences::user)
                .do_update()
                .set((
                    user_preferences::values.eq(&row.values),
                    user_preferences::updated_at.eq(now),
                ))
                .execute(conn.as_mut())
                .await?;
            // The event names the user and the time, never the values, and goes to the user's
            // own subject alone.
            publish_event(
                state,
                conn.as_mut(),
                EventScope::User(user),
                &ServerEvent::UserPreferencesChanged {
                    user,
                    updated_at: now,
                },
            )
            .await?;
            Ok(Preferences {
                values,
                updated_at: Some(now),
            })
        }
        .scope_boxed()
    })
    .await
}
