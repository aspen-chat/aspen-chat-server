//! Benchmark populations: written straight into the database by `bench seed`, removed by
//! `bench purge` (`crate::operator`).
//!
//! Seeding skips the API on purpose: registering a hundred thousand users one request at a time
//! would take hours and trip the registration limits. It writes what the API would, in batches,
//! and tags the run's users and communities in `benchmark_user` and `benchmark_community`.
//!
//! Purging removes a run and everything that came to depend on it, found from the database's
//! own foreign keys rather than a list here, so a table added later is covered without a change
//! to this file. It starts from the run's users and communities, plus any community or DM whose
//! every member is one of the run's users (the run made those), and follows every foreign key
//! from rows being removed to the rows that reference them. Nullable references that close a
//! cycle among the doomed rows are cleared first, which breaks every cycle (a thread and its
//! starter message name each other), and the rows are deleted children first. Every other
//! reference is left for the order of deletion to honour, since clearing it could break a rule of
//! its table (a notification setting names a community or a channel, never neither). Files in object storage that only those rows
//! used (attachments, icons, link preview images) are deleted after the transaction commits.

use crate::CHACHA_RNG;
use crate::api::ChannelType;
use crate::app::{self, media_store::MediaStore};
use crate::database::schema::{
    benchmark_community, benchmark_run, benchmark_user, channel, community, community_user,
    message, user,
};
use aspen_bench_protocol::{Manifest, SeedPlan, SeededCommunity, SeededUser, user_name};
use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, Bool, Text};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use rand::RngExt;
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

/// Rows per insert statement, well under Postgres's 65535 bind parameters.
const BATCH: usize = 2000;
/// How far back seeded history reaches.
const HISTORY_SPAN: Duration = Duration::days(7);

const HISTORY_LINES: [&str; 8] = [
    "Anyone around tonight?",
    "That patch notes thread is wild.",
    "I'll bring snacks next time.",
    "Can someone pin the schedule?",
    "Good game, everyone.",
    "Link's in the other channel.",
    "Back in five.",
    "Has anyone tried the new map yet?",
];

/// A UUIDv7 for `at`, so seeded history sorts by time as live messages do.
fn id_at(at: DateTime<Utc>) -> Uuid {
    let seconds = u64::try_from(at.timestamp()).unwrap_or(0);
    Uuid::new_v7(uuid::Timestamp::from_unix(
        uuid::NoContext,
        seconds,
        at.timestamp_subsec_nanos(),
    ))
}

fn random_below(n: usize) -> usize {
    CHACHA_RNG.with(|rng| rng.borrow_mut().random_range(0..n))
}

