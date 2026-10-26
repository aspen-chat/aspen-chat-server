//! Message search: the messages a user may read that match words, who wrote them, whom they tag,
//! and what they hold, newest first.
//!
//! Words are matched by PostgreSQL full-text search in the `simple` configuration, which
//! lowercases every word and does no more, so every language is searched alike; the query takes
//! the syntax web search engines use (`websearch_to_tsquery`): words all of which must appear,
//! `"a phrase"`, `or`, and `-word` to leave one out. The index `message_search` is on the very
//! expression written here.
//!
//! What may be searched is what the user may read: the channels they may view in the
//! communities they belong to, the DMs they are in, and the threads of both. Messages by anyone
//! they blocked are left out, as they are of their unread counts.
//!
//! No index gives the matches of a word newest first, so a search never asks for them that way.
//! It reads in three parts, each bounded ([`search_messages`]):
//!
//! 1. **The probe**, when there are words: up to [`PROBE_CAP`] matches through `message_search`,
//!    in no order. Fewer than the cap is every match there is, and the page is the newest of
//!    them. The cap says the words are common, and so near the present.
//! 2. **The walk**: history read backwards a window of time at a time ([`Walk`]), each window
//!    through an index that leads with what the search is of (a place, an author, a tagged
//!    user) and keeps the messages that pass. It reads [`FREE_ROWS`] messages for anyone.
//! 3. **Going deep**, when that found nothing: the walk goes on until it finds something, for
//!    up to `[search] round_seconds`, as `app::deep_search` lets it.
//!
//! A page says how far back it read (`searched_to`) and whether anything lies beyond (`more`),
//! so a page may be short, or empty, with more to come: the caller asks again from there.

use crate::context::GlobalServerContext;
use crate::message::MessageKind;
use crate::message::{Message, MessageWithRelations};
use crate::t;
use crate::visibility::Visibility;
use crate::{ChannelId, CommunityId, MessageId, UserId};
use aspen_schema::{community_user, dm_recipient, message};
use diesel::dsl::exists;
use diesel::pg::Pg;
use diesel::prelude::*;
use diesel::query_builder::{BoxedSqlQuery, SqlQuery};
use diesel::sql_types::{Array, BigInt, Bool, Nullable, Text, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl, SimpleAsyncConnection};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::time::Instant;
use uuid::Uuid;

/// The most messages one search returns.
pub const MAX_RESULTS: u32 = 50;
/// The longest search text accepted, in characters.
pub const MAX_TEXT_CHARS: usize = 200;

/// Something a message may hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Holding {
    /// Any attachment.
    Attachment,
    /// An attachment that is a picture.
    Image,
    Poll,
}

#[derive(Debug, Clone, Copy)]
pub enum SearchScope {
    /// Everything the user may read.
    Everywhere,
    /// One community the user belongs to.
    Community(CommunityId),
    /// One channel or DM the user may read, with its threads.
    Channel(ChannelId),
}

/// What to search for. At least one of `text`, `author`, `mentions`, and `holding` narrows it.
#[derive(Debug, Clone)]
pub struct MessageSearch {
    pub text: Option<String>,
    pub scope: SearchScope,
    pub author: Option<UserId>,
    /// Messages tagging this user by name.
    pub mentions: Option<UserId>,
    /// Messages holding every one of these.
    pub holding: Vec<Holding>,
    /// Only messages older than this one.
    pub before: Option<MessageId>,
    pub limit: u32,
}

/// The most matches of its words a search reads through `message_search` before it takes the
/// words for common and walks history instead. Reading them costs a row of the table each.
pub const PROBE_CAP: usize = 2000;
/// How many messages a walk's first step aims to read. Common words fill a page within it.
const FIRST_STEP_ROWS: u64 = 2_500;
/// How many messages a step aims to read at the most: each step that does not end the walk
/// aims for four times the last, up to this, and the window of time is widened or narrowed
/// toward the aim.
const STEP_ROWS: u64 = 20_000;
/// The most messages one step reads. A window found to hold more is read again narrower.
const STEP_ROWS_MAX: u64 = 80_000;
/// How many messages the walk reads for any search, before going on means going deep.
pub const FREE_ROWS: u64 = 40_000;
/// The first window of a walk, in milliseconds: a day.
const FIRST_SPAN_MS: u64 = 24 * 60 * 60 * 1000;
/// How far past this server's clock the first window reaches, for ids another server made.
const CLOCK_SLACK_MS: u64 = 60_000;

/// Whether the client a search is for has gone away, which a request's handler is not told
/// otherwise: it runs to its end whoever is listening (`api::settle_after_request`). A search
/// going deep asks between steps, and stops.
#[derive(Clone, Default)]
pub struct Abandoned(Arc<AtomicBool>);

/// Marks its [`Abandoned`] when dropped.
pub struct Leaving(Abandoned);

impl Abandoned {
    /// A guard to hold for as long as the client is being answered.
    pub fn when_dropped(&self) -> Leaving {
        Leaving(self.clone())
    }

