//! The activity feed: every message that tells a user of itself, from everywhere they read on
//! this deployment, newest first. Apps signed in to several deployments merge each one's feed.
//!
//! A message tells its reader by the same rule as their notifications (`app::push`, and the
//! apps' own): in a DM, a community, a category, or a channel, every message where their level
//! is `all` (a DM's and a small community's by default), only those that tag them where it is
//! `tags` (a large community's by default, `app::large_community`), and nothing where it is
//! `nothing`, a thread counting as its parent; and every reply in a thread they follow
//! (`app::thread_follow`), whatever their level. Never in a channel they muted, or whose
//! category they muted unless they kept the channel unmuted (a thread's parent included), never
//! by them or anyone they blocked, never from before they joined, and never a message that only
//! records something (an echo, a call, a command). What is read is decided as they may read it
//! now, as search decides it, and a page is read a place at a time ([`page_sql`]).
//!
//! That rule is the feed's own view ([`ActivityView::Notifying`]). The other views read wider or
//! narrower than it, whatever the user's levels: everything said where they read, what tags
//! them, their DMs, or the threads they follow. Mutes, blocks, joining, and the kinds left out
//! hold in every view.

use crate::channel::ChannelType;
use crate::context::GlobalServerContext;
use crate::cursor::{List, Paged};
use crate::message::{Message, MessageKind, MessageWithRelations};
use crate::notification_setting::{NotificationLevel, level_of};
use crate::visibility::Visibility;
use crate::{CategoryId, ChannelId, CommunityId, MessageId, RoleId, UserId};
use aspen_schema::{channel, community_member_role, dm_recipient, message, thread_follow};
use diesel::prelude::*;
use diesel::sql_types::{Array, Bool, Uuid as PgUuid};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use std::collections::{HashMap, HashSet};

/// The most messages one page of the feed holds.
pub const MAX_PAGE: u32 = 50;

/// Which messages a read of the feed is of. Each leaves out what every view does: muted
/// channels, the user's own messages and those of anyone they blocked, what was said before
/// they joined, and messages that only record something.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ActivityView {
    /// What tells the user of itself, by their notification levels and the threads they follow.
    #[default]
    Notifying,
    /// Everything said where they read, threads included.
    All,
    /// What tags them: by name, a role they hold, or everyone.
    Mentions,
    /// Their DMs and group DMs, and those DMs' threads.
    Dms,
    /// Replies in the threads they follow.
    Threads,
}

/// Which part of the feed to read.
#[derive(Debug, Clone, Default)]
pub struct ActivityQuery {
    pub view: ActivityView,
    /// Only these communities' messages, or every community's when `None`.
    pub communities: Option<Vec<CommunityId>>,
    /// Whether DMs' messages are read.
    pub dms: bool,
    /// Only messages the reader has not read.
    pub unread: bool,
    /// The `next_cursor` of the page before the one asked for; the first page when `None`.
    pub cursor: Option<String>,
    pub limit: u32,
}

/// Where messages tell the user of themselves, decided from their settings, mutes, follows, and
/// what they may read.
#[derive(Debug, Default, PartialEq, Eq)]
struct Places {
    /// Every message here tells them, and in these channels' threads.
    all: Vec<ChannelId>,
    /// Messages tagging them here tell them, and in these channels' threads.
    tags: Vec<ChannelId>,
    /// Threads every reply in which tells them.
    followed: Vec<ChannelId>,
    /// Every channel and DM they read and have not muted, whatever their level there.
    unmuted: Vec<ChannelId>,
    /// The DMs among `unmuted`.
    dms: Vec<ChannelId>,
}

/// One channel or DM the user may read, and their level and mute there.
struct Place {
    channel: ChannelId,
    dm: bool,
    level: NotificationLevel,
    muted: bool,
}

