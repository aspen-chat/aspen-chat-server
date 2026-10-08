//! What plugins keep (`plugin_storage`), and what they count (Valkey).
//!
//! Each value lives in a scope: the deployment, a community, a channel, or a user. Who may read
//! it through a route is decided by its scope (`host`), and it goes with what its scope names:
//! deleting a channel, a community, or an account deletes what every plugin kept there
//! (`forget`). Each scope draws on an owner's share (`Owner`): a community's for it and its
//! channels and their threads, a DM's or group DM's for it and its threads, a user's for their own
//! scope, and the deployment's for the plugin's own. A plugin keeps at most its manifest's
//! `storageQuota` of keys and values together in each owner's share, counted in
//! `plugin_storage_usage` as values are written and deleted, so no one community can use up what
//! the plugin may keep for the rest. A plugin's total is its shares' sum (`totals`), read for the
//! operator and the dashboard.

use super::host::wit;
use crate::context::GlobalServerContext;
use crate::{ChannelId, CommunityId, UserId};
use aspen_schema::{plugin_storage, plugin_storage_usage, plugin_timer};
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "camelCase")]
pub enum Scope {
    Deployment,
    Community(CommunityId),
    Channel(ChannelId),
    User(UserId),
}

impl Scope {
    pub(super) fn kind(&self) -> &'static str {
        match self {
            Scope::Deployment => "deployment",
            Scope::Community(_) => "community",
            Scope::Channel(_) => "channel",
            Scope::User(_) => "user",
        }
    }

    pub(super) fn id(&self) -> Uuid {
        match self {
            Scope::Deployment => Uuid::nil(),
            Scope::Community(id) => id.0,
            Scope::Channel(id) => id.0,
            Scope::User(id) => id.0,
        }
    }
}

/// Whose share of a plugin's storage and timers a scope draws on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Deployment,
    Community(CommunityId),
    /// A DM or group DM, by its channel, for it and its threads.
    Direct(ChannelId),
    User(UserId),
}

impl Owner {
    pub(super) fn kind(&self) -> &'static str {
        match self {
            Owner::Deployment => "deployment",
            Owner::Community(_) => "community",
            Owner::Direct(_) => "direct",
            Owner::User(_) => "user",
        }
    }

    pub(super) fn id(&self) -> Uuid {
        match self {
            Owner::Deployment => Uuid::nil(),
            Owner::Community(id) => id.0,
            Owner::Direct(id) => id.0,
            Owner::User(id) => id.0,
        }
    }

    /// Whose share `scope` draws on: a channel's community, its thread's parent's, or, outside
    /// any community, the DM it is or is a thread of.
    pub async fn of(conn: &mut AsyncPgConnection, scope: &Scope) -> crate::Result<Self> {
        use aspen_schema::channel;
        Ok(match *scope {
            Scope::Deployment => Owner::Deployment,
            Scope::Community(id) => Owner::Community(id),
            Scope::User(id) => Owner::User(id),
            Scope::Channel(id) => {
                let (community, parent): (Option<CommunityId>, Option<ChannelId>) = channel::table
                    .select((channel::community, channel::parent_channel))
                    .filter(channel::id.eq(id))
                    .first(conn)
                    .await?;
                match (community, parent) {
                    (Some(community), _) => Owner::Community(community),
                    (None, Some(parent)) => channel::table
                        .select(channel::community)
                        .filter(channel::id.eq(parent))
                        .first::<Option<CommunityId>>(conn)
                        .await?
                        .map_or(Owner::Direct(parent), Owner::Community),
                    (None, None) => Owner::Direct(id),
                }
            }
        })
    }
}

/// Adds `delta` bytes to `owner`'s share of `plugin_id`'s storage, refusing growth past `quota`;
/// answers whether it fit. Only the owner's row is written, so writes for different owners
/// never wait on each other; the plugin's total is the shares' sum, read when it is shown.
async fn count_usage(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    owner: Owner,
    delta: i64,
    quota: i64,
) -> Result<bool, diesel::result::Error> {
    if delta == 0 {
        return Ok(true);
    }
    if delta > quota {
        return Ok(false);
    }
    // Growing takes room under the quota; shrinking always fits.
    let counted = diesel::sql_query(
        "INSERT INTO plugin_storage_usage (plugin, owner_kind, owner, bytes) \
         VALUES ($1, $2, $3, GREATEST($4, 0)) \
         ON CONFLICT (plugin, owner_kind, owner) DO UPDATE \
         SET bytes = plugin_storage_usage.bytes + $4 \
         WHERE $4 <= 0 OR plugin_storage_usage.bytes + $4 <= $5",
    )
    .bind::<diesel::sql_types::Text, _>(plugin_id)
    .bind::<diesel::sql_types::Text, _>(owner.kind())
    .bind::<diesel::sql_types::Uuid, _>(owner.id())
    .bind::<diesel::sql_types::BigInt, _>(delta)
    .bind::<diesel::sql_types::BigInt, _>(quota)
    .execute(conn)
    .await?;
    Ok(counted > 0)
}

