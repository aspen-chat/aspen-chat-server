//! What plugins keep (`plugin_storage`), and what they count (Valkey).
//!
//! Each value lives in a scope: the deployment, a community, a channel, or a user. Who may read
//! it through a route is decided by its scope (`host`), and it goes with what its scope names:
//! deleting a channel, a community, or an account deletes what every plugin kept there
//! (`forget`). A plugin keeps at most its manifest's `storageQuota` of keys and values together,
//! counted in `plugin.storage_bytes` as values are written and deleted.

use super::host::wit;
use crate::app::context::GlobalServerContext;
use crate::app::{self, ChannelId, CommunityId, UserId};
use crate::database::schema::{plugin, plugin_storage};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::collections::HashMap;
use uuid::Uuid;

/// The longest a key may be, in bytes.
pub const MAX_KEY: usize = 256;
/// The largest one value may be, in bytes.
pub const MAX_VALUE: usize = 64 << 10;
/// The most values one list returns.
pub const MAX_LIST: u32 = 100;

/// Where a value lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Deployment,
    Community(CommunityId),
    Channel(ChannelId),
    User(UserId),
}

impl Scope {
    fn kind(&self) -> &'static str {
        match self {
            Scope::Deployment => "deployment",
            Scope::Community(_) => "community",
            Scope::Channel(_) => "channel",
            Scope::User(_) => "user",
        }
    }

    fn id(&self) -> Uuid {
        match self {
            Scope::Deployment => Uuid::nil(),
            Scope::Community(id) => id.0,
            Scope::Channel(id) => id.0,
            Scope::User(id) => id.0,
        }
    }
}

fn check_key(key: &str) -> Result<(), wit::Error> {
    if key.is_empty() || key.len() > MAX_KEY {
        return Err(wit::Error::Invalid(format!(
            "a key is 1 to {MAX_KEY} bytes"
        )));
    }
    Ok(())
}

pub async fn get(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    scope: &Scope,
    key: &str,
) -> app::Result<Option<Vec<u8>>> {
    Ok(plugin_storage::table
        .select(plugin_storage::value)
        .filter(
            plugin_storage::plugin
                .eq(plugin_id)
                .and(plugin_storage::scope_kind.eq(scope.kind()))
                .and(plugin_storage::scope.eq(scope.id()))
                .and(plugin_storage::key.eq(key)),
        )
        .first(conn)
        .await
        .optional()?)
}

/// Writes `value` under `key`, within `quota` bytes for the plugin's keys and values together.
pub async fn set(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    quota: u64,
    scope: &Scope,
    key: &str,
    value: &[u8],
) -> Result<(), wit::Error> {
    check_key(key)?;
    if value.len() > MAX_VALUE {
        return Err(wit::Error::Limit(format!(
            "a value is at most {MAX_VALUE} bytes"
        )));
    }
    let quota = i64::try_from(quota).unwrap_or(i64::MAX);
    let written = conn
        .transaction(|conn| {
            async move {
                let previous: Option<i32> = plugin_storage::table
                    .select(diesel::dsl::sql::<diesel::sql_types::Integer>(
                        "octet_length(value)",
                    ))
                    .filter(
                        plugin_storage::plugin
                            .eq(plugin_id)
                            .and(plugin_storage::scope_kind.eq(scope.kind()))
                            .and(plugin_storage::scope.eq(scope.id()))
                            .and(plugin_storage::key.eq(key)),
                    )
                    .for_update()
                    .first(conn)
                    .await
                    .optional()?;
                let key_bytes = key.len() as i64;
                let delta = value.len() as i64
                    - match previous {
                        Some(length) => i64::from(length),
                        None => -key_bytes,
                    };
                // A value rewritten at the same length, as a count usually is, leaves the
                // plugin's row alone, so writes of one plugin seldom wait on each other.
                if delta != 0 {
                    // Growing takes room under the quota; shrinking always fits.
                    let ceiling = if delta > 0 { quota - delta } else { i64::MAX };
                    let counted = diesel::update(
                        plugin::table.filter(
                            plugin::id
                                .eq(plugin_id)
                                .and(plugin::storage_bytes.le(ceiling)),
                        ),
                    )
                    .set(plugin::storage_bytes.eq(plugin::storage_bytes + delta))
                    .execute(conn)
                    .await?;
                    if counted == 0 {
                        return Ok(false);
                    }
                }
                diesel::insert_into(plugin_storage::table)
                    .values((
                        plugin_storage::plugin.eq(plugin_id),
                        plugin_storage::scope_kind.eq(scope.kind()),
                        plugin_storage::scope.eq(scope.id()),
                        plugin_storage::key.eq(key),
                        plugin_storage::value.eq(value),
                    ))
                    .on_conflict((
                        plugin_storage::plugin,
                        plugin_storage::scope_kind,
                        plugin_storage::scope,
                        plugin_storage::key,
                    ))
                    .do_update()
                    .set(plugin_storage::value.eq(value))
                    .execute(conn)
                    .await?;
                Ok::<_, app::Error>(true)
            }
            .scope_boxed()
        })
        .await
        .map_err(|e| super::host::from_app(plugin_id, e))?;
    if written {
        Ok(())
    } else {
        Err(wit::Error::Limit(format!(
            "the plugin's storage is full ({quota} bytes)"
        )))
    }
}