fn sort_places(places: &[Place], followed: &[(ChannelId, ChannelId)]) -> Places {
    let mut sorted = Places::default();
    let mut unmuted = HashSet::new();
    for place in places.iter().filter(|p| !p.muted) {
        unmuted.insert(place.channel);
        sorted.unmuted.push(place.channel);
        if place.dm {
            sorted.dms.push(place.channel);
        }
        match place.level {
            NotificationLevel::All => sorted.all.push(place.channel),
            NotificationLevel::Tags => sorted.tags.push(place.channel),
            NotificationLevel::Nothing => {}
        }
    }
    sorted.followed = followed
        .iter()
        .filter(|(_, parent)| unmuted.contains(parent))
        .map(|(thread, _)| *thread)
        .collect();
    sorted
}

#[derive(QueryableByName)]
struct Found {
    #[diesel(sql_type = PgUuid)]
    id: MessageId,
}

/// What every message of the feed passes, written of `m`: not deleted, nor its thread; by
/// someone else the user (`$1`) has not blocked; of no kind in `$8`; and, when `$3`, after the
/// user's position where it was said.
const PASSES: &str = r#"
    m.deleted_at IS NULL
    AND m.author <> $1
    AND m.kind <> ALL($8)
    AND NOT EXISTS (
        SELECT 1 FROM user_block b WHERE b.blocker = $1 AND b.blocked = m.author
    )
    AND NOT EXISTS (
        SELECT 1 FROM channel gone WHERE gone.id = m.channel AND gone.deleted_at IS NOT NULL
    )
    AND (NOT $3 OR NOT EXISTS (
        SELECT 1 FROM read_state rs
        WHERE rs."user" = $1 AND rs.channel = m.channel AND rs.message >= m.id
    ))
"#;

/// Each place `p.channel` with `j.floor`, the position of the moment the user joined where it
/// belongs (`aspen_uuid_floor`): nothing at or before it is theirs to read, and a place they
/// belong to nowhere has none, so nothing of it is read.
const JOINED_PLACE: &str = r#"
    JOIN channel pc ON pc.id = p.channel
    LEFT JOIN community_user cu ON cu.community = pc.community AND cu."user" = $1
    LEFT JOIN dm_recipient dr ON dr.channel = pc.id AND dr."user" = $1
    CROSS JOIN LATERAL (
        SELECT aspen_uuid_floor(COALESCE(cu.joined_at, dr.joined_at)) AS floor
    ) j
"#;

/// The newest `limit` ids (`key`, of `from`) older than `$2` that pass `passing`, across every
/// row `x` of `source`, each row's walked on its own: those whose `place` is `x.channel` and
/// whose id is after `x.after`. A row whose `after` is null has nothing read.
///
/// The newest of each `x` is read first, one row apiece, and only the `limit` rows of `source`
/// with the newest of those are walked for a page: a message among the newest `limit` of all
/// has fewer than `limit` messages newer than it, so fewer than `limit` other rows of `source`
/// can begin with a newer one. That is one index row for each `x` and at most `limit` squared
/// after, where walking every `x` for a page would be `limit` for each.
///
/// The range is written as two row comparisons, `(place, key)` between `(x.channel, x.after)`
/// and `(x.channel, $2)`, which only rows of `x.channel` satisfy, rather than as `place =
/// x.channel` with a range of `key`. Both bound the same stretch of an index on `(place, key)`,
/// but the planner may take a two-sided range of one column, one end of which it cannot see,
/// for a sliver, and then collect and sort everything the place holds rather than walk back
/// from its newest (it does when `$3` is set). It measures a row comparison by its first column,
/// and walks.
fn walk(source: &str, from: &str, place: &str, key: &str, passing: &str, limit: u32) -> String {
    let newest = |rows: u32| {
        format!(
            r#"
            SELECT {key} AS id FROM {from}
            WHERE ({place}, {key}) > (x.channel, x.after) AND ({place}, {key}) < (x.channel, $2)
              AND {passing}
            ORDER BY {place} DESC, {key} DESC
            LIMIT {rows}
            "#
        )
    };
    format!(
        r#"
        (SELECT found.id
         FROM (
             SELECT x.*
             FROM {source}
             CROSS JOIN LATERAL ({head}) head
             ORDER BY head.id DESC
             LIMIT {limit}
         ) x
         CROSS JOIN LATERAL ({page}) found)
        "#,
        head = newest(1),
        page = newest(limit),
    )
}

