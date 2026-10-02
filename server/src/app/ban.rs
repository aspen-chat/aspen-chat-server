//! Bans from a community (`community_ban`): a member removed and refused every way back in,
//! with a reason they are told, until the ban ends or is lifted. Banning takes Ban members, and
//! ranks as removing does: only someone below the banner's highest role, never the owner; a
//! deployment moderator's ban is logged. Banning may also delete the person's recent messages
//! in the community, which takes Manage messages besides.

use crate::api::message_enum::server_event::{CommunityBanEvent, ServerEvent};
use crate::api::message_enum::{self};
use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::events::{EventScope, publish_event};
use crate::app::moderation_log::{ModerationAction, log_moderation};
use crate::app::permissions::{Permissions, community_access, require_member};
use crate::app::{CommunityId, UserId};
use crate::database::schema::community_ban;
use crate::t;
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

/// What banning did: the ban, whether one stood already, and the messages deleted.
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
) -> app::Result<()> {
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
        Some(reason) => Err(app::Error::Banned { reason }),
        None => Ok(()),
    }
}

/// The community's standing bans, newest first, for a holder of Ban members.
pub async fn read_bans(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
) -> app::Result<Vec<message_enum::CommunityBan>> {
    let mut conn = state.connection_pool.get().await?;
    let access = require_member(conn.as_mut(), caller, community).await?;
    access.require(Permissions::BAN_MEMBERS)?;
    let rows: Vec<CommunityBanRow> = community_ban::table
        .select(CommunityBanRow::as_select())
        .filter(community_ban::community.eq(community))
        .filter(
            community_ban::until
                .is_null()
                .or(community_ban::until.gt(Utc::now())),
        )
        .order(community_ban::banned_at.desc())
        .load(conn.as_mut())
        .await?;
    Ok(rows.iter().map(message_enum::CommunityBan::from).collect())
}

fn validate(request: &BanRequest) -> app::Result<(Option<String>, Option<DateTime<Utc>>)> {
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
        return Err(app::Error::Validation(t!(
            "banReasonLength",
            max = REASON_MAX_CHARS
        )));
    }
    let until = match request.duration_seconds {
        None => None,
        Some(seconds) if seconds >= MIN_SECONDS => {
            Some(Utc::now() + Duration::seconds(i64::from(seconds)))
        }
        Some(_) => return Err(app::Error::Validation(t!("banDurationRange"))),
    };
    if request
        .delete_messages_seconds
        .is_some_and(|w| !DELETE_WINDOWS_SECONDS.contains(&w))
    {
        return Err(app::Error::Validation(t!("banDeleteWindow")));
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
) -> app::Result<Banned> {
    let (reason, until) = validate(request)?;
    let mut conn = state.connection_pool.get().await?;
    let (banned, deleted) = conn
        .transaction(|conn| {
            async move {
                let access = require_member(conn.as_mut(), caller, community).await?;
                access.require(Permissions::BAN_MEMBERS)?;
                if member == caller {
                    return Err(app::Error::Validation(t!("banSelf")));
                }
                // Someone still a member must rank below the banner; the owner never may be
                // banned, member or not.
                let theirs = community_access(conn.as_mut(), member, community).await?;
                let by_rank = match &theirs {
                    Some(theirs) if theirs.owner => {
                        return Err(app::Error::Forbidden(t!("banOwner")));
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
                    None => Vec::new(),
                    Some(window) => {
                        if !access.has(Permissions::MANAGE_MESSAGES) {
                            return Err(app::permissions::missing(Permissions::MANAGE_MESSAGES));
                        }
                        if access.moderating(Permissions::MANAGE_MESSAGES) {
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
                        app::message::delete_recent_by(
                            state,
                            conn.as_mut(),
                            community,
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
                let replaced: Option<CommunityBanRow> = community_ban::table
                    .select(CommunityBanRow::as_select())
                    .filter(community_ban::community.eq(community))
                    .filter(community_ban::user.eq(member))
                    .first(conn.as_mut())
                    .await
                    .optional()?;
                if replaced.is_some() {
                    diesel::update(community_ban::table)
                        .filter(community_ban::community.eq(community))
                        .filter(community_ban::user.eq(member))
                        .set((
                            community_ban::banned_by.eq(row.banned_by),
                            community_ban::reason.eq(&row.reason),
                            community_ban::banned_at.eq(row.banned_at),
                            community_ban::until.eq(row.until),
                        ))
                        .execute(conn.as_mut())
                        .await?;
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
                } else {
                    diesel::insert_into(community_ban::table)
                        .values(&row)
                        .execute(conn.as_mut())
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
                app::community::end_membership(state, conn.as_mut(), member, community).await?;
                Ok::<_, app::Error>((
                    Banned {
                        ban: record,
                        replaced: replaced.is_some(),
                        deleted_messages: deleted.len(),
                    },
                    deleted,
                ))
            }
            .scope_boxed()
        })
        .await?;
    for id in deleted {
        app::link_preview::delete_images_for_message(state, conn.as_mut(), id).await?;
    }
    Ok(banned)
}

/// Lifts a ban. Nothing standing is not an error. Takes Ban members.
pub async fn lift_ban(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    member: UserId,
) -> app::Result<()> {
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
