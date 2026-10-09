//! Bans from a community (`community_ban`): a member removed and refused every way back in,
//! with a reason they are told, until the ban ends or is lifted. Banning takes Ban members, and
//! ranks as removing does: only someone below the banner's highest role, never the owner; a
//! deployment moderator's ban is logged. Banning may also delete the person's recent messages
//! in the community, which takes Manage messages besides.

use crate::context::GlobalServerContext;
use crate::events::{EventScope, publish_event};
use crate::moderation_log::{ModerationAction, log_moderation};
use crate::permissions::{Permissions, community_access, require_member};
use crate::t;
use crate::visibility::Visibility;
use crate::{CommunityId, UserId};
use aspen_schema::community_ban;
use aspen_wire::message_enum::server_event::{CommunityBanEvent, ServerEvent};
use aspen_wire::message_enum::{self};
use chrono::{DateTime, Duration, Utc};
use diesel::{
    BoolExpressionMethods, ExpressionMethods, Insertable, OptionalExtension, QueryDsl, Queryable,
    Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

/// How long a reason may be, in characters.
pub const REASON_MAX_CHARS: usize = 512;
/// The shortest ban, in seconds; anything shorter is a removal.
pub const MIN_SECONDS: u32 = 60;
/// How far back a ban may delete the person's messages: the last hour, or the last day.
pub const DELETE_WINDOWS_SECONDS: [u32; 2] = [3_600, 86_400];

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = community_ban)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CommunityBanRow {
    pub community: CommunityId,
    pub user: UserId,
    pub banned_by: Option<UserId>,
    pub reason: Option<String>,
    pub banned_at: DateTime<Utc>,
    pub until: Option<DateTime<Utc>>,
}

impl From<&CommunityBanRow> for message_enum::CommunityBan {
    fn from(row: &CommunityBanRow) -> Self {
        message_enum::CommunityBan {
            community: row.community,
            user: row.user,
            reason: row.reason.clone(),
            until: row.until,
            banned_by: row.banned_by,
            banned_at: row.banned_at,
        }
    }
}

/// What a ban asks for.
#[derive(Debug, Clone, Default)]
pub struct BanRequest {
    pub reason: Option<String>,
    /// How long it lasts, or `None` until lifted.
    pub duration_seconds: Option<u32>,
    /// How far back the person's messages in the community are deleted, one of
    /// `DELETE_WINDOWS_SECONDS`, or `None` to leave them.
    pub delete_messages_seconds: Option<u32>,
}

/// What banning did: the ban, whether one stood already, and how many messages are being
/// deleted, a batch at a time once the ban commits (`message::queue_deletion_of_recent`).
pub struct Banned {
    pub ban: message_enum::CommunityBan,
    pub replaced: bool,
    pub deleted_messages: usize,
}

/// Refuses `user` the community while a ban of them stands (`Error::Banned`, with the reason).
/// Every way in checks this (`app::community::add_member`).
pub async fn check_not_banned(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    user: UserId,
) -> crate::Result<()> {
    let standing: Option<Option<String>> = community_ban::table
        .select(community_ban::reason)
        .filter(community_ban::community.eq(community))
        .filter(community_ban::user.eq(user))
        .filter(
            community_ban::until
                .is_null()
                .or(community_ban::until.gt(Utc::now())),
        )
        .first(conn)
        .await
        .optional()?;
    match standing {
        Some(reason) => Err(crate::Error::Banned { reason }),
        None => Ok(()),
    }
}

