//! What waits for a channel's mana: messages, slash commands, edits, polls, and echoes a busy
//! channel could not pay for when they were sent, kept as held messages
//! (`app::message::held`, `waiting = 'mana'`) and posted as the channel recharges.
//!
//! An author's waiting rows in one pool are one item; its messages post in the order sent.
//! Items go by their authors' [`Standing`]: the higher ranked in the channel's community first,
//! then the longer there in whole days, and among equals by a keyed hash of the channel, the
//! author, and their username (`server_secret::MANA_ORDER`), lowest first. While anything waits,
//! what the pool earns is split into three portions credited to the first three authors in that
//! order (`mana::share`): one waiting takes all three, two take two and one. An author's oldest
//! row posts once their credit covers it (`mana::pay_turn`); credit left over stays theirs while
//! they have rows waiting and while they are among the first three, and otherwise goes back to the
//! pool. One job per busy pool drains it ([`drain_step`], `drainManaQueue`, keyed by the
//! channel, so one runner holds it anywhere): it ticks at 5 Hz for [`STEP`] at a time. What has
//! waited [`MAX_WAIT`] is dropped, and its author told; a full queue makes room for a higher
//! author by dropping the last one's ([`make_room`]).

use super::{Pool, TICK, Turn};
use crate::context::GlobalServerContext;
use crate::jobs::{self, Claimed, JobClass, JobKind, NewJob, Outcome};
use crate::message::held::{self, COLUMNS, HeldKind, HeldMessage, HeldWaiting};
use crate::permissions::OWNER_RANK;
use crate::server_secret::HmacKey;
use crate::t;
use crate::{AttachmentId, ChannelId, HeldMessageId, MessageId, UserId};
use aspen_schema::held_message;
use chrono::{TimeDelta, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Bool, Integer, Text, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::cmp::Ordering;
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
/// How many times one tick shares the pool out and pays what its credit covers: each round posts
/// at most one row of each of the first three authors.
const ROUNDS_PER_TICK: usize = 10;
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
                check_bounds(conn.as_mut(), waiting.channel, waiting.author, waiting.cost).await?;
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

/// Refuses with `channelBusy` something of `author`'s costing `cost` to wait in `channel`'s pool
/// when their item is full, or when they have none and the queue holds [`MAX_ITEMS`].
async fn check_bounds(
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
    author: UserId,
    cost: u32,
) -> crate::Result<()> {
    let item: ItemSize = diesel::sql_query(
        "SELECT count(*) AS messages, COALESCE(sum(cost), 0)::bigint AS mana \
         FROM held_message WHERE channel = $1 AND author = $2 AND waiting = 'mana'",
    )
    .bind::<PgUuid, _>(channel)
    .bind::<PgUuid, _>(author)
    .get_result(&mut *conn)
    .await?;
    if item.messages >= MAX_ITEM_MESSAGES
        || (item.messages > 0 && item.mana + i64::from(cost) > MAX_ITEM_MANA)
    {
        return Err(crate::Error::ChannelBusy(t!(
            "channelBusyItem",
            messages = MAX_ITEM_MESSAGES,
            mana = MAX_ITEM_MANA
        )));
    }
    if item.messages == 0 {
        let items: Count = diesel::sql_query(format!(
            "SELECT count(*) AS count FROM ({AUTHORS}) items WHERE author IS NOT NULL"
        ))
        .bind::<PgUuid, _>(channel)
        .bind::<BigInt, _>(MAX_ITEMS)
        .get_result(conn)
        .await?;
        if items.count >= MAX_ITEMS {
            return Err(crate::Error::ChannelBusy(t!(
                "channelBusyQueue",
                items = MAX_ITEMS
            )));
        }
    }
    Ok(())
}

/// The first `$2` authors waiting in pool `$1`, one row each read through
/// `held_message_mana_queue` by skipping from one author to the next, however many rows each
/// has waiting; the last row's author is `NULL`.
const AUTHORS: &str = r#"
    WITH RECURSIVE authors(author, n) AS (
        (SELECT author, 1 FROM held_message WHERE channel = $1 AND waiting = 'mana'
         ORDER BY author LIMIT 1)
        UNION ALL
        SELECT (SELECT h.author FROM held_message h
                WHERE h.channel = $1 AND h.waiting = 'mana' AND h.author > a.author
                ORDER BY h.author LIMIT 1),
               a.n + 1
        FROM authors a WHERE a.author IS NOT NULL AND a.n < $2
    )
    SELECT author FROM authors
"#;

/// Turns a message held for its previews, released, into one waiting for its channel's mana,
/// in its author's item, costing `cost`, within the bounds of what may wait ([`make_room`],
/// [`check_bounds`]): one past them is dropped, its author told the channel is too busy.
pub async fn wait_held(
    state: &GlobalServerContext,
    id: HeldMessageId,
    author: UserId,
    channel: ChannelId,
    cost: u32,
) -> crate::Result<()> {
    let bounded = match make_room(state, channel, author).await {
        Ok(()) => {
            let mut conn = state.connection_pool.get().await?;
            conn.transaction::<_, crate::Error, _>(|conn| {
                async move {
                    check_bounds(conn.as_mut(), channel, author, cost).await?;
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
            .await
        }
        Err(error) => Err(error),
    };
    match bounded {
        Ok(()) => {}
        Err(crate::Error::ChannelBusy(reason)) => {
            return held::drop_held(state, id, author, channel, reason).await;
        }
        Err(error) => return Err(error),
    }
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
    #[diesel(sql_type = diesel::sql_types::Nullable<PgUuid>)]
    author: Option<UserId>,
}

#[derive(QueryableByName)]
struct Evicted {
    #[diesel(sql_type = PgUuid)]
    id: HeldMessageId,
    #[diesel(sql_type = diesel::sql_types::Text)]
    locale: String,
}

/// When `channel`'s queue holds [`MAX_ITEMS`] authors and `author` is not among them, drops
/// every waiting row of the last of them in queue order, telling them why, if `author` is of
/// higher priority; refuses with `channelBusy` if not. A raid of more accounts than
/// the queue holds so cannot keep out those who rank above them. The check is made before the
/// row is saved, apart from its transaction (dropping announces each row, on connections of
/// its own); two newcomers at once may both find room, and the queue then holds a few more.
async fn make_room(
    state: &GlobalServerContext,
    channel: ChannelId,
    author: UserId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let authors: Vec<UserId> = diesel::sql_query(AUTHORS)
        .bind::<PgUuid, _>(channel)
        .bind::<BigInt, _>(MAX_ITEMS)
        .load::<Author>(conn.as_mut())
        .await?
        .into_iter()
        .filter_map(|row| row.author)
        .collect();
    if (authors.len() as i64) < MAX_ITEMS || authors.contains(&author) {
        return Ok(());
    }
    let mut asked = authors.clone();
    asked.push(author);
    let standings = standings_of(&state.mana_order_key, conn.as_mut(), channel, &asked).await?;
    let standing = |of: &UserId| standings.get(of).copied().unwrap_or_default();
    let Some(victim) = authors.iter().copied().max_by_key(standing) else {
        return Ok(());
    };
    if !standing(&author)
        .priority
        .may_displace(standing(&victim).priority)
    {
        return Err(crate::Error::ChannelBusy(t!(
            "channelBusyQueue",
            items = MAX_ITEMS
        )));
    }
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

/// An author's priority in a pool's queue, higher first: their rank in the channel's
/// community as `CommunityAccess::role_rank` gives it (the owner above every role, otherwise
/// their highest role's position, 0 with the everyone role alone; 0 in a group DM), then how
/// many whole days they have belonged to the community or the group DM. Equals are ordered by
/// their [`Standing`]'s tie.
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

/// Where an author stands in a pool's queue: by [`Priority`], higher first, then by `tie`, lowest
/// first. It orders as the queue does, the first author least.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    priority: Priority,
    /// The HMAC under the deployment's `MANA_ORDER` secret of the channel, the author, and their
    /// username ([`tie`]): an order among equals that is arbitrary, the same on every server and
    /// every tick, and that no one outside the servers can work out, so no one can pick a name
    /// to win it.
    tie: [u8; 32],
}

impl Ord for Standing {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .priority
            .cmp(&self.priority)
            .then_with(|| self.tie.cmp(&other.tie))
    }
}

impl PartialOrd for Standing {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The tie of `author`, named `name`, in `channel`'s queue: HMAC-SHA256 under `key` of the
/// channel's id, the author's, and the name, the ids being of fixed length so the three cannot
/// run together.
fn tie(key: &HmacKey, channel: ChannelId, author: UserId, name: &str) -> [u8; 32] {
    let mut hmac = aws_lc_rs::hmac::Context::with_key(key.key());
    hmac.update(channel.0.as_bytes());
    hmac.update(author.0.as_bytes());
    hmac.update(name.as_bytes());
    let mut tie = [0; 32];
    tie.copy_from_slice(hmac.sign().as_ref());
    tie
}

#[derive(QueryableByName)]
struct AuthorPriority {
    #[diesel(sql_type = PgUuid)]
    author: UserId,
    #[diesel(sql_type = Integer)]
    rank: i32,
    #[diesel(sql_type = BigInt)]
    days: i64,
    #[diesel(sql_type = Text)]
    name: String,
}

/// The [`Standing`] of each of `authors` in `channel`'s pool, in one query, their ties under
/// `key`: a thread's priorities are in its parent's community or group DM. An author who no
/// longer belongs there has the lowest priority.
async fn standings_of(
    key: &HmacKey,
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
    authors: &[UserId],
) -> crate::Result<HashMap<UserId, Standing>> {
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
                              / 86400), 0)::bigint AS days,
               COALESCE(u.name, '') AS name
        FROM unnest($2::uuid[]) AS a(author)
        CROSS JOIN home
        LEFT JOIN "user" u ON u.id = a.author
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
                Standing {
                    priority: Priority {
                        rank: row.rank,
                        days: row.days,
                    },
                    tie: tie(key, channel, row.author, &row.name),
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

/// Each author's oldest waiting row in `channel`, of at most twice [`MAX_ITEMS`] authors (two
/// newcomers at once may both find room in a full queue, [`make_room`]): one row read for each,
/// through `held_message_mana_queue`, however many they have waiting.
async fn items(
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
) -> crate::Result<Vec<HeldMessage>> {
    Ok(diesel::sql_query(format!(
        "SELECT head.* FROM ({AUTHORS}) a CROSS JOIN LATERAL ( \
             SELECT {COLUMNS} FROM held_message \
             WHERE channel = $1 AND author = a.author AND waiting = 'mana' \
             ORDER BY held_at, id LIMIT 1) head \
         WHERE a.author IS NOT NULL"
    ))
    .bind::<PgUuid, _>(channel)
    .bind::<BigInt, _>(MAX_ITEMS * 2)
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

/// Drops what has waited past [`MAX_WAIT`], a batch a tick, telling each author, oldest first
/// through `held_message_mana_by_age`.
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

/// One step of a pool's drainer: ticks for [`STEP`], then hands the job back; done when nothing
/// waits, once what the queue's authors were credited is back in the pool. Authors' standings
/// are read once a step as they first wait in it, so a role given or taken counts from the next
/// step. What its authors were credited lives with the pool in Valkey, so the job carries
/// nothing from one step to the next.
pub async fn drain_step(state: &GlobalServerContext, job: &Claimed) -> crate::Result<Outcome> {
    let Some(channel) = job
        .key
        .as_deref()
        .and_then(|key| key.parse().ok())
        .map(ChannelId)
    else {
        return Ok(Outcome::Done);
    };
    let until = tokio::time::Instant::now() + STEP;
    let mut ticks = tokio::time::interval(TICK);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut standings: HashMap<UserId, Standing> = HashMap::new();
    loop {
        ticks.tick().await;
        expire(state, channel).await?;
        let (pool, heads) = {
            let mut conn = state.connection_pool.get().await?;
            // `None` when the channel can no longer have a pool: what waits there is posted
            // without paying, as it would be anywhere else.
            let pool = super::pool_of(state, conn.as_mut(), channel).await?;
            let heads = items(conn.as_mut(), channel).await?;
            if heads.is_empty() {
                drop(conn);
                if let Some(pool) = &pool {
                    super::share(state, pool, &[]).await;
                }
                let mut conn = state.connection_pool.get().await?;
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
                .filter(|author| !standings.contains_key(author))
                .collect();
            if !unknown.is_empty() {
                standings.extend(
                    standings_of(&state.mana_order_key, conn.as_mut(), channel, &unknown).await?,
                );
            }
            (pool, heads)
        };
        tick(state, channel, pool.as_ref(), heads, &standings).await?;
        if tokio::time::Instant::now() >= until {
            return Ok(Outcome::Progress(serde_json::Value::Null));
        }
    }
}

fn cost_of(row: &HeldMessage) -> u32 {
    u32::try_from(row.cost.max(0)).unwrap_or(0)
}

/// One tick: shares what the pool has earned among the first three authors waiting and posts the
/// oldest row of each whose credit covers it, in up to [`ROUNDS_PER_TICK`] rounds. `heads` are
/// each author's oldest row. With no pool, each author's oldest row is posted unpaid.
async fn tick(
    state: &GlobalServerContext,
    channel: ChannelId,
    pool: Option<&Pool>,
    mut heads: Vec<HeldMessage>,
    standings: &HashMap<UserId, Standing>,
) -> crate::Result<()> {
    let standing = |author: UserId| standings.get(&author).copied().unwrap_or_default();
    heads.sort_by_key(|head| standing(head.author));
    let Some(pool) = pool else {
        for head in heads {
            if !held::drop_if_unpostable(state, &head).await? {
                held::post_waiting(state, head).await;
            }
        }
        return Ok(());
    };
    for _ in 0..ROUNDS_PER_TICK {
        let turns: Vec<Turn> = heads
            .iter()
            .zip(super::weights(heads.len()))
            .map(|(head, weight)| Turn {
                author: head.author,
                cost: cost_of(head),
                weight: *weight,
            })
            .collect();
        if turns.is_empty() {
            return Ok(());
        }
        let payable = super::share(state, pool, &turns).await;
        // Each author whose head changed, and the row posted for them, if one was.
        let mut changed: Vec<(UserId, Option<HeldMessageId>)> = Vec::new();
        for ((turn, head), can) in turns.iter().zip(&heads).zip(payable) {
            if !can {
                continue;
            }
            // What its author can no longer post would be refused once paid for: it goes
            // unpaid.
            if held::drop_if_unpostable(state, head).await? {
                changed.push((turn.author, None));
                continue;
            }
            if !super::pay_turn(state, pool, *turn).await {
                continue;
            }
            held::post_waiting(state, head.clone()).await;
            changed.push((turn.author, Some(head.id)));
        }
        if changed.is_empty() {
            return Ok(());
        }
        let mut stuck = false;
        let mut conn = state.connection_pool.get().await?;
        for (author, posted) in changed {
            let Some(at) = heads.iter().position(|head| head.author == author) else {
                continue;
            };
            match oldest(conn.as_mut(), channel, author).await? {
                // Left to wait by a failure that may pass: tried again next tick.
                Some(row) if Some(row.id) == posted => stuck = true,
                Some(row) => heads[at] = row,
                None => {
                    heads.remove(at);
                }
            }
        }
        if stuck {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing(rank: i32, days: i64, tie: u8) -> Standing {
        Standing {
            priority: Priority { rank, days },
            tie: [tie; 32],
        }
    }

    #[test]
    fn the_queue_goes_by_priority_then_the_lowest_tie() {
        let mut queue = vec![
            standing(0, 0, 9),
            standing(0, 3, 200),
            standing(2, 0, 255),
            standing(0, 0, 1),
        ];
        queue.sort();
        assert_eq!(
            queue,
            vec![
                standing(2, 0, 255),
                standing(0, 3, 200),
                standing(0, 0, 1),
                standing(0, 0, 9),
            ]
        );
    }

    #[test]
    fn a_tie_is_keyed_and_names_its_channel_author_and_name() {
        let key = HmacKey::from_secret(b"one");
        let channel = ChannelId(uuid::Uuid::from_u128(1));
        let author = UserId(uuid::Uuid::from_u128(2));
        let base = tie(&key, channel, author, "raider");
        assert_eq!(base, tie(&key, channel, author, "raider"));
        assert_ne!(
            base,
            tie(&HmacKey::from_secret(b"two"), channel, author, "raider")
        );
        assert_ne!(
            base,
            tie(&key, ChannelId(uuid::Uuid::from_u128(3)), author, "raider")
        );
        assert_ne!(
            base,
            tie(&key, channel, UserId(uuid::Uuid::from_u128(3)), "raider")
        );
        assert_ne!(base, tie(&key, channel, author, "raider2"));
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
