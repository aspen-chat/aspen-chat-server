use crate::CHACHA_RNG;
use crate::context::GlobalServerContext;
use crate::cursor::{List, Paged};
use crate::events::Publishing;
use crate::permissions::{Permissions, require_member};
use crate::t;
use crate::{CommunityId, EventScope, UserId, publish_event};
use aspen_schema::{community, invite};
use aspen_wire::message_enum;
use aspen_wire::message_enum::server_event::{InviteEvent, ServerEvent};
use chrono::Utc;
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable,
    Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use rand::RngExt;

const INVITE_CODE_LENGTH: usize = 16;
/// The most invites a community holds that are neither revoked nor expired.
pub const MAX_ACTIVE_INVITES: i64 = 1000;
/// How long after it stops working an invite, a community's or a registration invite, is still
/// listed; after that only the terminal lists it (registration invites) or nothing does.
pub const STALE_AFTER_DAYS: i64 = 7;
const ALPHANUMERIC: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = invite)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Invite {
    pub code: String,
    pub community: CommunityId,
    pub created_by: UserId,
    pub created_at: chrono::DateTime<Utc>,
    pub expires_at: Option<chrono::DateTime<Utc>>,
    pub deleted_at: Option<chrono::DateTime<Utc>>,
}

impl From<&Invite> for message_enum::Invite {
    fn from(invite: &Invite) -> Self {
        message_enum::Invite {
            code: invite.code.clone(),
            created_by: invite.created_by,
            created_at: invite.created_at,
            community: invite.community,
            expires_at: invite.expires_at,
        }
    }
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = invite)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InviteChangeset {
    pub expires_at: Option<Option<chrono::DateTime<Utc>>>,
}

pub fn generate_invite_code() -> String {
    CHACHA_RNG.with(|rng| {
        let mut rng = rng.borrow_mut();
        (0..INVITE_CODE_LENGTH)
            .map(|_| {
                let idx = rng.random_range(0..ALPHANUMERIC.len());
                ALPHANUMERIC[idx] as char
            })
            .collect()
    })
}

fn validate_custom_code(code: &str) -> crate::Result<()> {
    if code.is_empty() || code.len() > INVITE_CODE_LENGTH {
        return Err(crate::Error::Validation(t!("inviteCodeLength")));
    }
    if !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(crate::Error::Validation(t!("inviteCodeAlphanumeric")));
    }
    Ok(())
}

