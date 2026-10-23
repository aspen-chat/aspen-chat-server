//! What waits for a channel's mana: messages, slash commands, edits, polls, and echoes a busy
//! channel could not pay for when they were sent, kept as held messages
//! (`app::message::held`, `waiting = 'mana'`) and posted as the channel recharges.
//!
//! An author's waiting rows in one pool are one item; its messages post in the order sent.
//! Items go by their authors' [`Priority`]: the higher ranked in the channel's community first,
//! then the longer there in whole days, and among equals one drawn at random for each message.
//! One job per busy pool drains it ([`drain_step`], `drainManaQueue`, keyed by the channel, so
//! one runner holds it anywhere): it ticks at 5 Hz for [`STEP`] at a time, and posts the drawn
//! author's oldest message whenever the pool can pay for it. What has waited [`MAX_WAIT`] is
//! dropped, and its author told; a full queue makes room for a higher author by dropping the
//! lowest one's ([`make_room`]).

use super::{Pool, TICK};
use crate::context::GlobalServerContext;
use crate::jobs::{self, Claimed, JobClass, JobKind, NewJob, Outcome};
use crate::message::held::{self, COLUMNS, HeldKind, HeldMessage, HeldWaiting};
use crate::permissions::OWNER_RANK;
use crate::t;
use crate::{AttachmentId, ChannelId, HeldMessageId, MessageId, UserId};
use aspen_schema::held_message;
use chrono::{TimeDelta, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Bool, Integer, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

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
    /// The message a message replies to (`crate::message::reply`), checked as it was sent.
    pub reply: Option<crate::message::reply::Reply>,
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
/// is full, or the queue is and its author ranks no higher than anyone in it.
pub async fn wait(state: &GlobalServerContext, waiting: Waiting) -> crate::Result<HeldMessage> {
    make_room(state, waiting.channel, waiting.author).await?;
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
                    reply_to: waiting.reply.map(|reply| reply.to),
                    tags_replied: waiting.reply.is_some_and(|reply| reply.tag),
                    locale,
                    held_at: Utc::now(),
                    waiting: HeldWaiting::Mana,
                    kind: waiting.kind,
                    target: waiting.target,
                    payload: waiting.payload,
                    cost: i32::try_from(waiting.cost).unwrap_or(i32::MAX),
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
                        held_message::reply_to.eq(held.reply_to),
                        held_message::tags_replied.eq(held.tags_replied),
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
    tell_level(state, held.channel).await?;
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
    tell_level(state, channel).await
}

/// Tells viewers of `channel` its level and that its priority queue is in force or no longer,
/// as something begins waiting there or the queue empties: then nothing may have been spent,
/// as when a long message waits for the pool to recharge enough to pay for it, and spending is
/// what tells them otherwise.
async fn tell_level(state: &GlobalServerContext, channel: ChannelId) -> crate::Result<()> {
    let pool = {
        let mut conn = state.connection_pool.get().await?;
        super::pool_of(state, conn.as_mut(), channel).await?
    };
    if let Some(pool) = pool
        && let Some(level) = super::level(state, &pool).await
    {
        super::live::changed(state, &pool, level).await;
    }
    Ok(())
}

#[derive(QueryableByName)]
struct Author {
    #[diesel(sql_type = PgUuid)]
    author: UserId,
}

#[derive(QueryableByName)]
struct Evicted {
    #[diesel(sql_type = PgUuid)]
    id: HeldMessageId,
    #[diesel(sql_type = diesel::sql_types::Text)]
    locale: String,
}

/// When `channel`'s queue holds [`MAX_ITEMS`] authors and `author` is not among them, drops
/// every waiting row of the lowest of them (one drawn at random among equals), telling them
/// why, if `author` is higher; refuses with `channelBusy` if not. A raid of more accounts than
/// the queue holds so cannot keep out those who rank above them. The check is made before the
/// row is saved, apart from its transaction (dropping announces each row, on connections of
/// its own); two newcomers at once may both find room, and the queue then holds a few more.
async fn make_room(
    state: &GlobalServerContext,
    channel: ChannelId,
    author: UserId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let authors: Vec<UserId> = diesel::sql_query(
        "SELECT DISTINCT author FROM held_message WHERE channel = $1 AND waiting = 'mana' \
         LIMIT $2",
    )
    .bind::<PgUuid, _>(channel)
    .bind::<BigInt, _>(MAX_ITEMS)
    .load::<Author>(conn.as_mut())
    .await?
    .into_iter()
    .map(|row| row.author)
    .collect();
    if (authors.len() as i64) < MAX_ITEMS || authors.contains(&author) {
        return Ok(());
    }
    let mut asked = authors.clone();
    asked.push(author);
    let priorities = priorities_of(conn.as_mut(), channel, &asked).await?;
    let priority = |of: &UserId| priorities.get(of).copied().unwrap_or_default();
    let lowest = authors.iter().map(priority).min().unwrap_or_default();
    if !priority(&author).may_displace(lowest) {
        return Err(crate::Error::ChannelBusy(t!(
            "channelBusyQueue",
            items = MAX_ITEMS
        )));
    }
    let candidates: Vec<UserId> = authors
        .into_iter()
        .filter(|candidate| priority(candidate) == lowest)
        .collect();
    let victim = candidates[rand::random_range(0..candidates.len())];
    let rows: Vec<Evicted> = diesel::sql_query(
        "SELECT id, locale FROM held_message WHERE channel = $1 AND author = $2 \
         AND waiting = 'mana'",
    )
    .bind::<PgUuid, _>(channel)
    .bind::<PgUuid, _>(victim)
    .load(conn.as_mut())
    .await?;
    drop(conn);
    for row in rows {
        let reason = crate::locale::scope(crate::locale::negotiate(&row.locale), async {
            t!("manaMadeRoom")
        })
        .await;
        held::drop_held(state, row.id, victim, channel, reason).await?;
    }
    Ok(())
}

/// Where an author stands in a pool's queue, higher first: their rank in the channel's
/// community as `CommunityAccess::role_rank` gives it (the owner above every role, otherwise
/// their highest role's position, 0 with the everyone role alone; 0 in a group DM), then how
/// many whole days they have belonged to the community or the group DM. Equals are drawn at
/// random.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Priority {
    rank: i32,
    days: i64,
}

