//! Plugins' timers: each due once, at a time the plugin set, with a payload of its own. Each is
//! a job (`firePluginTimer`, keyed `plugin/key`; `app::jobs`) due when the timer is, which calls
//! the plugin's `observe` with `timer-fired` and is done once the plugin has handled it. One
//! whose handling failed is tried again, [`MAX_ATTEMPTS`] times in all. Only plugins that are on
//! are called; a plugin's timers wait while it is off, and fall due again as it is turned on
//! ([`wake`]). A timer set in a scope (`set-timer-in`) goes with it, as the plugin's storage
//! there does (`storage::forget`). A plugin keeps at most `MAX_TIMERS_PER_OWNER` timers in each
//! owner's share (`storage::Owner`: a community, a DM, a user, or the deployment for timers set
//! in no scope), so no one community can use up the timers the plugin may set for the rest, and
//! at most `MAX_TIMERS_PER_PLUGIN` in all. Timers' jobs share a bound of their own on how many
//! run at once (`jobs::bounded`), so a plugin's timers cannot take every place the runner has.

use super::host::wit;
use super::registry::Running;
use super::storage::{Owner, Scope};
use crate::context::GlobalServerContext;
use crate::jobs::{Claimed, JobClass, JobKind, Outcome};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Bool, Jsonb, SmallInt, Text, Timestamptz, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

/// A timer as its job holds it.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Timer {
    plugin: String,
    key: String,
    payload: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scope: Option<Uuid>,
    owner_kind: String,
    owner: Uuid,
}

/// The key of the job of `plugin`'s timer `key`. A plugin's id holds no `/`.
fn job_key(plugin: &str, key: &str) -> String {
    format!("{plugin}/{key}")
}

/// The most timers one plugin keeps in one owner's share.
pub const MAX_TIMERS_PER_OWNER: i64 = 1_000;
/// The most timers one plugin keeps in all, every owner's share together, so that a plugin
/// running in many communities cannot fill the job table.
pub const MAX_TIMERS_PER_PLUGIN: i64 = 100_000;
/// How many times a timer is handed to its plugin before it is given up.
pub const MAX_ATTEMPTS: i32 = 3;
/// How long a timer of a plugin that is off waits before it looks again; turning the plugin on
/// makes it due at once.
const OFF_WAIT: Duration = Duration::from_secs(600);
/// How long a timer of a plugin that is on, but not yet loaded on the server that claimed it,
/// waits for it to be, within [`LOADING_FOR`] of the plugin's last change; past that, the server
/// is taken to be one that cannot load it, and the timer waits [`OFF_WAIT`] for another.
const LOADING_WAIT: Duration = Duration::from_secs(1);
const LOADING_FOR: chrono::TimeDelta = chrono::TimeDelta::minutes(1);
/// How long a key or payload may be.
const MAX_KEY: usize = 200;
const MAX_PAYLOAD: usize = 16 << 10;

