use crate::api::message_enum;
use crate::api::message_enum::request::{CommunityCreateRequest, CommunityUpdateRequest};
use crate::api::message_enum::server_event::{CommunityEvent, ServerEvent, UserCommunityEvent};
use crate::app;
use crate::app::channel::ChannelType;
use crate::app::context::GlobalServerContext;
use crate::app::icon::Icon;
use crate::app::moderation_log::{ModerationAction, log_moderation};
use crate::app::permissions::{Permissions, community_access, missing, require_member};
use crate::app::{
    ChannelId, CommunityId, EventScope, IconId, Loadable, MaybeLoaded, RoleId, UserId,
    publish_event,
};
use crate::database::schema::community;
use crate::database::schema::community_member_role;
use crate::database::schema::community_user;
use crate::t;
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable,
    QueryableByName, Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use futures_util::future::try_join_all;
use std::collections::HashSet;

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = community)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Community {
    pub id: CommunityId,
    pub name: String,
    pub icon: Option<MaybeLoaded<Icon>>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Who holds every permission and alone may delete or hand on the community; `None` until
    /// a deployment operator names one for a community made before owners existed.
    pub owner: Option<UserId>,
}

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = community_user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CommunityUser {
    pub user: UserId,
    pub community: CommunityId,
    pub sort_index: i32,
    pub nickname: Option<String>,
}

impl Loadable for Community {
    type Id = CommunityId;

    async fn load_from_db(state: &GlobalServerContext, id: CommunityId) -> app::Result<Self> {
        community::table
            .select(Community::as_select())
            .filter(
                community::dsl::id
                    .eq(id)
                    .and(community::deleted_at.is_null()),
            )
            .first(&mut state.connection_pool.get().await?)
            .await
            .map_err(Into::into)
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

/// The longest name a community, channel, or category may have, in characters.
pub const MAX_NAME_CHARS: usize = 100;

/// `name` with the space around it taken off, when that leaves from 1 to [`MAX_NAME_CHARS`]
/// characters, of any script; otherwise the refusal `refusal` gives, told the limit.
pub(crate) fn trimmed_name(
    name: &str,
    refusal: impl FnOnce(usize) -> std::borrow::Cow<'static, str>,
) -> app::Result<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(app::Error::Validation(refusal(MAX_NAME_CHARS)));
    }
    Ok(name.to_owned())
}

fn community_name(name: &str) -> app::Result<String> {
    trimmed_name(name, |max| t!("communityNameLength", max = max))
}

/// Makes a community owned by `user`, with the default roles (the creator holding Admin too, so
/// they keep it should they hand the community on) and a first text and voice channel.
pub(crate) async fn create_community(
    state: GlobalServerContext,
    user: UserId,
    command: &CommunityCreateRequest,
) -> Result<Community, app::Error> {
    let name = community_name(&command.name)?;
    let mut conn = state.connection_pool.get().await?;
    let state = &state;
    conn.transaction(|conn| {
        async move {
            if let Some(icon) = command.icon {
                app::icon::require_own(conn.as_mut(), user, icon, t!("iconMissing")).await?;
            }
            let community = Community {
                id: CommunityId::new(),
                icon: command.icon.map(MaybeLoaded::NotLoaded),
                name,
                deleted_at: None,
                owner: Some(user),
            };
            diesel::insert_into(community::table)
                .values(community.clone())
                .execute(conn.as_mut())
                .await?;
            // Nobody else can see the community yet, so it needs no event of its own.
            let admin = app::role::create_default_roles(conn.as_mut(), community.id).await?;
            add_member(state, conn.as_mut(), user, community.id, &[admin]).await?;
            for (name, ty) in [
                (t!("firstTextChannelName"), ChannelType::Text),
                (t!("firstVoiceChannelName"), ChannelType::Voice),
            ] {
                let new = app::channel::NewChannel {
                    name: name.to_string(),
                    sort_index: 0,
                    ty,
                    community: community.id,
                    parent_category: None,
                    plugin_type: None,
                };
                app::channel::insert_channel(state, conn.as_mut(), new, &[]).await?;
            }
            Ok(community)
        }
        .scope_boxed()
    })
    .await
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = community)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CommunityChangeset {
    pub name: Option<String>,
    pub icon: Option<Option<IconId>>,
}