impl Priority {
    /// Whether an author of this priority may have `lowest`'s waiting rows dropped to make room:
    /// only above it, so never anyone of higher rank, nor an equal.
    fn may_displace(self, lowest: Priority) -> bool {
        self > lowest
    }
}

#[derive(QueryableByName)]
struct AuthorPriority {
    #[diesel(sql_type = PgUuid)]
    author: UserId,
    #[diesel(sql_type = Integer)]
    rank: i32,
    #[diesel(sql_type = BigInt)]
    days: i64,
}

/// The [`Priority`] of each of `authors` in `channel`'s pool, in one query: a thread's is in
/// its parent's community or group DM. An author who no longer belongs there is lowest.
async fn priorities_of(
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
    authors: &[UserId],
) -> crate::Result<HashMap<UserId, Priority>> {
    let rows: Vec<AuthorPriority> = diesel::sql_query(
        r#"
        WITH home AS (
            SELECT COALESCE(c.community, parent.community) AS community,
                   COALESCE(c.parent_channel, c.id) AS root
            FROM channel c LEFT JOIN channel parent ON parent.id = c.parent_channel
            WHERE c.id = $1
        )
        SELECT a.author,
               CASE WHEN co.owner = a.author THEN $3
                    ELSE COALESCE((
                        SELECT max(r.position) FROM community_member_role mr
                        JOIN community_role r ON r.id = mr.role AND r.deleted_at IS NULL
                        WHERE mr.community = home.community AND mr."user" = a.author
                    ), 0)
               END AS rank,
               COALESCE(floor(extract(epoch FROM now() - COALESCE(cu.joined_at, dr.joined_at))
                              / 86400), 0)::bigint AS days
        FROM unnest($2::uuid[]) AS a(author)
        CROSS JOIN home
        LEFT JOIN community co ON co.id = home.community
        LEFT JOIN community_user cu ON cu.community = home.community AND cu."user" = a.author
        LEFT JOIN dm_recipient dr
               ON home.community IS NULL AND dr.channel = home.root AND dr."user" = a.author
        "#,
    )
    .bind::<PgUuid, _>(channel)
    .bind::<Array<PgUuid>, _>(authors)
    .bind::<Integer, _>(OWNER_RANK)
    .load(conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.author,
                Priority {
                    rank: row.rank,
                    days: row.days,
                },
            )
        })
        .collect())
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

/// The author whose oldest row has the next turn, drawn among the highest waiting, and kept until
/// that row is paid for: drawn afresh each tick, cheap messages would be posted whenever the pool
/// could pay for them and a costly one never. The drainer's checkpoint.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Drawn {
    author: Option<UserId>,
}