    fn is_so(&self) -> bool {
        AtomicBool::load(&self.0, Ordering::Relaxed)
    }
}

impl Drop for Leaving {
    fn drop(&mut self) {
        self.0.0.store(true, Ordering::Relaxed);
    }
}

/// One page of a search, newest first.
pub struct SearchPage {
    pub messages: Vec<MessageWithRelations>,
    /// How far back the search read: every match newer than this, and older than what was
    /// asked for, is in `messages`. The next page is asked for as older than it. `None` when
    /// nothing is left to read.
    pub searched_to: Option<MessageId>,
    /// Whether anything older than `searched_to` is left to read. A page may be short, or
    /// empty, and still have more.
    pub more: bool,
}

/// What a search is of, as its statements are given it.
struct Sought<'a> {
    caller: Uuid,
    places: &'a [Uuid],
    /// Whether the one place is a thread, whose replies are found by the channel they are in
    /// rather than by their home channel, which is the thread's parent.
    in_thread: bool,
    text: Option<&'a str>,
    author: Option<Uuid>,
    mentions: Option<Uuid>,
    attachment: bool,
    image: bool,
    poll: bool,
    limit: usize,
}

/// Which index a walk reads history through: the one that leads with the narrowest thing the
/// search is of, so a window holds as little as it can that the search is not of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    /// Each place's messages, through `message_by_home_channel`, or a thread's through
    /// `message_channel_id_idx`.
    Places,
    /// One author's messages, through `message_by_author`.
    Author,
    /// The messages tagging one user, through `mention_user_recent`.
    Tagged,
}

impl Sought<'_> {
    fn source(&self) -> Source {
        if self.mentions.is_some() {
            Source::Tagged
        } else if self.author.is_some() {
            Source::Author
        } else {
            Source::Places
        }
    }
}

/// Whether a message `m` is one the search is of, cheapest test first, since the words are
/// read from the message's text there and then. `$1` is the caller, `$5` the kinds left out,
/// `$6` the words, `$7` the author, `$8` the user tagged, and `$9`, `$10`, and `$11` whether an
/// attachment, a picture, and a poll are asked for. A filter not asked for is null or false.
///
/// Whether the message's thread was deleted is asked last, of the few that pass the rest, and
/// as a value looked up for each: written as `NOT EXISTS` it is answered by reading every
/// deleted channel of the deployment once for the statement.
const PASSES: &str = r#"
    m.kind <> ALL($5)
    AND ($7 IS NULL OR m.author = $7)
    AND (NOT $11 OR m.kind = 'poll')
    AND NOT EXISTS (
        SELECT 1 FROM user_block b WHERE b.blocker = $1 AND b.blocked = m.author
    )
    AND ($8 IS NULL OR EXISTS (
        SELECT 1 FROM mention tag WHERE tag.message = m.id AND tag.target_user = $8
    ))
    AND (NOT $9 OR EXISTS (
        SELECT 1 FROM message_attachment held WHERE held.message_id = m.id
    ))
    AND (NOT $10 OR EXISTS (
        SELECT 1 FROM message_attachment held
        JOIN attachment a ON a.id = held.attachment_id
        WHERE held.message_id = m.id AND a.mime_type LIKE 'image/%'
    ))
    AND ($6 IS NULL OR to_tsvector('simple', m.content) @@ websearch_to_tsquery('simple', $6))
    AND (SELECT gone.deleted_at FROM channel gone WHERE gone.id = m.channel) IS NULL
"#;

/// How each statement of a walk is planned: one core, no compiling of a statement that is run
/// once, and its window read through the index that bounds it. The planner takes a window for
/// far wider than it is (it measures a row comparison by its first column alone), and left to
/// itself reads the whole table in passing, since a capped read wants no order.
const WALK_SETTINGS: &str = "SET LOCAL jit = off; \
     SET LOCAL max_parallel_workers_per_gather = 0; \
     SET LOCAL enable_seqscan = off; \
     SET LOCAL enable_bitmapscan = off";

/// How the probe is planned: through `message_search`, the only index that knows the words,
/// with room for its bitmap to name rows rather than pages, which it would then read whole.
const PROBE_SETTINGS: &str = "SET LOCAL jit = off; \
     SET LOCAL max_parallel_workers_per_gather = 0; \
     SET LOCAL enable_seqscan = off; \
     SET LOCAL enable_indexscan = off; \
     SET LOCAL work_mem = '64MB'";