/// Writes `plan` in one transaction and says what was made.
pub async fn seed(
    conn: &mut AsyncPgConnection,
    plan: &SeedPlan,
    max_communities_per_user: u32,
) -> app::Result<Manifest> {
    plan.validate(max_communities_per_user)
        .map_err(|reason| app::Error::Validation(reason.into()))?;
    let password_hash = app::login::hash_password(plan.password.clone()).await?;
    let plan_json = serde_json::to_value(plan)?;
    let now = Utc::now();
    conn.transaction(|conn| {
        async move {
            diesel::insert_into(benchmark_run::table)
                .values((
                    benchmark_run::run.eq(&plan.run),
                    benchmark_run::plan.eq(&plan_json),
                ))
                .execute(conn)
                .await?;

            let users: Vec<SeededUser> = (0..plan.users)
                .map(|index| SeededUser {
                    id: Uuid::now_v7(),
                    name: user_name(&plan.run, index),
                })
                .collect();
            for chunk in users.chunks(BATCH) {
                let rows: Vec<_> = chunk
                    .iter()
                    .map(|u| {
                        (
                            user::id.eq(u.id),
                            user::name.eq(&u.name),
                            user::password_hash.eq(&password_hash),
                            user::created_at.eq(now),
                            user::last_seen_at.eq(now),
                        )
                    })
                    .collect();
                diesel::insert_into(user::table)
                    .values(rows)
                    .execute(conn)
                    .await?;
                let tags: Vec<_> = chunk
                    .iter()
                    .map(|u| {
                        (
                            benchmark_user::run.eq(&plan.run),
                            benchmark_user::user.eq(u.id),
                        )
                    })
                    .collect();
                diesel::insert_into(benchmark_user::table)
                    .values(tags)
                    .execute(conn)
                    .await?;
            }

            let mut communities = Vec::with_capacity(plan.communities.len());
            // Each user's communities are ordered as they were joined.
            let mut next_sort_index = vec![0i32; plan.users as usize];
            for (index, planned) in plan.communities.iter().enumerate() {
                let id = Uuid::now_v7();
                // Its first member owns it; everyone holds the default everyone role.
                let owner =
                    <[u32]>::first(&planned.members).map(|member| users[*member as usize].id);
                diesel::insert_into(community::table)
                    .values((
                        community::id.eq(id),
                        community::name.eq(format!("bench {} {index}", plan.run)),
                        community::owner.eq(owner),
                    ))
                    .execute(conn)
                    .await?;
                app::role::create_default_roles(conn, crate::app::CommunityId(id)).await?;
                diesel::insert_into(benchmark_community::table)
                    .values((
                        benchmark_community::run.eq(&plan.run),
                        benchmark_community::community.eq(id),
                    ))
                    .execute(conn)
                    .await?;
                for chunk in planned.members.chunks(BATCH) {
                    let rows: Vec<_> = chunk
                        .iter()
                        .map(|member| {
                            let slot = &mut next_sort_index[*member as usize];
                            let sort_index = *slot;
                            *slot += 1;
                            (
                                community_user::user.eq(users[*member as usize].id),
                                community_user::community.eq(id),
                                community_user::sort_index.eq(sort_index),
                            )
                        })
                        .collect();
                    diesel::insert_into(community_user::table)
                        .values(rows)
                        .execute(conn)
                        .await?;
                }
                let mut text_channels = Vec::new();
                let mut voice_channels = Vec::new();
                let channels: Vec<(Uuid, String, ChannelType, i32)> = (0..planned.text_channels)
                    .map(|n| (format!("text-{n}"), ChannelType::Text))
                    .chain(
                        (0..planned.voice_channels)
                            .map(|n| (format!("voice-{n}"), ChannelType::Voice)),
                    )
                    .enumerate()
                    .map(|(sort, (name, ty))| {
                        (
                            Uuid::now_v7(),
                            name,
                            ty,
                            i32::try_from(sort).unwrap_or(i32::MAX),
                        )
                    })
                    .collect();
                for (channel_id, name, ty, sort_index) in &channels {
                    diesel::insert_into(channel::table)
                        .values((
                            channel::id.eq(channel_id),
                            channel::community.eq(Some(id)),
                            channel::name.eq(name),
                            channel::ty.eq(ty),
                            channel::sort_index.eq(sort_index),
                        ))
                        .execute(conn)
                        .await?;
                    match ty {
                        ChannelType::Text => text_channels.push(*channel_id),
                        _ => voice_channels.push(*channel_id),
                    }
                }
                for channel_id in &text_channels {
                    seed_history(
                        conn,
                        *channel_id,
                        planned.history_per_channel,
                        &planned.members,
                        &users,
                        now,
                    )
                    .await?;
                }
                communities.push(SeededCommunity {
                    id,
                    members: planned.members.clone(),
                    text_channels,
                    voice_channels,
                });
            }
            Ok(Manifest {
                run: plan.run.clone(),
                password: plan.password.clone(),
                users,
                communities,
            })
        }
        .scope_boxed()
    })
    .await
}

async fn seed_history(
    conn: &mut AsyncPgConnection,
    channel_id: Uuid,
    count: u32,
    members: &[u32],
    users: &[SeededUser],
    now: DateTime<Utc>,
) -> app::Result<()> {
    if count == 0 {
        return Ok(());
    }
    let step = HISTORY_SPAN / i32::try_from(count).unwrap_or(i32::MAX);
    let start = now - HISTORY_SPAN;
    let messages: Vec<(Uuid, Uuid, String, DateTime<Utc>)> = (0..count)
        .map(|n| {
            let at = start + step * i32::try_from(n).unwrap_or(i32::MAX);
            let author = users[members[random_below(members.len())] as usize].id;
            let line = HISTORY_LINES[random_below(HISTORY_LINES.len())];
            (id_at(at), author, line.to_string(), at)
        })
        .collect();
    for chunk in messages.chunks(BATCH) {
        let rows: Vec<_> = chunk
            .iter()
            .map(|(id, author, content, at)| {
                (
                    message::id.eq(id),
                    message::author.eq(author),
                    message::channel.eq(channel_id),
                    message::content.eq(content),
                    message::timestamp.eq(at),
                )
            })
            .collect();
        diesel::insert_into(message::table)
            .values(rows)
            .execute(conn)
            .await?;
    }
    Ok(())
}