pub(crate) async fn update_community(
    state: &GlobalServerContext,
    caller: UserId,
    id: CommunityId,
    mut command: CommunityUpdateRequest,
) -> app::error::Result<Community> {
    command.name = command.name.as_deref().map(community_name).transpose()?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = require_member(conn.as_mut(), caller, id).await?;
            // A deployment moderator may rename a community, and do nothing else to it here.
            if !access.has(Permissions::MANAGE_COMMUNITY) {
                if !(access.moderator && command.icon.is_none()) {
                    return Err(missing(Permissions::MANAGE_COMMUNITY));
                }
                log_moderation(
                    conn.as_mut(),
                    caller,
                    ModerationAction::RenameCommunity,
                    Some(id),
                    None,
                    command.name.clone(),
                )
                .await?;
            }
            // A new icon must be one the caller uploaded; the one it has may stay.
            if let Some(Some(icon)) = command.icon {
                let current: Option<IconId> = community::table
                    .select(community::icon)
                    .filter(community::id.eq(id))
                    .first(conn.as_mut())
                    .await?;
                if current != Some(icon) {
                    app::icon::require_own(conn.as_mut(), caller, icon, t!("iconMissing")).await?;
                }
            }
            let Some(community) = diesel::update(community::table)
                .set(CommunityChangeset {
                    name: command.name.clone(),
                    icon: command.icon,
                })
                .filter(community::id.eq(id).and(community::deleted_at.is_null()))
                .returning(Community::as_select())
                .load(conn.as_mut())
                .await?
                .into_iter()
                .next()
            else {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            };
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Community(id),
                &ServerEvent::Community(CommunityEvent::Update {
                    id,
                    name: command.name,
                    icon: command.icon,
                    owner: None,
                }),
            )
            .await?;
            Ok(community)
        }
        .scope_boxed()
    })
    .await
}

/// A community the caller belongs to; any other is answered as not found.
pub(crate) async fn read_community(
    state: &GlobalServerContext,
    caller: UserId,
    id: CommunityId,
) -> app::error::Result<Community> {
    require_member(state.connection_pool.get().await?.as_mut(), caller, id).await?;
    Community::load_from_db(state, id).await
}

/// The communities named in `ids` that still exist, for a deployment reviewer who need not
/// belong to them (`app::report`); the caller has decided who may.
pub(crate) async fn read_communities(
    state: &GlobalServerContext,
    ids: &[CommunityId],
) -> app::error::Result<Vec<Community>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(community::table
        .select(Community::as_select())
        .filter(
            community::id
                .eq_any(ids)
                .and(community::deleted_at.is_null()),
        )
        .load(state.connection_pool.get().await?.as_mut())
        .await?)
}

/// The community an invite leads to, which holding its code is enough to see.
pub(crate) async fn read_invited_community(
    state: &GlobalServerContext,
    id: CommunityId,
) -> app::error::Result<Community> {
    Community::load_from_db(state, id).await
}

/// Deletes a community. Only its owner may, or a deployment moderator.
pub(crate) async fn delete_community(
    state: &GlobalServerContext,
    caller: UserId,
    id: CommunityId,
) -> app::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let access = require_member(conn.as_mut(), caller, id).await?;
            if !access.owner {
                if !access.moderator {
                    return Err(app::Error::Forbidden(t!("permissionOwnerOnly")));
                }
                log_moderation(
                    conn.as_mut(),
                    caller,
                    ModerationAction::DeleteCommunity,
                    Some(id),
                    None,
                    None,
                )
                .await?;
            }
            let deleted = diesel::update(community::table)
                .set(community::deleted_at.eq(diesel::dsl::now))
                .filter(community::id.eq(id).and(community::deleted_at.is_null()))
                .execute(conn.as_mut())
                .await?;
            if deleted == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            // What plugins kept about it goes with it.
            app::plugin::storage::forget(conn.as_mut(), app::plugin::storage::Scope::Community(id))
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Community(id),
                &ServerEvent::Community(CommunityEvent::Delete { id }),
            )
            .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Adds `user` to the community with an invite to it, and returns the membership as the event