/// The messages of one window of time, `$3` up to but not `$4`, that `source` reads, as `m`.
///
/// A place's window is bounded by a row comparison, which only `message_by_home_channel` can
/// serve: written as the place and a range of ids, the primary key could serve the range, and
/// would read everything the deployment said in that time. An author's or a tagged user's is
/// written as that user and a range of ids, which the planner measures truly; it takes a row
/// comparison for a tenth of the table where the user wrote a tenth of it, and reads every
/// message of the places instead. The places are asked of what the window read (`OFFSET 0`
/// keeps them out of the read) for the same reason.
///
/// The places are home channels, which hold their threads' replies too, or, `in_thread`, one
/// thread, whose replies are read by the channel they are in.
fn window_of(source: Source, in_thread: bool) -> String {
    let place = if in_thread { "channel" } else { "home_channel" };
    match source {
        Source::Places => format!(
            r#"unnest($2::uuid[]) AS p(channel)
            CROSS JOIN LATERAL (
                SELECT r.* FROM message r
                WHERE (r.{place}, r.id) >= (p.channel, $3)
                  AND (r.{place}, r.id) < (p.channel, $4)
                  AND r.deleted_at IS NULL
            ) m"#
        ),
        Source::Author => format!(
            r#"(
                SELECT r.* FROM message r
                WHERE r.author = $7 AND r.id >= $3 AND r.id < $4 AND r.deleted_at IS NULL
                OFFSET 0
            ) m
            WHERE m.{place} = ANY($2)"#
        ),
        Source::Tagged => format!(
            r#"(
                SELECT t.message, t.{place} AS place FROM mention t
                WHERE t.target_user = $8 AND t.message >= $3 AND t.message < $4
                OFFSET 0
            ) mn
            JOIN message m ON m.id = mn.message
            WHERE mn.place = ANY($2) AND m.deleted_at IS NULL"#
        ),
    }
}

/// Reads one window: how many messages it held, and the newest `limit` of them that pass.
/// With `capped`, it reads at most one more than [`STEP_ROWS_MAX`], in no order, so a count
/// past the cap says the window was not read whole and what it found is not to be used.
fn window_sql(source: Source, in_thread: bool, limit: usize, capped: bool) -> String {
    let cap = if capped {
        format!("LIMIT {}", STEP_ROWS_MAX + 1)
    } else {
        String::new()
    };
    format!(
        r#"
        SELECT count(*) AS examined,
               COALESCE(
                   (array_agg(w.id ORDER BY w.id DESC) FILTER (WHERE w.hit))[1:{limit}],
                   '{{}}'::uuid[]
               ) AS found
        FROM (SELECT m.id, ({PASSES}) AS hit FROM {window} {cap}) w
        "#,
        window = window_of(source, in_thread),
    )
}

/// The newest `limit` and one more of the messages `$12` that are older than `$4` and pass.
fn among_sql(limit: usize) -> String {
    format!(
        r#"
        SELECT 0::bigint AS examined,
               COALESCE(array_agg(f.id ORDER BY f.id DESC), '{{}}'::uuid[]) AS found
        FROM (
            SELECT m.id FROM message m
            WHERE m.id = ANY($12) AND m.id < $4 AND m.deleted_at IS NULL AND {PASSES}
            ORDER BY m.id DESC
            LIMIT {more}
        ) f
        "#,
        more = limit + 1,
    )
}

/// Up to one more than [`PROBE_CAP`] messages of the places that match the words, in no order
/// and whatever else is asked of them: only what `message_search` itself decides, so every row
/// read is one counted.
fn probe_sql() -> String {
    format!(
        r#"
        SELECT m.id FROM message m
        WHERE m.deleted_at IS NULL AND m.home_channel = ANY($2)
          AND to_tsvector('simple', m.content) @@ websearch_to_tsquery('simple', $6)
        LIMIT {}
        "#,
        PROBE_CAP + 1
    )
}

#[derive(QueryableByName)]
struct Window {
    #[diesel(sql_type = BigInt)]
    examined: i64,
    #[diesel(sql_type = Array<PgUuid>)]
    found: Vec<Uuid>,
}

#[derive(QueryableByName)]
struct Matched {
    #[diesel(sql_type = PgUuid)]
    id: Uuid,
}

/// `sql` with what every statement of a search is given, in the order [`PASSES`] names:
/// `$2` the places, `$3` and `$4` the window, and `$12` the messages to choose among.
fn bound<'a>(
    sql: String,
    sought: &'a Sought<'a>,
    text: Option<&'a str>,
    window: (Uuid, Uuid),
    among: &'a [Uuid],
) -> BoxedSqlQuery<'a, Pg, SqlQuery> {
    diesel::sql_query(sql)
        .into_boxed::<Pg>()
        .bind::<PgUuid, _>(sought.caller)
        .bind::<Array<PgUuid>, _>(sought.places)
        .bind::<PgUuid, _>(window.0)
        .bind::<PgUuid, _>(window.1)
        .bind::<Array<aspen_schema::sql_types::MessageKind>, _>(vec![
            MessageKind::ThreadEcho,
            MessageKind::PollClosed,
            MessageKind::Call,
            MessageKind::MissedCall,
            MessageKind::MemberJoined,
            MessageKind::CommunityGrew,
        ])
        .bind::<Nullable<Text>, _>(text)
        .bind::<Nullable<PgUuid>, _>(sought.author)
        .bind::<Nullable<PgUuid>, _>(sought.mentions)
        .bind::<Bool, _>(sought.attachment)
        .bind::<Bool, _>(sought.image)
        .bind::<Bool, _>(sought.poll)
        .bind::<Array<PgUuid>, _>(among)
}