/// How much each plugin keeps, every owner's share together, summed through
/// `plugin_storage_usage`'s key; a plugin keeping nothing is absent.
pub async fn totals(conn: &mut AsyncPgConnection) -> crate::Result<HashMap<String, i64>> {
    Ok(plugin_storage_usage::table
        .group_by(plugin_storage_usage::plugin)
        .select((
            plugin_storage_usage::plugin,
            diesel::dsl::sql::<diesel::sql_types::BigInt>("COALESCE(sum(bytes), 0)::bigint"),
        ))
        .load::<(String, i64)>(conn)
        .await?
        .into_iter()
        .collect())
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
) -> crate::Result<Option<Vec<u8>>> {
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
    App(crate::Error),
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
    let owner = Owner::of(conn, scope)
        .await
        .map_err(|e| super::host::from_app(plugin_id, e))?;
    for _ in 0..WRITE_ATTEMPTS {
        let result = conn
            .transaction(|conn| {
                async move {
                    write_once(conn, plugin_id, quota, scope, owner, key, expect, value).await
                }
                .scope_boxed()
            })
            .await;
        return match result {
            Ok(()) => Ok(true),
            Err(Abort::Stale) => Ok(false),
            Err(Abort::Raced) => continue,
            Err(Abort::Full) => Err(wit::Error::Limit(format!(
                "the plugin's storage for this {} is full ({quota} bytes)",
                owner.kind()
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
#[allow(clippy::too_many_arguments)]
async fn write_once(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    quota: i64,
    scope: &Scope,
    owner: Owner,
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
    // A value rewritten at the same length, as a count usually is, leaves the owner's count
    // alone, so writes in one community seldom wait on each other.
    if !count_usage(conn, plugin_id, owner, delta, quota).await? {
        return Err(Abort::Full);
    }
    Ok(())
}

pub async fn delete(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    scope: &Scope,
    key: &str,
) -> crate::Result<()> {
    let owner = Owner::of(conn, scope).await?;
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
                count_usage(conn, plugin_id, owner, -i64::from(freed), i64::MAX).await?;
            }
            Ok::<_, crate::Error>(())
        }
        .scope_boxed()
    })
    .await
}

/// The least string greater than every string that begins with `prefix`, in byte order (which
/// is code point order in UTF-8), or `None` when there is none: `prefix` with its last
/// character moved one on, dropping characters that cannot move.
fn past_prefix(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        let next = (u32::from(last) + 1..=u32::from(char::MAX)).find_map(char::from_u32);
        if let Some(next) = next {
            chars.push(next);
            return Some(chars.into_iter().collect());
        }
    }
    None
}

/// The keys after `after` that begin with `prefix`, in order of their bytes, with their values:
/// a range of the key's index (`key` compares in the `C` collation), so a page reads what it
/// returns and no more.
pub async fn list(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    scope: &Scope,
    prefix: &str,
    after: Option<&str>,
    limit: u32,
) -> crate::Result<Vec<(String, Vec<u8>)>> {
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
    if !prefix.is_empty() {
        query = query.filter(plugin_storage::key.ge(prefix.to_string()));
    }
    if let Some(past) = past_prefix(prefix) {
        query = query.filter(plugin_storage::key.lt(past));
    }
    if let Some(after) = after {
        query = query.filter(plugin_storage::key.gt(after.to_string()));
    }
    Ok(query.load(conn).await?)
}

/// Starts deleting what every plugin kept in `scope`, and the timers it set there, as what it
/// names goes: for a channel, in its threads too; for a community, in its channels and their
/// threads too. A job saved in the caller's transaction (`forgetPluginScope`, [`forget_step`])
/// does it a batch at a time, however much there is.
pub async fn forget(conn: &mut AsyncPgConnection, scope: Scope) -> crate::Result<()> {
    crate::jobs::enqueue(
        conn,
        crate::jobs::NewJob::new(
            crate::jobs::JobKind::ForgetPluginScope,
            crate::jobs::JobClass::Normal,
            &scope,
        )?
        .keyed(format!("{}:{}", scope.kind(), scope.id())),
    )
    .await?;
    Ok(())
}

/// How far forgetting a scope has come: whether the scope's own values are gone, and the last
/// of its channels (or threads) whose values are.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Forgotten {
    own: bool,
    after: Option<ChannelId>,
}

/// How many values, or channels, one step of forgetting takes.
const FORGET_BATCH: i64 = 500;

