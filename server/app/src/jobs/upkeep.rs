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

/// What has ended and nothing reads any more: community bans past their end, through
/// `community_ban_until` (every read of bans already leaves them out); plugins' notices past
/// `plugin::notice::KEPT`, through `plugin_notice_by_created`; and voice failure reports older
/// than `[voice] failure_window_seconds`, which count for nothing, through
/// `voice_server_failure_reported`.
pub async fn sweep_expired(state: &GlobalServerContext, _job: &Claimed) -> crate::Result<Outcome> {
    let window = state.config.voice.failure_window_seconds;
    let notices = crate::plugin::notice::KEPT.num_seconds();
    let mut outcome = Outcome::Done;
    for sql in [
        "DELETE FROM community_ban WHERE (community, \"user\") IN (SELECT community, \"user\" \
         FROM community_ban WHERE until < now() LIMIT $1)"
            .to_owned(),
        format!(
            "DELETE FROM plugin_notice WHERE id IN (SELECT id FROM plugin_notice \
             WHERE created_at < now() - make_interval(secs => {notices}) LIMIT $1)"
        ),
        format!(
            "DELETE FROM voice_server_failure WHERE ctid IN (SELECT ctid FROM voice_server_failure \
             WHERE reported_at < now() - make_interval(secs => {window}) LIMIT $1)"
        ),
    ] {
        if let Outcome::Progress(_) = sweep(state, &sql).await? {
            outcome = Outcome::Progress(serde_json::Value::Null);
        }
    }
    Ok(outcome)
}

/// How many communities one step of a recount counts.
const RECOUNT_BATCH: i64 = 500;

/// Recounts each community's members (`community.member_count`), [`RECOUNT_BATCH`] communities
/// a step in order of id, each through `community_user`'s primary key, writing only the counts
/// that changed. The count is what the dashboard lists and sorts by, so it may be behind by up
/// to the job's period.
pub async fn recount_members(state: &GlobalServerContext, job: &Claimed) -> crate::Result<Outcome> {
    #[derive(diesel::QueryableByName)]
    struct Last {
        #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Uuid>)]
        last: Option<uuid::Uuid>,
    }
    let after: Option<uuid::Uuid> = job.progress()?;
    let mut conn = state.connection_pool.get().await?;
    let Last { last } = diesel::sql_query(
        r#"
        WITH batch AS (
            SELECT id FROM community WHERE $1::uuid IS NULL OR id > $1 ORDER BY id LIMIT $2
        ),
        counted AS (
            UPDATE community c SET member_count = n.count
            FROM (SELECT b.id, (SELECT count(*) FROM community_user cu
                                WHERE cu.community = b.id)::int AS count FROM batch b) n
            WHERE c.id = n.id AND c.member_count <> n.count
        )
        SELECT (SELECT id FROM batch ORDER BY id DESC LIMIT 1) AS last
        "#,
    )
    .bind::<diesel::sql_types::Nullable<diesel::sql_types::Uuid>, _>(after)
    .bind::<diesel::sql_types::BigInt, _>(RECOUNT_BATCH)
    .get_result(conn.as_mut())
    .await?;
    Ok(match last {
        Some(last) => Outcome::Progress(serde_json::to_value(last)?),
        None => Outcome::Done,
    })
}

/// Writes the deployment's totals for today (`deployment_stats`): the latest row's, with what
/// changed since it was taken (`admin::totals_since`), so it never counts a whole table.
pub async fn record_stats(state: &GlobalServerContext, _job: &Claimed) -> crate::Result<Outcome> {
    let mut conn = state.connection_pool.get().await?;
    let totals = crate::admin::current_totals(conn.as_mut()).await?;
    diesel::sql_query(
        r#"
        INSERT INTO deployment_stats (day, taken_at, users, communities)
        VALUES (($1 AT TIME ZONE 'UTC')::date, $1, $2, $3)
        ON CONFLICT (day) DO UPDATE
        SET taken_at = excluded.taken_at, users = excluded.users,
            communities = excluded.communities
        WHERE deployment_stats.taken_at < excluded.taken_at
        "#,
    )
    .bind::<diesel::sql_types::Timestamptz, _>(totals.at)
    .bind::<diesel::sql_types::BigInt, _>(totals.users)
    .bind::<diesel::sql_types::BigInt, _>(totals.communities)
    .execute(conn.as_mut())
    .await?;
    Ok(Outcome::Done)
}
