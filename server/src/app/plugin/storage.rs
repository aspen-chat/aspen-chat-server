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
    write(
        conn,
        plugin_id,
        quota,
        scope,
        key,
        Expect::Anything,
        Some(value),
    )
    .await
    .map(|_| ())
}

/// Writes `value` under `key`, or deletes it when `value` is `None`, only while it holds
/// `expected` (is absent, when `expected` is `None`), answering whether it did. Two calls that
/// read the same value and swap it cannot both succeed, so a plugin answering requests on several
/// servers at once can change what it keeps without losing either change.
pub async fn swap(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    quota: u64,
    scope: &Scope,
    key: &str,
    expected: Option<&[u8]>,
    value: Option<&[u8]>,
) -> Result<bool, wit::Error> {
    write(
        conn,
        plugin_id,
        quota,
        scope,
        key,
        Expect::Value(expected),
        value,
    )
    .await
}

/// What a write requires `key` to hold.
#[derive(Clone, Copy)]
enum Expect<'a> {
    Anything,
    /// This value, or no value at all.
    Value(Option<&'a [u8]>),
}

/// Why a write's transaction is rolled back.
enum Abort {
    /// The key does not hold what the write expected.
    Stale,
    /// Another write made the key while this one found it absent; trying again finds it.
    Raced,
    /// The plugin's storage has no room for it.
    Full,
    App(app::Error),
}

impl From<diesel::result::Error> for Abort {
    fn from(error: diesel::result::Error) -> Self {
        Abort::App(error.into())
    }
}

/// How many times a write is tried when another keeps making its key first.
const WRITE_ATTEMPTS: usize = 3;

async fn write(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    quota: u64,
    scope: &Scope,
    key: &str,
    expect: Expect<'_>,
    value: Option<&[u8]>,
) -> Result<bool, wit::Error> {
    check_key(key)?;
    if value.is_some_and(|value| value.len() > MAX_VALUE) {
        return Err(wit::Error::Limit(format!(
            "a value is at most {MAX_VALUE} bytes"
        )));
    }
    let quota = i64::try_from(quota).unwrap_or(i64::MAX);
    for _ in 0..WRITE_ATTEMPTS {
        let result = conn
            .transaction(|conn| {
                async move { write_once(conn, plugin_id, quota, scope, key, expect, value).await }
                    .scope_boxed()
            })
            .await;
        return match result {
            Ok(()) => Ok(true),
            Err(Abort::Stale) => Ok(false),
            Err(Abort::Raced) => continue,
            Err(Abort::Full) => Err(wit::Error::Limit(format!(
                "the plugin's storage is full ({quota} bytes)"
            ))),
            Err(Abort::App(error)) => Err(super::host::from_app(plugin_id, error)),
        };
    }
    Err(wit::Error::Unavailable(
        "other writes kept making this key first; try again".into(),
    ))
}

/// One attempt at `write`, inside its transaction: the key's row, when it has one, is locked
/// until the transaction ends, so the value compared is the value replaced.
async fn write_once(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    quota: i64,
    scope: &Scope,
    key: &str,
    expect: Expect<'_>,
    value: Option<&[u8]>,
) -> Result<(), Abort> {
    let row = plugin_storage::table.filter(
        plugin_storage::plugin
            .eq(plugin_id)
            .and(plugin_storage::scope_kind.eq(scope.kind()))
            .and(plugin_storage::scope.eq(scope.id()))
            .and(plugin_storage::key.eq(key)),
    );
    // The previous value's length, having checked it is what the write expects.
    let previous: Option<i64> = match expect {
        Expect::Anything => row
            .select(diesel::dsl::sql::<diesel::sql_types::Integer>(
                "octet_length(value)",
            ))
            .for_update()
            .first::<i32>(conn)
            .await
            .optional()?
            .map(i64::from),
        Expect::Value(expected) => {
            let current: Option<Vec<u8>> = row
                .select(plugin_storage::value)
                .for_update()
                .first(conn)
                .await
                .optional()?;
            if current.as_deref() != expected {
                return Err(Abort::Stale);
            }
            current.map(|current| current.len() as i64)
        }
    };
    let key_bytes = key.len() as i64;
    let delta = match (previous, value) {
        (None, None) => return Ok(()),
        (Some(previous), None) => {
            diesel::delete(row).execute(conn).await?;
            -(key_bytes + previous)
        }
        (Some(previous), Some(value)) => {
            diesel::update(row)
                .set(plugin_storage::value.eq(value))
                .execute(conn)
                .await?;
            value.len() as i64 - previous
        }
        (None, Some(value)) => {
            let made = diesel::insert_into(plugin_storage::table)
                .values((
                    plugin_storage::plugin.eq(plugin_id),
                    plugin_storage::scope_kind.eq(scope.kind()),
                    plugin_storage::scope.eq(scope.id()),
                    plugin_storage::key.eq(key),
                    plugin_storage::value.eq(value),
                ))
                .on_conflict_do_nothing()
                .execute(conn)
                .await?;
            if made == 0 {
                // Another write made the key after this one found it absent. A swap expecting
                // it absent has lost; anything else tries again, finding (and locking) it.
                return Err(match expect {
                    Expect::Value(_) => Abort::Stale,
                    Expect::Anything => Abort::Raced,
                });
            }
            key_bytes + value.len() as i64
        }
    };
    // A value rewritten at the same length, as a count usually is, leaves the plugin's row
    // alone, so writes of one plugin seldom wait on each other.
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
            return Err(Abort::Full);
        }
    }
    Ok(())
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
