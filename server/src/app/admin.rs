//! The Administration Dashboard's reads: who may open it, the deployment's totals and their
//! growth, and searchable, sortable lists of its users and communities. Registration invites are
//! `app::registration_invite`, and fleet health `app::fleet`.
//!
//! Who may read them is a deployment permission (`app::deployment`), which community roles
//! never reach, since a dashboard anyone could open would let anyone mint the invites an
//! invite-only deployment is closed by.

use crate::api::GlobalServerContext;
use crate::app::{self, CommunityId, IconId, UserId};
use crate::database::schema::{community, user};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Nullable, Text, Timestamptz, Uuid as PgUuid};
use diesel_async::RunQueryDsl;

/// The most rows one page of a list returns.
pub const MAX_PAGE: i64 = 100;
/// The furthest into a list a page may start. Pages are counted by offset so any column can
/// sort them, which costs a scan of the rows skipped; searching narrows a long list faster.
pub const MAX_OFFSET: i64 = 100_000;

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
    pub users: i64,
    /// Accounts made in the last seven days.
    pub new_users_this_week: i64,
    pub communities: i64,
}

pub async fn overview(state: &GlobalServerContext) -> app::Result<Overview> {
    let mut conn = state.connection_pool.get().await?;
    let users: i64 = user::table
        .filter(user::deleted_at.is_null())
        .count()
        .get_result(conn.as_mut())
        .await?;
    let new_users_this_week: i64 = user::table
        .filter(user::deleted_at.is_null())
        .filter(user::created_at.gt(Utc::now() - chrono::Duration::days(7)))
        .count()
        .get_result(conn.as_mut())
        .await?;
    let communities: i64 = community::table
        .filter(community::deleted_at.is_null())
        .count()
        .get_result(conn.as_mut())
        .await?;
    Ok(Overview {
        users,
        new_users_this_week,
        communities,
    })
}

/// `text` as a case-insensitive `LIKE` pattern matching any name containing it, with the
/// pattern's own special characters escaped; `None` for an empty search.
pub(crate) fn contains_pattern(text: Option<&str>) -> Option<String> {
    let text = text?.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    let escaped: String = text
        .chars()
        .flat_map(|c| match c {
            '\\' | '%' | '_' => vec!['\\', c],
            other => vec![other],
        })
        .collect();
    Some(format!("%{escaped}%"))
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
}