/// carried it.
pub(crate) async fn join_community(
    state: &GlobalServerContext,
    user: UserId,
    community: CommunityId,
    invite_code: String,
) -> app::error::Result<message_enum::UserCommunity> {
    let mut conn = state.connection_pool.get().await?;
    let membership = conn
        .transaction(|conn| {
            async move {
                let invite_community = app::invite::validate_invite(conn, &invite_code).await?;
                if invite_community != community {
                    return Err(app::Error::Validation(t!("inviteCodeCommunityMismatch")));
                }
                ensure_room_for_another(state, conn, user).await?;
                add_member(state, conn, user, community, &[]).await
            }
            .scope_boxed()
        })
        .await?;
    app::everyone_limit::after_join(state, community).await;
    Ok(membership)
}

/// Adds `user` to `community` holding `roles` besides everyone's, at the end of their own list,
/// and announces the membership.
/// Refuses `user` another community once they belong to as many as a user may.
pub(crate) async fn ensure_room_for_another(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user: UserId,
) -> app::Result<()> {
    let held: i64 = community_user::table
        .filter(community_user::user.eq(user))
        .count()
        .get_result(conn)
        .await?;
    let cap = state.config.limits.max_communities_per_user;
    if held >= i64::from(cap) {
        return Err(app::Error::Validation(t!("communityLimit", max = cap)));
    }
    Ok(())
}

pub(crate) async fn add_member(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user: UserId,
    community: CommunityId,
    roles: &[RoleId],
) -> app::Result<message_enum::UserCommunity> {
    // Every way in passes here, so a standing ban refuses them all.
    app::ban::check_not_banned(conn, community, user).await?;
    let last: Option<i32> = community_user::table
        .filter(community_user::user.eq(user))
        .select(diesel::dsl::max(community_user::sort_index))
        .first(conn)
        .await?;
    let sort_index = last.map_or(0, |last| last + 1);
    diesel::insert_into(community_user::table)
        .values(&CommunityUser {
            user,
            community,
            sort_index,
            nickname: None,
        })
        .execute(conn)
        .await?;
    let rows: Vec<_> = roles
        .iter()
        .map(|role| {
            (
                community_member_role::user.eq(user),
                community_member_role::community.eq(community),
                community_member_role::role.eq(*role),
            )
        })
        .collect();
    if !rows.is_empty() {
        diesel::insert_into(community_member_role::table)
            .values(rows)
            .execute(conn)
            .await?;
    }
    app::user_status::list_in_community(state, user, community);
    let membership = message_enum::UserCommunity {
        community,
        user,
        sort_index: Some(sort_index),
        roles: roles.to_vec(),
        nickname: None,
    };
    let event = ServerEvent::UserCommunity(UserCommunityEvent::Create(membership.clone()));
    app::publish_event(
        state,
        conn,
        EventScope::Membership { community, user },
        &event,
    )
    .await?;
    Ok(membership)
}

/// The caller's membership of a community, or `NotFound` when they are not a member.
pub(crate) async fn read_membership(
    state: &GlobalServerContext,
    user: UserId,
    community: CommunityId,
) -> app::error::Result<message_enum::UserCommunity> {
    let mut conn = state.connection_pool.get().await?;
    let row: CommunityUser = community_user::table
        .select(CommunityUser::as_select())
        .filter(
            community_user::community
                .eq(community)
                .and(community_user::user.eq(user)),
        )
        .first(conn.as_mut())
        .await?;
    let roles = app::role::roles_of_members(conn.as_mut(), &[(community, user)])
        .await?
        .remove(&(community, user))
        .unwrap_or_default();
    Ok(message_enum::UserCommunity {
        community: row.community,
        user: row.user,
        sort_index: Some(row.sort_index),
        roles,
        nickname: row.nickname,
    })
}

/// Checks a nickname, which must be non-blank and no longer than a display name when set, and
/// trims it; `None` clears it.
fn checked_nickname(nickname: Option<String>) -> app::Result<Option<String>> {
    let Some(nickname) = nickname else {
        return Ok(None);
    };
    let nickname = nickname.trim();
    if nickname.is_empty() || nickname.chars().count() > app::user::DISPLAY_NAME_MAX_CHARS {
        return Err(app::Error::Validation(t!(
            "nicknameLength",
            max = app::user::DISPLAY_NAME_MAX_CHARS
        )));
    }
    Ok(Some(nickname.to_string()))
}

