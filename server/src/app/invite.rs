use crate::CHACHA_RNG;
use crate::api::message_enum;
use crate::api::message_enum::server_event::{InviteEvent, ServerEvent};
use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::permissions::{Permissions, require_member};
use crate::app::{CommunityId, EventScope, UserId, publish_event};
use crate::database::schema::invite;
use crate::t;
use chrono::Utc;
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable,
    Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use rand::RngExt;

const INVITE_CODE_LENGTH: usize = 16;
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

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = invite)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct InviteChangeset {
    pub expires_at: Option<Option<chrono::DateTime<Utc>>>,
}

pub(crate) fn generate_invite_code() -> String {
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

fn validate_custom_code(code: &str) -> app::Result<()> {
    if code.is_empty() || code.len() > INVITE_CODE_LENGTH {
        return Err(app::Error::Validation(t!("inviteCodeLength")));
    }
    if !code.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err(app::Error::Validation(t!("inviteCodeAlphanumeric")));
    }
    Ok(())
}

pub(crate) async fn create_invite(
    state: &GlobalServerContext,
    user: UserId,
    community: CommunityId,
    custom_code: Option<String>,
    expires_at: Option<chrono::DateTime<Utc>>,
) -> app::Result<Invite> {
    let mut conn = state.connection_pool.get().await?;
    require_member(conn.as_mut(), user, community)
        .await?
        .require(Permissions::CREATE_INVITES)?;

    let code = match custom_code {
        Some(c) => {
            validate_custom_code(&c)?;
            c
        }
        None => generate_invite_code(),
    };

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
        .execute(conn.as_mut())
        .await?;
    publish_event(
        state,
        conn.as_mut(),
        EventScope::Community(community),
        &ServerEvent::Invite(InviteEvent::Create(message_enum::Invite {
            code: invite.code.clone(),
            created_by: invite.created_by,
            created_at: invite.created_at,
            community: invite.community,
            expires_at: invite.expires_at,
        })),
    )
    .await?;
    Ok(invite)
}

/// Validates an invite code is active and not expired. Returns the community it belongs to.
/// Designed to be called within an existing transaction.
pub(crate) async fn validate_invite(
    conn: &mut AsyncPgConnection,
    code: &str,
) -> app::Result<CommunityId> {
    let inv: Invite = invite::table
        .select(Invite::as_select())
        .filter(invite::code.eq(code).and(invite::deleted_at.is_null()))
        .first(conn)
        .await
        .map_err(|e| match e {
            diesel::result::Error::NotFound => app::Error::Validation(t!("inviteCodeInvalid")),
            other => other.into(),
        })?;

    if let Some(expires_at) = inv.expires_at
        && expires_at < Utc::now()
    {
        return Err(app::Error::Validation(t!("inviteExpired")));
    }

    Ok(inv.community)
}

/// Refuses anyone but the invite's creator, while still a member, and those with Manage
/// invites in its community.
async fn ensure_may_manage(
    conn: &mut AsyncPgConnection,
    user: UserId,
    invite: &Invite,
) -> app::Result<()> {
    let access = require_member(conn, user, invite.community).await?;
    if invite.created_by == user {
        return Ok(());
    }
    access.require(Permissions::MANAGE_INVITES)
}

pub(crate) async fn update_invite(
    state: &GlobalServerContext,
    user: UserId,
    code: String,
    expires_at: Option<Option<chrono::DateTime<Utc>>>,
) -> app::Result<Invite> {
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
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
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

pub(crate) async fn revoke_invite(
    state: &GlobalServerContext,
    user: UserId,
    code: String,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;

    let inv: Invite = invite::table
        .select(Invite::as_select())
        .filter(invite::code.eq(&code).and(invite::deleted_at.is_null()))
        .first(conn.as_mut())
        .await?;

    ensure_may_manage(conn.as_mut(), user, &inv).await?;

    conn.transaction(|conn| {
        async move {
            diesel::update(invite::table)
                .set(invite::deleted_at.eq(diesel::dsl::now))
                .filter(invite::code.eq(&code).and(invite::deleted_at.is_null()))
                .execute(conn)
                .await?;
            publish_event(
                state,
                conn,
                EventScope::CommunityOfInvite(code.clone()),
                &ServerEvent::Invite(InviteEvent::Delete { code }),
            )
            .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// The invite with this code, whether or not it has expired, so a caller can tell the user an
/// expired link is expired rather than unknown. Revoked invites read as not found.
pub(crate) async fn read_invite(state: &GlobalServerContext, code: &str) -> app::Result<Invite> {
    let mut conn = state.connection_pool.get().await?;
    invite::table
        .select(Invite::as_select())
        .filter(invite::code.eq(code).and(invite::deleted_at.is_null()))
        .first(conn.as_mut())
        .await
        .map_err(Into::into)
}

/// A community's invites, newest first: every one for those with Manage invites, and only
/// their own for other members. Revoked ones are gone; expired ones are listed for
/// `STALE_AFTER_DAYS` after they expire, so a link that just stopped working can still be seen
/// for what it was.
pub(crate) async fn read_community_invites(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
) -> app::Result<Vec<Invite>> {
    let mut conn = state.connection_pool.get().await?;
    let access = require_member(conn.as_mut(), caller, community).await?;
    let cutoff = Utc::now() - chrono::Duration::days(STALE_AFTER_DAYS);
    let mut query = invite::table.select(Invite::as_select()).into_boxed();
    if !access.has(Permissions::MANAGE_INVITES) {
        query = query.filter(invite::created_by.eq(caller));
    }
    let invites = query
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
        .order_by(invite::created_at.desc())
        .load(conn.as_mut())
        .await?;
    Ok(invites)
}
