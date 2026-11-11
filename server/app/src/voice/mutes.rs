//! A moderator's mute of someone in a community's calls (`voice_mute`). It stands in every call
//! of the community until a moderator lifts it: the join token of anyone muted says so, so the
//! voice server makes their participant muted from the start, every recheck of their calls
//! sends their voice server the mute as it stands (`sessions::recheck_seat`), and it outlives
//! their leaving the community, so leaving and coming back does not lift it. Muting and lifting
//! take Manage calls in the community, over someone below the caller's highest role and never
//! the owner, as removing does. Its events (`voiceMute`) reach holders of Manage calls and the
//! muted person, and each makes their calls be rechecked (`app::events::rechecks_of`), which is
//! how a mute reaches a call already under way.

use crate::context::GlobalServerContext;
use crate::cursor::{List, Paged};
use crate::events::{EventScope, publish_event};
use crate::permissions::{CommunityAccess, Permissions, community_access, require_member};
use crate::t;
use crate::{CommunityId, UserId};
use aspen_schema::voice_mute;
use aspen_wire::message_enum::server_event::{ServerEvent, VoiceMuteEvent};
use aspen_wire::message_enum::{self};
use chrono::{DateTime, Utc};
use diesel::SelectableHelper;
use diesel::{ExpressionMethods, Insertable, OptionalExtension, QueryDsl, Queryable, Selectable};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = voice_mute)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct VoiceMuteRow {
    community: CommunityId,
    user: UserId,
    muted_by: Option<UserId>,
    muted_at: DateTime<Utc>,
}

impl From<&VoiceMuteRow> for message_enum::VoiceMute {
    fn from(row: &VoiceMuteRow) -> Self {
        message_enum::VoiceMute {
            community: row.community,
            user: row.user,
            muted_by: row.muted_by,
            muted_at: row.muted_at,
        }
    }
}

/// Whether a moderator's mute of `user` stands in `community`.
pub async fn is_muted(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    user: UserId,
) -> crate::Result<bool> {
    Ok(diesel::select(diesel::dsl::exists(
        voice_mute::table
            .filter(voice_mute::community.eq(community))
            .filter(voice_mute::user.eq(user)),
    ))
    .get_result(conn)
    .await?)
}

/// Where a server mute stands among a community's, the columns they are ordered by: when it
/// was made, in microseconds since the epoch, and whom it mutes. The mute list's cursor.
#[derive(serde::Serialize, serde::Deserialize)]
struct MutePosition(i64, UserId);

/// The place a mute list's cursor holds.
fn mute_position(cursor: &str) -> crate::Result<(DateTime<Utc>, UserId)> {
    let MutePosition(micros, user) = crate::cursor::decode(List::VoiceMutes, cursor)?;
    Ok((crate::cursor::instant(micros)?, user))
}

/// A page of the community's standing server mutes, newest first, for a holder of Manage calls:
/// at most `limit` of them (up to [`crate::LIST_PAGE`]), after the place `cursor` holds when
/// given, which is the `next_cursor` of the page before.
pub async fn read_mutes(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    cursor: Option<&str>,
    limit: i64,
) -> crate::Result<Paged<message_enum::VoiceMute>> {
    let after = cursor.map(mute_position).transpose()?;
    let mut conn = state.connection_pool.get().await?;
    let access = require_member(conn.as_mut(), caller, community).await?;
    access.require(Permissions::MANAGE_CALLS)?;
    mutes_page(conn.as_mut(), community, after, limit).await
}