/// What a purge removed.
#[derive(Debug, Default)]
pub struct PurgeReport {
    /// Rows deleted, by table.
    pub rows: BTreeMap<String, usize>,
    /// Files deleted from object storage.
    pub objects: usize,
    /// Files that could not be deleted, left for an operator.
    pub failed_objects: Vec<String>,
}

#[derive(QueryableByName, Debug, Clone)]
struct ForeignKey {
    #[diesel(sql_type = Text)]
    child: String,
    #[diesel(sql_type = Text)]
    parent: String,
    #[diesel(sql_type = Array<Text>)]
    child_columns: Vec<String>,
    #[diesel(sql_type = Array<Text>)]
    parent_columns: Vec<String>,
    /// Every referencing column may be null, so the reference can be cleared.
    #[diesel(sql_type = Bool)]
    nullable: bool,
}

#[derive(QueryableByName, Debug)]
struct PrimaryKey {
    #[diesel(sql_type = Text)]
    table: String,
    #[diesel(sql_type = Array<Text>)]
    columns: Vec<String>,
}

#[derive(QueryableByName, Debug)]
struct StorageKey {
    #[diesel(sql_type = Text)]
    key: String,
}

fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// The temporary table marking a table's doomed rows by primary key.
fn marks(table: &str) -> String {
    quote(&format!("purge_{table}"))
}