/// The millisecond a UUIDv7 was made in, or `None` of an id that is not one.
fn millis_of(id: Uuid) -> Option<u64> {
    let (seconds, nanos) = id.get_timestamp()?.to_unix();
    Some(seconds * 1000 + u64::from(nanos) / 1_000_000)
}

/// The smallest id of a millisecond, as `read_state::position_at` gives a moment's.
fn floor_of(millis: u64) -> Uuid {
    uuid::Builder::from_unix_timestamp_millis(millis, &[0; 10]).into_uuid()
}

/// A walk back through history a window of time at a time, each as wide as the last suggests
/// will hold the messages it aims for. Ids are UUIDv7, so a stretch of time is a range of ids.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Walk {
    /// Everything at or after this has been read.
    upper: Uuid,
    upper_millis: u64,
    span_millis: u64,
    /// How many messages the next step aims to read.
    aim: u64,
    /// The messages read so far, those of windows read again narrower among them.
    examined: u64,
    /// Whether the walk has read back to the beginning.
    done: bool,
}

impl Walk {
    /// A walk of what is older than `before`, or of everything.
    fn new(before: Option<Uuid>, now_millis: u64) -> Self {
        let upper = before.unwrap_or(Uuid::max());
        Self {
            upper,
            upper_millis: before
                .and_then(millis_of)
                .unwrap_or(now_millis + CLOCK_SLACK_MS),
            span_millis: FIRST_SPAN_MS,
            aim: FIRST_STEP_ROWS,
            examined: 0,
            done: false,
        }
    }

    /// The next window's older end, and its millisecond: the beginning of time once the span
    /// reaches it.
    fn lower(&self) -> (Uuid, u64) {
        match self.upper_millis.saturating_sub(self.span_millis) {
            0 => (Uuid::nil(), 0),
            millis => (floor_of(millis), millis),
        }
    }

    /// Whether the next window is read with a cap. One a millisecond wide cannot be narrowed,
    /// so it is read whole.
    fn capped(&self) -> bool {
        self.span_millis > 1
    }

    /// Takes in what reading the window down to `lower` found, adding to `found` up to `limit`
    /// in all, and moves on: past the window, or, when the page filled within it, to the last
    /// message taken, so what the window held beyond is read again for the next page.
    fn absorb(&mut self, window: Window, lower: (Uuid, u64), found: &mut Vec<Uuid>, limit: usize) {
        let examined = u64::try_from(window.examined).unwrap_or(0);
        if self.capped() && examined > STEP_ROWS_MAX {
            self.examined += STEP_ROWS_MAX;
            self.span_millis = (self.span_millis / 8).max(1);
            return;
        }
        self.examined += examined;
        let room = limit.saturating_sub(found.len());
        // The statement gives at most `limit` of what passed, so that many may not be all.
        let cut = window.found.len() > room || window.found.len() >= limit;
        found.extend(window.found.into_iter().take(room));
        match found.last().copied().filter(|_| cut) {
            Some(last) => {
                self.upper = last;
                self.upper_millis = millis_of(last).unwrap_or(lower.1);
            }
            None => {
                (self.upper, self.upper_millis) = lower;
                self.done = lower.1 == 0;
            }
        }
        self.aim = (self.aim * 4).min(STEP_ROWS);
        let wanted =
            u128::from(self.span_millis) * u128::from(self.aim) / u128::from(examined.max(1));
        let wanted = u64::try_from(wanted).unwrap_or(u64::MAX);
        self.span_millis = wanted.clamp(
            (self.span_millis / 4).max(1),
            self.span_millis.saturating_mul(8),
        );
    }
}

/// Reads the walk's next window in a transaction of its own. A deep step is given the
/// deployment's `slots` and reads only once it holds one of them (`deep_search::take_slot`),
/// until its transaction ends: `None` when every place was taken.
async fn step(
    conn: &mut AsyncPgConnection,
    sought: &Sought<'_>,
    walk: &Walk,
    lower: Uuid,
    slots: Option<i32>,
) -> crate::Result<Option<Window>> {
    let query = bound(
        window_sql(
            sought.source(),
            sought.in_thread,
            sought.limit,
            walk.capped(),
        ),
        sought,
        sought.text,
        (lower, walk.upper),
        &[],
    );
    Ok(conn
        .transaction::<_, diesel::result::Error, _>(|conn| {
            async move {
                if let Some(slots) = slots
                    && !crate::deep_search::take_slot(conn, slots).await?
                {
                    return Ok(None);
                }
                conn.batch_execute(WALK_SETTINGS).await?;
                query.get_result::<Window>(conn).await.map(Some)
            }
            .scope_boxed()
        })
        .await?)
}

