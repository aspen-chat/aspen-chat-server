//! The Administration Dashboard's reads: who may open it, the deployment's totals and their
//! growth, and searchable, sortable lists of its users and communities. Registration invites are
//! `app::registration_invite`, and fleet health `app::fleet`.
//!
//! Who may read them is a deployment permission (`app::deployment`), which community roles
//! never reach, since a dashboard anyone could open would let anyone mint the invites an
//! invite-only deployment is closed by.

use crate::context::GlobalServerContext;
use crate::t;
use crate::{CommunityId, IconId, UserId};
use aspen_schema::user;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text, Timestamptz, Uuid as PgUuid};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

/// The most rows one page of a list returns.
pub const MAX_PAGE: i64 = 100;
/// The furthest into a list a page may start. Pages are counted by offset so any column can
/// sort them, each through an index in its order, which still costs a walk past the rows
/// skipped; searching narrows a long list faster.
pub const MAX_OFFSET: i64 = 10_000;

/// How a list is ordered: by a column, ascending or descending, then by id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sort<C> {
    pub column: C,
    pub descending: bool,
}

/// The columns the user list sorts by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserColumn {
    /// The display name, or the username where there is none.
    Name,
    Joined,
}

/// The columns the community list sorts by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommunityColumn {
    Name,
    Members,
    /// When it was made, which its UUIDv7 id orders.
    Created,
}

fn order_by(column: &str, descending: bool, id: &str) -> String {
    let direction = if descending { "DESC" } else { "ASC" };
    format!("{column} {direction}, {id} {direction}")
}

/// The deployment's totals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Overview {
    /// The deployment's people, leaving out the system account (`app::system_account`), which
    /// is the deployment itself.
    pub users: i64,
    /// Accounts made in the last seven days.
    pub new_users_this_week: i64,
    pub communities: i64,
}

pub async fn overview(state: &GlobalServerContext) -> crate::Result<Overview> {
    let mut conn = state.connection_pool.get().await?;
    let totals = current_totals(conn.as_mut()).await?;
    let new_users_this_week: i64 = user::table
        .filter(user::deleted_at.is_null())
        .filter(user::system.eq(false))
        .filter(user::created_at.gt(Utc::now() - chrono::Duration::days(7)))
        .count()
        .get_result(conn.as_mut())
        .await?;
    Ok(Overview {
        users: totals.users,
        new_users_this_week,
        communities: totals.communities,
    })
}

/// The deployment's people and communities now, neither deleted, the system account left out.
#[derive(Debug, Clone, QueryableByName)]
pub struct Totals {
    #[diesel(sql_type = Timestamptz)]
    pub at: DateTime<Utc>,
    #[diesel(sql_type = BigInt)]
    pub users: i64,
    #[diesel(sql_type = BigInt)]
    pub communities: i64,
}

/// The totals now: the latest row of `deployment_stats`, which `recordStats` writes each hour,
/// with what was made and deleted since it was taken, each found through an index of its own
/// (`user_created`, `user_deleted`, `community`'s UUIDv7 ids, `community_deleted`), so the cost
/// follows what changed in an hour, not the size of the deployment. Without a row yet, it counts
/// from the beginning.
pub async fn current_totals(conn: &mut AsyncPgConnection) -> crate::Result<Totals> {
    Ok(diesel::sql_query(
        r#"
        WITH last AS (
            SELECT taken_at, users, communities FROM deployment_stats ORDER BY day DESC LIMIT 1
        ),
        since AS (
            SELECT COALESCE((SELECT taken_at FROM last), '-infinity') AS at
        )
        SELECT now() AS at,
               COALESCE((SELECT users FROM last), 0)
               + (SELECT count(*) FROM "user"
                  WHERE NOT system AND created_at > (SELECT at FROM since)
                    AND deleted_at IS NULL)
               - (SELECT count(*) FROM "user"
                  WHERE NOT system AND deleted_at > (SELECT at FROM since)
                    AND created_at <= (SELECT at FROM since)) AS users,
               COALESCE((SELECT communities FROM last), 0)
               + (SELECT count(*) FROM community
                  WHERE id >= aspen_uuid_floor((SELECT at FROM since)) AND deleted_at IS NULL)
               - (SELECT count(*) FROM community
                  WHERE deleted_at > (SELECT at FROM since)
                    AND id < aspen_uuid_floor((SELECT at FROM since))) AS communities
        "#,
    )
    .get_result(conn)
    .await?)
}

