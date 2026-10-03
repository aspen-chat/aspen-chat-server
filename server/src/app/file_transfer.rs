//! The deployment's record of files offered in calls and sent between people, for its
//! moderators (`file_offer`, `file_transfer`). The voice servers report each offer, and each
//! transfer's start and end (`voice_protocol::control::VoiceReport`); the files themselves go
//! between the two devices, or through a voice server's relay as ciphertext, and never reach
//! this server. Anyone who may view the Administration Dashboard reads the record, newest offer
//! first, each with its transfers.

use crate::app::context::GlobalServerContext;
use crate::app::{self, ChannelId, UserId};
use crate::database::schema::{file_offer, file_transfer};
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

app::wire_name_traits!(FileTransferMode);
app::text_sql_traits!(FileTransferMode);

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
}

app::wire_name_traits!(FileTransferOutcome);
app::text_sql_traits!(FileTransferOutcome);

impl From<TransferEnd> for FileTransferOutcome {
    fn from(reason: TransferEnd) -> Self {
        match reason {
            TransferEnd::Completed => Self::Completed,
            TransferEnd::Cancelled => Self::Cancelled,
            TransferEnd::Failed => Self::Failed,
            TransferEnd::Left => Self::Left,
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

pub async fn record_offer(conn: &mut AsyncPgConnection, offer: NewOffer) -> app::Result<()> {
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
) -> app::Result<()> {
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
) -> app::Result<()> {
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

/// Offers, newest first, before `before` when given, each with its transfers in the order they
/// began; with `user`, only the offers they made or received.
pub async fn read_log(
    state: &GlobalServerContext,
    before: Option<Uuid>,
    user: Option<UserId>,
    limit: i64,
) -> app::Result<Vec<(OfferRow, Vec<TransferRow>)>> {
    let mut conn = state.connection_pool.get().await?;
    let mut query = file_offer::table
        .select(OfferRow::as_select())
        .order(file_offer::id.desc())
        .limit(limit)
        .into_boxed();
    if let Some(before) = before {
        query = query.filter(file_offer::id.lt(before));
    }
    if let Some(user) = user {
        let received = file_transfer::table
            .select(file_transfer::offer)
            .filter(file_transfer::receiver.eq(user));
        query = query.filter(
            file_offer::sender
                .eq(user)
                .or(file_offer::id.eq_any(received)),
        );
    }
    let offers: Vec<OfferRow> = query.load(conn.as_mut()).await?;
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
    Ok(offers
        .into_iter()
        .map(|offer| {
            let of = transfers.remove(&offer.id).unwrap_or_default();
            (offer, of)
        })
        .collect())
}
