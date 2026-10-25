//! How much one person may upload in a rolling day: the deployment setting `upload_quota_gib`
//! (25 GiB by default; 0 sets no limit). Each upload started, an attachment or an icon, is
//! recorded in `upload_usage` with the bytes its URL is signed for, which is all it can write,
//! whether or not it is then confirmed, sent, or deleted; [`reserve`] refuses one that would take
//! the day's uploads past the quota, saying when there will be room. Records older than a day
//! count for nothing and are pruned with the upload sweep ([`prune`]). A bot's uploads count
//! toward its owner's quota ([`charged_account`]), so an owner with many bots has one quota, not
//! one for each.

use crate::UserId;
use crate::context::GlobalServerContext;
use crate::t;
use aspen_schema::{upload_usage, user};
use chrono::{DateTime, Utc};
use diesel::{ExpressionMethods, OptionalExtension, QueryDsl};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::time::Duration;

/// How long an upload counts toward its uploader's quota.
pub const WINDOW: chrono::Duration = chrono::Duration::hours(24);

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(diesel::QueryableByName)]
struct Used {
    #[diesel(sql_type = diesel::sql_types::BigInt)]
    used: i64,
}

/// The account `user_id`'s uploads are charged to, its owner's for a bot and its own for anyone
/// else, with that account's row locked until the transaction ends, so what is decided against
/// its uploads is decided one upload at a time. A bot's owner never changes, so it is read
/// without a lock.
pub async fn charged_account(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
) -> crate::Result<UserId> {
    let owner: Option<UserId> = user::table
        .select(user::bot_owner)
        .filter(user::id.eq(user_id))
        .first(conn)
        .await?;
    let charged = owner.unwrap_or(user_id);
    let locked: Option<UserId> = user::table
        .select(user::id)
        .filter(user::id.eq(charged))
        .for_no_key_update()
        .first(conn)
        .await
        .optional()?;
    match locked {
        Some(account) => Ok(account),
        // An owner gone since: the bot answers for itself.
        None => {
            user::table
                .select(user::id)
                .filter(user::id.eq(user_id))
                .for_no_key_update()
                .first::<UserId>(conn)
                .await?;
            Ok(user_id)
        }
    }
}

/// Records an upload of `bytes` by `user`, refusing it with [`crate::Error::UploadQuotaExceeded`]
/// when the uploads of the last [`WINDOW`] charged to the same account ([`charged_account`]) and
/// it would come to more than the quota. That account's row is locked while it is decided, so
/// two uploads at once cannot both take the last room.
pub async fn reserve(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    bytes: u64,
) -> crate::Result<()> {
    let quota = u64::from(state.settings().upload_quota_gib) * GIB;
    if quota == 0 {
        return Ok(());
    }
    let wanted = i64::try_from(bytes).unwrap_or(i64::MAX);
    conn.transaction(|conn| {
        async move {
            let user_id = charged_account(conn, user_id).await?;
            let since = Utc::now() - WINDOW;
            let Used { used } = diesel::sql_query(
                "SELECT COALESCE(SUM(bytes), 0)::BIGINT AS used FROM upload_usage \
                 WHERE user_id = $1 AND at > $2",
            )
            .bind::<diesel::sql_types::Uuid, _>(user_id)
            .bind::<diesel::sql_types::Timestamptz, _>(since)
            .get_result(conn)
            .await?;
            let used = used.unsigned_abs();
            if used.saturating_add(bytes) > quota {
                let room_at = if bytes > quota {
                    None
                } else {
                    room_returns(conn, user_id, since, used + bytes - quota).await?
                };
                return Err(refusal(quota, room_at));
            }
            diesel::insert_into(upload_usage::table)
                .values((
                    upload_usage::id.eq(uuid::Uuid::now_v7()),
                    upload_usage::user_id.eq(user_id),
                    upload_usage::bytes.eq(wanted),
                ))
                .execute(conn)
                .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// When enough of `user_id`'s uploads since `since` will have left the window to free `needed`
/// bytes: a day after the upload that frees the last of them, oldest first.
async fn room_returns(
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    since: DateTime<Utc>,
    needed: u64,
) -> crate::Result<Option<DateTime<Utc>>> {
    let uploads: Vec<(i64, DateTime<Utc>)> = upload_usage::table
        .select((upload_usage::bytes, upload_usage::at))
        .filter(upload_usage::user_id.eq(user_id))
        .filter(upload_usage::at.gt(since))
        .order((upload_usage::at, upload_usage::id))
        .load(conn)
        .await?;
    let mut freed: u64 = 0;
    for (bytes, at) in uploads {
        freed = freed.saturating_add(bytes.unsigned_abs());
        if freed >= needed {
            return Ok(Some(at + WINDOW));
        }
    }
    Ok(None)
}

/// The refusal: the quota, and how long until there is room, when there will be.
fn refusal(quota: u64, room_at: Option<DateTime<Utc>>) -> crate::Error {
    let limit = quota / GIB;
    let Some(room_at) = room_at else {
        return crate::Error::UploadQuotaExceeded {
            detail: t!("uploadQuotaFileTooLarge", limit = limit),
            retry_after: None,
        };
    };
    let wait = (room_at - Utc::now())
        .to_std()
        .unwrap_or(Duration::ZERO)
        .max(Duration::from_secs(1));
    let minutes = wait.as_secs().div_ceil(60);
    let detail = if minutes >= 120 {
        t!(
            "uploadQuotaExceededHours",
            limit = limit,
            hours = minutes.div_ceil(60)
        )
    } else {
        t!(
            "uploadQuotaExceededMinutes",
            limit = limit,
            minutes = minutes
        )
    };
    crate::Error::UploadQuotaExceeded {
        detail,
        retry_after: Some(wait),
    }
}

/// Deletes the records of uploads that no longer count, answering how many.
pub async fn prune(state: &GlobalServerContext) -> crate::Result<usize> {
    let mut conn = state.connection_pool.get().await?;
    Ok(
        diesel::delete(upload_usage::table.filter(upload_usage::at.le(Utc::now() - WINDOW)))
            .execute(conn.as_mut())
            .await?,
    )
}