/// Sets `key` due at `due` with `payload`, replacing a timer of that key, to go with `scope`
/// when it names a community, a channel, or a user. A `due` in the past makes it due now: the
/// plugin is handed `due` as it set it, but its job waits its turn behind every job already due
/// (`jobs::claim` takes the longest due first), so no plugin can put its timers ahead of them.
pub async fn set(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    key: &str,
    due: &str,
    payload: &str,
    scope: &Scope,
) -> Result<(), wit::Error> {
    if key.is_empty() || key.len() > MAX_KEY || payload.len() > MAX_PAYLOAD {
        return Err(wit::Error::Invalid(format!(
            "a timer's key is 1 to {MAX_KEY} bytes and its payload at most {MAX_PAYLOAD}"
        )));
    }
    let due: DateTime<Utc> = DateTime::parse_from_rfc3339(due)
        .map_err(|_| wit::Error::Invalid("a timer's time is RFC 3339".into()))?
        .with_timezone(&Utc);
    let fail = |e: crate::Error| super::host::from_app(plugin_id, e);
    let owner = Owner::of(conn, scope).await.map_err(fail)?;
    let job_key = job_key(plugin_id, key);
    let replacing: bool = diesel::select(diesel::dsl::exists(
        aspen_schema::job::table
            .filter(aspen_schema::job::kind.eq(JobKind::FirePluginTimer))
            .filter(aspen_schema::job::key.eq(&job_key))
            .filter(aspen_schema::job::failed_at.is_null()),
    ))
    .get_result(conn)
    .await
    .map_err(|e| fail(e.into()))?;
    if !replacing && total_full(state, conn, plugin_id).await.map_err(fail)? {
        return Err(wit::Error::Limit(format!(
            "a plugin keeps at most {MAX_TIMERS_PER_PLUGIN} timers in all"
        )));
    }
    let (scope_kind, scope) = match scope {
        Scope::Deployment => (None, None),
        scope => (Some(scope.kind().to_owned()), Some(scope.id())),
    };
    let timer = Timer {
        plugin: plugin_id.to_owned(),
        key: key.to_owned(),
        payload: payload.to_owned(),
        scope_kind,
        scope,
        owner_kind: owner.kind().to_owned(),
        owner: owner.id(),
    };
    let timer = serde_json::to_value(&timer).map_err(|e| fail(e.into()))?;
    conn.transaction(|conn| {
        async move {
            // Timers of one owner's share are set one at a time, so two set at once cannot each
            // find the last place free.
            diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind::<Text, _>(format!(
                    "plugin-timers/{plugin_id}/{}/{}",
                    owner.kind(),
                    owner.id()
                ))
                .execute(conn)
                .await?;
            #[derive(QueryableByName)]
            struct Held {
                #[diesel(sql_type = diesel::sql_types::BigInt)]
                held: i64,
                #[diesel(sql_type = Bool)]
                replacing: bool,
            }
            // Setting a key again that already counts in this share takes no more room.
            let Held { held, replacing } = diesel::sql_query(
                r#"
                SELECT count(*) AS held, coalesce(bool_or(key = $4), false) AS replacing FROM job
                WHERE kind = $1 AND payload->>'plugin' = $2 AND payload->>'ownerKind' = $3
                  AND payload->>'owner' = $5 AND failed_at IS NULL
                "#,
            )
            .bind::<Text, _>(JobKind::FirePluginTimer)
            .bind::<Text, _>(plugin_id)
            .bind::<Text, _>(owner.kind())
            .bind::<Text, _>(&job_key)
            .bind::<Text, _>(owner.id().to_string())
            .get_result(conn)
            .await?;
            if held >= MAX_TIMERS_PER_OWNER && !replacing {
                return Ok(Err(wit::Error::Limit(format!(
                    "a plugin keeps at most {MAX_TIMERS_PER_OWNER} timers for one {}",
                    owner.kind()
                ))));
            }
            // Setting it again replaces it whole, under a new id, so a run of the timer it
            // replaces, still handing it to the plugin, finishes without deleting this one.
            diesel::sql_query(
                r#"
                INSERT INTO job (id, kind, key, class, due, not_before, payload)
                VALUES ($1, $2, $3, $4, $5, GREATEST($5, now()), $6)
                ON CONFLICT (kind, key) DO UPDATE
                SET id = excluded.id, due = excluded.due, not_before = excluded.not_before,
                    payload = excluded.payload, progress = NULL, attempts = 0,
                    running_since = NULL, failed_at = NULL, error = NULL
                "#,
            )
            .bind::<PgUuid, _>(Uuid::now_v7())
            .bind::<Text, _>(JobKind::FirePluginTimer)
            .bind::<Text, _>(&job_key)
            .bind::<SmallInt, _>(JobClass::Normal.rank())
            .bind::<Timestamptz, _>(due)
            .bind::<Jsonb, _>(timer)
            .execute(conn)
            .await?;
            Ok::<_, crate::Error>(Ok(()))
        }
        .scope_boxed()
    })
    .await
    .map_err(fail)?
}