/// The longest name search, in characters, longer than any name it could match a part of.
pub const MAX_SEARCH_CHARS: usize = 100;

/// The fewest characters a search finds anywhere in a name; a shorter one finds names that
/// start with it. A trigram index (`pg_trgm`) finds a part of a name only from three
/// characters, but finds a start of one from the first, so every search is served by one.
pub const MIN_CONTAINS_CHARS: usize = 3;

/// `text` as a `LIKE` pattern matching any name containing it, or, for fewer than
/// [`MIN_CONTAINS_CHARS`] characters, starting with it, with the pattern's own special
/// characters escaped; `None` for an empty search. One of more than [`MAX_SEARCH_CHARS`] is
/// refused.
///
/// The pattern is in the case `text` was written in. A query that ignores case folds it with
/// the function that folded what it is compared with, the database's (`lower(name) LIKE
/// lower($1)`): Rust lowers some letters otherwise than PostgreSQL does (a word's last sigma,
/// a dotted capital I, and under some collations every letter outside ASCII), and a pattern
/// lowered here would not find the names holding them.
pub fn like_pattern(text: Option<&str>) -> crate::Result<Option<String>> {
    let Some(text) = text else {
        return Ok(None);
    };
    let text = text.trim();
    let chars = text.chars().count();
    if chars > MAX_SEARCH_CHARS {
        return Err(crate::Error::Validation(t!(
            "searchTooLong",
            max = MAX_SEARCH_CHARS
        )));
    }
    if text.is_empty() {
        return Ok(None);
    }
    let escaped: String = text
        .chars()
        .flat_map(|c| match c {
            '\\' | '%' | '_' => vec!['\\', c],
            other => vec![other],
        })
        .collect();
    if chars < MIN_CONTAINS_CHARS {
        Ok(Some(format!("{escaped}%")))
    } else {
        Ok(Some(format!("%{escaped}%")))
    }
}

/// A user as the dashboard lists them.
#[derive(Debug, Clone, QueryableByName)]
pub struct UserEntry {
    #[diesel(sql_type = PgUuid)]
    pub id: UserId,
    #[diesel(sql_type = Text)]
    pub name: String,
    #[diesel(sql_type = Nullable<Text>)]
    pub display_name: Option<String>,
    #[diesel(sql_type = Nullable<PgUuid>)]
    pub icon: Option<IconId>,
    #[diesel(sql_type = Timestamptz)]
    pub created_at: DateTime<Utc>,
    /// The registration invite the account was made with, if one was.
    #[diesel(sql_type = Nullable<Text>)]
    pub registered_with: Option<String>,
    #[diesel(sql_type = diesel::sql_types::Bool)]
    pub bot: bool,
    #[diesel(sql_type = Nullable<PgUuid>)]
    pub bot_owner: Option<UserId>,
    /// For a user of another deployment, that deployment.
    #[diesel(sql_type = Nullable<Text>)]
    pub home_domain: Option<String>,
    /// The ban from the deployment standing now, if one does (`app::user_ban`): when it was
    /// made, the reason they were given, and when it ends.
    #[diesel(sql_type = Nullable<Timestamptz>)]
    pub banned_at: Option<DateTime<Utc>>,
    #[diesel(sql_type = Nullable<Text>)]
    pub ban_reason: Option<String>,
    #[diesel(sql_type = Nullable<Timestamptz>)]
    pub banned_until: Option<DateTime<Utc>>,
}