pub async fn delete(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    scope: &Scope,
    key: &str,
) -> app::Result<()> {
    conn.transaction(|conn| {
        async move {
            let freed: Option<i32> = diesel::delete(
                plugin_storage::table.filter(
                    plugin_storage::plugin
                        .eq(plugin_id)
                        .and(plugin_storage::scope_kind.eq(scope.kind()))
                        .and(plugin_storage::scope.eq(scope.id()))
                        .and(plugin_storage::key.eq(key)),
                ),
            )
            .returning(diesel::dsl::sql::<diesel::sql_types::Integer>(
                "octet_length(key) + octet_length(value)",
            ))
            .get_result(conn)
            .await
            .optional()?;
            if let Some(freed) = freed {
                diesel::update(plugin::table.filter(plugin::id.eq(plugin_id)))
                    .set(plugin::storage_bytes.eq(plugin::storage_bytes - i64::from(freed)))
                    .execute(conn)
                    .await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// The keys after `after` that begin with `prefix`, in order, with their values.
pub async fn list(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    scope: &Scope,
    prefix: &str,
    after: Option<&str>,
    limit: u32,
) -> app::Result<Vec<(String, Vec<u8>)>> {
    let escaped = prefix
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    let mut query = plugin_storage::table
        .select((plugin_storage::key, plugin_storage::value))
        .filter(
            plugin_storage::plugin
                .eq(plugin_id)
                .and(plugin_storage::scope_kind.eq(scope.kind()))
                .and(plugin_storage::scope.eq(scope.id()))
                .and(plugin_storage::key.like(format!("{escaped}%"))),
        )
        .order(plugin_storage::key.asc())
        .limit(i64::from(limit))
        .into_boxed();
    if let Some(after) = after {
        query = query.filter(plugin_storage::key.gt(after.to_string()));
    }
    Ok(query.load(conn).await?)
}

/// Deletes what every plugin kept in `scope`, as what it names goes, inside the caller's
/// transaction: for a channel, in its threads too; for a community, in its channels and their
/// threads too.
pub async fn forget(conn: &mut AsyncPgConnection, scope: Scope) -> app::Result<()> {
    use crate::database::schema::channel;
    let channels: Vec<Uuid> = match scope {
        Scope::Channel(id) => channel::table
            .select(channel::id)
            .filter(channel::parent_channel.eq(id))
            .load::<ChannelId>(conn)
            .await?
            .into_iter()
            .map(|c| c.0)
            .collect(),
        Scope::Community(id) => channel::table
            .select(channel::id)
            .filter(channel::community.eq(id))
            .load::<ChannelId>(conn)
            .await?
            .into_iter()
            .map(|c| c.0)
            .collect(),
        Scope::Deployment | Scope::User(_) => Vec::new(),
    };
    let freed: Vec<(String, i32)> = diesel::delete(
        plugin_storage::table.filter(
            plugin_storage::scope_kind
                .eq(scope.kind())
                .and(plugin_storage::scope.eq(scope.id()))
                .or(plugin_storage::scope_kind
                    .eq("channel")
                    .and(plugin_storage::scope.eq_any(&channels))),
        ),
    )
    .returning((
        plugin_storage::plugin,
        diesel::dsl::sql::<diesel::sql_types::Integer>("octet_length(key) + octet_length(value)"),
    ))
    .get_results(conn)
    .await?;
    let mut by_plugin: HashMap<String, i64> = HashMap::new();
    for (plugin_id, bytes) in freed {
        *by_plugin.entry(plugin_id).or_default() += i64::from(bytes);
    }
    for (plugin_id, bytes) in by_plugin {
        diesel::update(plugin::table.filter(plugin::id.eq(plugin_id)))
            .set(plugin::storage_bytes.eq(plugin::storage_bytes - bytes))
            .execute(conn)
            .await?;
    }
    Ok(())
}

/// Adds `amount` to `key`'s count in the current window of `window` seconds, aligned to the
/// epoch, and returns the count. Each window's key expires once it is over.
pub async fn count(
    state: &GlobalServerContext,
    plugin_id: &str,
    key: &str,
    window: u32,
    amount: u32,
) -> app::Result<u64> {
    use fred::prelude::KeysInterface;
    let now = chrono::Utc::now().timestamp().max(0) as u64;
    let index = now / u64::from(window);
    let name = format!("plugin:{plugin_id}:count:{window}:{index}:{key}");
    let count: i64 = state.valkey.incr_by(&name, i64::from(amount)).await?;
    if count == i64::from(amount) {
        let _: () = state
            .valkey
            .expire(&name, i64::from(window) + 1, None)
            .await?;
    }
    Ok(u64::try_from(count).unwrap_or(0))
}
