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

/// The community's standing mutes, newest first, for a holder of Manage calls.
pub async fn read_mutes(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
) -> crate::Result<Vec<message_enum::VoiceMute>> {
    let mut conn = state.connection_pool.get().await?;
    let access = require_member(conn.as_mut(), caller, community).await?;
    access.require(Permissions::MANAGE_CALLS)?;
    let rows: Vec<VoiceMuteRow> = voice_mute::table
        .select(VoiceMuteRow::as_select())
        .filter(voice_mute::community.eq(community))
        .order(voice_mute::muted_at.desc())
        .load(conn.as_mut())
        .await?;
    Ok(rows.iter().map(message_enum::VoiceMute::from).collect())
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
                    // Two moderators muting at once: the second finds it standing.
                    let inserted = diesel::insert_into(voice_mute::table)
                        .values(&row)
                        .on_conflict_do_nothing()
                        .execute(conn.as_mut())
                        .await?;
                    let record = message_enum::VoiceMute::from(&row);
                    if inserted > 0 {
                        publish_event(
                            state,
                            conn.as_mut(),
                            EventScope::Community(community),
                            &ServerEvent::VoiceMute(VoiceMuteEvent::Create(record.clone())),
                        )
                        .await?;
                    }
                    Ok((Some(record), inserted > 0))
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
