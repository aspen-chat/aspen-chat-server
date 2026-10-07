//! Plugins' timers (`plugin_timer`): each due once, at a time the plugin set, with a payload of
//! its own. Every API server looks for due timers each second and claims a few at a time
//! (`FOR UPDATE SKIP LOCKED`, so no two claim one), calls the plugin's `observe` with
//! `timer-fired`, and deletes the timer once it is handled. A claim lasts a minute, so a timer
//! whose handling failed, or whose server stopped, is tried again then, `MAX_ATTEMPTS` times at
//! most. Only plugins that are on are called; a plugin's timers wait while it is off. A timer set
//! in a scope (`set-timer-in`) goes with it, as the plugin's storage there does
//! (`storage::forget`).

use super::host::wit;
use super::registry::Running;
use super::storage::Scope;
use crate::context::GlobalServerContext;
use aspen_schema::plugin_timer;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, Text};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use std::time::Duration;

/// The most timers one plugin keeps.
pub const MAX_TIMERS: i64 = 10_000;
/// How many times a timer is handed to its plugin before it is dropped.
const MAX_ATTEMPTS: i32 = 3;
/// How long a key or payload may be.
const MAX_KEY: usize = 200;
const MAX_PAYLOAD: usize = 16 << 10;

/// Sets `key` due at `due` with `payload`, replacing a timer of that key, to go with `scope`
/// when it names a community, a channel, or a user.
pub async fn set(
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
    let fail = |e: diesel::result::Error| super::host::from_app(plugin_id, e.into());
    let held: i64 = plugin_timer::table
        .filter(plugin_timer::plugin.eq(plugin_id))
        .count()
        .get_result(conn)
        .await
        .map_err(fail)?;
    if held >= MAX_TIMERS {
        let replacing: bool = diesel::select(diesel::dsl::exists(
            plugin_timer::table.filter(
                plugin_timer::plugin
                    .eq(plugin_id)
                    .and(plugin_timer::key.eq(key)),
            ),
        ))
        .get_result(conn)
        .await
        .map_err(fail)?;
        if !replacing {
            return Err(wit::Error::Limit(format!(
                "a plugin keeps at most {MAX_TIMERS} timers"
            )));
        }
    }
    let (scope_kind, scope) = match scope {
        Scope::Deployment => (None, None),
        scope => (Some(scope.kind()), Some(scope.id())),
    };
    diesel::insert_into(plugin_timer::table)
        .values((
            plugin_timer::plugin.eq(plugin_id),
            plugin_timer::key.eq(key),
            plugin_timer::due.eq(due),
            plugin_timer::payload.eq(payload),
            plugin_timer::scope_kind.eq(scope_kind),
            plugin_timer::scope.eq(scope),
        ))
        .on_conflict((plugin_timer::plugin, plugin_timer::key))
        .do_update()
        .set((
            plugin_timer::due.eq(due),
            plugin_timer::payload.eq(payload),
            plugin_timer::attempts.eq(0),
            plugin_timer::claimed_until.eq(None::<DateTime<Utc>>),
            plugin_timer::scope_kind.eq(scope_kind),
            plugin_timer::scope.eq(scope),
        ))
        .execute(conn)
        .await
        .map_err(fail)?;
    Ok(())
}

pub async fn cancel(conn: &mut AsyncPgConnection, plugin_id: &str, key: &str) -> crate::Result<()> {
    diesel::delete(
        plugin_timer::table.filter(
            plugin_timer::plugin
                .eq(plugin_id)
                .and(plugin_timer::key.eq(key)),
        ),
    )
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(QueryableByName)]
struct Due {
    #[diesel(sql_type = Text)]
    plugin: String,
    #[diesel(sql_type = Text)]
    key: String,
    #[diesel(sql_type = diesel::sql_types::Timestamptz)]
    due: DateTime<Utc>,
    #[diesel(sql_type = Text)]
    payload: String,
}

/// Claims the due timers of `plugins`, a few at a time.
async fn claim(conn: &mut AsyncPgConnection, plugins: &[String]) -> crate::Result<Vec<Due>> {
    Ok(diesel::sql_query(
        r#"
        UPDATE plugin_timer SET claimed_until = now() + interval '1 minute', attempts = attempts + 1
        WHERE (plugin, key) IN (
            SELECT plugin, key FROM plugin_timer
            WHERE due <= now() AND plugin = ANY($1)
              AND (claimed_until IS NULL OR claimed_until < now()) AND attempts < $2
            ORDER BY due
            LIMIT 20
            FOR UPDATE SKIP LOCKED
        )
        RETURNING plugin, key, due, payload
        "#,
    )
    .bind::<Array<Text>, _>(plugins)
    .bind::<diesel::sql_types::Integer, _>(MAX_ATTEMPTS)
    .load(conn)
    .await?)
}

/// Looks for due timers each second and hands them to their plugins, for as long as the server
/// runs.
pub fn spawn(state: GlobalServerContext) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            if let Err(e) = round(&state).await {
                tracing::warn!("handing plugins their timers failed: {e}");
            }
        }
    });
}

async fn round(state: &GlobalServerContext) -> crate::Result<()> {
    let loaded = state.plugins.loaded();
    let timing: Vec<String> = loaded
        .iter()
        .filter(|p| {
            p.holds(super::PluginPermission::Timers)
                && p.manifest
                    .hooks
                    .observe
                    .contains(&super::manifest::ObserveHook::TimerFire)
        })
        .map(|p| p.id.clone())
        .collect();
    if timing.is_empty() {
        return Ok(());
    }
    let due = {
        let mut conn = state.connection_pool.get().await?;
        // A timer tried as often as it may be is dropped.
        diesel::delete(
            plugin_timer::table.filter(
                plugin_timer::attempts
                    .ge(MAX_ATTEMPTS)
                    .and(plugin_timer::claimed_until.lt(diesel::dsl::now)),
            ),
        )
        .execute(conn.as_mut())
        .await?;
        claim(conn.as_mut(), &timing).await?
    };
    for timer in due {
        let Some(plugin) = loaded.iter().find(|p| p.id == timer.plugin).cloned() else {
            continue;
        };
        let state = state.clone();
        tokio::spawn(async move {
            let observed = wit::Observed::TimerFired(wit::Timer {
                key: timer.key.clone(),
                payload: timer.payload,
                due: timer.due.to_rfc3339(),
            });
            let running = Running {
                plugin: plugin.clone(),
                community_settings: None,
            };
            match super::observe::deliver(&state, &running, observed, None, None, Vec::new()).await
            {
                Ok(()) => {
                    let done = async {
                        let mut conn = state.connection_pool.get().await?;
                        // Unless the plugin set the key again while handling it.
                        diesel::delete(
                            plugin_timer::table.filter(
                                plugin_timer::plugin
                                    .eq(&plugin.id)
                                    .and(plugin_timer::key.eq(&timer.key))
                                    .and(plugin_timer::due.eq(timer.due)),
                            ),
                        )
                        .execute(conn.as_mut())
                        .await?;
                        Ok::<_, crate::Error>(())
                    }
                    .await;
                    if let Err(e) = done {
                        tracing::warn!(plugin = plugin.id, "a handled timer stays: {e}");
                    }
                }
                Err(e) => {
                    tracing::warn!(plugin = plugin.id, key = timer.key, "a timer failed: {e}")
                }
            }
        });
    }
    Ok(())
}
