//! Deleting the messages a ban's deletion window covers (`message::queue_deletion_of_recent`),
//! newest first, [`BATCH`] at a time, each batch one transaction that deletes it as
//! `message::soft_delete_many` does and moves the job's place on past it.

use super::{Claimed, Outcome, checkpoint};
use crate::context::GlobalServerContext;
use crate::{ChannelId, MessageId, UserId};
use diesel::QueryableByName;
use diesel::sql_types::{Array, BigInt, Nullable, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

/// How many messages one step deletes.
const BATCH: i64 = 200;

/// What to delete, fixed when the ban decided it: `author`'s messages after `after` and before
/// `before` (ids, so times: they are UUIDv7), in `places` and their threads, or anywhere.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Deletion {
    pub author: UserId,
    pub after: MessageId,
    pub before: MessageId,
    /// The channels, DMs, and threads' parents it reaches, as `message.home_channel` names them;
    /// `None` for everywhere.
    pub places: Option<Vec<ChannelId>>,
}

/// How far a deletion has come: everything it covers from `before` on is deleted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Done {
    before: MessageId,
}

#[derive(QueryableByName)]
struct Found {
    #[diesel(sql_type = PgUuid)]
    id: MessageId,
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

/// The messages `deletion` still covers before `before`, newest first, through
/// `message_by_author`: at most `limit`.
const COVERED_SQL: &str = r#"
    SELECT id FROM message
    WHERE author = $1 AND id > $2 AND id < $3 AND deleted_at IS NULL
      AND kind <> 'thread_echo'
      AND ($4::uuid[] IS NULL OR home_channel = ANY($4))
    ORDER BY id DESC
    LIMIT $5
"#;

async fn covered(
    conn: &mut AsyncPgConnection,
    deletion: &Deletion,
    before: MessageId,
    limit: i64,
) -> crate::Result<Vec<MessageId>> {
    let places: Option<Vec<uuid::Uuid>> = deletion
        .places
        .as_ref()
        .map(|places| places.iter().map(|c| c.0).collect());
    let found: Vec<Found> = diesel::sql_query(COVERED_SQL)
        .bind::<PgUuid, _>(deletion.author.0)
        .bind::<PgUuid, _>(deletion.after.0)
        .bind::<PgUuid, _>(before.0)
        .bind::<Nullable<Array<PgUuid>>, _>(places)
        .bind::<BigInt, _>(limit)
        .load(conn)
        .await?;
    Ok(found.into_iter().map(|f| f.id).collect())
}

/// How many messages `deletion` covers, as they stand.
pub async fn count(conn: &mut AsyncPgConnection, deletion: &Deletion) -> crate::Result<usize> {
    let places: Option<Vec<uuid::Uuid>> = deletion
        .places
        .as_ref()
        .map(|places| places.iter().map(|c| c.0).collect());
    let counted: Count = diesel::sql_query(format!(
        "SELECT count(*) AS count FROM ({}) covered",
        COVERED_SQL.replace("LIMIT $5", "")
    ))
    .bind::<PgUuid, _>(deletion.author.0)
    .bind::<PgUuid, _>(deletion.after.0)
    .bind::<PgUuid, _>(deletion.before.0)
    .bind::<Nullable<Array<PgUuid>>, _>(places)
    .get_result(conn)
    .await?;
    Ok(usize::try_from(counted.count).unwrap_or(0))
}

/// Deletes the next batch, and moves the job's place past it in the same transaction.
pub async fn step(state: &GlobalServerContext, job: &Claimed) -> crate::Result<Outcome> {
    let deletion: Deletion = job.payload()?;
    let before = job
        .progress::<Done>()?
        .map_or(deletion.before, |done| done.before);
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let batch = covered(conn, &deletion, before, BATCH).await?;
            let Some(last) = batch.last().copied() else {
                return Ok(Outcome::Done);
            };
            crate::message::soft_delete_many(state, conn, &batch).await?;
            checkpoint(conn, job.id, &Done { before: last }).await?;
            Ok(Outcome::Continue)
        }
        .scope_boxed()
    })
    .await
}