/// Changes `user`'s own membership: where the community sits in their list, and their nickname
/// there, which takes Change nickname to set but nothing to clear. Absent fields are unchanged.
/// Announces what changed, the list position to them alone, and returns the membership.
pub(crate) async fn update_membership(
    state: &GlobalServerContext,
    user: UserId,
    community: CommunityId,
    sort_index: Option<i32>,
    nickname: Option<Option<String>>,
) -> app::error::Result<message_enum::UserCommunity> {
    let nickname = nickname.map(checked_nickname).transpose()?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let row: CommunityUser = community_user::table
                .select(CommunityUser::as_select())
                .filter(
                    community_user::community
                        .eq(community)
                        .and(community_user::user.eq(user)),
                )
                .for_update()
                .first(conn.as_mut())
                .await?;
            if let Some(Some(_)) = &nickname {
                require_member(conn.as_mut(), user, community)
                    .await?
                    .require(Permissions::CHANGE_NICKNAME)?;
            }
            let sort_index = sort_index.filter(|index| *index != row.sort_index);
            let nickname = nickname.filter(|nickname| *nickname != row.nickname);
            if sort_index.is_some() || nickname.is_some() {
                #[derive(AsChangeset)]
                #[diesel(table_name = community_user)]
                struct Change {
                    sort_index: Option<i32>,
                    nickname: Option<Option<String>>,
                }
                diesel::update(community_user::table)
                    .filter(
                        community_user::community
                            .eq(community)
                            .and(community_user::user.eq(user)),
                    )
                    .set(Change {
                        sort_index,
                        nickname: nickname.clone(),
                    })
                    .execute(conn.as_mut())
                    .await?;
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::Membership { community, user },
                    &ServerEvent::UserCommunity(UserCommunityEvent::Update {
                        community,
                        user,
                        sort_index: sort_index.map(Some),
                        roles: None,
                        nickname: nickname.clone(),
                    }),
                )
                .await?;
            }
            let roles = app::role::roles_of_members(conn.as_mut(), &[(community, user)])
                .await?
                .remove(&(community, user))
                .unwrap_or_default();
            Ok(message_enum::UserCommunity {
                community,
                user,
                sort_index: Some(sort_index.unwrap_or(row.sort_index)),
                roles,
                nickname: nickname.unwrap_or(row.nickname),
            })
        }
        .scope_boxed()
    })
    .await
}

/// Clears `member`'s nickname in `community`, if they have one, and announces it, inside the
/// caller's transaction, with no checks. Returns whether there was one to clear.
pub(crate) async fn erase_nickname(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    member: UserId,
) -> app::Result<bool> {
    let cleared = diesel::update(community_user::table)
        .filter(
            community_user::community
                .eq(community)
                .and(community_user::user.eq(member))
                .and(community_user::nickname.is_not_null()),
        )
        .set(community_user::nickname.eq(None::<String>))
        .execute(conn)
        .await?;
    if cleared > 0 {
        publish_event(
            state,
            conn,
            EventScope::Membership {
                community,
                user: member,
            },
            &ServerEvent::UserCommunity(UserCommunityEvent::Update {
                community,
                user: member,
                sort_index: None,
                roles: None,
                nickname: Some(None),
            }),
        )
        .await?;
    }
    Ok(cleared > 0)
}

/// Takes the caller out of a community. Its owner cannot leave; they must hand it on first.
pub(crate) async fn leave_community(
    state: &GlobalServerContext,
    user: UserId,
    community: CommunityId,
) -> app::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    if let Some(access) = community_access(conn.as_mut(), user, community).await?
        && access.owner
    {
        return Err(app::Error::Validation(t!("ownerCannotLeave")));
    }
    end_membership(state, conn.as_mut(), user, community).await
}

