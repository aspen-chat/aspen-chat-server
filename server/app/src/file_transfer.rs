//! The deployment's record of files offered in calls and sent between people, for its
//! moderators (`file_offer`, `file_transfer`). The voice servers report each offer, and each
//! transfer's start and end (`voice_protocol::control::VoiceReport`); the files themselves go
//! between the two devices, or through a voice server's relay as ciphertext, and never reach
//! this server. Anyone who may view the Administration Dashboard reads the record, newest offer
//! first, each with its transfers.

use crate::context::GlobalServerContext;
use crate::cursor::{List, Paged};
use crate::{ChannelId, UserId};
use aspen_schema::{file_offer, file_transfer};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::{AsExpression, FromSqlRow};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;
use voice_protocol::signal::{TransferEnd, TransferMode};

/// How a transfer travelled, as its receiver chose.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum FileTransferMode {
    DirectPreferred,
    RelayOnly,
}

crate::wire_name_traits!(FileTransferMode);
crate::text_sql_traits!(FileTransferMode);

impl From<TransferMode> for FileTransferMode {
    fn from(mode: TransferMode) -> Self {
        match mode {
            TransferMode::DirectPreferred => Self::DirectPreferred,
            TransferMode::RelayOnly => Self::RelayOnly,
        }
    }
}

/// How a transfer ended.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum FileTransferOutcome {
    Completed,
    Cancelled,
    Failed,
    /// One side left the call.
    Left,
    /// The sender lost Transfer files in the call.
    NotPermitted,
}

crate::wire_name_traits!(FileTransferOutcome);
crate::text_sql_traits!(FileTransferOutcome);

impl From<TransferEnd> for FileTransferOutcome {
    fn from(reason: TransferEnd) -> Self {
        match reason {
            TransferEnd::Completed => Self::Completed,
            TransferEnd::Cancelled => Self::Cancelled,
            TransferEnd::Failed => Self::Failed,
            TransferEnd::Left => Self::Left,
            TransferEnd::NotPermitted => Self::NotPermitted,
        }
    }
}

/// An offer, as a voice server reported it.
pub struct NewOffer {
    pub channel: Uuid,
    /// The voice server's id for the offer, time-ordered.
    pub record: Uuid,
    pub sender: Uuid,
    pub name: String,
    pub size: u64,
    pub allow_direct: bool,
    pub valid_for_seconds: u32,
}

pub async fn record_offer(conn: &mut AsyncPgConnection, offer: NewOffer) -> crate::Result<()> {
    diesel::insert_into(file_offer::table)
        .values((
            file_offer::id.eq(offer.record),
            file_offer::channel.eq(Some(ChannelId(offer.channel))),
            file_offer::sender.eq(Some(UserId(offer.sender))),
            file_offer::file_name.eq(offer.name),
            file_offer::file_size.eq(i64::try_from(offer.size).unwrap_or(i64::MAX)),
            file_offer::allow_direct.eq(offer.allow_direct),
            file_offer::valid_for_seconds
                .eq(i32::try_from(offer.valid_for_seconds).unwrap_or(i32::MAX)),
        ))
        .on_conflict_do_nothing()
        .execute(conn)
        .await?;
    Ok(())
}

/// Opens a transfer of the offer `record` to `receiver`, unless one is open already: a receiver
/// has at most one at a time, and the report of its start may be applied twice.
pub async fn record_start(
    conn: &mut AsyncPgConnection,
    record: Uuid,
    receiver: Uuid,
    mode: TransferMode,
) -> crate::Result<()> {
    let open: i64 = file_transfer::table
        .filter(file_transfer::offer.eq(record))
        .filter(file_transfer::receiver.eq(UserId(receiver)))
        .filter(file_transfer::ended_at.is_null())
        .count()
        .get_result(conn)
        .await?;
    if open > 0 {
        return Ok(());
    }
    diesel::insert_into(file_transfer::table)
        .values((
            file_transfer::id.eq(Uuid::now_v7()),
            file_transfer::offer.eq(record),
            file_transfer::receiver.eq(Some(UserId(receiver))),
            file_transfer::mode.eq(FileTransferMode::from(mode)),
        ))
        .execute(conn)
        .await?;
    Ok(())
}