/// Each author's oldest waiting row in `channel`.
async fn items(
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
) -> crate::Result<Vec<HeldMessage>> {
    Ok(diesel::sql_query(format!(
        "SELECT DISTINCT ON (author) {COLUMNS} FROM held_message \
         WHERE channel = $1 AND waiting = 'mana' ORDER BY author, held_at, id"
    ))
    .bind::<PgUuid, _>(channel)
    .load(conn)
    .await?)
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

/// One step of a pool's drainer: ticks for [`STEP`], then hands the job back with whom it drew;
/// done when nothing waits. Authors' priorities are read once a step as they first wait in it,
/// so a role given or taken counts from the next step.
pub async fn drain_step(state: &GlobalServerContext, job: &Claimed) -> crate::Result<Outcome> {
    let Some(channel) = job
        .key
        .as_deref()
        .and_then(|key| key.parse().ok())
        .map(ChannelId)
    else {
        return Ok(Outcome::Done);
    };
    let mut drawn: Drawn = job.progress()?.unwrap_or_default();
    let until = tokio::time::Instant::now() + STEP;
    let mut ticks = tokio::time::interval(TICK);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut priorities: HashMap<UserId, Priority> = HashMap::new();
    loop {
        ticks.tick().await;
        expire(state, channel).await?;
        let (pool, heads) = {
            let mut conn = state.connection_pool.get().await?;
            let heads = items(conn.as_mut(), channel).await?;
            if heads.is_empty() {
                if finish_if_empty(conn.as_mut(), job.id, channel).await? {
                    drop(conn);
                    tell_level(state, channel).await?;
                    return Ok(Outcome::Done);
                }
                continue;
            }
            let unknown: Vec<UserId> = heads
                .iter()
                .map(|head| head.author)
                .filter(|author| !priorities.contains_key(author))
                .collect();
            if !unknown.is_empty() {
                priorities.extend(priorities_of(conn.as_mut(), channel, &unknown).await?);
            }
            // `None` when the channel can no longer have a pool: what waits there is posted
            // without paying, as it would be anywhere else.
            (super::pool_of(state, conn.as_mut(), channel).await?, heads)
        };
        tick(
            state,
            channel,
            pool.as_ref(),
            heads,
            &priorities,
            &mut drawn,
        )
        .await?;
        if tokio::time::Instant::now() >= until {
            return Ok(Outcome::Progress(serde_json::to_value(&drawn)?));
        }
    }
}

/// One tick: while the pool can pay, posts the oldest row of an author drawn among the highest
/// waiting, drawing again after each.
async fn tick(
    state: &GlobalServerContext,
    channel: ChannelId,
    pool: Option<&Pool>,
    mut heads: Vec<HeldMessage>,
    priorities: &HashMap<UserId, Priority>,
    drawn: &mut Drawn,
) -> crate::Result<()> {
    let priority = |author: UserId| priorities.get(&author).copied().unwrap_or_default();
    while let Some(top) = heads.iter().map(|head| priority(head.author)).max() {
        let at = match drawn
            .author
            .and_then(|author| heads.iter().position(|head| head.author == author))
            .filter(|at| priority(heads[*at].author) == top)
        {
            Some(at) => at,
            None => {
                let tier: Vec<usize> = (0..heads.len())
                    .filter(|at| priority(heads[*at].author) == top)
                    .collect();
                let at = tier[rand::random_range(0..tier.len())];
                drawn.author = Some(heads[at].author);
                at
            }
        };
        let head = heads[at].clone();
        let (author, posted) = (head.author, head.id);
        // What its author can no longer post would be refused once paid for: it goes unpaid.
        if held::drop_if_unpostable(state, &head).await? {
            drawn.author = None;
            let mut conn = state.connection_pool.get().await?;
            match oldest(conn.as_mut(), channel, author).await? {
                Some(row) => heads[at] = row,
                None => {
                    heads.swap_remove(at);
                }
            }
            continue;
        }
        let paid = match pool {
            Some(pool) => {
                super::pay_waiting(
                    state,
                    pool,
                    author,
                    u32::try_from(head.cost.max(0)).unwrap_or(0),
                )
                .await
            }
            None => true,
        };
        if !paid {
            return Ok(());
        }
        held::post_waiting(state, head).await;
        drawn.author = None;
        let mut conn = state.connection_pool.get().await?;
        match oldest(conn.as_mut(), channel, author).await? {
            // Left to wait by a failure that may pass: tried again next tick, not spent on now.
            Some(row) if row.id == posted => return Ok(()),
            Some(row) => heads[at] = row,
            None => {
                heads.swap_remove(at);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draw_survives_its_checkpoint() {
        let drawn = Drawn {
            author: Some(UserId(uuid::Uuid::from_u128(7))),
        };
        let saved = serde_json::to_value(&drawn).unwrap();
        assert_eq!(serde_json::from_value::<Drawn>(saved).unwrap(), drawn);
    }

    #[test]
    fn rank_comes_before_tenure() {
        let moderator = Priority { rank: 3, days: 0 };
        let veteran = Priority { rank: 0, days: 900 };
        let newcomer = Priority { rank: 0, days: 0 };
        assert!(moderator > veteran && veteran > newcomer);
        assert!(
            Priority {
                rank: OWNER_RANK,
                days: 0
            } > moderator
        );
    }

    #[test]
    fn no_one_displaces_a_higher_rank_or_an_equal() {
        let veteran = Priority { rank: 0, days: 900 };
        let moderator = Priority { rank: 3, days: 0 };
        assert!(!veteran.may_displace(moderator));
        assert!(!veteran.may_displace(veteran));
        assert!(moderator.may_displace(veteran));
        assert!(veteran.may_displace(Priority { rank: 0, days: 2 }));
    }
}