/// One page of the users whose username or display name contains `search`, and with
/// `banned_only` only those banned from the deployment now, in `sort` order: `limit` of them
/// (at most `MAX_PAGE`) from `offset` (at most `MAX_OFFSET`). The system account
/// (`app::system_account`) is left out: it is the deployment itself, nobody to moderate.
pub async fn search_users(
    state: &GlobalServerContext,
    search: Option<&str>,
    banned_only: bool,
    sort: Sort<UserColumn>,
    offset: i64,
    limit: i64,
) -> crate::Result<Vec<UserEntry>> {
    let mut conn = state.connection_pool.get().await?;
    let column = match sort.column {
        UserColumn::Name => "lower(COALESCE(display_name, name))",
        // UUIDv7 ids order by creation, through the key.
        UserColumn::Joined => "id",
    };
    Ok(diesel::sql_query(format!(
        r#"
        SELECT id, name, display_name, icon, created_at, registered_with, bot, bot_owner,
               home_domain,
               CASE WHEN {banned} THEN banned_at END AS banned_at,
               CASE WHEN {banned} THEN ban_reason END AS ban_reason,
               CASE WHEN {banned} THEN banned_until END AS banned_until
        FROM "user"
        WHERE deleted_at IS NULL
          AND NOT system
          AND ($1::text IS NULL
               OR lower(name) LIKE lower($1) OR lower(display_name) LIKE lower($1))
          AND (NOT $4 OR {banned})
        ORDER BY {order}
        OFFSET $2 LIMIT $3
        "#,
        banned = crate::user_ban::BANNED_SQL,
        order = order_by(column, sort.descending, "id")
    ))
    .bind::<Nullable<Text>, _>(like_pattern(search)?)
    .bind::<BigInt, _>(offset.clamp(0, MAX_OFFSET))
    .bind::<BigInt, _>(limit.clamp(1, MAX_PAGE))
    .bind::<diesel::sql_types::Bool, _>(banned_only)
    .load(conn.as_mut())
    .await?)
}

/// A community as the dashboard lists it.
#[derive(Debug, Clone, QueryableByName)]
pub struct CommunityEntry {
    #[diesel(sql_type = PgUuid)]
    pub id: CommunityId,
    #[diesel(sql_type = Text)]
    pub name: String,
    #[diesel(sql_type = Nullable<PgUuid>)]
    pub icon: Option<IconId>,
    #[diesel(sql_type = BigInt)]
    pub members: i64,
    #[diesel(sql_type = Timestamptz)]
    pub created_at: DateTime<Utc>,
}

/// One page of the communities whose name contains `search`, in `sort` order: `limit` of them
/// (at most `MAX_PAGE`) from `offset` (at most `MAX_OFFSET`).
pub async fn search_communities(
    state: &GlobalServerContext,
    search: Option<&str>,
    sort: Sort<CommunityColumn>,
    offset: i64,
    limit: i64,
) -> crate::Result<Vec<CommunityEntry>> {
    let mut conn = state.connection_pool.get().await?;
    let column = match sort.column {
        CommunityColumn::Name => "lower(name)",
        // Recounted by `recountMembers`, and sorted through `community_by_members`.
        CommunityColumn::Members => "member_count",
        // UUIDv7 ids order by creation.
        CommunityColumn::Created => "id",
    };
    Ok(diesel::sql_query(format!(
        r#"
        SELECT * FROM (
            SELECT c.id, c.name, c.icon,
                   c.member_count::bigint AS members,
                   {COMMUNITY_CREATED_AT} AS created_at
            FROM community c
            WHERE c.deleted_at IS NULL
              AND ($1::text IS NULL OR lower(c.name) LIKE lower($1))
        ) listed
        ORDER BY {}
        OFFSET $2 LIMIT $3
        "#,
        order_by(column, sort.descending, "id")
    ))
    .bind::<Nullable<Text>, _>(like_pattern(search)?)
    .bind::<BigInt, _>(offset.clamp(0, MAX_OFFSET))
    .bind::<BigInt, _>(limit.clamp(1, MAX_PAGE))
    .load(conn.as_mut())
    .await?)
}

/// How far back the growth charts reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrowthRange {
    ThreeMonths,
    SixMonths,
    OneYear,
    FiveYears,
    AllTime,
}