/// Closes the open transfer of the offer `record` to `receiver`: a receiver has at most one at
/// a time.
pub async fn record_end(
    conn: &mut AsyncPgConnection,
    record: Uuid,
    receiver: Uuid,
    ended_by: Uuid,
    reason: TransferEnd,
) -> crate::Result<()> {
    diesel::update(
        file_transfer::table
            .filter(file_transfer::offer.eq(record))
            .filter(file_transfer::receiver.eq(UserId(receiver)))
            .filter(file_transfer::ended_at.is_null()),
    )
    .set((
        file_transfer::ended_at.eq(Some(Utc::now())),
        file_transfer::outcome.eq(Some(FileTransferOutcome::from(reason))),
        file_transfer::ended_by.eq(Some(UserId(ended_by))),
    ))
    .execute(conn)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = file_offer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct OfferRow {
    pub id: Uuid,
    pub channel: Option<ChannelId>,
    pub sender: Option<UserId>,
    pub file_name: String,
    pub file_size: i64,
    pub allow_direct: bool,
    pub valid_for_seconds: i32,
    pub offered_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = file_transfer)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct TransferRow {
    pub offer: Uuid,
    pub receiver: Option<UserId>,
    pub mode: FileTransferMode,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub outcome: Option<FileTransferOutcome>,
    pub ended_by: Option<UserId>,
}

/// The most offers one page of the record holds.
pub const MAX_PAGE: i64 = 100;

/// The place a cursor of the record holds: the id of the last offer of the page before, which
/// is all the record is ordered by.
fn offer_position(cursor: &str) -> crate::Result<Uuid> {
    crate::cursor::decode(List::FileOffers, cursor)
}

/// One page of offers, newest first by id, after `after`, and the cursor of the next; with
/// `user`, only the offers they made or received.
async fn offers_page(
    conn: &mut AsyncPgConnection,
    after: Option<Uuid>,
    user: Option<UserId>,
    limit: i64,
) -> crate::Result<Paged<OfferRow>> {
    let limit = limit.clamp(1, MAX_PAGE);
    let offers: Vec<OfferRow> = match user {
        // One person's: what they offered and what they received, each newest first through
        // its own index (`file_offer_sender`, `file_transfer_receiver`), merged, so the page
        // reads only their offers however many others there are.
        Some(user) => {
            #[derive(diesel::QueryableByName)]
            struct Listed {
                #[diesel(sql_type = diesel::sql_types::Uuid)]
                id: Uuid,
            }
            // The comparison is in the statement only when there is a cursor: behind
            // `$3 IS NULL OR …` a plan made for any parameters could not start either index
            // scan at it.
            let (sent_after, received_after) = if after.is_some() {
                ("AND id < $3", "AND offer < $3")
            } else {
                ("", "")
            };
            let query = diesel::sql_query(format!(
                r#"
                SELECT id FROM (
                    (SELECT id FROM file_offer
                     WHERE sender = $1 {sent_after}
                     ORDER BY id DESC LIMIT $2)
                    UNION
                    (SELECT DISTINCT offer AS id FROM file_transfer
                     WHERE receiver = $1 {received_after}
                     ORDER BY offer DESC LIMIT $2)
                ) theirs
                ORDER BY id DESC
                LIMIT $2
                "#,
            ))
            .bind::<diesel::sql_types::Uuid, _>(user.0)
            .bind::<diesel::sql_types::BigInt, _>(limit + 1)
            .into_boxed::<diesel::pg::Pg>();
            let query = match after {
                Some(after) => query.bind::<diesel::sql_types::Uuid, _>(after),
                None => query,
            };
            let listed: Vec<Listed> = query.load(conn).await?;
            let ids: Vec<Uuid> = listed.into_iter().map(|l| l.id).collect();
            file_offer::table
                .select(OfferRow::as_select())
                .filter(file_offer::id.eq_any(&ids))
                .order(file_offer::id.desc())
                .load(conn)
                .await?
        }
        None => {
            let mut query = file_offer::table
                .select(OfferRow::as_select())
                .order(file_offer::id.desc())
                .limit(limit + 1)
                .into_boxed();
            if let Some(after) = after {
                query = query.filter(file_offer::id.lt(after));
            }
            query.load(conn).await?
        }
    };
    Ok(crate::cursor::cut(
        List::FileOffers,
        offers,
        limit as usize,
        |last| last.id,
    ))
}

/// A page of the record of offers, newest first: at most `limit` of them (up to
/// [`MAX_PAGE`]), after the place `cursor` holds when given, which is the `next_cursor` of the
/// page before, each with its transfers in the order they began; with `user`, only the offers
/// they made or received.
pub async fn read_log(
    state: &GlobalServerContext,
    cursor: Option<&str>,
    user: Option<UserId>,
    limit: i64,
) -> crate::Result<Paged<(OfferRow, Vec<TransferRow>)>> {
    let after = cursor.map(offer_position).transpose()?;
    let mut conn = state.connection_pool.get().await?;
    let Paged {
        items: offers,
        next_cursor,
    } = offers_page(conn.as_mut(), after, user, limit).await?;
    let ids: Vec<Uuid> = offers.iter().map(|offer| offer.id).collect();
    let mut transfers: HashMap<Uuid, Vec<TransferRow>> = HashMap::new();
    for transfer in file_transfer::table
        .select(TransferRow::as_select())
        .filter(file_transfer::offer.eq_any(&ids))
        .order(file_transfer::id)
        .load::<TransferRow>(conn.as_mut())
        .await?
    {
        transfers.entry(transfer.offer).or_default().push(transfer);
    }
    let items = offers
        .into_iter()
        .map(|offer| {
            let of = transfers.remove(&offer.id).unwrap_or_default();
            (offer, of)
        })
        .collect();
    Ok(Paged { items, next_cursor })
}

#[cfg(test)]
mod database_tests {
    use super::*;
    use diesel_async::{AsyncConnection, SimpleAsyncConnection};

    const ANN: &str = "f17e7000-0000-4000-8000-000000000001";
    const BEN: &str = "f17e7000-0000-4000-8000-000000000002";

    fn offer(n: u8) -> Uuid {
        format!("ffffffff-ffff-7fff-bfff-ffffffffff{n:02x}")
            .parse()
            .unwrap()
    }

    /// Which offers one page holds, by their numbers, and the next page's cursor.
    async fn page(
        conn: &mut AsyncPgConnection,
        user: Option<&str>,
        cursor: Option<&str>,
        limit: i64,
    ) -> (Vec<u8>, Option<String>) {
        let page = offers_page(
            conn,
            cursor.map(|c| offer_position(c).unwrap()),
            user.map(|u| UserId(u.parse().unwrap())),
            limit,
        )
        .await
        .unwrap();
        let offers = page
            .items
            .into_iter()
            .map(|row| (1..=u8::MAX).find(|n| offer(*n) == row.id).unwrap())
            .collect();
        (offers, page.next_cursor)
    }

    /// The record pages by its offers' ids, newest first, everyone's or one person's, on past
    /// an offer that is gone, and its last page says it is the last however many it holds.
    #[tokio::test]
    async fn the_record_pages_to_a_last_page_that_is_full() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        // The record is the deployment's, so the test's offers are the only ones only once
        // the rest are gone, for as long as its transaction lasts.
        conn.batch_execute(&format!(
            r#"DELETE FROM file_offer;
               INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at)
                   VALUES ('{ANN}', '{ANN}', '', now(), now()), ('{BEN}', '{BEN}', '', now(), now())"#
        ))
        .await
        .unwrap();
        // Ann made the odd offers, and received the second and the fourth from Ben, the
        // fourth twice.
        for n in 1..=6_u8 {
            let id = offer(n);
            let sender = if n % 2 == 1 { ANN } else { BEN };
            conn.batch_execute(&format!(
                r#"INSERT INTO file_offer (id, file_name, file_size, allow_direct, valid_for_seconds, sender)
                       VALUES ('{id}', 'f', 1, true, 60, '{sender}')"#
            ))
            .await
            .unwrap();
        }
        conn.batch_execute(&format!(
            r#"INSERT INTO file_transfer (id, offer, mode, receiver) VALUES
                   ('{}', '{}', 'relayOnly', '{ANN}'),
                   ('{}', '{}', 'relayOnly', '{ANN}'),
                   ('{}', '{}', 'relayOnly', '{ANN}')"#,
            offer(101),
            offer(2),
            offer(102),
            offer(4),
            offer(103),
            offer(4),
        ))
        .await
        .unwrap();

        let (offers, cursor) = page(&mut conn, None, None, 2).await;
        assert_eq!(offers, [6, 5]);
        // The fifth, which the cursor was made from, is gone.
        conn.batch_execute(&format!("DELETE FROM file_offer WHERE id = '{}'", offer(5)))
            .await
            .unwrap();
        let (offers, cursor) = page(&mut conn, None, cursor.as_deref(), 3).await;
        assert_eq!(offers, [4, 3, 2]);
        assert!(cursor.is_some());
        let (offers, cursor) = page(&mut conn, None, cursor.as_deref(), 3).await;
        assert_eq!(offers, [1]);
        assert_eq!(cursor, None);

        // Ann's: the fourth, third, second, and first.
        let (offers, cursor) = page(&mut conn, Some(ANN), None, 2).await;
        assert_eq!(offers, [4, 3]);
        let (offers, cursor) = page(&mut conn, Some(ANN), cursor.as_deref(), 2).await;
        assert_eq!(offers, [2, 1]);
        assert_eq!(cursor, None);
    }

    #[test]
    fn a_cursor_of_no_record_is_refused() {
        let refused =
            |cursor: &str| matches!(offer_position(cursor), Err(crate::Error::Validation(_)));
        assert!(refused("garbage"));
        // An offer's id as it is, which is not a cursor.
        assert!(refused(&offer(1).to_string()));
        // A place of another shape, and a moderation log's cursor, whose place is an id
        // alone too.
        assert!(refused(&crate::cursor::encode(
            List::FileOffers,
            &(1_i64, offer(1))
        )));
        assert!(refused(&crate::cursor::encode(
            List::ModerationLog,
            &offer(1)
        )));
        assert!(!refused(&crate::cursor::encode(
            List::FileOffers,
            &offer(1)
        )));
    }
}