/// The messages matching `search` that `caller` may read, newest first, as far back as this
/// request reads (see the module's own account of the three parts).
pub async fn search_messages(
    state: &GlobalServerContext,
    caller: UserId,
    search: MessageSearch,
    abandoned: &Abandoned,
) -> crate::Result<SearchPage> {
    let text = search
        .text
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty());
    if text.is_none()
        && search.author.is_none()
        && search.mentions.is_none()
        && search.holding.is_empty()
    {
        return Err(crate::Error::Validation(t!("searchEmpty")));
    }
    if text.is_some_and(|text| text.chars().count() > MAX_TEXT_CHARS) {
        return Err(crate::Error::Validation(t!(
            "searchTooLong",
            max = MAX_TEXT_CHARS
        )));
    }
    let started = Instant::now();
    let Readable { places, in_thread } = readable_channels(state, caller, search.scope).await?;
    if places.is_empty() {
        return page(state, Vec::new(), None, &places, in_thread).await;
    }
    let limit = usize::try_from(search.limit.clamp(1, MAX_RESULTS)).unwrap_or(1);
    let sought = Sought {
        caller: caller.0,
        places: &places,
        in_thread,
        text,
        author: search.author.map(|author| author.0),
        mentions: search.mentions.map(|tagged| tagged.0),
        attachment: search.holding.contains(&Holding::Attachment),
        image: search.holding.contains(&Holding::Image),
        poll: search.holding.contains(&Holding::Poll),
        limit,
    };
    let before = search.before.map(|before| before.0);

    // A thread's replies are among its parent's matches, which may be many more than the
    // thread's; the walk reads the thread's alone.
    if text.is_some() && !in_thread {
        let mut conn = state.connection_pool.get().await?;
        let matched = probe(conn.as_mut(), &sought).await?;
        if matched.len() <= PROBE_CAP {
            // Every match there is: the page is the newest of them that pass the rest.
            let mut newest = newest_among(conn.as_mut(), &sought, &matched, before).await?;
            drop(conn);
            let more = newest.len() > limit;
            newest.truncate(limit);
            let searched_to = newest.last().copied().filter(|_| more);
            return page(state, newest, searched_to, &places, in_thread).await;
        }
    }

    let now = u64::try_from(chrono::Utc::now().timestamp_millis()).unwrap_or(0);
    let mut walk = Walk::new(before, now);
    let mut found = Vec::new();
    while !walk.done && found.len() < limit && walk.examined < FREE_ROWS {
        let lower = walk.lower();
        let mut conn = state.connection_pool.get().await?;
        let read = step(conn.as_mut(), &sought, &walk, lower.0, None).await?;
        drop(conn);
        if let Some(window) = read {
            walk.absorb(window, lower, &mut found, limit);
        }
    }
    let searched_to;
    if found.is_empty() && !walk.done {
        let deadline = started + Duration::from_secs(state.config.search.round_seconds.max(1));
        let turn = state.deep_searches.turn(caller);
        let outcome = go_deep(
            state, &sought, &mut walk, &mut found, deadline, abandoned, &turn,
        )
        .await?;
        drop(turn);
        metrics::counter!(aspen_metrics::api::DEEP_SEARCH_ROUNDS, "outcome" => outcome)
            .increment(1);
        searched_to = (!walk.done).then_some(walk.upper);
        // A round reads for seconds, in which the caller may have lost some of the places: what
        // was found is kept only where they may still read.
        if !found.is_empty() {
            let still = readable_channels(state, caller, search.scope).await?;
            return page(state, found, searched_to, &still.places, still.in_thread).await;
        }
    } else {
        searched_to = (!walk.done).then_some(walk.upper);
    }
    page(state, found, searched_to, &places, in_thread).await
}

/// Walks on until something is found, the walk reaches the beginning, `deadline` comes, the
/// client goes away, or the same caller starts another search going deep (`turn`), each step
/// in one of the deployment's places (`app::deep_search`). Says how the round ended, for its
/// metric.
async fn go_deep(
    state: &GlobalServerContext,
    sought: &Sought<'_>,
    walk: &mut Walk,
    found: &mut Vec<Uuid>,
    deadline: Instant,
    abandoned: &Abandoned,
    turn: &crate::deep_search::Turn,
) -> crate::Result<&'static str> {
    let slots = state.deep_searches.slots();
    while found.is_empty() && !walk.done {
        if abandoned.is_so() {
            return Ok("abandoned");
        }
        if turn.superseded() {
            return Ok("superseded");
        }
        let asked = Instant::now();
        if asked >= deadline {
            return Ok("unfinished");
        }
        // Held while this step waits on other servers' steps, so this server's own searches
        // take their turns in the order they asked.
        let Some(_trying) = state.deep_searches.place(deadline).await else {
            return Ok("unfinished");
        };
        let lower = walk.lower();
        let window = loop {
            // The connection goes back between tries, so a search waiting for a place holds
            // none.
            let mut conn = state.connection_pool.get().await?;
            let read = step(conn.as_mut(), sought, walk, lower.0, Some(slots)).await?;
            drop(conn);
            match read {
                Some(window) => break window,
                None if abandoned.is_so() => return Ok("abandoned"),
                None if turn.superseded() => return Ok("superseded"),
                None if crate::deep_search::wait_for_slot(deadline).await => {}
                None => return Ok("unfinished"),
            }
        };
        metrics::histogram!(aspen_metrics::api::DEEP_SEARCH_STEP_DURATION)
            .record(asked.elapsed().as_secs_f64());
        walk.absorb(window, lower, found, sought.limit);
    }
    Ok(if found.is_empty() {
        "exhausted"
    } else {
        "found"
    })
}

