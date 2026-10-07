//! Bans from the whole deployment (`user.banned_at`): the person can no longer use it at all.
//! Banning ends every sign-in they hold, closes their event streams (the `accountBanned` event
//! tells each one why, `app::event_feed`), and takes them out of every call, and does the same
//! to the bots they own, which act for them; while the ban stands, signing in is refused with
//! `deploymentBanned` and the reason they were given, a bot's token is refused though kept,
//! whether the bot or its owner is banned (`shut_out`), so lifting the ban restores it, and
//! other deployments asking
//! after them are told they are no longer in good standing (`app::federation::standing`). Their
//! memberships stay, so their name and what they posted still show.
//!
//! A ban takes Ban users and ranks by deployment roles: only someone whose highest role is below
//! the banner's, never the banner themselves or the system account. It may carry a reason, an
//! end, and the deletion of the person's messages anywhere on the deployment from the last hour
//! or day, which takes Remove content besides. A bot may be banned with its owner. Each
//! ban and lift is written to the moderation log.

use crate::ban::{BanRequest, validate};
use crate::context::GlobalServerContext;
use crate::deployment::{DeploymentAccess, DeploymentPermission, deployment_access};
use crate::moderation_log::{ModerationAction, log_moderation};
use crate::t;
use crate::{EventScope, UserId, publish_event};
use aspen_schema::user;
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use diesel::sql_types::Bool;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

/// Whether the `user` row in a query is banned now: a ban that has no end, or one whose end is
/// still to come.
pub const BANNED_SQL: &str = "(\"user\".banned_at IS NOT NULL \
     AND (\"user\".banned_until IS NULL OR \"user\".banned_until > now()))";

/// The SQL condition that the `user` row in a query is banned now.
pub fn banned() -> diesel::expression::SqlLiteral<Bool> {
    diesel::dsl::sql::<Bool>(BANNED_SQL)
}

/// Whether the `user` row in a query is shut out of the deployment now: banned itself, or a
/// bot whose owner is banned. A bot acts for its owner, so it is shut out while they are.
pub const SHUT_OUT_SQL: &str = "((\"user\".banned_at IS NOT NULL \
     AND (\"user\".banned_until IS NULL OR \"user\".banned_until > now())) \
     OR EXISTS (SELECT 1 FROM \"user\" owner_row WHERE owner_row.id = \"user\".bot_owner \
     AND owner_row.banned_at IS NOT NULL \
     AND (owner_row.banned_until IS NULL OR owner_row.banned_until > now())))";

/// The SQL condition that the `user` row in a query is shut out now (`SHUT_OUT_SQL`).
pub fn shut_out() -> diesel::expression::SqlLiteral<Bool> {
    diesel::dsl::sql::<Bool>(SHUT_OUT_SQL)
}

/// A ban as it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserBan {
    pub reason: Option<String>,
    pub until: Option<DateTime<Utc>>,
    pub banned_at: DateTime<Utc>,
    pub banned_by: Option<UserId>,
}

/// A ban's columns of `user`.
#[derive(Queryable, Selectable)]
#[diesel(table_name = user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
struct BanColumns {
    ban_reason: Option<String>,
    banned_until: Option<DateTime<Utc>>,
    banned_at: Option<DateTime<Utc>>,
    banned_by: Option<UserId>,
}

/// The ban of `user` standing now, if one does.
pub async fn standing(
    conn: &mut AsyncPgConnection,
    user: UserId,
) -> crate::Result<Option<UserBan>> {
    let row: Option<BanColumns> = user::table
        .select(BanColumns::as_select())
        .filter(user::id.eq(user))
        .filter(banned())
        .first(conn)
        .await
        .optional()?;
    Ok(row.and_then(|row| {
        Some(UserBan {
            reason: row.ban_reason,
            until: row.banned_until,
            banned_at: row.banned_at?,
            banned_by: row.banned_by,
        })
    }))
}

/// Refuses `user` with `DeploymentBanned` while a ban of them stands. Every sign-in passes
/// through this (`app::login::issue_session`).
pub async fn check_not_banned(conn: &mut AsyncPgConnection, user: UserId) -> crate::Result<()> {
    match standing(conn, user).await? {
        Some(ban) => Err(crate::Error::DeploymentBanned {
            reason: ban.reason,
            until: ban.until,
        }),
        None => Ok(()),
    }
}

/// What a deployment ban asks for: a community ban's reason, end, and deletion window, the
/// window reaching every channel and DM, and for a bot whether its owner is banned too.
#[derive(Debug, Clone, Default)]
pub struct UserBanRequest {
    pub ban: BanRequest,
    pub with_owner: bool,
}

/// What banning did.
#[derive(Debug, Clone, Default)]
pub struct UserBanned {
    /// Whether a ban stood already, which this one replaced.
    pub replaced: bool,
    /// The people banned: the one named, and a bot's owner when asked.
    pub banned: Vec<UserId>,
    pub deleted_messages: usize,
}