pub async fn create_invite(
    state: &GlobalServerContext,
    user: UserId,
    community: CommunityId,
    custom_code: Option<String>,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> crate::Result<Invite> {
    let mut conn = state.connection_pool.get().await?;
    insert(
        state,
        conn.as_mut(),
        user,
        community,
        custom_code,
        expires_at,
    )
    .await
}

/// Makes an invite to `community` from `user`, who needs Create invites there, and Manage
/// invites besides to choose its code, on `conn`, and announces it. Inside a transaction, the
/// invite and its announcement go with it. A chosen code is unique across the deployment, so
/// whoever chooses one takes it from every other community; it is left to those who manage a
/// community's invites.
pub async fn insert(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user: UserId,
    community: CommunityId,
    custom_code: Option<String>,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> crate::Result<Invite> {
    let access = require_member(conn, user, community).await?;
    access.require(Permissions::CREATE_INVITES)?;
    if custom_code.is_some() {
        access.require(Permissions::MANAGE_INVITES)?;
    }

    let code = match custom_code {
        Some(c) => {
            validate_custom_code(&c)?;
            c
        }
        None => generate_invite_code(),
    };
    crate::community::hold_for_count(conn, community).await?;
    let active: i64 = invite::table
        .filter(invite::community.eq(community))
        .filter(invite::deleted_at.is_null())
        .filter(
            invite::expires_at
                .is_null()
                .or(invite::expires_at.gt(Utc::now())),
        )
        .count()
        .get_result(conn)
        .await?;
    if active >= MAX_ACTIVE_INVITES {
        return Err(crate::Error::Validation(t!(
            "inviteLimit",
            max = MAX_ACTIVE_INVITES
        )));
    }

    let invite = Invite {
        code,
        community,
        created_by: user,
        created_at: Utc::now(),
        expires_at,
        deleted_at: None,
    };
    diesel::insert_into(invite::table)
        .values(&invite)
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        EventScope::Community(community),
        &ServerEvent::Invite(InviteEvent::Create(message_enum::Invite::from(&invite))),
    )
    .await?;
    Ok(invite)
}

/// Validates an invite code is active and not expired. Returns the community it belongs to.
/// Designed to be called within an existing transaction.
pub async fn validate_invite(
    conn: &mut AsyncPgConnection,
    code: &str,
) -> crate::Result<CommunityId> {
    let inv: Invite = invite::table
        .inner_join(community::table)
        .select(Invite::as_select())
        .filter(invite::code.eq(code).and(invite::deleted_at.is_null()))
        .filter(community::deleted_at.is_null())
        .first(conn)
        .await
        .map_err(|e| match e {
            diesel::result::Error::NotFound => crate::Error::Validation(t!("inviteCodeInvalid")),
            other => other.into(),
        })?;

    if let Some(expires_at) = inv.expires_at
        && expires_at < Utc::now()
    {
        return Err(crate::Error::Validation(t!("inviteExpired")));
    }

    Ok(inv.community)
}

/// Refuses anyone but the invite's creator, while still a member, and those with Manage
/// invites in its community.
async fn ensure_may_manage(
    conn: &mut AsyncPgConnection,
    user: UserId,
    invite: &Invite,
) -> crate::Result<()> {
    let access = require_member(conn, user, invite.community).await?;
    if invite.created_by == user {
        return Ok(());
    }
    access.require(Permissions::MANAGE_INVITES)
}

pub async fn update_invite(
    state: &GlobalServerContext,
    user: UserId,
    code: String,
    expires_at: Option<Option<chrono::DateTime<Utc>>>,
) -> crate::Result<Invite> {
    let mut conn = state.connection_pool.get().await?;

    let inv: Invite = invite::table
        .select(Invite::as_select())
        .filter(invite::code.eq(&code).and(invite::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;

    ensure_may_manage(conn.as_mut(), user, &inv).await?;

    conn.transaction(|conn| {
        async move {
            let rows = diesel::update(invite::table)
                .set(InviteChangeset { expires_at })
                .filter(invite::code.eq(&code).and(invite::deleted_at.is_null()))
                .returning(Invite::as_returning())
                .load(conn)
                .await?;
            let Some(updated) = rows.into_iter().next() else {
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            };
            publish_event(
                state,
                conn,
                EventScope::Community(updated.community),
                &ServerEvent::Invite(InviteEvent::Update { code, expires_at }),
            )
            .await?;
            Ok(updated)
        }
        .scope_boxed()
    })
    .await
}

pub async fn revoke_invite(
    state: &GlobalServerContext,
    user: UserId,
    code: String,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;

    let inv: Invite = invite::table
        .select(Invite::as_select())
        .filter(invite::code.eq(&code).and(invite::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;

    ensure_may_manage(conn.as_mut(), user, &inv).await?;

    conn.transaction(|conn| {
        async move {
            delete(state, conn, &code).await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Revokes the invite `code`, on `conn`, which must be inside a transaction, and announces it.
/// `false` when no unrevoked invite has that code. Who may is the caller's to decide.
pub async fn delete(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    code: &str,
) -> crate::Result<bool> {
    let revoked = diesel::update(invite::table)
        .set(invite::deleted_at.eq(diesel::dsl::now))
        .filter(invite::code.eq(code).and(invite::deleted_at.is_null()))
        .execute(conn)
        .await?;
    if revoked == 0 {
        return Ok(false);
    }
    publish_event(
        state,
        conn,
        EventScope::CommunityOfInvite(code.to_string()),
        &ServerEvent::Invite(InviteEvent::Delete {
            code: code.to_string(),
        }),
    )
    .await?;
    Ok(true)
}

/// The invite with this code, whether or not it has expired, so a caller can tell the user an
/// expired link is expired rather than unknown. Revoked invites, and those to a deleted
/// community, read as not found.
pub async fn read_invite(state: &GlobalServerContext, code: &str) -> crate::Result<Invite> {
    let mut conn = state.connection_pool.get().await?;
    invite::table
        .inner_join(community::table)
        .select(Invite::as_select())
        .filter(invite::code.eq(code).and(invite::deleted_at.is_null()))
        .filter(community::deleted_at.is_null())
        .first(conn.as_mut())
        .await
        .map_err(Into::into)
}

/// Where an invite stands among a community's, the columns they are ordered by: when it was
/// made, in microseconds since the epoch, and its code. The invite list's cursor.
#[derive(serde::Serialize, serde::Deserialize)]
struct InvitePosition(i64, String);

/// The place an invite list's cursor holds.
fn invite_position(cursor: &str) -> crate::Result<(chrono::DateTime<Utc>, String)> {
    let InvitePosition(micros, code) = crate::cursor::decode(List::Invites, cursor)?;
    Ok((crate::cursor::instant(micros)?, code))
}

/// A page of a community's invites, newest first: every one for those with Manage invites, and
/// only their own for other members, at most `limit` of them (up to [`crate::LIST_PAGE`]),
/// after the place `cursor` holds when given, which is the `next_cursor` of the page before.
/// Revoked ones are gone; expired ones are listed for `STALE_AFTER_DAYS` after they expire, so
/// a link that just stopped working can still be seen for what it was.
pub async fn read_community_invites(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    cursor: Option<&str>,
    limit: i64,
) -> crate::Result<Paged<Invite>> {
    let after = cursor.map(invite_position).transpose()?;
    let mut conn = state.connection_pool.get().await?;
    let access = require_member(conn.as_mut(), caller, community).await?;
    let creator = (!access.has(Permissions::MANAGE_INVITES)).then_some(caller);
    invites_page(conn.as_mut(), community, creator, after, limit).await
}

/// One page of `community`'s invites, those `creator` made when given, and the cursor of the
/// next, by `(created_at, code)` descending, the order `invite_listed` and
/// `invite_listed_by_creator` keep them in, after `after`. The place is compared as a row
/// value against the cursor's own, so the index scan starts there whether or not that invite
/// is still listed. Revoked and long expired ones are passed over before the limit.
async fn invites_page(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    creator: Option<UserId>,
    after: Option<(chrono::DateTime<Utc>, String)>,
    limit: i64,
) -> crate::Result<Paged<Invite>> {
    use diesel::sql_types::{Bool, Text, Timestamptz};
    let limit = limit.clamp(1, crate::LIST_PAGE);
    let cutoff = Utc::now() - chrono::Duration::days(STALE_AFTER_DAYS);
    let mut query = invite::table
        .select(Invite::as_select())
        .filter(
            invite::community
                .eq(community)
                .and(invite::deleted_at.is_null())
                .and(
                    invite::expires_at
                        .is_null()
                        .or(invite::expires_at.gt(cutoff)),
                ),
        )
        .into_boxed();
    if let Some(creator) = creator {
        query = query.filter(invite::created_by.eq(creator));
    }
    if let Some((at, code)) = after {
        query = query.filter(
            diesel::dsl::sql::<Bool>("(invite.created_at, invite.code) < (")
                .bind::<Timestamptz, _>(at)
                .sql(", ")
                .bind::<Text, _>(code)
                .sql(")"),
        );
    }
    let rows: Vec<Invite> = query
        .order_by((invite::created_at.desc(), invite::code.desc()))
        .limit(limit + 1)
        .load(conn)
        .await?;
    Ok(crate::cursor::cut(
        List::Invites,
        rows,
        limit as usize,
        |last| InvitePosition(last.created_at.timestamp_micros(), last.code.clone()),
    ))
}

#[cfg(test)]
mod database_tests {
    use super::*;
    use diesel_async::SimpleAsyncConnection;

    const COMMUNITY: &str = "14b17000-0000-4000-8000-000000000001";
    const ANN: &str = "14b17000-0000-4000-8000-000000000002";
    const BEN: &str = "14b17000-0000-4000-8000-000000000003";

    /// A connection to the database `DATABASE_URL` names, inside a transaction that is never
    /// committed, with a community of `invites` invites, `page-test-1` the newest and each
    /// after it a second older, but for the third and fourth, made at the same instant; Ben
    /// made the even ones and Ann the rest. `None` without a database, when the test checks
    /// nothing.
    async fn community_with(invites: u8) -> Option<AsyncPgConnection> {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return None;
        };
        let mut conn = crate::database::connect(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(&format!(
            r#"INSERT INTO community (id, name) VALUES ('{COMMUNITY}', 'c');
               INSERT INTO "user" (id, name, password_hash, created_at, last_seen_at)
                   VALUES ('{ANN}', '{ANN}', '', now(), now()), ('{BEN}', '{BEN}', '', now(), now());"#
        ))
        .await
        .unwrap();
        for n in 1..=invites {
            let creator = if n % 2 == 0 { BEN } else { ANN };
            let second = if n == 4 { 3 } else { n };
            conn.batch_execute(&format!(
                r#"INSERT INTO invite (code, community, created_by, created_at)
                     VALUES ('page-test-{n}', '{COMMUNITY}', '{creator}',
                             '2026-01-01T00:00:00.123456Z'::timestamptz - interval '{second} seconds');"#,
            ))
            .await
            .unwrap();
        }
        Some(conn)
    }

    /// Which invites one page holds, by their numbers, and the next page's cursor.
    async fn page(
        conn: &mut AsyncPgConnection,
        creator: Option<&str>,
        cursor: Option<&str>,
        limit: i64,
    ) -> (Vec<u8>, Option<String>) {
        let page = invites_page(
            conn,
            CommunityId(COMMUNITY.parse().unwrap()),
            creator.map(|c| UserId(c.parse().unwrap())),
            cursor.map(|c| invite_position(c).unwrap()),
            limit,
        )
        .await
        .unwrap();
        let invites = page
            .items
            .into_iter()
            .map(|invite| invite.code["page-test-".len()..].parse().unwrap())
            .collect();
        (invites, page.next_cursor)
    }

    /// The invite list's cursor is a place, not an invite: the pages go on after an invite
    /// revoked since, through invites made at one instant, past one long expired, and the last
    /// page says it is the last however many it holds.
    #[tokio::test]
    async fn the_invites_page_past_an_invite_that_is_gone() {
        let Some(mut conn) = community_with(7).await else {
            return;
        };
        // Of two invites made at one instant, the later code comes first.
        let (invites, cursor) = page(&mut conn, None, None, 3).await;
        assert_eq!(invites, [1, 2, 4]);
        // The fourth, which the cursor was made from, is revoked; the third was made at the
        // same instant and is still next. The fifth expired long ago.
        conn.batch_execute(&format!(
            r#"UPDATE invite SET deleted_at = now() WHERE code = 'page-test-4';
               UPDATE invite SET expires_at = now() - interval '{} days' WHERE code = 'page-test-5';"#,
            STALE_AFTER_DAYS + 1
        ))
        .await
        .unwrap();
        let (invites, cursor) = page(&mut conn, None, cursor.as_deref(), 2).await;
        assert_eq!(invites, [3, 6]);
        assert!(cursor.is_some());
        // The sixth, the boundary again, is revoked too.
        conn.batch_execute("UPDATE invite SET deleted_at = now() WHERE code = 'page-test-6'")
            .await
            .unwrap();
        let (invites, cursor) = page(&mut conn, None, cursor.as_deref(), 1).await;
        assert_eq!(invites, [7]);
        assert_eq!(cursor, None);
        // A page that ends on the last invite has none after it, though it is full.
        let (invites, cursor) = page(&mut conn, None, None, 4).await;
        assert_eq!(invites, [1, 2, 3, 7]);
        assert_eq!(cursor, None);
        // Someone without Manage invites pages through their own alone.
        let (invites, cursor) = page(&mut conn, Some(ANN), None, 2).await;
        assert_eq!(invites, [1, 3]);
        let (invites, cursor) = page(&mut conn, Some(ANN), cursor.as_deref(), 2).await;
        assert_eq!(invites, [7]);
        assert_eq!(cursor, None);
    }

    #[test]
    fn a_cursor_of_no_invite_list_is_refused() {
        let refused =
            |cursor: &str| matches!(invite_position(cursor), Err(crate::Error::Validation(_)));
        let user = UserId(ANN.parse().unwrap());
        assert!(refused("garbage"));
        // An invite's code, which is not a place in the list.
        assert!(refused("page-test-1"));
        // Places of other shapes, a time no clock tells, and a cursor of another list holding
        // a place of an invite's shape.
        assert!(refused(&crate::cursor::encode(List::Invites, &user)));
        assert!(refused(&crate::cursor::encode(
            List::Invites,
            &("ada", user)
        )));
        assert!(refused(&crate::cursor::encode(
            List::Invites,
            &(i64::MAX, "code")
        )));
        assert!(refused(&crate::cursor::encode(
            List::Bans,
            &(1_i64, "code")
        )));
        assert!(!refused(&crate::cursor::encode(
            List::Invites,
            &(1_i64, "code")
        )));
    }
}