/// The messages of the places that match the words, up to one more than [`PROBE_CAP`].
async fn probe(conn: &mut AsyncPgConnection, sought: &Sought<'_>) -> crate::Result<Vec<Uuid>> {
    let query = bound(
        probe_sql(),
        sought,
        sought.text,
        (Uuid::nil(), Uuid::max()),
        &[],
    );
    let matched: Vec<Matched> = conn
        .transaction::<_, diesel::result::Error, _>(|conn| {
            async move {
                conn.batch_execute(PROBE_SETTINGS).await?;
                query.load(conn).await
            }
            .scope_boxed()
        })
        .await?;
    Ok(matched.into_iter().map(|m| m.id).collect())
}

/// The newest `limit` and one more of `matched` that are older than `before` and pass
/// everything but the words, which they are known to match.
async fn newest_among(
    conn: &mut AsyncPgConnection,
    sought: &Sought<'_>,
    matched: &[Uuid],
    before: Option<Uuid>,
) -> crate::Result<Vec<Uuid>> {
    Ok(bound(
        among_sql(sought.limit),
        sought,
        None,
        (Uuid::nil(), before.unwrap_or(Uuid::max())),
        matched,
    )
    .get_result::<Window>(conn)
    .await?
    .found)
}

/// The page of the messages `ids` that lie in `places` (home channels, or one thread
/// `in_thread`), newest first, read back to `searched_to` with more beyond it, or with nothing
/// left when `None`.
async fn page(
    state: &GlobalServerContext,
    ids: Vec<Uuid>,
    searched_to: Option<Uuid>,
    places: &[Uuid],
    in_thread: bool,
) -> crate::Result<SearchPage> {
    let mut conn = state.connection_pool.get().await?;
    let query = message::table
        .select(Message::as_select())
        .filter(message::id.eq_any(ids))
        .order_by(message::id.desc())
        .into_boxed();
    let query = if in_thread {
        query.filter(message::channel.eq_any(places))
    } else {
        query.filter(message::home_channel.eq_any(places))
    };
    let messages: Vec<Message> = query.load(conn.as_mut()).await?;
    Ok(SearchPage {
        messages: crate::channel::with_relations(state, conn.as_mut(), messages).await?,
        searched_to: searched_to.map(MessageId),
        more: searched_to.is_some(),
    })
}

/// Where a search may read.
struct Readable {
    places: Vec<Uuid>,
    /// Whether the one place is a thread (`Sought::in_thread`).
    in_thread: bool,
}

async fn readable_channels(
    state: &GlobalServerContext,
    caller: UserId,
    scope: SearchScope,
) -> crate::Result<Readable> {
    let homes = |channels: Vec<ChannelId>| Readable {
        places: channels.into_iter().map(|channel| channel.0).collect(),
        in_thread: false,
    };
    match scope {
        SearchScope::Channel(channel) => {
            let mut conn = state.connection_pool.get().await?;
            // Searching is reading; a deployment moderator reads DMs they are not in only by
            // opening them, where the reading is logged, so `channel_access` finds none here.
            let access =
                crate::permissions::channel_access(state, conn.as_mut(), caller, channel).await?;
            Ok(Readable {
                places: vec![channel.0],
                in_thread: access.thread,
            })
        }
        SearchScope::Community(community) => {
            let mut conn = state.connection_pool.get().await?;
            let member: bool = diesel::select(exists(
                community_user::table.filter(
                    community_user::community
                        .eq(community)
                        .and(community_user::user.eq(caller))
                        .and(community_user::community.eq_any(crate::community::live())),
                ),
            ))
            .get_result(conn.as_mut())
            .await?;
            if !member {
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            }
            drop(conn);
            Ok(homes(
                Visibility::load(state, caller, &[community])
                    .await?
                    .visible_channels(),
            ))
        }
        SearchScope::Everywhere => Ok(homes(readable_everywhere(state, caller).await?)),
    }
}