/// The ids of the feed's newest `limit` messages older than `$2`, newest first: a page and the
/// one message past it that says another follows, as [`read_activity`] asks. `$1` is the user,
/// `$3` whether only what is unread is read, `$4` the roles the user holds, `$5` the places read
/// whole with their threads, `$6` the threads read alone, `$7` the places read for what tags
/// the user, and `$8` the kinds of message left out.
///
/// The planner chooses a walk over collecting and sorting everything a place holds only when it
/// can see how few rows are wanted and how wide the range is. So `limit` is written into the
/// statement rather than bound, and the statement must be planned for the values it is run
/// with, as `sql_query` is, which Diesel prepares afresh each time; a plan made once for every
/// value (a cached prepared statement) sorts.
///
/// PostgreSQL cannot read "the newest across these channels" in order from an index that
/// leads with the channel, so no branch asks it to: each walks one index backwards from the
/// newest for one place, thread, or tagged target at a time ([`walk`]), and stops at the page's
/// length. The page is then the newest of what the branches found, so its cost grows with the
/// places and the page, not with what the places hold.
///
/// - A place is walked through `message_by_home_channel`, a thread through `message (channel,
///   id)`, each back to the moment the user joined, and a thread read for what is unread back
///   to their position in it.
/// - Tags are walked through `mention`: the user's own through `mention_user_recent`, each
///   role's through `mention_role_recent`, and each place's tags of everyone through
///   `mention_everyone_recent`. The first two pass over tags in places not asked for.
///
/// The unread filter over a place read whole is a filter, not a bound: a thread's position is
/// its own, so what is read of a place and its threads is walked past until the page fills.
fn page_sql(limit: u32) -> String {
    let places = |listed: &str| {
        format!(
            "unnest({listed}::uuid[]) AS p(channel) {JOINED_PLACE} \
             CROSS JOIN LATERAL (SELECT p.channel, j.floor AS after) x"
        )
    };
    let whole = walk(
        &places("$5"),
        "message m",
        "m.home_channel",
        "m.id",
        PASSES,
        limit,
    );
    let threads = walk(
        // A thread read for what is unread is walked back to the user's position in it.
        &format!(
            r#"unnest($6::uuid[]) AS t(thread)
            JOIN channel tc ON tc.id = t.thread
            CROSS JOIN LATERAL (SELECT tc.parent_channel AS channel) p
            {JOINED_PLACE}
            LEFT JOIN read_state pos ON pos.channel = t.thread AND pos."user" = $1
            CROSS JOIN LATERAL (
                SELECT t.thread AS channel,
                       CASE WHEN j.floor IS NOT NULL
                            THEN GREATEST(j.floor, CASE WHEN $3 THEN pos.message END)
                       END AS after
            ) x"#
        ),
        "message m",
        "m.channel",
        "m.id",
        PASSES,
        limit,
    );
    let everyone = walk(
        &places("$7"),
        "mention mn JOIN message m ON m.id = mn.message",
        "mn.home_channel",
        "mn.message",
        &format!("mn.everyone AND {PASSES}"),
        limit,
    );
    // A tag by name or by role is found before its place is known, so joining is checked
    // against the message's time there rather than bounding the walk.
    let tagged = format!(
        r#"
            cardinality($7::uuid[]) > 0
            AND mn.message < $2
            AND m.home_channel = ANY($7)
            AND (EXISTS (
                SELECT 1 FROM channel hc
                JOIN community_user cu ON cu.community = hc.community AND cu."user" = $1
                WHERE hc.id = m.home_channel AND cu.joined_at < m."timestamp"
            ) OR EXISTS (
                SELECT 1 FROM dm_recipient dr
                WHERE dr."user" = $1 AND dr.channel = m.home_channel
                  AND dr.joined_at < m."timestamp"
            ))
            AND {PASSES}
            "#
    );
    format!(
        r#"
            SELECT id FROM (
                {whole}
                UNION
                {threads}
                UNION
                (SELECT m.id
                 FROM mention mn
                 JOIN message m ON m.id = mn.message
                 WHERE mn.target_user = $1 AND {tagged}
                 ORDER BY mn.message DESC
                 LIMIT {limit})
                UNION
                (SELECT found.id
                 FROM unnest($4::uuid[]) AS r(role)
                 CROSS JOIN LATERAL (
                     SELECT m.id
                     FROM mention mn
                     JOIN message m ON m.id = mn.message
                     WHERE mn.target_role = r.role AND {tagged}
                     ORDER BY mn.message DESC
                     LIMIT {limit}
                 ) found)
                UNION
                {everyone}
            ) page
            ORDER BY id DESC
            LIMIT {limit}
            "#
    )
}