/// One page of `community`'s server mutes and the cursor of the next, by
/// `(muted_at, "user")` descending, the order `voice_mute_listed` keeps them in, after `after`.
/// The place is compared as a row value against the cursor's own, so the index scan starts
/// there whether or not that mute still stands.
async fn mutes_page(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    after: Option<(DateTime<Utc>, UserId)>,
    limit: i64,
) -> crate::Result<Paged<message_enum::VoiceMute>> {
    use diesel::sql_types::{Bool, Timestamptz, Uuid};
    let limit = limit.clamp(1, crate::LIST_PAGE);
    let mut query = voice_mute::table
        .select(VoiceMuteRow::as_select())
        .filter(voice_mute::community.eq(community))
        .into_boxed();
    if let Some((at, user)) = after {
        query = query.filter(
            diesel::dsl::sql::<Bool>(r#"(voice_mute.muted_at, voice_mute."user") < ("#)
                .bind::<Timestamptz, _>(at)
                .sql(", ")
                .bind::<Uuid, _>(user)
                .sql(")"),
        );
    }
    let rows: Vec<VoiceMuteRow> = query
        .order((voice_mute::muted_at.desc(), voice_mute::user.desc()))
        .limit(limit + 1)
        .load(conn)
        .await?;
    let page = crate::cursor::cut(List::VoiceMutes, rows, limit as usize, |last| {
        MutePosition(last.muted_at.timestamp_micros(), last.user)
    });
    Ok(Paged {
        items: page
            .items
            .iter()
            .map(message_enum::VoiceMute::from)
            .collect(),
        next_cursor: page.next_cursor,
    })
}

/// `user`'s standing server mute in `community`, if they have one, for a holder of Manage
/// calls.
pub async fn read_mute(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    user: UserId,
) -> crate::Result<Option<message_enum::VoiceMute>> {
    let mut conn = state.connection_pool.get().await?;
    let access = require_member(conn.as_mut(), caller, community).await?;
    access.require(Permissions::MANAGE_CALLS)?;
    let row: Option<VoiceMuteRow> = voice_mute::table
        .select(VoiceMuteRow::as_select())
        .filter(voice_mute::community.eq(community))
        .filter(voice_mute::user.eq(user))
        .first(conn.as_mut())
        .await
        .optional()?;
    Ok(row.as_ref().map(message_enum::VoiceMute::from))
}

/// Refuses `caller` muting or lifting the mute of `user` in `community` unless they hold
/// Manage calls there and `user` ranks below them (never the owner; one who is not a member
/// ranks as nobody). A deployment moderator acting by Moderate any community must outrank
/// `user` in the deployment too.
pub(super) async fn require_moderates(
    conn: &mut AsyncPgConnection,
    caller: UserId,
    community: CommunityId,
    user: UserId,
) -> crate::Result<CommunityAccess> {
    let access = require_member(conn, caller, community).await?;
    access.require(Permissions::MANAGE_CALLS)?;
    if user == caller {
        return Ok(access);
    }
    match community_access(conn, user, community).await? {
        Some(theirs) if theirs.owner => return Err(crate::Error::Forbidden(t!("permissionRank"))),
        Some(theirs) if theirs.member => access.require_above(theirs.role_rank())?,
        _ => {}
    }
    if access.moderating(Permissions::MANAGE_CALLS) {
        crate::deployment::require_outranks(conn, caller, user).await?;
    }
    Ok(access)
}

/// Mutes `user` in every call of `community`, or with `muted` false lifts the mute, announcing
/// the change inside the transaction that makes it. Returns the mute that now stands, if any,
/// and whether this call changed anything.
pub async fn set_muted(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    user: UserId,
    muted: bool,
) -> crate::Result<(Option<message_enum::VoiceMute>, bool)> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            require_moderates(conn.as_mut(), caller, community, user).await?;
            let standing: Option<VoiceMuteRow> = voice_mute::table
                .select(VoiceMuteRow::as_select())
                .filter(voice_mute::community.eq(community))
                .filter(voice_mute::user.eq(user))
                .for_update()
                .first(conn.as_mut())
                .await
                .optional()?;
            match (standing, muted) {
                (Some(row), true) => Ok((Some(message_enum::VoiceMute::from(&row)), false)),
                (None, false) => Ok((None, false)),
                (None, true) => {
                    let row = VoiceMuteRow {
                        community,
                        user,
                        muted_by: Some(caller),
                        muted_at: Utc::now(),
                    };
                    let inserted = diesel::insert_into(voice_mute::table)
                        .values(&row)
                        .on_conflict_do_nothing()
                        .execute(conn.as_mut())
                        .await?;
                    if inserted == 0 {
                        // Two moderators muting at once: the lock above held no row for
                        // either, and the second's insert waited for the first's. The mute
                        // that stands is the first's, which is what is answered.
                        let standing: Option<VoiceMuteRow> = voice_mute::table
                            .select(VoiceMuteRow::as_select())
                            .filter(voice_mute::community.eq(community))
                            .filter(voice_mute::user.eq(user))
                            .first(conn.as_mut())
                            .await
                            .optional()?;
                        return Ok((standing.as_ref().map(message_enum::VoiceMute::from), false));
                    }
                    let record = message_enum::VoiceMute::from(&row);
                    publish_event(
                        state,
                        conn.as_mut(),
                        EventScope::Community(community),
                        &ServerEvent::VoiceMute(VoiceMuteEvent::Create(record.clone())),
                    )
                    .await?;
                    Ok((Some(record), true))
                }
                (Some(_), false) => {
                    diesel::delete(voice_mute::table)
                        .filter(voice_mute::community.eq(community))
                        .filter(voice_mute::user.eq(user))
                        .execute(conn.as_mut())
                        .await?;
                    publish_event(
                        state,
                        conn.as_mut(),
                        EventScope::Community(community),
                        &ServerEvent::VoiceMute(VoiceMuteEvent::Delete { community, user }),
                    )
                    .await?;
                    Ok((None, true))
                }
            }
        }
        .scope_boxed()
    })
    .await
}

/// Mutes `user` in every call of `community` (`set_muted`): the mute that stands, and whether
/// this made it.
pub async fn mute(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    user: UserId,
) -> crate::Result<(message_enum::VoiceMute, bool)> {
    match set_muted(state, caller, community, user, true).await? {
        (Some(mute), made) => Ok((mute, made)),
        (None, _) => Err(crate::Error::Diesel(diesel::result::Error::NotFound)),
    }
}