/// Every channel and DM `user` may read: the channels they may view in the communities they
/// belong to, and the DMs they are in. Their threads are read as they are.
pub async fn readable_everywhere(
    state: &GlobalServerContext,
    user: UserId,
) -> crate::Result<Vec<ChannelId>> {
    let mut conn = state.connection_pool.get().await?;
    let communities = crate::events::memberships(conn.as_mut(), user).await?;
    let dms: Vec<ChannelId> = dm_recipient::table
        .select(dm_recipient::channel)
        .filter(dm_recipient::user.eq(user))
        .load(conn.as_mut())
        .await?;
    drop(conn);
    let mut channels = Visibility::load(state, user, &communities)
        .await?
        .visible_channels();
    channels.extend(dms);
    Ok(channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(examined: i64, found: &[Uuid]) -> Window {
        Window {
            examined,
            found: found.to_vec(),
        }
    }

    const NOW: u64 = 1_800_000_000_000;

    #[test]
    fn a_window_holding_too_much_is_read_again_narrower() {
        let mut walk = Walk::new(None, NOW);
        let (before, lower) = (walk.clone(), walk.lower());
        let mut found = Vec::new();
        let over = i64::try_from(STEP_ROWS_MAX + 1).unwrap();
        walk.absorb(window(over, &[floor_of(NOW)]), lower, &mut found, 25);
        assert!(
            found.is_empty(),
            "nothing of a window not read whole is kept"
        );
        assert_eq!(walk.upper, before.upper);
        assert_eq!(walk.span_millis, before.span_millis / 8);
        assert_eq!(walk.examined, STEP_ROWS_MAX);
    }

    #[test]
    fn a_window_read_whole_moves_the_walk_past_it_and_toward_its_aim() {
        let mut walk = Walk::new(None, NOW);
        let lower = walk.lower();
        let mut found = Vec::new();
        let hit = floor_of(NOW - 5);
        walk.absorb(window(100, &[hit]), lower, &mut found, 25);
        assert_eq!(found, vec![hit]);
        assert_eq!((walk.upper, walk.upper_millis), lower);
        assert!(!walk.done);
        // A hundred messages where ten thousand were hoped for: as much wider as it may grow.
        assert_eq!(walk.span_millis, FIRST_SPAN_MS * 8);
        assert_eq!(walk.aim, FIRST_STEP_ROWS * 4);
    }

    #[test]
    fn a_page_that_fills_within_a_window_resumes_from_its_last_message() {
        let mut walk = Walk::new(None, NOW);
        let lower = walk.lower();
        let hits: Vec<Uuid> = (1..=4).map(|i| floor_of(NOW - i)).collect();
        // Two were found by earlier windows, so one place is left on a page of three.
        let mut found = vec![floor_of(NOW + 2), floor_of(NOW + 1)];
        walk.absorb(window(50, &hits), lower, &mut found, 3);
        assert_eq!(found.len(), 3);
        assert_eq!(walk.upper, hits[0]);
        assert_eq!(walk.upper_millis, NOW - 1);
        assert!(!walk.done);
    }

    #[test]
    fn as_many_hits_as_the_statement_gives_may_not_be_all_the_window_held() {
        let mut walk = Walk::new(None, NOW);
        let lower = walk.lower();
        let hits: Vec<Uuid> = (1..=3).map(|i| floor_of(NOW - i)).collect();
        let mut found = Vec::new();
        walk.absorb(window(50, &hits), lower, &mut found, 3);
        assert_eq!(walk.upper, hits[2]);
    }

    #[test]
    fn the_walk_is_done_once_a_window_reaches_the_beginning() {
        let mut walk = Walk::new(Some(floor_of(1000)), NOW);
        let lower = walk.lower();
        assert_eq!(lower, (Uuid::nil(), 0));
        walk.absorb(window(0, &[]), lower, &mut Vec::new(), 25);
        assert!(walk.done);
    }

    #[test]
    fn a_walk_starts_from_the_message_it_is_older_than() {
        let before = floor_of(NOW - 77);
        let walk = Walk::new(Some(before), NOW);
        assert_eq!((walk.upper, walk.upper_millis), (before, NOW - 77));
        assert_eq!(
            walk.lower(),
            (floor_of(NOW - 77 - FIRST_SPAN_MS), NOW - 77 - FIRST_SPAN_MS)
        );
    }

    const READER: &str = "5ea70000-0000-4000-8000-000000000001";
    const ANN: &str = "5ea70000-0000-4000-8000-000000000002";
    const BEN: &str = "5ea70000-0000-4000-8000-000000000003";
    const BLOCKED: &str = "5ea70000-0000-4000-8000-000000000004";
    const COMMUNITY: &str = "5ea70000-0000-4000-8000-000000000010";
    const CHANNEL: &str = "5ea70000-0000-4000-8000-000000000011";
    const THREAD: &str = "5ea70000-0000-4000-8000-000000000012";
    const GONE_THREAD: &str = "5ea70000-0000-4000-8000-000000000013";
    const ELSEWHERE: &str = "5ea70000-0000-4000-8000-000000000014";

    fn id(text: &str) -> Uuid {
        text.parse().unwrap()
    }

    /// The message said `minutes` before [`NOW`].
    fn said(minutes: u64) -> Uuid {
        floor_of(NOW - minutes * 60_000)
    }

    /// Runs against the database `DATABASE_URL` names, inside a transaction that is never
    /// committed; without one it checks nothing.
    #[tokio::test]
    async fn each_way_of_reading_finds_what_passes_and_nothing_else() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        let users: String = [READER, ANN, BEN, BLOCKED]
            .iter()
            .map(|user| format!("('{user}', '{user}', '', now(), now())"))
            .collect::<Vec<_>>()
            .join(", ");
        // (minutes ago, author, channel, text)
        let messages = [
            (1, ANN, CHANNEL, "apples and pears"),
            (2, BEN, CHANNEL, "pears alone"),
            (3, BLOCKED, CHANNEL, "apples from someone blocked"),
            (4, ANN, THREAD, "apples in a thread"),
            (5, ANN, GONE_THREAD, "apples in a deleted thread"),
            (6, ANN, ELSEWHERE, "apples somewhere not searched"),
            (7, BEN, CHANNEL, "apples long ago"),
        ];
        let rows: String = messages
            .iter()
            .map(|(minutes, author, channel, text)| {
                format!(
                    "('{}', '{author}', '{channel}', '{text}', now())",
                    said(*minutes)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        conn.batch_execute(&format!(
            r#"INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at) VALUES {users};
             INSERT INTO community (id, name) VALUES ('{COMMUNITY}', 'c');
             INSERT INTO channel (id, name, ty, sort_index, community)
                 VALUES ('{CHANNEL}', 'a', 'text', 0, '{COMMUNITY}'),
                        ('{ELSEWHERE}', 'b', 'text', 1, '{COMMUNITY}');
             INSERT INTO channel (id, name, ty, sort_index, parent_channel, deleted_at)
                 VALUES ('{THREAD}', 't', 'thread', 0, '{CHANNEL}', NULL),
                        ('{GONE_THREAD}', 'g', 'thread', 0, '{CHANNEL}', now());
             INSERT INTO user_block (blocker, blocked) VALUES ('{READER}', '{BLOCKED}');
             INSERT INTO message (id, author, channel, content, "timestamp") VALUES {rows};
             INSERT INTO mention (message, channel, target_user)
                 VALUES ('{tagged}', '{CHANNEL}', '{READER}');"#,
            tagged = said(7),
        ))
        .await
        .unwrap();

        let places = [id(CHANNEL)];
        let sought =
            |text: Option<&'static str>, author: Option<&str>, tagged: Option<&str>| Sought {
                caller: id(READER),
                places: &places,
                in_thread: false,
                text,
                author: author.map(id),
                mentions: tagged.map(id),
                attachment: false,
                image: false,
                poll: false,
                limit: 25,
            };
        let whole = Walk {
            span_millis: NOW,
            ..Walk::new(Some(floor_of(NOW)), NOW)
        };
        let apples = [said(1), said(4), said(7)];

        // The words alone, each way a window is read.
        let by_place = sought(Some("apples"), None, None);
        assert_eq!(by_place.source(), Source::Places);
        let read = step(&mut conn, &by_place, &whole, Uuid::nil(), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.found, apples);
        // Everything said in the place and its threads, the deleted thread's too: the window
        // counts what it read, not what passed.
        assert_eq!(read.examined, 6);

        let by_author = sought(Some("apples"), Some(BEN), None);
        assert_eq!(by_author.source(), Source::Author);
        let read = step(&mut conn, &by_author, &whole, Uuid::nil(), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.found, [said(7)]);
        assert_eq!(read.examined, 2);

        let by_tag = sought(None, None, Some(READER));
        assert_eq!(by_tag.source(), Source::Tagged);
        let read = step(&mut conn, &by_tag, &whole, Uuid::nil(), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.found, [said(7)]);

        // A window ends where it says: at its older end and not before its newer.
        let narrow = Walk {
            upper: said(1),
            ..whole.clone()
        };
        let read = step(&mut conn, &by_place, &narrow, said(4), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(read.found, [said(4)]);

        // The probe reads matches whatever else is asked, and the page is chosen among them.
        let mut matched = probe(&mut conn, &by_place).await.unwrap();
        matched.sort_unstable_by(|a, b| b.cmp(a));
        assert_eq!(matched, [said(1), said(3), said(4), said(5), said(7)]);
        assert_eq!(
            newest_among(&mut conn, &by_place, &matched, None)
                .await
                .unwrap(),
            apples
        );
        assert_eq!(
            newest_among(&mut conn, &by_place, &matched, Some(said(1)))
                .await
                .unwrap(),
            [said(4), said(7)]
        );

        // A deep step reads once it holds one of the deployment's places.
        let read = step(&mut conn, &by_place, &whole, Uuid::nil(), Some(2))
            .await
            .unwrap();
        assert_eq!(read.unwrap().found, apples);

        // A thread is searched by the channel its replies are in, each way a window is read.
        let thread = [id(THREAD)];
        for (author, tagged) in [(None, None), (Some(ANN), None)] {
            let in_thread = Sought {
                places: &thread,
                in_thread: true,
                ..sought(Some("apples"), author, tagged)
            };
            let read = step(&mut conn, &in_thread, &whole, Uuid::nil(), None)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(read.found, [said(4)]);
        }
    }
}
