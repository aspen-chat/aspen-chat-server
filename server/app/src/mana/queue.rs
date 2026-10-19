//! What waits for a channel's mana: messages, slash commands, edits, polls, and echoes a busy
//! channel could not pay for when they were sent, kept as held messages
//! (`app::message::held`, `waiting = 'mana'`) and posted as the channel recharges.
//!
//! An author's waiting rows in one pool are one item, which takes turns with the others; its
//! messages post in the order sent. One job per busy pool drains it ([`drain_step`],
//! `drainManaQueue`, keyed by the channel, so one runner holds it anywhere): it ticks at 5 Hz
//! for [`STEP`] at a time and spends what the pool earns by deficit round-robin. A pointer goes
//! round the items in the order each began waiting and is kept across ticks; each turn gives
//! the item [`QUANTUM`] of credit; the item posts its oldest message whenever its credit and the
//! pool both cover that message's cost; and a tick ends when the pool cannot pay the item at the
//! pointer, the next carrying on from there. Every item gets the same credit a trip round, and
//! whole messages are paid with whole mana, so shares never grow vanishingly small however long
//! the queue. What has waited [`MAX_WAIT`] is dropped, and its author told.

use super::{MILLI, Pool, TICK};
use crate::context::GlobalServerContext;
use crate::jobs::{self, Claimed, JobClass, JobKind, NewJob, Outcome};
use crate::message::held::{self, COLUMNS, HeldKind, HeldMessage, HeldWaiting};
use crate::t;
use crate::{AttachmentId, ChannelId, HeldMessageId, MessageId, UserId};
use aspen_schema::held_message;
use chrono::{TimeDelta, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Bool, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

/// The credit an item is given each turn, in thousandths of mana: about one ordinary message.
pub const QUANTUM: i64 = 100 * MILLI;
/// The most messages one author's item may hold.
pub const MAX_ITEM_MESSAGES: i64 = 50;
/// The most mana one author's item may cost in all; its first message may cost more, which a
/// full pool would have let through.
pub const MAX_ITEM_MANA: i64 = 2_000;
/// The most items one pool's queue may hold.
pub const MAX_ITEMS: i64 = 1_000;
/// How long a message may wait before it is dropped and its author told.
pub const MAX_WAIT: TimeDelta = TimeDelta::minutes(2);
/// How long a drainer's step ticks before handing its job back, so its lease renews and another
/// runner may take over.
const STEP: Duration = Duration::from_secs(10);
/// How many messages past `MAX_WAIT` one tick drops.
const EXPIRE_BATCH: i64 = 100;
/// How long a refused sender is asked to wait (`channelBusy`).
pub const BUSY_RETRY: Duration = Duration::from_secs(10);

#[derive(QueryableByName)]
struct Exists {
    #[diesel(sql_type = Bool)]
    exists: bool,
}

/// Whether anything waits for `channel`'s mana, through `held_message_mana_queue`.
pub async fn waiting_in(conn: &mut AsyncPgConnection, channel: ChannelId) -> crate::Result<bool> {
    let found: Exists = diesel::sql_query(
        "SELECT EXISTS (SELECT 1 FROM held_message WHERE channel = $1 AND waiting = 'mana') \
         AS exists",
    )
    .bind::<PgUuid, _>(channel)
    .get_result(conn)
    .await?;
    Ok(found.exists)
}

/// Something to wait for `channel`'s mana: its author, what it is, and what it costs.
pub struct Waiting {
    pub author: UserId,
    pub channel: ChannelId,
    pub kind: HeldKind,
    pub content: String,
    pub attachments: Vec<AttachmentId>,
    pub echo_to_parent: bool,
    pub target: Option<MessageId>,
    pub payload: Option<serde_json::Value>,
    pub cost: u32,
}

#[derive(QueryableByName)]
struct ItemSize {
    #[diesel(sql_type = BigInt)]
    messages: i64,
    #[diesel(sql_type = BigInt)]
    mana: i64,
}

#[derive(QueryableByName)]
struct Count {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

/// Puts `waiting` in its pool's queue, in its author's item, and sees that a drainer runs: the
/// held row and the drainer's job are saved together. Refused with `channelBusy` when the item
/// or the queue is full.
pub async fn wait(state: &GlobalServerContext, waiting: Waiting) -> crate::Result<HeldMessage> {
    let mut conn = state.connection_pool.get().await?;
    let locale = crate::locale::current().to_string();
    let held = conn
        .transaction::<_, crate::Error, _>(|conn| {
            async move {
                let item: ItemSize = diesel::sql_query(
                    "SELECT count(*) AS messages, COALESCE(sum(cost), 0)::bigint AS mana \
                     FROM held_message WHERE channel = $1 AND author = $2 AND waiting = 'mana'",
                )
                .bind::<PgUuid, _>(waiting.channel)
                .bind::<PgUuid, _>(waiting.author)
                .get_result(conn.as_mut())
                .await?;
                if item.messages >= MAX_ITEM_MESSAGES
                    || (item.messages > 0 && item.mana + i64::from(waiting.cost) > MAX_ITEM_MANA)
                {
                    return Err(crate::Error::ChannelBusy(t!(
                        "channelBusyItem",
                        messages = MAX_ITEM_MESSAGES,
                        mana = MAX_ITEM_MANA
                    )));
                }
                if item.messages == 0 {
                    let items: Count = diesel::sql_query(
                        "SELECT count(*) AS count FROM (SELECT DISTINCT author FROM held_message \
                         WHERE channel = $1 AND waiting = 'mana' LIMIT $2) items",
                    )
                    .bind::<PgUuid, _>(waiting.channel)
                    .bind::<BigInt, _>(MAX_ITEMS)
                    .get_result(conn.as_mut())
                    .await?;
                    if items.count >= MAX_ITEMS {
                        return Err(crate::Error::ChannelBusy(t!(
                            "channelBusyQueue",
                            items = MAX_ITEMS
                        )));
                    }
                }
                let held = HeldMessage {
                    id: HeldMessageId::new(),
                    author: waiting.author,
                    channel: waiting.channel,
                    content: waiting.content,
                    attachments: waiting.attachments,
                    echo_to_parent: waiting.echo_to_parent,
                    locale,
                    held_at: Utc::now(),
                    waiting: HeldWaiting::Mana,
                    kind: waiting.kind,
                    target: waiting.target,
                    payload: waiting.payload,
                    cost: i32::try_from(waiting.cost).unwrap_or(i32::MAX),
                    credit: 0,
                };
                diesel::insert_into(held_message::table)
                    .values((
                        held_message::id.eq(held.id),
                        held_message::author.eq(held.author),
                        held_message::channel.eq(held.channel),
                        held_message::content.eq(&held.content),
                        held_message::attachments.eq(held
                            .attachments
                            .iter()
                            .copied()
                            .map(Some)
                            .collect::<Vec<_>>()),
                        held_message::echo_to_parent.eq(held.echo_to_parent),
                        held_message::locale.eq(&held.locale),
                        held_message::held_at.eq(held.held_at),
                        held_message::waiting.eq(held.waiting),
                        held_message::kind.eq(held.kind),
                        held_message::target.eq(held.target),
                        held_message::payload.eq(&held.payload),
                        held_message::cost.eq(held.cost),
                    ))
                    .execute(conn.as_mut())
                    .await?;
                ensure_drainer(conn.as_mut(), held.channel).await?;
                Ok(held)
            }
            .scope_boxed()
        })
        .await?;
    drop(conn);
    jobs::wake(state).await;
    Ok(held)
}

/// Turns a message held for its previews, released, into one waiting for its channel's mana,
/// in its author's item, costing `cost`. It was accepted when sent, so no bound refuses it.
pub async fn wait_held(
    state: &GlobalServerContext,
    id: HeldMessageId,
    channel: ChannelId,
    cost: u32,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction::<_, crate::Error, _>(|conn| {
        async move {
            diesel::update(held_message::table.filter(held_message::id.eq(id)))
                .set((
                    held_message::waiting.eq(HeldWaiting::Mana),
                    held_message::cost.eq(i32::try_from(cost).unwrap_or(i32::MAX)),
                ))
                .execute(conn.as_mut())
                .await?;
            ensure_drainer(conn.as_mut(), channel).await
        }
        .scope_boxed()
    })
    .await?;
    drop(conn);
    jobs::wake(state).await;
    Ok(())
}

/// Saves the drainer of `channel`'s pool unless it has one. A drainer that finds the queue
/// empty deletes its job only while nothing waits ([`finish_if_empty`]), so a row saved as it
/// finishes has this make it a new one.
async fn ensure_drainer(conn: &mut AsyncPgConnection, channel: ChannelId) -> crate::Result<()> {
    jobs::enqueue(
        conn,
        NewJob::new(JobKind::DrainManaQueue, JobClass::Interactive, &())?
            .keyed(channel.0.to_string()),
    )
    .await?;
    Ok(())
}

/// Deletes the drainer's own job when nothing waits in its pool, in one statement, so a row
/// saved meanwhile either is seen here or finds the job gone and saves another.
async fn finish_if_empty(
    conn: &mut AsyncPgConnection,
    job: uuid::Uuid,
    channel: ChannelId,
) -> crate::Result<bool> {
    let deleted = diesel::sql_query(
        "DELETE FROM job WHERE id = $1 AND NOT EXISTS (SELECT 1 FROM held_message \
         WHERE channel = $2 AND waiting = 'mana')",
    )
    .bind::<PgUuid, _>(job)
    .bind::<PgUuid, _>(channel)
    .execute(conn)
    .await?;
    Ok(deleted == 1)
}

/// Where the pointer stands: the author whose item has the turn, and whether that turn's
/// quantum has been given. The drainer's checkpoint.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Turn {
    at: Option<UserId>,
    granted: bool,
}

/// Each item's oldest row, which holds its credit, in the order the items began waiting.
async fn items(
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
) -> crate::Result<Vec<HeldMessage>> {
    let mut heads: Vec<HeldMessage> = diesel::sql_query(format!(
        "SELECT DISTINCT ON (author) {COLUMNS} FROM held_message \
         WHERE channel = $1 AND waiting = 'mana' ORDER BY author, held_at, id"
    ))
    .bind::<PgUuid, _>(channel)
    .load(conn)
    .await?;
    heads.sort_by_key(|head| (head.held_at, head.id));
    Ok(heads)
}

/// The oldest of `author`'s waiting rows in `channel`.
async fn oldest(
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
    author: UserId,
) -> crate::Result<Option<HeldMessage>> {
    Ok(diesel::sql_query(format!(
        "SELECT {COLUMNS} FROM held_message WHERE channel = $1 AND author = $2 \
         AND waiting = 'mana' ORDER BY held_at, id LIMIT 1"
    ))
    .bind::<PgUuid, _>(channel)
    .bind::<PgUuid, _>(author)
    .get_result(conn)
    .await
    .optional()?)
}

/// Drops what has waited past [`MAX_WAIT`], a batch a tick, telling each author.
async fn expire(state: &GlobalServerContext, channel: ChannelId) -> crate::Result<()> {
    let old: Vec<HeldMessage> = {
        let mut conn = state.connection_pool.get().await?;
        diesel::sql_query(format!(
            "SELECT {COLUMNS} FROM held_message WHERE channel = $1 AND waiting = 'mana' \
             AND held_at < now() - make_interval(secs => $2) ORDER BY held_at, id LIMIT $3"
        ))
        .bind::<PgUuid, _>(channel)
        .bind::<diesel::sql_types::Double, _>(MAX_WAIT.as_seconds_f64())
        .bind::<BigInt, _>(EXPIRE_BATCH)
        .load(conn.as_mut())
        .await?
    };
    for row in old {
        let locale = crate::locale::negotiate(&row.locale);
        let reason = crate::locale::scope(locale, async {
            t!("manaWaitedTooLong", minutes = MAX_WAIT.num_minutes())
        })
        .await;
        held::drop_held(state, row.id, row.author, row.channel, reason).await?;
    }
    Ok(())
}

/// Saves each item's credit on its oldest row, in one statement, as the step hands over.
async fn save_credits(
    conn: &mut AsyncPgConnection,
    credits: &HashMap<HeldMessageId, i64>,
) -> crate::Result<()> {
    if credits.is_empty() {
        return Ok(());
    }
    let (ids, amounts): (Vec<HeldMessageId>, Vec<i64>) =
        credits.iter().map(|(id, credit)| (*id, *credit)).unzip();
    diesel::sql_query(
        "UPDATE held_message SET credit = given.credit \
         FROM unnest($1::uuid[], $2::bigint[]) AS given(id, credit) \
         WHERE held_message.id = given.id",
    )
    .bind::<Array<PgUuid>, _>(ids)
    .bind::<Array<BigInt>, _>(amounts)
    .execute(conn)
    .await?;
    Ok(())
}

/// One step of a pool's drainer: ticks for [`STEP`], then hands the job back with where its
/// pointer stands; done when nothing waits.
pub async fn drain_step(state: &GlobalServerContext, job: &Claimed) -> crate::Result<Outcome> {
    let Some(channel) = job
        .key
        .as_deref()
        .and_then(|key| key.parse().ok())
        .map(ChannelId)
    else {
        return Ok(Outcome::Done);
    };
    let mut turn: Turn = job.progress()?.unwrap_or_default();
    let until = tokio::time::Instant::now() + STEP;
    let mut ticks = tokio::time::interval(TICK);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Each item's credit since the step began, by its oldest row; an item not yet in it has
    // what its row holds.
    let mut credits: HashMap<HeldMessageId, i64> = HashMap::new();
    loop {
        ticks.tick().await;
        expire(state, channel).await?;
        let (pool, heads) = {
            let mut conn = state.connection_pool.get().await?;
            let heads = items(conn.as_mut(), channel).await?;
            if heads.is_empty() {
                save_credits(conn.as_mut(), &credits).await?;
                if finish_if_empty(conn.as_mut(), job.id, channel).await? {
                    return Ok(Outcome::Done);
                }
                continue;
            }
            // `None` when the channel can no longer have a pool: what waits there is posted
            // without paying, as it would be anywhere else.
            (super::pool_of(state, conn.as_mut(), channel).await?, heads)
        };
        tick(
            state,
            channel,
            pool.as_ref(),
            &heads,
            &mut turn,
            &mut credits,
        )
        .await?;
        if tokio::time::Instant::now() >= until {
            let mut conn = state.connection_pool.get().await?;
            save_credits(conn.as_mut(), &credits).await?;
            return Ok(Outcome::Progress(serde_json::to_value(&turn)?));
        }
    }
}

/// One tick: spends what the pool holds on the items, by deficit round-robin from the pointer,
/// for at most one trip round them.
async fn tick(
    state: &GlobalServerContext,
    channel: ChannelId,
    pool: Option<&Pool>,
    heads: &[HeldMessage],
    turn: &mut Turn,
    credits: &mut HashMap<HeldMessageId, i64>,
) -> crate::Result<()> {
    let start = turn
        .at
        .and_then(|at| heads.iter().position(|head| head.author == at))
        .unwrap_or(0);
    for offset in 0..heads.len() {
        let head = &heads[(start + offset) % heads.len()];
        let author = head.author;
        if turn.at != Some(author) {
            *turn = Turn {
                at: Some(author),
                granted: false,
            };
        }
        let mut next = head.clone();
        let mut credit = credits.remove(&next.id).unwrap_or(head.credit);
        if !turn.granted {
            credit = credit.saturating_add(QUANTUM);
            turn.granted = true;
        }
        loop {
            let cost = i64::from(next.cost.max(0)) * MILLI;
            if credit < cost {
                credits.insert(next.id, credit);
                break;
            }
            let paid = match pool {
                Some(pool) => {
                    super::pay_waiting(state, pool, u32::try_from(next.cost.max(0)).unwrap_or(0))
                        .await
                }
                None => true,
            };
            if !paid {
                credits.insert(next.id, credit);
                return Ok(());
            }
            credit -= cost;
            held::post_waiting(state, next).await;
            let mut conn = state.connection_pool.get().await?;
            match oldest(conn.as_mut(), channel, author).await? {
                Some(row) => next = row,
                None => break,
            }
        }
        // The turn passes to the next item, whose quantum is yet to be given.
        turn.granted = false;
        turn.at = heads
            .get((start + offset + 1) % heads.len())
            .map(|head| head.author);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_turn_survives_its_checkpoint() {
        let turn = Turn {
            at: Some(UserId(uuid::Uuid::from_u128(7))),
            granted: true,
        };
        let saved = serde_json::to_value(&turn).unwrap();
        assert_eq!(serde_json::from_value::<Turn>(saved).unwrap(), turn);
    }
}