#[cfg(test)]
mod database_tests {
    use super::*;
    use diesel_async::SimpleAsyncConnection;

    const COMMUNITY: &str = "50ce7000-0000-4000-8000-000000000001";

    fn muted(n: u8) -> UserId {
        UserId(
            format!("50ce7000-0000-4000-8000-0000000001{n:02x}")
                .parse()
                .unwrap(),
        )
    }

    /// A connection to the database `DATABASE_URL` names, inside a transaction that is never
    /// committed, with a community of `mutes` people muted, the n-th a second before the one
    /// before, but for the third and fourth, muted at the same instant; `None` without one,
    /// when the test checks nothing.
    async fn community_with(mutes: u8) -> Option<AsyncPgConnection> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return None;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(&format!(
            "INSERT INTO community (id, name) VALUES ('{COMMUNITY}', 'c')"
        ))
        .await
        .unwrap();
        for n in 1..=mutes {
            let user = muted(n).0;
            let second = if n == 4 { 3 } else { n };
            conn.batch_execute(&format!(
                r#"INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at)
                     VALUES ('{user}', '{user}', '', now(), now());
                 INSERT INTO voice_mute (community, "user", muted_at)
                     VALUES ('{COMMUNITY}', '{user}',
                             '2026-01-01T00:00:00.123456Z'::timestamptz - interval '{second} seconds');"#,
            ))
            .await
            .unwrap();
        }
        Some(conn)
    }

    /// Whom one page's mutes name, by their numbers, and the next page's cursor.
    async fn page(
        conn: &mut AsyncPgConnection,
        cursor: Option<&str>,
        limit: i64,
    ) -> (Vec<u8>, Option<String>) {
        let page = mutes_page(
            conn,
            CommunityId(COMMUNITY.parse().unwrap()),
            cursor.map(|c| mute_position(c).unwrap()),
            limit,
        )
        .await
        .unwrap();
        let users = page
            .items
            .into_iter()
            .map(|mute| (1..=u8::MAX).find(|n| muted(*n) == mute.user).unwrap())
            .collect();
        (users, page.next_cursor)
    }

    /// The mute list's cursor is a place, not a mute: the pages go on after a mute lifted
    /// since, through mutes made at one instant, and the last page says it is the last however
    /// many it holds.
    #[tokio::test]
    async fn the_mutes_page_past_a_mute_that_is_gone() {
        let Some(mut conn) = community_with(6).await else {
            return;
        };
        // Of two mutes made at one instant, the later id comes first.
        let (users, cursor) = page(&mut conn, None, 3).await;
        assert_eq!(users, [1, 2, 4]);
        // The fourth, whom the cursor was made from, is unmuted; the third was muted at the
        // same instant and is still next.
        conn.batch_execute(&format!(
            r#"DELETE FROM voice_mute WHERE "user" = '{}'"#,
            muted(4).0
        ))
        .await
        .unwrap();
        let (users, cursor) = page(&mut conn, cursor.as_deref(), 2).await;
        assert_eq!(users, [3, 5]);
        assert!(cursor.is_some());
        // The fifth, the boundary again, is unmuted too.
        conn.batch_execute(&format!(
            r#"DELETE FROM voice_mute WHERE "user" = '{}'"#,
            muted(5).0
        ))
        .await
        .unwrap();
        let (users, cursor) = page(&mut conn, cursor.as_deref(), 1).await;
        assert_eq!(users, [6]);
        assert_eq!(cursor, None);
        // A page that ends on the last mute has none after it, though it is full.
        let (users, cursor) = page(&mut conn, None, 4).await;
        assert_eq!(users, [1, 2, 3, 6]);
        assert_eq!(cursor, None);
    }

    #[test]
    fn a_cursor_of_no_mute_list_is_refused() {
        let refused =
            |cursor: &str| matches!(mute_position(cursor), Err(crate::Error::Validation(_)));
        assert!(refused("garbage"));
        // A muted person's id, which is not a place in the list.
        assert!(refused(&muted(1).0.to_string()));
        // A place of another shape, a time no clock tells, and a ban list's cursor, whose
        // place has a mute's shape.
        assert!(refused(&crate::cursor::encode(
            List::VoiceMutes,
            &(1_i64, "code")
        )));
        assert!(refused(&crate::cursor::encode(List::VoiceMutes, &muted(1))));
        assert!(refused(&crate::cursor::encode(
            List::VoiceMutes,
            &(i64::MAX, muted(1))
        )));
        assert!(refused(&crate::cursor::encode(
            List::Bans,
            &(1_i64, muted(1))
        )));
        assert!(!refused(&crate::cursor::encode(
            List::VoiceMutes,
            &(1_i64, muted(1))
        )));
    }
}