/// One page of the users whose username or display name contains `search`, in `sort` order:
/// `limit` of them (at most `MAX_PAGE`) from `offset` (at most `MAX_OFFSET`).
pub async fn search_users(
    state: &GlobalServerContext,
    search: Option<&str>,
    sort: Sort<UserColumn>,
    offset: i64,
    limit: i64,
) -> app::Result<Vec<UserEntry>> {
    let mut conn = state.connection_pool.get().await?;
    let column = match sort.column {
        UserColumn::Name => "lower(COALESCE(display_name, name))",
        UserColumn::Joined => "created_at",
    };
    Ok(diesel::sql_query(format!(
        r#"
        SELECT id, name, display_name, icon, created_at, registered_with
        FROM "user"
        WHERE deleted_at IS NULL
          AND ($1::text IS NULL OR lower(name) LIKE $1 OR lower(display_name) LIKE $1)
        ORDER BY {}
        OFFSET $2 LIMIT $3
        "#,
        order_by(column, sort.descending, "id")
    ))
    .bind::<Nullable<Text>, _>(contains_pattern(search))
    .bind::<BigInt, _>(offset.clamp(0, MAX_OFFSET))
    .bind::<BigInt, _>(limit.clamp(1, MAX_PAGE))
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
) -> app::Result<Vec<CommunityEntry>> {
    let mut conn = state.connection_pool.get().await?;
    let column = match sort.column {
        CommunityColumn::Name => "lower(name)",
        CommunityColumn::Members => "members",
        // UUIDv7 ids order by creation.
        CommunityColumn::Created => "id",
    };
    Ok(diesel::sql_query(format!(
        r#"
        SELECT * FROM (
            SELECT c.id, c.name, c.icon,
                   (SELECT count(*) FROM community_user cu WHERE cu.community = c.id) AS members,
                   {COMMUNITY_CREATED_AT} AS created_at
            FROM community c
            WHERE c.deleted_at IS NULL
              AND ($1::text IS NULL OR lower(c.name) LIKE $1)
        ) listed
        ORDER BY {}
        OFFSET $2 LIMIT $3
        "#,
        order_by(column, sort.descending, "id")
    ))
    .bind::<Nullable<Text>, _>(contains_pattern(search))
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

/// How many users and communities there were at each step of `range`, counting each from its
/// creation until its deletion. The steps are days, weeks, or months (`GrowthUnit::for_span`),
/// so a range is a few dozen to a few hundred points. It reads the whole of both tables, once,
/// as events of +1 and -1 grouped by step, and sums them in order.
pub async fn growth(
    state: &GlobalServerContext,
    range: GrowthRange,
) -> app::Result<(GrowthUnit, Vec<GrowthPoint>)> {
    let mut conn = state.connection_pool.get().await?;
    let now = Utc::now();
    let start = match range {
        GrowthRange::ThreeMonths => now - chrono::Months::new(3),
        GrowthRange::SixMonths => now - chrono::Months::new(6),
        GrowthRange::OneYear => now - chrono::Months::new(12),
        GrowthRange::FiveYears => now - chrono::Months::new(60),
        GrowthRange::AllTime => {
            let earliest: Earliest = diesel::sql_query(format!(
                r#"SELECT LEAST((SELECT min(created_at) FROM "user"),
                               (SELECT min({COMMUNITY_CREATED_AT}) FROM community)) AS earliest"#
            ))
            .get_result(conn.as_mut())
            .await?;
            earliest.earliest.unwrap_or(now)
        }
    };
    let unit = GrowthUnit::for_span((now - start).num_days());
    let points = diesel::sql_query(format!(
        r#"
        WITH user_events AS (
            SELECT date_trunc($1, created_at) AS at, count(*) AS delta FROM "user" GROUP BY 1
            UNION ALL
            SELECT date_trunc($1, deleted_at), -count(*) FROM "user"
            WHERE deleted_at IS NOT NULL GROUP BY 1
        ),
        community_events AS (
            SELECT date_trunc($1, {COMMUNITY_CREATED_AT}) AS at, count(*) AS delta
            FROM community GROUP BY 1
            UNION ALL
            SELECT date_trunc($1, deleted_at), -count(*) FROM community
            WHERE deleted_at IS NOT NULL GROUP BY 1
        ),
        steps AS (
            SELECT generate_series(date_trunc($1, $2::timestamptz), date_trunc($1, now()),
                                   ('1 ' || $1)::interval) AS at
        )
        SELECT steps.at,
               COALESCE((SELECT sum(delta) FROM user_events e WHERE e.at <= steps.at), 0)::bigint
                   AS users,
               COALESCE((SELECT sum(delta) FROM community_events e WHERE e.at <= steps.at), 0)::bigint
                   AS communities
        FROM steps
        ORDER BY steps.at
        "#
    ))
    .bind::<Text, _>(unit.sql())
    .bind::<Timestamptz, _>(start)
    .load(conn.as_mut())
    .await?;
    Ok((unit, points))
}

#[cfg(test)]
mod tests {
    use super::{GrowthUnit, contains_pattern};

    #[test]
    fn a_growth_series_steps_by_days_weeks_or_months_as_it_lengthens() {
        assert_eq!(GrowthUnit::for_span(92), GrowthUnit::Day);
        assert_eq!(GrowthUnit::for_span(184), GrowthUnit::Day);
        assert_eq!(GrowthUnit::for_span(366), GrowthUnit::Week);
        assert_eq!(GrowthUnit::for_span(1827), GrowthUnit::Month);
    }

    #[test]
    fn a_search_matches_anywhere_and_takes_like_characters_literally() {
        assert_eq!(contains_pattern(Some("Kate")), Some("%kate%".to_string()));
        assert_eq!(contains_pattern(Some("  ")), None);
        assert_eq!(contains_pattern(None), None);
        assert_eq!(
            contains_pattern(Some("50%_\\")),
            Some("%50\\%\\_\\\\%".to_string())
        );
    }
}