fn column_list(prefix: &str, columns: &[String]) -> String {
    columns
        .iter()
        .map(|c| format!("{prefix}{}", quote(c)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn equal_pairs(
    left: &str,
    left_columns: &[String],
    right: &str,
    right_columns: &[String],
) -> String {
    left_columns
        .iter()
        .zip(right_columns)
        .map(|(l, r)| format!("{left}.{} = {right}.{}", quote(l), quote(r)))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// Rows of `table` whose key is marked.
fn marked(table: &str, keys: &[String], alias: &str) -> String {
    format!(
        "({}) IN (SELECT * FROM {})",
        column_list(&format!("{alias}."), keys),
        marks(table)
    )
}

/// Orders `tables` so every table comes before the tables it references through `edges`
/// (child, parent). `None` if they reference each other in a cycle.
fn children_first(tables: &HashSet<String>, edges: &[(String, String)]) -> Option<Vec<String>> {
    let mut remaining: HashSet<String> = tables.clone();
    let mut order = Vec::new();
    while !remaining.is_empty() {
        // A table may go once no remaining table references it.
        let ready: Vec<String> = remaining
            .iter()
            .filter(|table| {
                !edges.iter().any(|(child, parent)| {
                    parent == *table && child != *table && remaining.contains(child)
                })
            })
            .cloned()
            .collect();
        if ready.is_empty() {
            return None;
        }
        let mut ready = ready;
        ready.sort();
        for table in ready {
            remaining.remove(&table);
            order.push(table);
        }
    }
    Some(order)
}

/// Whether the reference `child` → `parent` closes a cycle: `parent` reaches `child` through
/// references, itself included.
fn closes_cycle(edges: &[(String, String)], child: &str, parent: &str) -> bool {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut next = vec![parent];
    while let Some(table) = next.pop() {
        if table == child {
            return true;
        }
        if seen.insert(table) {
            next.extend(
                edges
                    .iter()
                    .filter(|(from, _)| from == table)
                    .map(|(_, to)| to.as_str()),
            );
        }
    }
    false
}

/// Removes run `run` and everything depending on it.
pub async fn purge(
    conn: &mut AsyncPgConnection,
    media: &MediaStore,
    run: &str,
) -> app::Result<PurgeReport> {
    let exists = benchmark_run::table
        .filter(benchmark_run::run.eq(run))
        .count()
        .get_result::<i64>(conn)
        .await?;
    if exists == 0 {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    }
    let run = run.to_string();
    let (mut report, keys) = conn
        .transaction(|conn| async move { purge_rows(conn, &run).await }.scope_boxed())
        .await?;
    for key in keys {
        match media.delete(&key).await {
            Ok(()) => report.objects += 1,
            Err(e) => {
                tracing::warn!(key, error = %e, "could not delete a purged file");
                report.failed_objects.push(key);
            }
        }
    }
    Ok(report)
}

async fn purge_rows(
    conn: &mut AsyncPgConnection,
    run: &str,
) -> app::Result<(PurgeReport, Vec<String>)> {
    let foreign_keys: Vec<ForeignKey> = diesel::sql_query(
        "SELECT cl.relname::text AS child, pl.relname::text AS parent,
                array_agg(ca.attname::text ORDER BY k.ord) AS child_columns,
                array_agg(pa.attname::text ORDER BY k.ord) AS parent_columns,
                bool_and(NOT ca.attnotnull) AS nullable
         FROM pg_constraint c
         JOIN pg_class cl ON cl.oid = c.conrelid
         JOIN pg_class pl ON pl.oid = c.confrelid
         JOIN pg_namespace n ON n.oid = cl.relnamespace
         CROSS JOIN LATERAL unnest(c.conkey, c.confkey) WITH ORDINALITY AS k(child_att, parent_att, ord)
         JOIN pg_attribute ca ON ca.attrelid = c.conrelid AND ca.attnum = k.child_att
         JOIN pg_attribute pa ON pa.attrelid = c.confrelid AND pa.attnum = k.parent_att
         WHERE c.contype = 'f' AND n.nspname = 'public'
         GROUP BY c.oid, cl.relname, pl.relname",
    )
    .load(conn)
    .await?;
    let primary_keys: HashMap<String, Vec<String>> = diesel::sql_query(
        "SELECT cl.relname::text AS table, array_agg(a.attname::text ORDER BY k.ord) AS columns
         FROM pg_index i
         JOIN pg_class cl ON cl.oid = i.indrelid
         JOIN pg_namespace n ON n.oid = cl.relnamespace
         CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(att, ord)
         JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.att
         WHERE i.indisprimary AND n.nspname = 'public'
         GROUP BY cl.relname",
    )
    .load::<PrimaryKey>(conn)
    .await?
    .into_iter()
    .map(|pk| (pk.table, pk.columns))
    .collect();
    let key_of = |table: &str| -> app::Result<&Vec<String>> {
        primary_keys.get(table).ok_or_else(|| {
            app::Error::Validation(format!("table {table} has no primary key to purge by").into())
        })
    };

    // Every table that can take part gets a marks table.
    let mut tables: HashSet<String> = HashSet::new();
    for fk in &foreign_keys {
        tables.insert(fk.child.clone());
        tables.insert(fk.parent.clone());
    }
    // Icons are named by `user.icon` and `community.icon` without a foreign key, and by
    // `custom_emoji.icon` with one; they are purged with the users and communities that used
    // them.
    tables.insert("icon".to_string());
    for table in &tables {
        let key = key_of(table)?;
        diesel::sql_query(format!(
            "CREATE TEMP TABLE {} ON COMMIT DROP AS SELECT {} FROM {} WITH NO DATA",
            marks(table),
            column_list("", key),
            quote(table)
        ))
        .execute(conn)
        .await?;
    }

    let seed = |sql: &str| diesel::sql_query(sql.to_string()).bind::<Text, _>(run.to_string());
    seed("INSERT INTO \"purge_benchmark_run\" SELECT run FROM benchmark_run WHERE run = $1")
        .execute(conn)
        .await?;
    seed(r#"INSERT INTO "purge_user" SELECT "user" FROM benchmark_user WHERE run = $1"#)
        .execute(conn)
        .await?;
    seed(
        r#"INSERT INTO "purge_community"
           SELECT community FROM benchmark_community WHERE run = $1
           UNION
           SELECT c.id FROM community c
           WHERE EXISTS (SELECT 1 FROM community_user cu WHERE cu.community = c.id)
             AND NOT EXISTS (
               SELECT 1 FROM community_user cu
               WHERE cu.community = c.id
                 AND cu."user" NOT IN (SELECT "user" FROM benchmark_user WHERE run = $1))"#,
    )
    .execute(conn)
    .await?;
    seed(
        r#"INSERT INTO "purge_channel"
           SELECT ch.id FROM channel ch
           WHERE ch.community IS NULL AND ch.parent_channel IS NULL
             AND EXISTS (SELECT 1 FROM dm_recipient r WHERE r.channel = ch.id)
             AND NOT EXISTS (
               SELECT 1 FROM dm_recipient r
               WHERE r.channel = ch.id
                 AND r."user" NOT IN (SELECT "user" FROM benchmark_user WHERE run = $1))"#,
    )
    .execute(conn)
    .await?;

    close_over(conn, &foreign_keys, &primary_keys).await?;

    // Files only doomed rows use, found before the rows go.
    let mut storage_keys: Vec<String> = Vec::new();
    diesel::sql_query(
        r#"INSERT INTO "purge_attachment"
           SELECT DISTINCT ma.attachment_id FROM message_attachment ma
           WHERE (ma.message_id, ma.attachment_id) IN (SELECT * FROM "purge_message_attachment")
             AND NOT EXISTS (
               SELECT 1 FROM message_attachment other
               WHERE other.attachment_id = ma.attachment_id
                 AND (other.message_id, other.attachment_id)
                     NOT IN (SELECT * FROM "purge_message_attachment"))
           EXCEPT SELECT * FROM "purge_attachment""#,
    )
    .execute(conn)
    .await?;
    storage_keys.extend(
        diesel::sql_query(
            r#"SELECT storage_key AS key FROM attachment WHERE id IN (SELECT * FROM "purge_attachment")"#,
        )
        .load::<StorageKey>(conn)
        .await?
        .into_iter()
        .map(|k| k.key),
    );
    diesel::sql_query(
        r#"INSERT INTO "purge_icon"
           SELECT icon FROM (
             SELECT icon FROM "user" WHERE id IN (SELECT * FROM "purge_user") AND icon IS NOT NULL
             UNION
             SELECT icon FROM community WHERE id IN (SELECT * FROM "purge_community") AND icon IS NOT NULL
             UNION
             SELECT icon FROM custom_emoji WHERE community IN (SELECT * FROM "purge_community")
           ) used
           WHERE icon NOT IN (
             SELECT icon FROM "user" WHERE icon IS NOT NULL AND id NOT IN (SELECT * FROM "purge_user")
             UNION
             SELECT icon FROM community WHERE icon IS NOT NULL AND id NOT IN (SELECT * FROM "purge_community")
             UNION
             SELECT icon FROM custom_emoji WHERE community NOT IN (SELECT * FROM "purge_community"))
           EXCEPT SELECT * FROM "purge_icon""#,
    )
    .execute(conn)
    .await?;
    storage_keys.extend(
        diesel::sql_query(
            r#"SELECT storage_key AS key FROM icon WHERE id IN (SELECT * FROM "purge_icon")"#,
        )
        .load::<StorageKey>(conn)
        .await?
        .into_iter()
        .map(|k| k.key),
    );
    let preview_images: Vec<StorageKey> = diesel::sql_query(
        r#"SELECT image_id::text AS key FROM message_link_preview
           WHERE image_id IS NOT NULL
             AND (message_id, position) IN (SELECT * FROM "purge_message_link_preview")"#,
    )
    .load(conn)
    .await?;
    storage_keys.extend(preview_images.into_iter().filter_map(|k| {
        k.key
            .parse::<Uuid>()
            .ok()
            .map(|id| crate::api::link_preview::image_storage_key(app::LinkPreviewImageId(id)))
    }));

    // Clear the references that close a cycle among the doomed rows, which breaks every cycle.
    let references: Vec<(String, String)> = foreign_keys
        .iter()
        .map(|fk| (fk.child.clone(), fk.parent.clone()))
        .collect();
    let cleared = |fk: &ForeignKey| fk.nullable && closes_cycle(&references, &fk.child, &fk.parent);
    for fk in foreign_keys.iter().filter(|fk| cleared(fk)) {
        let child_key = key_of(&fk.child)?;
        let assignments = fk
            .child_columns
            .iter()
            .map(|c| format!("{} = NULL", quote(c)))
            .collect::<Vec<_>>()
            .join(", ");
        diesel::sql_query(format!(
            "UPDATE {} AS t SET {assignments} WHERE {}",
            quote(&fk.child),
            marked(&fk.child, child_key, "t")
        ))
        .execute(conn)
        .await?;
    }

    let edges: Vec<(String, String)> = foreign_keys
        .iter()
        .filter(|fk| !cleared(fk))
        .map(|fk| (fk.child.clone(), fk.parent.clone()))
        .collect();
    let order = children_first(&tables, &edges).ok_or_else(|| {
        app::Error::Validation("tables reference each other through required columns".into())
    })?;
    let mut report = PurgeReport::default();
    for table in order {
        let key = key_of(&table)?;
        let deleted = diesel::sql_query(format!(
            "DELETE FROM {} AS t WHERE {}",
            quote(&table),
            marked(&table, key, "t")
        ))
        .execute(conn)
        .await?;
        if deleted > 0 {
            report.rows.insert(table, deleted);
        }
    }
    Ok((report, storage_keys))
}

/// Marks every row that references a marked row, until nothing more is found.
async fn close_over(
    conn: &mut AsyncPgConnection,
    foreign_keys: &[ForeignKey],
    primary_keys: &HashMap<String, Vec<String>>,
) -> app::Result<()> {
    loop {
        let mut added = 0;
        for fk in foreign_keys {
            let (Some(child_key), Some(parent_key)) =
                (primary_keys.get(&fk.child), primary_keys.get(&fk.parent))
            else {
                continue;
            };
            added += diesel::sql_query(format!(
                "INSERT INTO {child_marks}
                 SELECT DISTINCT {child_columns} FROM {child} c
                 JOIN {parent} p ON {join}
                 WHERE {parent_marked}
                 EXCEPT SELECT * FROM {child_marks}",
                child_marks = marks(&fk.child),
                child_columns = column_list("c.", child_key),
                child = quote(&fk.child),
                parent = quote(&fk.parent),
                join = equal_pairs("c", &fk.child_columns, "p", &fk.parent_columns),
                parent_marked = marked(&fk.parent, parent_key, "p"),
            ))
            .execute(conn)
            .await?;
        }
        if added == 0 {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_are_deleted_before_their_parents() {
        let tables: HashSet<String> = ["user", "message", "react", "channel"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let edges = vec![
            ("message".to_string(), "user".to_string()),
            ("message".to_string(), "channel".to_string()),
            ("react".to_string(), "message".to_string()),
            ("react".to_string(), "user".to_string()),
            // A table referencing itself does not hold itself up.
            ("channel".to_string(), "channel".to_string()),
        ];
        let order = children_first(&tables, &edges).unwrap();
        let at = |t: &str| order.iter().position(|x| x == t).unwrap();
        assert!(at("react") < at("message"));
        assert!(at("message") < at("user"));
        assert!(at("message") < at("channel"));
    }

    #[test]
    fn only_references_that_close_a_cycle_are_cleared() {
        let edges: Vec<(String, String)> = [
            ("channel", "message"),
            ("message", "channel"),
            ("message", "message"),
            ("notification_setting", "channel"),
        ]
        .iter()
        .map(|(c, p)| (c.to_string(), p.to_string()))
        .collect();
        assert!(closes_cycle(&edges, "channel", "message"));
        assert!(closes_cycle(&edges, "message", "message"));
        assert!(!closes_cycle(&edges, "notification_setting", "channel"));
    }

    #[test]
    fn required_cycles_are_refused() {
        let tables: HashSet<String> = ["a", "b"].iter().map(|s| s.to_string()).collect();
        let edges = vec![
            ("a".to_string(), "b".to_string()),
            ("b".to_string(), "a".to_string()),
        ];
        assert!(children_first(&tables, &edges).is_none());
    }

    #[test]
    fn identifiers_are_quoted() {
        assert_eq!(quote("user"), "\"user\"");
        assert_eq!(quote("we\"ird"), "\"we\"\"ird\"");
        assert_eq!(marks("user"), "\"purge_user\"");
    }

    #[test]
    fn history_ids_sort_by_time() {
        let now = Utc::now();
        assert!(id_at(now - Duration::hours(1)) < id_at(now));
    }
}
