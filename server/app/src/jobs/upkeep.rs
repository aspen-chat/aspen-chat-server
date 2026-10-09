//! Recurring upkeep that nothing else causes: sweeping what has expired, in batches of
//! [`BATCH`], each a step of its own, so a backlog of any size is worked through without one long
//! statement.

use super::{Claimed, Outcome};
use crate::context::GlobalServerContext;
use diesel_async::RunQueryDsl;

/// How many rows one step of a sweep deletes.
const BATCH: i64 = 1000;

/// Runs one batch of `sql`, a `DELETE` of at most `$1` rows: the job carries on while batches
/// come back full.
async fn sweep(state: &GlobalServerContext, sql: &str) -> crate::Result<Outcome> {
    let mut conn = state.connection_pool.get().await?;
    let deleted = diesel::sql_query(sql)
        .bind::<diesel::sql_types::BigInt, _>(BATCH)
        .execute(conn.as_mut())
        .await?;
    Ok(if deleted as i64 >= BATCH {
        Outcome::Progress(serde_json::Value::Null)
    } else {
        Outcome::Done
    })
}

/// Jobs given up more than `jobs::FAILED_KEPT_DAYS` ago, through `job_failed`, and previews no
/// server has made within `preview::GIVEN_UP_AFTER`, as none does of videos where no server can
/// run `ffmpeg`, through `job_next`.
pub async fn prune_failed_jobs(
    state: &GlobalServerContext,
    _job: &Claimed,
) -> crate::Result<Outcome> {
    let failed = sweep(
        state,
        &format!(
            "DELETE FROM job WHERE id IN (SELECT id FROM job \
             WHERE failed_at < now() - interval '{} days' ORDER BY failed_at DESC LIMIT $1)",
            super::FAILED_KEPT_DAYS
        ),
    )
    .await?;
    let kinds = crate::attachment::preview::KINDS
        .iter()
        .map(|kind| format!("'{kind}'"))
        .collect::<Vec<_>>()
        .join(", ");
    let unmade = sweep(
        state,
        &format!(
            "DELETE FROM job WHERE id IN (SELECT id FROM job \
             WHERE kind IN ({kinds}) AND failed_at IS NULL \
               AND not_before < now() - make_interval(secs => {}) LIMIT $1)",
            crate::attachment::preview::GIVEN_UP_AFTER.as_secs()
        ),
    )
    .await?;
    Ok(match (failed, unmade) {
        (Outcome::Done, Outcome::Done) => Outcome::Done,
        _ => Outcome::Progress(serde_json::Value::Null),
    })
}

/// Sessions and sign-ins that have ended: sessions an hour after they expire, and sign-ins (whose
/// sessions go with them) a day after theirs, revoked ones included, which `revoke` ends by
/// expiring. Nothing reads either once it has expired, and without this both tables would keep
/// a row for every session ever refreshed.
pub async fn sweep_sign_ins(state: &GlobalServerContext, _job: &Claimed) -> crate::Result<Outcome> {
    let sessions = sweep(
        state,
        "DELETE FROM session WHERE token IN (SELECT token FROM session \
         WHERE expires < (now() - interval '1 hour') AT TIME ZONE 'UTC' LIMIT $1)",
    )
    .await?;
    let sign_ins = sweep(
        state,
        "DELETE FROM refresh_token WHERE token IN (SELECT token FROM refresh_token \
         WHERE expires < (now() - interval '1 day') AT TIME ZONE 'UTC' LIMIT $1)",
    )
    .await?;
    Ok(match (sessions, sign_ins) {
        (Outcome::Done, Outcome::Done) => Outcome::Done,
        _ => Outcome::Progress(serde_json::Value::Null),
    })
}