/// A page of the community's standing bans, newest first, for a holder of Ban members: `limit`
/// of them (at most [`crate::LIST_PAGE`]) after the ban of `before`, through
/// `community_ban_listed`.
pub async fn read_bans(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    before: Option<UserId>,
    limit: i64,
) -> crate::Result<Vec<message_enum::CommunityBan>> {
    let mut conn = state.connection_pool.get().await?;
    let access = require_member(conn.as_mut(), caller, community).await?;
    access.require(Permissions::BAN_MEMBERS)?;
    let mut query = community_ban::table
        .select(CommunityBanRow::as_select())
        .filter(community_ban::community.eq(community))
        .filter(
            community_ban::until
                .is_null()
                .or(community_ban::until.gt(Utc::now())),
        )
        .into_boxed();
    if let Some(before) = before {
        let Some((at, user)) = community_ban::table
            .select((community_ban::banned_at, community_ban::user))
            .filter(community_ban::community.eq(community))
            .filter(community_ban::user.eq(before))
            .first::<(DateTime<Utc>, UserId)>(conn.as_mut())
            .await
            .optional()?
        else {
            // The ban the page continues after was lifted meanwhile; the caller reads again.
            return Ok(Vec::new());
        };
        query = query.filter(
            community_ban::banned_at.lt(at).or(community_ban::banned_at
                .eq(at)
                .and(community_ban::user.lt(user))),
        );
    }
    let rows: Vec<CommunityBanRow> = query
        .order((community_ban::banned_at.desc(), community_ban::user.desc()))
        .limit(limit.clamp(1, crate::LIST_PAGE))
        .load(conn.as_mut())
        .await?;
    Ok(rows.iter().map(message_enum::CommunityBan::from).collect())
}

pub fn validate(request: &BanRequest) -> crate::Result<(Option<String>, Option<DateTime<Utc>>)> {
    let reason = request
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_string);
    if reason
        .as_ref()
        .is_some_and(|r| r.chars().count() > REASON_MAX_CHARS)
    {
        return Err(crate::Error::Validation(t!(
            "banReasonLength",
            max = REASON_MAX_CHARS
        )));
    }
    let until = match request.duration_seconds {
        None => None,
        Some(seconds) if seconds >= MIN_SECONDS => {
            Some(Utc::now() + Duration::seconds(i64::from(seconds)))
        }
        Some(_) => return Err(crate::Error::Validation(t!("banDurationRange"))),
    };
    if request
        .delete_messages_seconds
        .is_some_and(|w| !DELETE_WINDOWS_SECONDS.contains(&w))
    {
        return Err(crate::Error::Validation(t!("banDeleteWindow")));
    }
    Ok((reason, until))
}

/// Bans `member` from the community: their membership ends as removal ends it, a ban that
/// stood is replaced, and, asked to, their messages in the community from the last hour or day
/// are deleted. Someone who has already left may be banned too, by their id.
pub async fn ban_member(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    member: UserId,
    request: &BanRequest,
) -> crate::Result<Banned> {
    let (reason, until) = validate(request)?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = require_member(conn.as_mut(), caller, community).await?;
            access.require(Permissions::BAN_MEMBERS)?;
            if member == caller {
                return Err(crate::Error::Validation(t!("banSelf")));
            }
            // Someone still a member must rank below the banner; the owner never may be
            // banned, member or not.
            let theirs = community_access(conn.as_mut(), member, community).await?;
            let by_rank = match &theirs {
                Some(theirs) if theirs.owner => {
                    return Err(crate::Error::Forbidden(t!("banOwner")));
                }
                Some(theirs) if theirs.member => {
                    access.require_above(theirs.role_rank())?;
                    theirs.role_rank() < access.role_rank()
                }
                _ => true,
            };
            if access.moderator
                && !(access.member_permissions.contains(Permissions::BAN_MEMBERS) && by_rank)
            {
                crate::deployment::require_outranks(conn.as_mut(), caller, member).await?;
                log_moderation(
                    conn.as_mut(),
                    caller,
                    ModerationAction::BanMember,
                    Some(community),
                    None,
                    Some(member.0.to_string()),
                )
                .await?;
            }
            let deleted = match request.delete_messages_seconds {
                None => 0,
                Some(window) => {
                    if !access.has(Permissions::MANAGE_MESSAGES) {
                        return Err(crate::permissions::missing(Permissions::MANAGE_MESSAGES));
                    }
                    if access.moderating(Permissions::MANAGE_MESSAGES) {
                        crate::deployment::require_outranks(conn.as_mut(), caller, member).await?;
                        log_moderation(
                            conn.as_mut(),
                            caller,
                            ModerationAction::DeleteRecentMessages,
                            Some(community),
                            None,
                            Some(member.0.to_string()),
                        )
                        .await?;
                    }
                    let since = Utc::now() - Duration::seconds(i64::from(window));
                    // Only where the banner may view, as deleting one by one would allow.
                    let visible = Visibility::load_on(conn.as_mut(), caller, &[community]).await?;
                    crate::message::queue_deletion_of_recent(
                        conn.as_mut(),
                        Some(&visible),
                        member,
                        since,
                    )
                    .await?
                }
            };
            let row = CommunityBanRow {
                community,
                user: member,
                banned_by: Some(caller),
                reason,
                banned_at: Utc::now(),
                until,
            };
            // One statement makes the ban or replaces the one standing, and says which, so
            // two bans of one member at once both succeed, the later one standing.
            let made: bool = diesel::insert_into(community_ban::table)
                .values(&row)
                .on_conflict((community_ban::community, community_ban::user))
                .do_update()
                .set((
                    community_ban::banned_by.eq(row.banned_by),
                    community_ban::reason.eq(&row.reason),
                    community_ban::banned_at.eq(row.banned_at),
                    community_ban::until.eq(row.until),
                ))
                .returning(diesel::dsl::sql::<diesel::sql_types::Bool>("xmax = 0"))
                .get_result(conn.as_mut())
                .await?;
            let replaced = !made;
            if replaced {
                // A ban's record does not change in place: the one that stood goes, and
                // the new one is announced whole.
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::Community(community),
                    &ServerEvent::CommunityBan(CommunityBanEvent::Delete {
                        community,
                        user: member,
                    }),
                )
                .await?;
            }
            let record = message_enum::CommunityBan::from(&row);
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Community(community),
                &ServerEvent::CommunityBan(CommunityBanEvent::Create(record.clone())),
            )
            .await?;
            crate::community::end_membership(state, conn.as_mut(), member, community).await?;
            Ok::<_, crate::Error>(Banned {
                ban: record,
                replaced,
                deleted_messages: deleted,
            })
        }
        .scope_boxed()
    })
    .await
}