/// The size of each step of a growth series.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrowthUnit {
    Day,
    Week,
    Month,
}

impl GrowthUnit {
    fn sql(self) -> &'static str {
        match self {
            GrowthUnit::Day => "day",
            GrowthUnit::Week => "week",
            GrowthUnit::Month => "month",
        }
    }

    /// The unit that keeps a span of `days` to a readable number of points: days up to half a
    /// year (however long its months), weeks up to two years, months beyond.
    fn for_span(days: i64) -> Self {
        if days <= 190 {
            GrowthUnit::Day
        } else if days <= 730 {
            GrowthUnit::Week
        } else {
            GrowthUnit::Month
        }
    }
}

/// How many users and communities there were at the end of one step.
#[derive(Debug, Clone, QueryableByName)]
pub struct GrowthPoint {
    /// When the step began.
    #[diesel(sql_type = Timestamptz)]
    pub at: DateTime<Utc>,
    #[diesel(sql_type = BigInt)]
    pub users: i64,
    #[diesel(sql_type = BigInt)]
    pub communities: i64,
}

#[derive(QueryableByName)]
struct Earliest {
    #[diesel(sql_type = Nullable<Timestamptz>)]
    earliest: Option<DateTime<Utc>>,
}

/// A community's creation time, read from its UUIDv7 id, whose first 48 bits are the
/// milliseconds it was made; communities keep no time of their own.
const COMMUNITY_CREATED_AT: &str = "to_timestamp(('x' || lpad(substr(replace(id::text, '-', ''), 1, 12), 16, '0'))::bit(64)::bigint / 1000.0)";

/// How many users and communities there were at each step of `range`, as `overview` counts
/// them: each step's figure is that of the last day recorded before it ends
/// (`deployment_stats`, one row a day), and the last step's is the totals now. The steps are
/// days, weeks, or months (`GrowthUnit::for_span`), so a range is a few dozen to a few hundred
/// points, each found through the table's key.
pub async fn growth(
    state: &GlobalServerContext,
    range: GrowthRange,
) -> crate::Result<(GrowthUnit, Vec<GrowthPoint>)> {
    let mut conn = state.connection_pool.get().await?;
    let now = Utc::now();
    let start = match range {
        GrowthRange::ThreeMonths => now - chrono::Months::new(3),
        GrowthRange::SixMonths => now - chrono::Months::new(6),
        GrowthRange::OneYear => now - chrono::Months::new(12),
        GrowthRange::FiveYears => now - chrono::Months::new(60),
        GrowthRange::AllTime => {
            let earliest: Earliest = diesel::sql_query(
                "SELECT (SELECT min(day) FROM deployment_stats)::timestamp AT TIME ZONE 'UTC' \
                 AS earliest",
            )
            .get_result(conn.as_mut())
            .await?;
            earliest.earliest.unwrap_or(now)
        }
    };
    let unit = GrowthUnit::for_span((now - start).num_days());
    let totals = current_totals(conn.as_mut()).await?;
    let mut points = growth_steps(conn.as_mut(), unit, start).await?;
    if let Some(last) = points.last_mut() {
        last.users = totals.users;
        last.communities = totals.communities;
    }
    Ok((unit, points))
}

/// The steps of `unit` from the one `start` falls in to the one now falls in, each with the
/// figures of the last day recorded before it ends. A step begins where its day, week, or month
/// does in UTC, as `deployment_stats` counts its days: the steps are made and compared as UTC
/// wall-clock times, so the connection's time zone, and its daylight saving, move none of them.
async fn growth_steps(
    conn: &mut AsyncPgConnection,
    unit: GrowthUnit,
    start: DateTime<Utc>,
) -> crate::Result<Vec<GrowthPoint>> {
    Ok(diesel::sql_query(
        r#"
        WITH steps AS (
            SELECT generate_series(date_trunc($1, $2::timestamptz AT TIME ZONE 'UTC'),
                                   date_trunc($1, now() AT TIME ZONE 'UTC'),
                                   ('1 ' || $1)::interval) AS at
        )
        SELECT steps.at AT TIME ZONE 'UTC' AS at,
               COALESCE(d.users, 0) AS users, COALESCE(d.communities, 0) AS communities
        FROM steps
        LEFT JOIN LATERAL (
            SELECT users, communities FROM deployment_stats
            WHERE day < (steps.at + ('1 ' || $1)::interval)::date
            ORDER BY day DESC LIMIT 1
        ) d ON true
        ORDER BY steps.at
        "#,
    )
    .bind::<Text, _>(unit.sql())
    .bind::<Timestamptz, _>(start)
    .load(conn)
    .await?)
}

