use crate::api::message_enum::request::{CommunityCreateRequest, CommunityUpdateRequest};
use crate::api::message_enum::server_event::{CommunityEvent, ServerEvent, UserCommunityEvent};
use crate::api::{ChannelType, GlobalServerContext, message_enum};
use crate::app;
use crate::app::deployment::{ModerationAction, log_moderation};
use crate::app::icon::Icon;
use crate::app::permissions::{Permissions, community_access, missing, require_member};
use crate::app::{
    CommunityId, EventScope, IconId, Loadable, MaybeLoaded, RoleId, UserId, publish_event,
};
use crate::database::schema::channel;
use crate::database::schema::community;
use crate::database::schema::community_member_role;
use crate::database::schema::community_user;
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable,
    QueryableByName, Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use rust_i18n::t;

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

/// Makes a community owned by `user`, with the default roles (the creator holding Admin too, so
/// they keep it should they hand the community on) and a first text and voice channel.
pub(crate) async fn create_community(
    state: GlobalServerContext,
    user: UserId,
    command: &CommunityCreateRequest,
) -> Result<Community, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let state = &state;
    conn.transaction(|conn| {
        async move {
            let community = Community {
                id: CommunityId::new(),
                icon: command.icon.map(MaybeLoaded::NotLoaded),
                name: command.name.clone(),
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
                app::channel::insert_channel(
                    state,
                    conn.as_mut(),
                    name.to_string(),
                    0,
                    ty,
                    community.id,
                    None,
                )
                .await?;
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
    command: CommunityUpdateRequest,
) -> app::error::Result<Community> {
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
    conn.transaction(|conn| {
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
    .await
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
    let membership = message_enum::UserCommunity {
        community,
        user,
        sort_index: Some(sort_index),
        roles: roles.to_vec(),
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
    })
}

/// Moves a community within `user`'s own list. Returns the membership as the event carried it.
pub(crate) async fn reorder_membership(
    state: &GlobalServerContext,
    user: UserId,
    community: CommunityId,
    sort_index: i32,
) -> app::error::Result<message_enum::UserCommunity> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let updated = diesel::update(community_user::table)
                .filter(
                    community_user::community
                        .eq(community)
                        .and(community_user::user.eq(user)),
                )
                .set(community_user::sort_index.eq(sort_index))
                .execute(conn)
                .await?;
            if updated == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            publish_event(
                state,
                conn.as_mut(),
                EventScope::Membership { community, user },
                &ServerEvent::UserCommunity(UserCommunityEvent::Update {
                    community,
                    user,
                    sort_index: Some(Some(sort_index)),
                    roles: None,
                }),
            )
            .await?;
            let roles = app::role::roles_of_members(conn.as_mut(), &[(community, user)])
                .await?
                .remove(&(community, user))
                .unwrap_or_default();
            Ok(message_enum::UserCommunity {
                community,
                user,
                sort_index: Some(sort_index),
                roles,
            })
        }
        .scope_boxed()
    })
    .await
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
    state: &GlobalServerContext,
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

/// Members a single community read returns: the most recently seen users, capped so a large
/// community cannot make its member list unbounded.
pub const MEMBERS_PER_COMMUNITY: i64 = 100;

/// One row of [`read_community_members`]: a member together with the community the row was
/// selected for. A user in several of the requested communities appears once per community.
#[derive(Debug, QueryableByName)]
pub struct CommunityMember {
    #[diesel(sql_type = diesel::sql_types::Uuid)]
    pub community: CommunityId,
    #[diesel(sql_type = diesel::sql_types::Integer)]
    pub sort_index: i32,
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
}

/// The most recently seen members of each of `communities`, at most [`MEMBERS_PER_COMMUNITY`]
/// per community, grouped by community and ordered most recently seen first within each group.
/// The caller's own membership of each community is always among them, however long ago they
/// were seen, because their memberships carry the order of their own community list. One query
/// serves any number of communities: the per-community cap is a window function rather than a
/// `LIMIT`, so sideloading members for a user's whole community list costs one round trip.
pub(crate) async fn read_community_members(
    state: &GlobalServerContext,
    caller: UserId,
    communities: &[CommunityId],
) -> app::error::Result<Vec<Membership>> {
    use diesel::sql_types::{Array, BigInt, Uuid};

    if communities.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<CommunityMember> = diesel::sql_query(
        r#"
        SELECT community, sort_index, id, name, password_hash, icon, created_at, last_seen_at,
               deleted_at, display_name, pronouns, bio, status_text, status_emoji, bot, bot_owner,
               bot_public, home_domain, home_id, home_icon
        FROM (
            SELECT cu.community, cu.sort_index, u.*,
                   ROW_NUMBER() OVER (PARTITION BY cu.community ORDER BY u.last_seen_at DESC) AS recency_rank
            FROM community_user cu
            JOIN "user" u ON u.id = cu."user"
            WHERE cu.community = ANY($1) AND u.deleted_at IS NULL
        ) ranked
        WHERE recency_rank <= $2 OR id = $3
        ORDER BY community, last_seen_at DESC
        "#,
    )
    .bind::<Array<Uuid>, _>(communities.iter().map(|c| c.0).collect::<Vec<_>>())
    .bind::<BigInt, _>(MEMBERS_PER_COMMUNITY)
    .bind::<Uuid, _>(caller.0)
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
    let mut users = Vec::with_capacity(rows.len());
    for row in rows {
        communities.push(row.community);
        sort_indexes.push(row.sort_index);
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
        .zip(users)
        .map(|((community, sort_index), user)| Membership {
            community,
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

/// One page of `community`'s members whose username or display name contains `search`, by name:
/// `limit` of them from `offset`.
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
        SELECT cu.community, cu.sort_index, u.id, u.name, u.password_hash, u.icon, u.created_at,
               u.last_seen_at, u.deleted_at, u.display_name, u.pronouns, u.bio, u.status_text,
               u.status_emoji, u.bot, u.bot_owner, u.bot_public, u.home_domain, u.home_id,
               u.home_icon
        FROM community_user cu
        JOIN "user" u ON u.id = cu."user"
        WHERE cu.community = $1 AND u.deleted_at IS NULL
          AND ($2::text IS NULL OR lower(u.name) LIKE $2 OR lower(u.display_name) LIKE $2)
        ORDER BY lower(COALESCE(u.display_name, u.name)), u.id
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

/// Every live channel of each of `communities`, including those filed under a category, ordered
/// by community and then sort index. This is the batch a client needs to render the channel
/// tree of every community it belongs to in one request.
pub(crate) async fn read_communities_channels(
    state: &GlobalServerContext,
    communities: &[CommunityId],
) -> app::error::Result<Vec<app::channel::Channel>> {
    if communities.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let channels = channel::table
        .select(app::channel::Channel::as_select())
        .filter(
            channel::community
                .eq_any(communities)
                // Threads record their community too, but belong under their parent channel.
                .and(channel::parent_channel.is_null())
                .and(channel::deleted_at.is_null()),
        )
        .order_by((channel::community.asc(), channel::sort_index.asc()))
        .load(conn.as_mut())
        .await?;
    Ok(channels)
}

/// The top-level channels of a community the caller may view.
pub(crate) async fn read_community_channels(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
) -> app::error::Result<Vec<app::channel::Channel>> {
    let mut conn = state.connection_pool.get().await?;
    require_member(conn.as_mut(), caller, community).await?;
    let visibility = app::visibility::Visibility::load(state, caller, &[community]).await?;
    let channels = channel::table
        .select(app::channel::Channel::as_select())
        .filter(
            channel::community
                .eq(community)
                .and(channel::parent_category.is_null())
                .and(channel::parent_channel.is_null())
                .and(channel::deleted_at.is_null()),
        )
        .order_by(channel::sort_index.asc())
        .load::<app::channel::Channel>(conn.as_mut())
        .await?
        .into_iter()
        .filter(|c| visibility.can_view(c.id))
        .collect();
    Ok(channels)
}