/// Lifts a ban. Nothing standing is not an error. Takes Ban members.
pub async fn lift_ban(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    member: UserId,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = require_member(conn.as_mut(), caller, community).await?;
            access.require(Permissions::BAN_MEMBERS)?;
            if access.moderating(Permissions::BAN_MEMBERS) {
                log_moderation(
                    conn.as_mut(),
                    caller,
                    ModerationAction::LiftBan,
                    Some(community),
                    None,
                    Some(member.0.to_string()),
                )
                .await?;
            }
            let deleted = diesel::delete(community_ban::table)
                .filter(community_ban::community.eq(community))
                .filter(community_ban::user.eq(member))
                .execute(conn.as_mut())
                .await?;
            if deleted > 0 {
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::Community(community),
                    &ServerEvent::CommunityBan(CommunityBanEvent::Delete {
                        community,
                        user: member,
                    }),
                )
                .await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reason_is_trimmed_and_bounded() {
        let (reason, until) = validate(&BanRequest {
            reason: Some("  spam  ".into()),
            ..BanRequest::default()
        })
        .unwrap();
        assert_eq!(reason.as_deref(), Some("spam"));
        assert!(until.is_none());
        assert!(
            validate(&BanRequest {
                reason: Some("x".repeat(REASON_MAX_CHARS + 1)),
                ..BanRequest::default()
            })
            .is_err()
        );
        assert!(
            validate(&BanRequest {
                reason: Some("   ".into()),
                ..BanRequest::default()
            })
            .unwrap()
            .0
            .is_none()
        );
    }

    #[test]
    fn a_duration_is_at_least_a_minute_and_a_window_is_an_hour_or_a_day() {
        assert!(
            validate(&BanRequest {
                duration_seconds: Some(30),
                ..BanRequest::default()
            })
            .is_err()
        );
        assert!(
            validate(&BanRequest {
                duration_seconds: Some(3_600),
                ..BanRequest::default()
            })
            .unwrap()
            .1
            .is_some()
        );
        assert!(
            validate(&BanRequest {
                delete_messages_seconds: Some(600),
                ..BanRequest::default()
            })
            .is_err()
        );
        assert!(
            validate(&BanRequest {
                delete_messages_seconds: Some(86_400),
                ..BanRequest::default()
            })
            .is_ok()
        );
    }
}
