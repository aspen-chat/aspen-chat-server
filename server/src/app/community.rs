use crate::api::message_enum::request::{CommunityCreateRequest, CommunityUpdateRequest};
use crate::api::message_enum::server_event::{CommunityEvent, ServerEvent, UserCommunityEvent};
use crate::api::{ChannelType, GlobalServerContext, message_enum};
use crate::app;
use crate::app::icon::Icon;
use crate::app::permissions::{Permissions, community_access, require_member};
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
            require_member(conn.as_mut(), caller, id)
                .await?
                .require(Permissions::MANAGE_COMMUNITY)?;
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

/// Deletes a community. Only its owner may.
pub(crate) async fn delete_community(
    state: &GlobalServerContext,
    caller: UserId,
    id: CommunityId,
) -> app::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            if !require_member(conn.as_mut(), caller, id).await?.owner {
                return Err(app::Error::Forbidden(t!("permissionOwnerOnly")));
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
            let held: i64 = community_user::table
                .filter(community_user::user.eq(user))
                .count()
                .get_result(conn)
                .await?;
            let cap = state.config.limits.max_communities_per_user;
            if held >= i64::from(cap) {
                return Err(app::Error::Validation(t!("communityLimit", max = cap)));
            }
            add_member(state, conn, user, community, &[]).await
        }
        .scope_boxed()
    })
    .await
}

/// Adds `user` to `community` holding `roles` besides everyone's, at the end of their own list,
/// and announces the membership.
async fn add_member(
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
        sort_index,
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
        sort_index: row.sort_index,
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
                    sort_index: Some(sort_index),
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
                sort_index,
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
/// it in the same transaction. Nothing happens when they are not a member.
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
    /// Where the community sits in this user's own list.
    pub sort_index: i32,
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
               deleted_at, display_name, pronouns, bio, status_text, status_emoji
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
    let mut roles = app::role::roles_of_members(conn.as_mut(), &keys).await?;
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
            user,
            sort_index,
        })
        .collect())
}

pub(crate) async fn read_community_users(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
) -> app::error::Result<Vec<app::user::User>> {
    require_member(
        state.connection_pool.get().await?.as_mut(),
        caller,
        community,
    )
    .await?;
    Ok(read_community_members(state, caller, &[community])
        .await?
        .into_iter()
        .map(|membership| membership.user)
        .collect())
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