#[cfg(test)]
mod tests {
    use super::{GrowthUnit, MAX_SEARCH_CHARS, like_pattern};

    #[test]
    fn a_growth_series_steps_by_days_weeks_or_months_as_it_lengthens() {
        assert_eq!(GrowthUnit::for_span(92), GrowthUnit::Day);
        assert_eq!(GrowthUnit::for_span(184), GrowthUnit::Day);
        assert_eq!(GrowthUnit::for_span(366), GrowthUnit::Week);
        assert_eq!(GrowthUnit::for_span(1827), GrowthUnit::Month);
    }

    #[test]
    fn a_search_matches_anywhere_and_takes_like_characters_literally() {
        assert_eq!(
            like_pattern(Some(" Kate ")).unwrap(),
            Some("%Kate%".to_string())
        );
        assert_eq!(like_pattern(Some("Ka")).unwrap(), Some("Ka%".to_string()));
        assert_eq!(like_pattern(Some("  ")).unwrap(), None);
        assert_eq!(like_pattern(None).unwrap(), None);
        assert_eq!(
            like_pattern(Some("50%_\\")).unwrap(),
            Some("%50\\%\\_\\\\%".to_string())
        );
        assert!(like_pattern(Some(&"名".repeat(MAX_SEARCH_CHARS))).is_ok());
        assert!(like_pattern(Some(&"名".repeat(MAX_SEARCH_CHARS + 1))).is_err());
        // The case is the database's to fold, which lowers a last sigma as any other.
        assert_eq!(
            like_pattern(Some("ΟΔΟΣ")).unwrap(),
            Some("%ΟΔΟΣ%".to_string())
        );
    }

    /// Runs against the database `DATABASE_URL` names, inside a transaction that is never
    /// committed; without one it checks nothing. On a connection whose time zone is hours from
    /// UTC and changed its clocks within the range, every step still begins at a UTC midnight,
    /// a day after the one before, and takes the figures of the UTC day it covers.
    #[tokio::test]
    async fn growth_steps_begin_at_utc_midnights_whatever_the_connections_time_zone() {
        use chrono::{Duration, NaiveTime, Utc};
        use diesel_async::{AsyncConnection, SimpleAsyncConnection};
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(
            "SET LOCAL TIME ZONE 'America/Los_Angeles';
             DELETE FROM deployment_stats;
             INSERT INTO deployment_stats (day, taken_at, users, communities)
                 VALUES ((now() AT TIME ZONE 'UTC')::date - 1, now(), 7, 3);",
        )
        .await
        .unwrap();
        let start = Utc::now() - Duration::days(400);
        let points = super::growth_steps(&mut conn, GrowthUnit::Day, start)
            .await
            .unwrap();
        assert_eq!(points.len(), 401);
        assert_eq!(points[0].at.date_naive(), start.date_naive());
        for pair in points.windows(2) {
            assert_eq!(pair[0].at.time(), NaiveTime::MIN);
            assert_eq!(pair[1].at - pair[0].at, Duration::days(1));
        }
        // Yesterday's row is the figure of yesterday's step and of today's, and of none before.
        let users: Vec<i64> = points.iter().rev().take(3).map(|p| p.users).collect();
        assert_eq!(users, vec![7, 7, 0]);
        let months = super::growth_steps(&mut conn, GrowthUnit::Month, start)
            .await
            .unwrap();
        assert!(months.iter().all(|p| {
            p.at.time() == NaiveTime::MIN && chrono::Datelike::day(&p.at.date_naive()) == 1
        }));
    }
}