/// The place an activity feed's cursor holds: the id of the last message of the page before,
/// which is all the feed is ordered by.
fn activity_position(cursor: &str) -> crate::Result<MessageId> {
    crate::cursor::decode(List::Activity, cursor)
}

/// One page of the user's feed that `query` asks for, newest first, and the cursor of the page
/// after it.
pub async fn read_activity(
    state: &GlobalServerContext,
    user: UserId,
    query: ActivityQuery,
) -> crate::Result<Paged<MessageWithRelations>> {
    let before = query.cursor.as_deref().map(activity_position).transpose()?;
    let mut conn = state.connection_pool.get().await?;
    let mut communities = crate::events::memberships(conn.as_mut(), user).await?;
    if let Some(asked) = &query.communities {
        communities.retain(|c| asked.contains(c));
    }
    let dms: Vec<ChannelId> = if query.dms {
        dm_recipient::table
            .select(dm_recipient::channel)
            .filter(dm_recipient::user.eq(user))
            .load(conn.as_mut())
            .await?
    } else {
        Vec::new()
    };
    let roles: Vec<RoleId> = community_member_role::table
        .select(community_member_role::role)
        .filter(community_member_role::user.eq(user))
        .filter(community_member_role::community.eq_any(&communities))
        .load(conn.as_mut())
        .await?;
    drop(conn);
    let visible = Visibility::load(state, user, &communities).await?;
    let (community_settings, dm_settings, community_mutes, dm_mutes, category_mutes) = tokio::try_join!(
        crate::notification_setting::read_community_settings(state, &visible),
        crate::notification_setting::read_channel_settings(state, user, &dms),
        crate::channel_mute::read_community_mutes(state, &visible),
        crate::channel_mute::read_channel_mutes(state, user, &dms),
        crate::category_mute::read_category_mutes(state, &visible),
    )?;
    let mut conn = state.connection_pool.get().await?;
    let small = crate::large_community::small_among(state, conn.as_mut(), &communities).await?;
    let channels = visible.visible_channels();
    let homes: HashMap<ChannelId, (Option<CommunityId>, Option<CategoryId>)> = channel::table
        .select((channel::id, channel::community, channel::parent_category))
        .filter(channel::id.eq_any(&channels))
        .load::<(ChannelId, Option<CommunityId>, Option<CategoryId>)>(conn.as_mut())
        .await?
        .into_iter()
        .map(|(channel, community, category)| (channel, (community, category)))
        .collect();
    let mut for_channel = HashMap::new();
    let mut for_category = HashMap::new();
    let mut for_community = HashMap::new();
    for setting in community_settings.into_iter().chain(dm_settings) {
        match (setting.channel, setting.category, setting.community) {
            (Some(channel), _, _) => for_channel.insert(channel, setting.level),
            (None, Some(category), _) => for_category.insert(category, setting.level),
            (None, None, Some(community)) => for_community.insert(community, setting.level),
            (None, None, None) => None,
        };
    }
    // A channel's own setting, muted or kept unmuted, outranks its category's mute.
    let own_mute: HashMap<ChannelId, bool> = community_mutes
        .iter()
        .chain(&dm_mutes)
        .map(|m| (m.channel, m.muted))
        .collect();
    let muted_categories: HashSet<CategoryId> = category_mutes.iter().map(|m| m.category).collect();
    let places: Vec<Place> = channels
        .iter()
        .map(|channel| (*channel, ChannelType::Text))
        .chain(dms.iter().map(|dm| (*dm, ChannelType::Dm)))
        .map(|(channel, ty)| {
            let (community, category) = homes.get(&channel).copied().unwrap_or((None, None));
            Place {
                channel,
                dm: ty == ChannelType::Dm,
                level: level_of(
                    for_channel.get(&channel).copied(),
                    category.and_then(|c| for_category.get(&c)).copied(),
                    community.and_then(|c| for_community.get(&c)).copied(),
                    ty,
                    community.is_some_and(|c| small.contains(&c)),
                ),
                muted: own_mute
                    .get(&channel)
                    .copied()
                    .unwrap_or_else(|| category.is_some_and(|c| muted_categories.contains(&c))),
            }
        })
        .collect();
    let readable: Vec<ChannelId> = places.iter().map(|p| p.channel).collect();
    let followed: Vec<(ChannelId, ChannelId)> = thread_follow::table
        .inner_join(channel::table.on(channel::id.eq(thread_follow::thread)))
        .select((
            thread_follow::thread,
            channel::parent_channel.assume_not_null(),
        ))
        .filter(thread_follow::user.eq(user))
        .filter(channel::deleted_at.is_null())
        .filter(channel::parent_channel.eq_any(&readable))
        .load(conn.as_mut())
        .await?;
    let places = sort_places(&places, &followed);
    if places.unmuted.is_empty() {
        return Ok(Paged {
            items: Vec::new(),
            next_cursor: None,
        });
    }
    // Each view is some of three reads: places read whole with their threads, threads read
    // alone, and places read for what tags the user.
    let none = Vec::new;
    let (whole, threads, tagging) = match query.view {
        ActivityView::Notifying => (places.all, places.followed, places.tags),
        ActivityView::All => (places.unmuted, none(), none()),
        ActivityView::Mentions => (none(), none(), places.unmuted),
        ActivityView::Dms => (places.dms, none(), none()),
        ActivityView::Threads => (none(), places.followed, none()),
    };
    let reads = Reads {
        whole,
        threads,
        tagging,
        roles,
        unread: query.unread,
    };
    let page = feed_page(conn.as_mut(), user, &reads, before, query.limit).await?;
    let messages: Vec<Message> = message::table
        .select(Message::as_select())
        .filter(message::id.eq_any(&page.items))
        .order_by(message::id.desc())
        .load(conn.as_mut())
        .await?;
    Ok(Paged {
        items: crate::message::with_relations(state, conn.as_mut(), messages).await?,
        next_cursor: page.next_cursor,
    })
}

