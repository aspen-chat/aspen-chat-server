//! Counts of what one account may do only so often, kept in PostgreSQL (`attempt_count`) so that
//! no restart, of this server or of Valkey, hands a guesser a fresh allowance. Each window starts
//! with its first count and lasts its kind's length, on the database's clock; a count whose window
//! has ended starts again at one, and `jobs::upkeep::sweep_expired` deletes it.

use crate::UserId;
use diesel::sql_types::{Array, Double, Integer, Text, Uuid};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

/// What is counted, stored as its wire name in `attempt_count.kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    /// Wrong passwords and codes in a quarter hour (`two_factor::limited`).
    SignInFailures,
    /// Wrong passwords and codes in a day (`two_factor::limited`).
    SignInFailuresDaily,
    /// Password reset codes mailed in an hour (`email::reset::send_code`).
    ResetCodesSent,
}

crate::wire_name_traits!(Kind);

/// Counts one more of each of `counts` for `user`, a kind and the seconds its window lasts,
/// answering each count as it stands now, in the order given. Each count and its window are set
/// in one statement, so attempts arriving at once are each counted.
pub async fn add<const N: usize>(
    conn: &mut AsyncPgConnection,
    user: UserId,
    counts: [(Kind, i64); N],
) -> crate::Result<[i64; N]> {
    #[derive(diesel::QueryableByName)]
    struct Counted {
        #[diesel(sql_type = Text)]
        kind: String,
        #[diesel(sql_type = Integer)]
        count: i32,
    }
    let kinds: Vec<String> = counts.iter().map(|(kind, _)| kind.to_string()).collect();
    let seconds: Vec<f64> = counts.iter().map(|&(_, seconds)| seconds as f64).collect();
    let counted: Vec<Counted> = diesel::sql_query(
        "INSERT INTO attempt_count (\"user\", kind, count, until) \
         SELECT $1, kind, 1, now() + make_interval(secs => seconds) \
         FROM unnest($2::text[], $3::float8[]) AS counted (kind, seconds) \
         ON CONFLICT (\"user\", kind) DO UPDATE SET \
           count = CASE WHEN attempt_count.until <= now() THEN 1 \
                        ELSE attempt_count.count + 1 END, \
           until = CASE WHEN attempt_count.until <= now() THEN EXCLUDED.until \
                        ELSE attempt_count.until END \
         RETURNING kind, count",
    )
    .bind::<Uuid, _>(user.0)
    .bind::<Array<Text>, _>(&kinds)
    .bind::<Array<Double>, _>(&seconds)
    .load(conn)
    .await?;
    let mut answered = [0; N];
    for (answer, kind) in answered.iter_mut().zip(&kinds) {
        *answer = counted
            .iter()
            .find(|c| &c.kind == kind)
            .map(|c| i64::from(c.count))
            .ok_or_else(|| {
                // The insert answers a row for every kind it was given.
                crate::Error::Diesel(diesel::result::Error::DeserializationError(
                    format!("attempt_count answered no row for {kind}").into(),
                ))
            })?;
    }
    Ok(answered)
}

/// How many of `kind` `user`'s window holds now; none once it has ended.
pub async fn read(conn: &mut AsyncPgConnection, user: UserId, kind: Kind) -> crate::Result<i64> {
    #[derive(diesel::QueryableByName)]
    struct Count {
        #[diesel(sql_type = Integer)]
        count: i32,
    }
    let found: Vec<Count> = diesel::sql_query(
        "SELECT count FROM attempt_count WHERE \"user\" = $1 AND kind = $2 AND until > now()",
    )
    .bind::<Uuid, _>(user.0)
    .bind::<Text, _>(kind.to_string())
    .load(conn)
    .await?;
    Ok(found.into_iter().next().map_or(0, |c| i64::from(c.count)))
}

/// Takes one back off each of `kinds` whose window is still open, as for an attempt that could
/// not be checked.
pub async fn take_back(
    conn: &mut AsyncPgConnection,
    user: UserId,
    kinds: &[Kind],
) -> crate::Result<()> {
    let kinds: Vec<String> = kinds.iter().map(Kind::to_string).collect();
    diesel::sql_query(
        "UPDATE attempt_count SET count = count - 1 \
         WHERE \"user\" = $1 AND kind = ANY($2) AND until > now() AND count > 0",
    )
    .bind::<Uuid, _>(user.0)
    .bind::<Array<Text>, _>(&kinds)
    .execute(conn)
    .await?;
    Ok(())
}

/// Ends `user`'s window of `kind`, so the next count starts a new one.
pub async fn clear(conn: &mut AsyncPgConnection, user: UserId, kind: Kind) -> crate::Result<()> {
    diesel::sql_query("DELETE FROM attempt_count WHERE \"user\" = $1 AND kind = $2")
        .bind::<Uuid, _>(user.0)
        .bind::<Text, _>(kind.to_string())
        .execute(conn)
        .await?;
    Ok(())
}