/// Whether `plugin_id` keeps [`MAX_TIMERS_PER_PLUGIN`] timers, every owner's share together.
/// The count is read through `job_plugin_timer_owner`, no further than the limit, and kept by
/// each server for a few seconds (`Plugins::timer_totals`), so servers setting timers at once
/// may together pass the limit by what they set in that time.
async fn total_full(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
) -> crate::Result<bool> {
    let total = match state.plugins.timer_totals.get(plugin_id) {
        Some(total) => total,
        None => {
            #[derive(QueryableByName)]
            struct Total {
                #[diesel(sql_type = diesel::sql_types::BigInt)]
                total: i64,
            }
            let Total { total } = diesel::sql_query(
                "SELECT count(*) AS total FROM (SELECT 1 FROM job \
                 WHERE kind = $1 AND payload->>'plugin' = $2 LIMIT $3) held",
            )
            .bind::<Text, _>(JobKind::FirePluginTimer)
            .bind::<Text, _>(plugin_id)
            .bind::<diesel::sql_types::BigInt, _>(MAX_TIMERS_PER_PLUGIN)
            .get_result(conn)
            .await?;
            state
                .plugins
                .timer_totals
                .insert(plugin_id.to_owned(), total);
            total
        }
    };
    Ok(total >= MAX_TIMERS_PER_PLUGIN)
}

pub async fn cancel(conn: &mut AsyncPgConnection, plugin_id: &str, key: &str) -> crate::Result<()> {
    diesel::sql_query("DELETE FROM job WHERE kind = $1 AND key = $2")
        .bind::<Text, _>(JobKind::FirePluginTimer)
        .bind::<Text, _>(job_key(plugin_id, key))
        .execute(conn)
        .await?;
    Ok(())
}

/// Deletes up to `limit` of the timers of `plugin_id`, through `job_plugin_timer_owner`, for
/// purging it; answers how many went.
pub async fn forget_batch(
    conn: &mut AsyncPgConnection,
    plugin_id: &str,
    limit: i64,
) -> crate::Result<i64> {
    let deleted = diesel::sql_query(
        "DELETE FROM job WHERE id IN (SELECT id FROM job \
         WHERE kind = $1 AND payload->>'plugin' = $2 LIMIT $3)",
    )
    .bind::<Text, _>(JobKind::FirePluginTimer)
    .bind::<Text, _>(plugin_id)
    .bind::<diesel::sql_types::BigInt, _>(limit)
    .execute(conn)
    .await?;
    Ok(deleted as i64)
}

/// Makes the timers of `plugin_id` that fell due while it was off due at once, on `conn`, in
/// the transaction that turns it on.
pub async fn wake(conn: &mut AsyncPgConnection, plugin_id: &str) -> crate::Result<()> {
    diesel::sql_query(
        r#"
        UPDATE job SET not_before = now()
        WHERE kind = $1 AND payload->>'plugin' = $2 AND due <= now() AND not_before > now()
          AND running_since IS NULL AND failed_at IS NULL
        "#,
    )
    .bind::<Text, _>(JobKind::FirePluginTimer)
    .bind::<Text, _>(plugin_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// One timer's job: hands the timer to its plugin, once the plugin is on and here.
pub async fn fire_step(state: &GlobalServerContext, job: &Claimed) -> crate::Result<Outcome> {
    let timer: Timer = job.payload()?;
    let plugin = state.plugins.get(&timer.plugin).filter(|p| {
        p.holds(super::PluginPermission::Timers)
            && p.manifest
                .hooks
                .observe
                .contains(&super::manifest::ObserveHook::TimerFire)
    });
    let Some(plugin) = plugin else {
        let installed: Option<(bool, DateTime<Utc>, Option<DateTime<Utc>>)> =
            aspen_schema::plugin::table
                .select((
                    aspen_schema::plugin::enabled,
                    aspen_schema::plugin::updated_at,
                    aspen_schema::plugin::removed_at,
                ))
                .filter(aspen_schema::plugin::id.eq(&timer.plugin))
                .first(state.connection_pool.get().await?.as_mut())
                .await
                .optional()?;
        return Ok(match installed {
            // Its purge takes the rest of its timers.
            None | Some((_, _, Some(_))) => Outcome::Done,
            Some((true, changed, None))
                if state.plugins.get(&timer.plugin).is_none()
                    && Utc::now() - changed < LOADING_FOR =>
            {
                Outcome::Later(LOADING_WAIT)
            }
            Some(_) => Outcome::Later(OFF_WAIT),
        });
    };
    let observed = wit::Observed::TimerFired(wit::Timer {
        key: timer.key,
        payload: timer.payload,
        due: job.due.to_rfc3339(),
    });
    let running = Running {
        plugin,
        community_settings: None,
    };
    super::observe::deliver(state, &running, observed, None, None, Vec::new())
        .await
        .map_err(|e| crate::Error::Plugin(format!("{e:#}")))?;
    Ok(Outcome::Done)
}