/// Ends `user`'s membership of `community`, and with it every role they held there, announcing
/// it in the same transaction; a bot's own role there goes too. Nothing happens when they are
/// not a member.
pub(crate) async fn end_membership(
    state: &impl app::events::Publishing,
    conn: &mut AsyncPgConnection,
    user: UserId,
    community: CommunityId,
) -> app::Result<()> {
    conn.transaction(|conn| {
        async move {
            let deleted = diesel::delete(community_user::table)
                .filter(
                    community_user::community
                        .eq(community)
                        .and(community_user::user.eq(user)),
                )
                .execute(conn)
                .await?;
            if deleted > 0 {
                let event =
                    ServerEvent::UserCommunity(UserCommunityEvent::Delete { community, user });
                app::publish_event(
                    state,
                    conn,
                    EventScope::Membership { community, user },
                    &event,
                )
                .await?;
                app::role::delete_bot_role(state, conn, community, user).await?;
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Members a single community read returns, capped so a large community cannot make its member
/// list unbounded; which ones is [`read_community_members`].
pub const MEMBERS_PER_COMMUNITY: i64 = 100;

/// One row of [`read_community_members`]: a member together with the community the row was
/// selected for. A user in several of the requested communities appears once per community.
#[derive(Debug, QueryableByName)]
pub struct CommunityMember {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub community: CommunityId,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub sort_index: i32,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    pub nickname: Option<String>,
    #[diesel(embed)]
    pub user: app::user::UserPg,
}

/// A membership as [`read_community_members`] returns it.
pub struct Membership {
    pub community: CommunityId,
    pub user: app::user::User,
    /// Where the community sits in this user's own list, for the caller's own membership only:
    /// it is theirs alone.
    pub sort_index: Option<i32>,
    /// The roles they hold there besides everyone's, lowest first.
    pub roles: Vec<RoleId>,
    /// Their name in the community, shown there in place of their display name.
    pub nickname: Option<String>,
}

/// The member sample of each of `communities`, at most [`MEMBERS_PER_COMMUNITY`] per community,
/// grouped by community and in order of priority within each group. Those with a connection
/// (online or away, `app::user_status::connected_members`) come first, and among them those
/// holding a role shown apart, by the rank of their highest such role, even when they fill the
/// whole sample; then the rest, connected before not, each by when they last came online. The
/// caller's own membership of each community is always among them, wherever they rank, because
/// their memberships carry the order of their own community list. One query serves any number of
/// communities: the per-community cap is a window function rather than a `LIMIT`, so sideloading
/// members for a user's whole community list costs one round trip.
pub(crate) async fn read_community_members(
    state: &GlobalServerContext,
    caller: UserId,
    communities: &[CommunityId],
) -> app::error::Result<Vec<Membership>> {
    use diesel::sql_types::{Array, BigInt, Uuid};

    if communities.is_empty() {
        return Ok(Vec::new());
    }
    // Presence is anyone's, whichever community they were found connected through.
    let connected: HashSet<UserId> = try_join_all(
        communities
            .iter()
            .map(|community| app::user_status::connected_members(state, *community)),
    )
    .await?
    .iter()
    .flat_map(|members| members.iter().copied())
    .collect();
    let mut conn = state.connection_pool.get().await?;
    // The rank of a member's highest role shown apart is looked up only for those connected,
    // since it orders no one else.
    let rows: Vec<CommunityMember> = diesel::sql_query(
        r#"
        SELECT community, sort_index, nickname, id, name, password_hash, icon, created_at, last_seen_at,
               deleted_at, display_name, pronouns, bio, status_text, status_emoji, bot, system,
               bot_owner, bot_public, home_domain, home_id, home_icon, name_hue, plugin,
               public_email
        FROM (
            SELECT cu.community, cu.sort_index, cu.nickname, u.*,
                   ROW_NUMBER() OVER (
                       PARTITION BY cu.community
                       ORDER BY connected.id IS NOT NULL DESC,
                                CASE WHEN connected.id IS NOT NULL THEN (
                                    SELECT max(r.position)
                                    FROM community_member_role mr
                                    JOIN community_role r ON r.id = mr.role
                                    WHERE mr.community = cu.community AND mr."user" = cu."user"
                                      AND r.hoist
                                ) END DESC NULLS LAST,
                                u.last_seen_at DESC,
                                u.id
                   ) AS priority
            FROM community_user cu
            JOIN "user" u ON u.id = cu."user"
            LEFT JOIN unnest($4::uuid[]) AS connected(id) ON connected.id = u.id
            WHERE cu.community = ANY($1) AND u.deleted_at IS NULL
        ) ranked
        WHERE priority <= $2 OR id = $3
        ORDER BY community, priority
        "#,
    )
    .bind::<Array<Uuid>, _>(communities.iter().map(|c| c.0).collect::<Vec<_>>())
    .bind::<BigInt, _>(MEMBERS_PER_COMMUNITY)
    .bind::<Uuid, _>(caller.0)
    .bind::<Array<Uuid>, _>(connected.iter().map(|u| u.0).collect::<Vec<_>>())
    .load(conn.as_mut())
    .await?;
    memberships_of(state, conn.as_mut(), caller, rows).await
}

/// Member rows as memberships, as `caller` may see them: with the roles each holds and their
/// online status, and the list position only of the caller's own.
async fn memberships_of(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    caller: UserId,
    rows: Vec<CommunityMember>,
) -> app::Result<Vec<Membership>> {
    let mut communities = Vec::with_capacity(rows.len());
    let mut sort_indexes = Vec::with_capacity(rows.len());
    let mut nicknames = Vec::with_capacity(rows.len());
    let mut users = Vec::with_capacity(rows.len());
    for row in rows {
        communities.push(row.community);
        sort_indexes.push(row.sort_index);
        nicknames.push(row.nickname);
        users.push(row.user);
    }
    let keys: Vec<(CommunityId, UserId)> = communities
        .iter()
        .zip(&users)
        .map(|(community, user)| (*community, user.id))
        .collect();
    let mut roles = app::role::roles_of_members(conn, &keys).await?;
    let users = app::user::with_online_status(state, users).await?;
    Ok(communities
        .into_iter()
        .zip(sort_indexes)
        .zip(nicknames)
        .zip(users)
        .map(|(((community, sort_index), nickname), user)| Membership {
            community,
            nickname,
            roles: roles
                .remove(&(community, user.user_pg.id))
                .unwrap_or_default(),
            sort_index: (user.user_pg.id == caller).then_some(sort_index),
            user,
        })
        .collect())
}

/// The most results one page of a member search holds.
pub const MAX_MEMBER_PAGE: i64 = 50;
/// The furthest into a member search a page may start.
pub const MAX_MEMBER_OFFSET: i64 = 10_000;

/// One page of `community`'s members whose username, display name, or nickname there contains
/// `search`, by the name the community shows: `limit` of them from `offset`.
///
/// In a community no bigger than the member sample (`MEMBERS_PER_COMMUNITY`), which every
/// member already reads whole, anyone in it may search. In a larger one only those who act on
/// members may: its owner, a deployment moderator, and holders of Assign roles, Remove members,
/// Manage channels, or Manage categories (whose access settings name members). Everyone else
/// sees the sample and no more, so a large community's full membership cannot be listed by any
/// member, a page or a search at a time.
pub(crate) async fn search_community_members(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    search: Option<&str>,
    offset: i64,
    limit: i64,
) -> app::Result<Vec<Membership>> {
    use diesel::sql_types::{BigInt, Nullable, Text, Uuid};
    let mut conn = state.connection_pool.get().await?;
    let access = require_member(conn.as_mut(), caller, community).await?;
    let privileged = access.owner
        || access.moderator
        || [
            Permissions::ASSIGN_ROLES,
            Permissions::REMOVE_MEMBERS,
            Permissions::MANAGE_CHANNELS,
            Permissions::MANAGE_CATEGORIES,
        ]
        .into_iter()
        .any(|p| access.has(p));
    if !privileged {
        let members: i64 = community_user::table
            .filter(community_user::community.eq(community))
            .count()
            .get_result(conn.as_mut())
            .await?;
        if members > MEMBERS_PER_COMMUNITY {
            return Err(app::Error::Forbidden(t!("memberSearchRefused")));
        }
    }
    let rows: Vec<CommunityMember> = diesel::sql_query(
        r#"
        SELECT cu.community, cu.sort_index, cu.nickname, u.id, u.name, u.password_hash, u.icon, u.created_at,
               u.last_seen_at, u.deleted_at, u.display_name, u.pronouns, u.bio, u.status_text,
               u.status_emoji, u.bot, u.system, u.bot_owner, u.bot_public, u.home_domain,
               u.home_id, u.home_icon, u.name_hue, u.plugin,
               u.public_email
        FROM community_user cu
        JOIN "user" u ON u.id = cu."user"
        WHERE cu.community = $1 AND u.deleted_at IS NULL
          AND ($2::text IS NULL OR lower(u.name) LIKE $2 OR lower(u.display_name) LIKE $2
               OR lower(cu.nickname) LIKE $2)
        ORDER BY lower(COALESCE(cu.nickname, u.display_name, u.name)), u.id
        OFFSET $3 LIMIT $4
        "#,
    )
    .bind::<Uuid, _>(community.0)
    .bind::<Nullable<Text>, _>(app::admin::contains_pattern(search))
    .bind::<BigInt, _>(offset.clamp(0, MAX_MEMBER_OFFSET))
    .bind::<BigInt, _>(limit.clamp(1, MAX_MEMBER_PAGE))
    .load(conn.as_mut())
    .await?;
    memberships_of(state, conn.as_mut(), caller, rows).await
}

/// The member sample of one community, for a member of it (or a deployment moderator).
pub(crate) async fn read_community_sample(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
) -> app::error::Result<Vec<Membership>> {
    require_member(
        state.connection_pool.get().await?.as_mut(),
        caller,
        community,
    )
    .await?;
    read_community_members(state, caller, &[community]).await
}

/// The memberships of the people who wrote messages, each of the community its channel is in (a
/// thread's included), with the roles they hold there: what a reader needs to draw an author's
/// name in their roles' colour, whether or not they are in the member sample. `written` pairs
/// each channel with an author; whoever reads a channel's messages may see the roles of those
/// who wrote them, so the caller's having read them is the check. Authors of DMs, and those who
/// have since left, have none.
pub(crate) async fn read_authors_memberships(
    state: &GlobalServerContext,
    caller: UserId,
    written: &[(ChannelId, UserId)],
) -> app::Result<Vec<Membership>> {
    use diesel::sql_types::{Array, Uuid};
    if written.is_empty() {
        return Ok(Vec::new());
    }
    let (channels, authors): (Vec<uuid::Uuid>, Vec<uuid::Uuid>) =
        written.iter().map(|(c, u)| (c.0, u.0)).unzip();
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<CommunityMember> = diesel::sql_query(
        r#"
        SELECT DISTINCT cu.community, cu.sort_index, cu.nickname, u.id, u.name, u.password_hash, u.icon,
               u.created_at, u.last_seen_at, u.deleted_at, u.display_name, u.pronouns, u.bio,
               u.status_text, u.status_emoji, u.bot, u.system, u.bot_owner, u.bot_public,
               u.home_domain, u.home_id, u.home_icon, u.name_hue, u.plugin,
               u.public_email
        FROM unnest($1::uuid[], $2::uuid[]) AS written(channel, author)
        JOIN channel c ON c.id = written.channel
        JOIN community_user cu ON cu.community = c.community AND cu."user" = written.author
        JOIN "user" u ON u.id = cu."user"
        WHERE u.deleted_at IS NULL
        "#,
    )
    .bind::<Array<Uuid>, _>(channels)
    .bind::<Array<Uuid>, _>(authors)
    .load(conn.as_mut())
    .await?;
    memberships_of(state, conn.as_mut(), caller, rows).await
}

/// One member of `community`, with their roles, for a member of it (or a deployment moderator):
/// who reads someone's messages there may see what roles they hold, whether or not they are in
/// the member sample. Someone who is not a member is not found.
pub(crate) async fn read_community_member(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    member: UserId,
) -> app::Result<Membership> {
    use diesel::sql_types::Uuid;
    let mut conn = state.connection_pool.get().await?;
    require_member(conn.as_mut(), caller, community).await?;
    let rows: Vec<CommunityMember> = diesel::sql_query(
        r#"
        SELECT cu.community, cu.sort_index, cu.nickname, u.id, u.name, u.password_hash, u.icon, u.created_at,
               u.last_seen_at, u.deleted_at, u.display_name, u.pronouns, u.bio, u.status_text,
               u.status_emoji, u.bot, u.system, u.bot_owner, u.bot_public, u.home_domain,
               u.home_id, u.home_icon, u.name_hue, u.plugin,
               u.public_email
        FROM community_user cu
        JOIN "user" u ON u.id = cu."user"
        WHERE cu.community = $1 AND cu."user" = $2 AND u.deleted_at IS NULL
        "#,
    )
    .bind::<Uuid, _>(community.0)
    .bind::<Uuid, _>(member.0)
    .load(conn.as_mut())
    .await?;
    memberships_of(state, conn.as_mut(), caller, rows)
        .await?
        .pop()
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))
}