/// Deletes up to [`FORGET_BATCH`] of the values in the scopes `kind` and `ids` name, and the
/// timers there, taking what they held off `owner`'s shares; answers how many values went.
async fn forget_in(
    conn: &mut AsyncPgConnection,
    owner: Owner,
    kind: &str,
    ids: &[Uuid],
) -> crate::Result<usize> {
    #[derive(diesel::QueryableByName)]
    struct Freed {
        #[diesel(sql_type = diesel::sql_types::Text)]
        plugin: String,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        bytes: i64,
        #[diesel(sql_type = diesel::sql_types::BigInt)]
        values: i64,
    }
    diesel::delete(
        plugin_timer::table.filter(
            plugin_timer::scope_kind
                .eq(kind)
                .and(plugin_timer::scope.eq_any(ids)),
        ),
    )
    .execute(conn)
    .await?;
    // What went is summed per plugin in the statement that deletes it.
    let freed: Vec<Freed> = diesel::sql_query(
        r#"
        WITH gone AS (
            DELETE FROM plugin_storage WHERE ctid IN (
                SELECT ctid FROM plugin_storage
                WHERE scope_kind = $1 AND scope = ANY($2)
                LIMIT $3
            )
            RETURNING plugin, octet_length(key) + octet_length(value) AS bytes
        )
        SELECT plugin, sum(bytes)::bigint AS bytes, count(*) AS values
        FROM gone GROUP BY plugin
        "#,
    )
    .bind::<diesel::sql_types::Text, _>(kind)
    .bind::<diesel::sql_types::Array<diesel::sql_types::Uuid>, _>(ids)
    .bind::<diesel::sql_types::BigInt, _>(FORGET_BATCH)
    .load(conn)
    .await?;
    let mut values = 0;
    for gone in freed {
        count_usage(conn, &gone.plugin, owner, -gone.bytes, i64::MAX).await?;
        values += usize::try_from(gone.values).unwrap_or(0);
    }
    Ok(values)
}

/// One step of forgetting a scope: its own values a batch at a time, then its channels' (or a
/// channel's threads') a batch of channels at a time, in order of id; once all are gone, a
/// community's or a user's share of every plugin's storage.
pub async fn forget_step(
    state: &GlobalServerContext,
    job: &crate::jobs::Claimed,
) -> crate::Result<crate::jobs::Outcome> {
    let scope: Scope = job.payload()?;
    let done: Forgotten = job.progress()?.unwrap_or_default();
    let mut conn = state.connection_pool.get().await?;
    Ok(match forget_some(conn.as_mut(), scope, done).await? {
        Some(done) => crate::jobs::Outcome::Progress(serde_json::to_value(&done)?),
        None => crate::jobs::Outcome::Done,
    })
}

/// A step of forgetting `scope` from `done` on: how far it came, or `None` once it is all gone.
async fn forget_some(
    conn: &mut AsyncPgConnection,
    scope: Scope,
    mut done: Forgotten,
) -> crate::Result<Option<Forgotten>> {
    use aspen_schema::channel;
    // A channel and its threads draw on one owner's share; a community or a user is an owner.
    let owner = Owner::of(conn, &scope).await?;
    if !done.own {
        let gone = forget_in(conn, owner, scope.kind(), &[scope.id()]).await?;
        if gone as i64 >= FORGET_BATCH {
            return Ok(Some(done));
        }
        done.own = true;
    }
    let mut channels = channel::table.select(channel::id).into_boxed();
    channels = match scope {
        Scope::Channel(id) => channels.filter(channel::parent_channel.eq(id)),
        Scope::Community(id) => channels.filter(channel::community.eq(id)),
        Scope::Deployment | Scope::User(_) => {
            channels.filter(diesel::dsl::sql::<diesel::sql_types::Bool>("false"))
        }
    };
    if let Some(after) = done.after {
        channels = channels.filter(channel::id.gt(after));
    }
    let batch: Vec<ChannelId> = channels
        .order(channel::id)
        .limit(FORGET_BATCH)
        .load(conn)
        .await?;
    if let Some(last) = batch.last().copied() {
        let ids: Vec<Uuid> = batch.iter().map(|c| c.0).collect();
        // A batch of channels may hold more than one step's values: the same channels are
        // taken again until they hold none.
        if forget_in(conn, owner, "channel", &ids).await? as i64 >= FORGET_BATCH {
            return Ok(Some(done));
        }
        done.after = Some(last);
        if batch.len() as i64 >= FORGET_BATCH {
            return Ok(Some(done));
        }
    }
    if matches!(scope, Scope::Community(_) | Scope::User(_)) {
        diesel::delete(
            plugin_storage_usage::table.filter(
                plugin_storage_usage::owner_kind
                    .eq(owner.kind())
                    .and(plugin_storage_usage::owner.eq(owner.id())),
            ),
        )
        .execute(conn)
        .await?;
    }
    Ok(None)
}