/// What a page of the feed is read of ([`page_sql`]).
struct Reads {
    /// The places read whole, with their threads.
    whole: Vec<ChannelId>,
    /// The threads read alone.
    threads: Vec<ChannelId>,
    /// The places read for what tags the user.
    tagging: Vec<ChannelId>,
    /// The roles the user holds, a tag of which tags them.
    roles: Vec<RoleId>,
    /// Only what the user has not read.
    unread: bool,
}

/// The ids of one page of `user`'s feed, at most `limit` of them (up to [`MAX_PAGE`]), newest
/// first, older than `before` when given, and the cursor of the page after it. The statement
/// is asked for one message more than the page holds, and only when it finds that one does
/// another page follow. The place is compared as an id alone, so the page is read whether or
/// not the message the cursor was made from still stands.
async fn feed_page(
    conn: &mut AsyncPgConnection,
    user: UserId,
    reads: &Reads,
    before: Option<MessageId>,
    limit: u32,
) -> crate::Result<Paged<MessageId>> {
    let limit = limit.clamp(1, MAX_PAGE);
    let uuids = |ids: &[ChannelId]| ids.iter().map(|id| id.0).collect::<Vec<_>>();
    let found: Vec<Found> = diesel::sql_query(page_sql(limit + 1))
        .bind::<PgUuid, _>(user.0)
        .bind::<PgUuid, _>(before.map_or(uuid::Uuid::max(), |before| before.0))
        .bind::<Bool, _>(reads.unread)
        .bind::<Array<PgUuid>, _>(reads.roles.iter().map(|role| role.0).collect::<Vec<_>>())
        .bind::<Array<PgUuid>, _>(uuids(&reads.whole))
        .bind::<Array<PgUuid>, _>(uuids(&reads.threads))
        .bind::<Array<PgUuid>, _>(uuids(&reads.tagging))
        // What only records something says nothing of its own, as it wakes no phone.
        .bind::<Array<aspen_schema::sql_types::MessageKind>, _>(
            <MessageKind as strum::VariantArray>::VARIANTS
                .iter()
                .copied()
                .filter(|kind| !kind.notifies())
                .collect::<Vec<_>>(),
        )
        .load(conn)
        .await?;
    Ok(crate::cursor::cut(
        List::Activity,
        found.into_iter().map(|found| found.id).collect(),
        limit as usize,
        |last: &MessageId| *last,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(level: NotificationLevel, muted: bool) -> Place {
        Place {
            channel: ChannelId::new(),
            dm: false,
            level,
            muted,
        }
    }

    #[test]
    fn levels_sort_places_and_mutes_silence_them_and_their_threads() {
        let all = place(NotificationLevel::All, false);
        let tags = place(NotificationLevel::Tags, false);
        let nothing = place(NotificationLevel::Nothing, false);
        let muted = place(NotificationLevel::All, true);
        let dm = Place {
            dm: true,
            ..place(NotificationLevel::All, false)
        };
        let muted_dm = Place {
            dm: true,
            ..place(NotificationLevel::All, true)
        };
        let (in_all, in_nothing, in_muted) = (ChannelId::new(), ChannelId::new(), ChannelId::new());
        let followed = [
            (in_all, all.channel),
            (in_nothing, nothing.channel),
            (in_muted, muted.channel),
        ];
        let expected = Places {
            all: vec![all.channel, dm.channel],
            tags: vec![tags.channel],
            // Following outranks a level of nothing; a mute outranks following.
            followed: vec![in_all, in_nothing],
            // A level of nothing still leaves a place read; only a mute takes it out.
            unmuted: vec![all.channel, tags.channel, nothing.channel, dm.channel],
            dms: vec![dm.channel],
        };
        assert_eq!(
            sort_places(&[all, tags, nothing, muted, dm, muted_dm], &followed),
            expected
        );
    }
}

#[cfg(test)]
mod database_tests {
    use super::*;
    use diesel_async::{AsyncConnection, SimpleAsyncConnection};

    const READER: &str = "ac717000-0000-4000-8000-000000000001";
    const WRITER: &str = "ac717000-0000-4000-8000-000000000002";
    const DM: &str = "ac717000-0000-4000-8000-000000000003";

    fn message(n: u8) -> MessageId {
        MessageId(
            format!("ac717000-0000-4000-8000-0000000001{n:02x}")
                .parse()
                .unwrap(),
        )
    }

    /// A connection to the database `DATABASE_URL` names, inside a transaction that is never
    /// committed, with a reader in a DM where someone else said `said` messages, the n-th
    /// after the one before; `None` without one, when the test checks nothing.
    async fn reader_of(said: u8) -> Option<AsyncPgConnection> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return None;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(&format!(
            r#"INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at)
                   VALUES ('{READER}', '{READER}', '', now(), now()),
                          ('{WRITER}', '{WRITER}', '', now(), now());
               INSERT INTO channel (id, name, ty, sort_index) VALUES ('{DM}', '', 'dm', 0);
               INSERT INTO dm_recipient (channel, "user", joined_at)
                   VALUES ('{DM}', '{READER}', '2000-01-01T00:00:00Z'),
                          ('{DM}', '{WRITER}', '2000-01-01T00:00:00Z');"#
        ))
        .await
        .unwrap();
        for n in 1..=said {
            conn.batch_execute(&format!(
                r#"INSERT INTO message (id, channel, author, content, timestamp)
                   VALUES ('{}', '{DM}', '{WRITER}', 'm', now());"#,
                message(n).0
            ))
            .await
            .unwrap();
        }
        Some(conn)
    }

    /// Which messages one page of the reader's DMs holds, by their numbers, and the next
    /// page's cursor.
    async fn page(
        conn: &mut AsyncPgConnection,
        cursor: Option<&str>,
        limit: u32,
    ) -> (Vec<u8>, Option<String>) {
        let reads = Reads {
            whole: vec![ChannelId(DM.parse().unwrap())],
            threads: Vec::new(),
            tagging: Vec::new(),
            roles: Vec::new(),
            unread: false,
        };
        let page = feed_page(
            conn,
            UserId(READER.parse().unwrap()),
            &reads,
            cursor.map(|c| activity_position(c).unwrap()),
            limit,
        )
        .await
        .unwrap();
        let messages = page
            .items
            .into_iter()
            .map(|id| (1..=u8::MAX).find(|n| message(*n) == id).unwrap())
            .collect();
        (messages, page.next_cursor)
    }

    /// The feed's cursor is a place, not a message: the pages go on after the message a
    /// cursor was made from is deleted, and the last page says it is the last however many it
    /// holds.
    #[tokio::test]
    async fn the_feed_pages_past_a_message_that_is_gone() {
        let Some(mut conn) = reader_of(7).await else {
            return;
        };
        let (messages, cursor) = page(&mut conn, None, 3).await;
        assert_eq!(messages, [7, 6, 5]);
        // The fifth, which the cursor was made from, is deleted.
        conn.batch_execute(&format!(
            "UPDATE message SET deleted_at = now() WHERE id = '{}'",
            message(5).0
        ))
        .await
        .unwrap();
        let (messages, cursor) = page(&mut conn, cursor.as_deref(), 2).await;
        assert_eq!(messages, [4, 3]);
        assert!(cursor.is_some());
        // A page that ends on the oldest message has none after it, though it is full.
        let (messages, cursor) = page(&mut conn, cursor.as_deref(), 2).await;
        assert_eq!(messages, [2, 1]);
        assert_eq!(cursor, None);
        let (messages, cursor) = page(&mut conn, None, 6).await;
        assert_eq!(messages, [7, 6, 4, 3, 2, 1]);
        assert_eq!(cursor, None);
        // One message more than a page holds is another page, of that one.
        let (messages, cursor) = page(&mut conn, None, 5).await;
        assert_eq!(messages, [7, 6, 4, 3, 2]);
        let (messages, cursor) = page(&mut conn, cursor.as_deref(), 5).await;
        assert_eq!(messages, [1]);
        assert_eq!(cursor, None);
    }

    #[test]
    fn a_cursor_of_no_feed_is_refused() {
        let refused =
            |cursor: &str| matches!(activity_position(cursor), Err(crate::Error::Validation(_)));
        assert!(refused("garbage"));
        // A message's id as it is, which is not a cursor.
        assert!(refused(&message(1).0.to_string()));
        // A place of another shape, and a saved message list's cursor, whose place is an id
        // alone too.
        assert!(refused(&crate::cursor::encode(
            List::Activity,
            &(1_i64, message(1))
        )));
        assert!(refused(&crate::cursor::encode(
            List::SavedMessages,
            &message(1)
        )));
        assert!(!refused(&crate::cursor::encode(
            List::Activity,
            &message(1)
        )));
    }
}