/// Bans `target` from the deployment, replacing a ban that stood. See the module's account of
/// what that does.
pub async fn ban_user(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    target: UserId,
    request: &UserBanRequest,
) -> crate::Result<UserBanned> {
    access.require(DeploymentPermission::BanUsers)?;
    let (reason, until) = validate(&request.ban)?;
    if request.ban.delete_messages_seconds.is_some() {
        access.require(DeploymentPermission::RemoveContent)?;
    }
    let mut conn = state.connection_pool.get().await?;
    let outcome = conn
        .transaction(|conn| {
            let (reason, until) = (reason.clone(), until);
            async move {
                let mut outcome = UserBanned::default();
                let owner = ban_one(
                    state,
                    conn.as_mut(),
                    access,
                    target,
                    &reason,
                    until,
                    request.ban.delete_messages_seconds,
                    &mut outcome,
                )
                .await?;
                if request.with_owner {
                    let Some(owner) = owner else {
                        return Err(crate::Error::Validation(t!("banOwnerNoBot")));
                    };
                    ban_one(
                        state,
                        conn.as_mut(),
                        access,
                        owner,
                        &reason,
                        until,
                        request.ban.delete_messages_seconds,
                        &mut outcome,
                    )
                    .await?;
                }
                Ok::<_, crate::Error>(outcome)
            }
            .scope_boxed()
        })
        .await?;
    // `accountBanned` takes each of them out of their calls once this settles
    // (`app::events::rechecks_of`).
    Ok(outcome)
}

/// Bans one person inside the caller's transaction, adding to `outcome`. Returns a bot's owner.
#[allow(clippy::too_many_arguments)]
async fn ban_one(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    access: &DeploymentAccess,
    target: UserId,
    reason: &Option<String>,
    until: Option<DateTime<Utc>>,
    delete_messages_seconds: Option<u32>,
    outcome: &mut UserBanned,
) -> crate::Result<Option<UserId>> {
    if target == access.user {
        return Err(crate::Error::Validation(t!("banSelf")));
    }
    let (system, bot_owner, was_banned): (bool, Option<UserId>, bool) = user::table
        .select((user::system, user::bot_owner, banned()))
        .filter(user::id.eq(target).and(user::deleted_at.is_null()))
        .for_update()
        .first(conn)
        .await?;
    if system {
        return Err(crate::Error::Validation(t!("banSystemAccount")));
    }
    access.require_above(deployment_access(conn, target).await?.rank())?;
    diesel::update(user::table.find(target))
        .set((
            user::banned_at.eq(diesel::dsl::now),
            user::banned_by.eq(Some(access.user)),
            user::ban_reason.eq(reason),
            user::banned_until.eq(until),
        ))
        .execute(conn)
        .await?;
    crate::login::revoke_all_sessions(state, conn, target).await?;
    log_moderation(
        conn,
        access.user,
        ModerationAction::BanUser,
        None,
        None,
        Some(target.0.to_string()),
    )
    .await?;
    if let Some(window) = delete_messages_seconds {
        log_moderation(
            conn,
            access.user,
            ModerationAction::DeleteRecentMessages,
            None,
            None,
            Some(target.0.to_string()),
        )
        .await?;
        let since = Utc::now() - Duration::seconds(i64::from(window));
        outcome.deleted_messages +=
            crate::message::delete_recent_by(state, conn, None, target, since)
                .await?
                .len();
    }
    // Their bots act for them, so their streams close and they leave their calls too; their
    // tokens are refused while the ban stands (`shut_out`).
    let bots: Vec<UserId> = user::table
        .select(user::id)
        .filter(user::bot_owner.eq(target).and(user::deleted_at.is_null()))
        .load(conn)
        .await?;
    for account in std::iter::once(target).chain(bots) {
        publish_event(
            state,
            conn,
            EventScope::User(account),
            &ServerEvent::AccountBanned {
                reason: reason.clone(),
                until,
            },
        )
        .await?;
    }
    outcome.replaced |= was_banned;
    outcome.banned.push(target);
    Ok(bot_owner)
}

/// Lifts a ban from the deployment. Nothing standing is not an error. Takes Ban users and, as
/// banning does, ranking above the person banned; written to the moderation log when it lifted
/// something.
pub async fn lift_ban(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    target: UserId,
) -> crate::Result<bool> {
    access.require(DeploymentPermission::BanUsers)?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            // Their row is locked as `ban_one` locks it, so a ban and a lift of one person take
            // turns.
            let exists = user::table
                .select(user::id)
                .filter(user::id.eq(target))
                .for_update()
                .first::<UserId>(conn.as_mut())
                .await
                .optional()?
                .is_some();
            if !exists {
                return Ok(false);
            }
            access.require_above(deployment_access(conn.as_mut(), target).await?.rank())?;
            let lifted = diesel::update(user::table.find(target))
                .filter(user::banned_at.is_not_null())
                .set((
                    user::banned_at.eq(None::<DateTime<Utc>>),
                    user::banned_by.eq(None::<UserId>),
                    user::ban_reason.eq(None::<String>),
                    user::banned_until.eq(None::<DateTime<Utc>>),
                ))
                .execute(conn.as_mut())
                .await?;
            if lifted > 0 {
                log_moderation(
                    conn.as_mut(),
                    access.user,
                    ModerationAction::LiftUserBan,
                    None,
                    None,
                    Some(target.0.to_string()),
                )
                .await?;
            }
            Ok(lifted > 0)
        }
        .scope_boxed()
    })
    .await
}