/// Forgets all of `scope` at once, step after step on one connection, as the tests do.
#[cfg(test)]
async fn forget_all(conn: &mut AsyncPgConnection, scope: Scope) -> crate::Result<()> {
    let mut done = Forgotten::default();
    while let Some(further) = forget_some(conn, scope, done).await? {
        done = further;
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
) -> crate::Result<u64> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_end_of_a_prefix_range_is_past_every_key_with_it() {
        assert_eq!(past_prefix("ab").as_deref(), Some("ac"));
        assert_eq!(past_prefix(""), None);
        assert_eq!(past_prefix("a\u{10FFFF}").as_deref(), Some("b"));
        assert_eq!(past_prefix("\u{D7FF}").as_deref(), Some("\u{E000}"));
        assert!("ab\u{10FFFF}zz" < "ac");
    }
    use diesel_async::SimpleAsyncConnection;

    const COMMUNITY: &str = "5e1f0000-0000-4000-8000-000000000001";
    const OTHER: &str = "5e1f0000-0000-4000-8000-000000000002";
    const CHANNEL: &str = "5e1f0000-0000-4000-8000-000000000003";
    const THREAD: &str = "5e1f0000-0000-4000-8000-000000000004";
    const DM: &str = "5e1f0000-0000-4000-8000-000000000005";

    fn id(text: &str) -> Uuid {
        text.parse().unwrap()
    }

    /// Runs against the database `DATABASE_URL` names, inside a transaction that is never
    /// committed; without one it checks nothing.
    #[tokio::test]
    async fn each_community_has_a_quota_of_its_own() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        let mut conn = AsyncPgConnection::establish(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(&format!(
            "INSERT INTO plugin (id, version, manifest, mode, position)
                 VALUES ('org.example.quota', '1.0.0', '{{}}', 'everywhere', 0);
             INSERT INTO community (id, name) VALUES ('{COMMUNITY}', 'a'), ('{OTHER}', 'b');
             INSERT INTO channel (id, name, ty, sort_index, community)
                 VALUES ('{CHANNEL}', 'c', 'text', 0, '{COMMUNITY}');
             INSERT INTO channel (id, name, ty, sort_index, parent_channel)
                 VALUES ('{THREAD}', 't', 'thread', 0, '{CHANNEL}');
             INSERT INTO channel (id, name, ty, sort_index) VALUES ('{DM}', 'd', 'dm', 0);"
        ))
        .await
        .unwrap();
        let plugin = "org.example.quota";
        let channel = Scope::Channel(ChannelId(id(CHANNEL)));
        let thread = Scope::Channel(ChannelId(id(THREAD)));
        let community = Scope::Community(CommunityId(id(COMMUNITY)));
        let other = Scope::Community(CommunityId(id(OTHER)));
        let dm = Scope::Channel(ChannelId(id(DM)));
        assert_eq!(
            Owner::of(&mut conn, &thread).await.unwrap(),
            Owner::Community(CommunityId(id(COMMUNITY)))
        );
        assert_eq!(
            Owner::of(&mut conn, &dm).await.unwrap(),
            Owner::Direct(ChannelId(id(DM)))
        );

        let quota = 100;
        set(&mut conn, plugin, quota, &channel, "k", &[0; 60])
            .await
            .unwrap();
        // A thread and the community's own scope draw on the same share.
        assert!(matches!(
            set(&mut conn, plugin, quota, &thread, "k", &[0; 40]).await,
            Err(wit::Error::Limit(_))
        ));
        set(&mut conn, plugin, quota, &community, "k", &[0; 30])
            .await
            .unwrap();
        // Another community, and a DM, have shares of their own.
        set(&mut conn, plugin, quota, &other, "k", &[0; 90])
            .await
            .unwrap();
        set(&mut conn, plugin, quota, &dm, "k", &[0; 90])
            .await
            .unwrap();
        // Deleting frees room in the share it was in.
        delete(&mut conn, plugin, &channel, "k").await.unwrap();
        set(&mut conn, plugin, quota, &thread, "k", &[0; 40])
            .await
            .unwrap();
        let total = totals(&mut conn).await.unwrap()[plugin];
        assert_eq!(total, (1 + 40) + (1 + 30) + (1 + 90) + (1 + 90));

        forget_all(&mut conn, Scope::Community(CommunityId(id(COMMUNITY))))
            .await
            .unwrap();
        let shares: Vec<(String, i64)> = plugin_storage_usage::table
            .select((
                plugin_storage_usage::owner_kind,
                plugin_storage_usage::bytes,
            ))
            .filter(plugin_storage_usage::plugin.eq(plugin))
            .order(plugin_storage_usage::owner_kind)
            .load(&mut conn)
            .await
            .unwrap();
        assert_eq!(
            shares,
            vec![("community".into(), 91), ("direct".into(), 91)]
        );
    }
}
