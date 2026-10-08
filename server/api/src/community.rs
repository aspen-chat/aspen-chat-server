use crate::auth::SessionUser;
use crate::error::{ApiResult, Problem};
use crate::extract::{Created, Json, NoContent, Path, Query, double_option};
use crate::include::{IncludeSet, Included, Sideloaded, SideloadedList};
use crate::message_enum::request::{
    CommunityCreateRequest, CommunityUpdateRequest, UserCommunityCreateRequest,
};
use crate::message_enum::{Channel, User, UserCommunity};
use crate::user::UserRef;
use crate::{API_PREFIX, TAG_COMMUNITIES, message_enum};
use aspen_app as app;
use aspen_app::context::GlobalServerContext;
use aspen_app::{CommunityId, UserId};
use axum::extract::State;
use axum::http::StatusCode;
use diesel::result::DatabaseErrorKind;
use serde::Deserialize;
use std::collections::HashSet;
use utoipa::{IntoParams, ToSchema};

/// Relationships a community read can sideload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum CommunityInclude {
    /// Every channel of the community, including those filed under a category, as
    /// `included.channels`.
    Channels,
    /// The community's categories, as `included.categories`.
    Categories,
    /// The most recently seen members, as `included.users` plus the `included.userCommunities`
    /// membership records that link them to each community. Capped at 100 per community, like
    /// `GET /communities/{community}/members`.
    Members,
    /// The calls in progress on the communities' voice channels, as `included.voiceSessions`
    /// and `included.voiceParticipants`.
    Voice,
    /// How far the caller has read each of the communities' channels, as `included.readStates`.
    ReadStates,
    /// The caller's mutes of the communities' channels, as `included.channelMutes`.
    Mutes,
    /// The communities' own emoji, as `included.customEmoji`.
    Emoji,
    /// The communities' categories the caller has collapsed, as `included.categoryCollapses`.
    Collapses,
    /// The caller's notification settings for the communities and their channels, as
    /// `included.notificationSettings`.
    Notifications,
    /// The communities' roles, as `included.roles`, and their channel and category overrides,
    /// as `included.channelOverrides` and `included.categoryOverrides`. Which roles each member
    /// holds is on their `userCommunities` record.
    Roles,
}

/// Body of a community read. Named aliases rather than `Sideloaded<Community>` at the handler
/// because `#[utoipa::path]` treats a generic type in `body = ...` as one it must compose
/// itself, bypassing the hand-written schema of the envelope.
pub type CommunityRead = Sideloaded<message_enum::Community>;
/// Body of a community list read; see [`CommunityRead`].
pub type CommunityList = SideloadedList<message_enum::Community>;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CommunityReadQuery {
    /// Related records to return alongside the community, comma separated.
    #[serde(default)]
    #[param(value_type = Option<Vec<CommunityInclude>>, style = Form, explode = false)]
    pub include: IncludeSet<CommunityInclude>,
}

/// Loads the relationships named in `include` for every community in `communities`. Each
/// requested relationship is one batched read regardless of how many communities there are,
/// and the reads run concurrently.
pub async fn sideload_communities(
    state: &GlobalServerContext,
    caller: UserId,
    communities: &[CommunityId],
    include: &IncludeSet<CommunityInclude>,
) -> ApiResult<Included> {
    // What lists channels, or things in them, is read through what the caller may view, which
    // its readers take.
    let visible = if [
        CommunityInclude::Channels,
        CommunityInclude::Categories,
        CommunityInclude::Collapses,
        CommunityInclude::Voice,
        CommunityInclude::ReadStates,
        CommunityInclude::Mutes,
        CommunityInclude::Notifications,
        CommunityInclude::Roles,
    ]
    .iter()
    .any(|i| include.contains(*i))
    {
        Some(app::visibility::Visibility::load(state, caller, communities).await?)
    } else {
        None
    };
    let visible = visible.as_ref();
    // Read one after another, each taking a connection only while it reads, so one read of a
    // community list never holds more than one of the pool's connections however much it
    // sideloads; every reader is bounded, so the reads are quick in turn.
    let (
        channels,
        categories,
        members,
        voice,
        read_states,
        mutes,
        notifications,
        collapses,
        roles,
        emoji,
    ) = (
        async {
            match visible {
                Some(visible) if include.contains(CommunityInclude::Channels) => {
                    app::channel::read_communities_channels(state, visible)
                        .await
                        .map(Some)
                }
                _ => Ok(None),
            }
        }
        .await?,
        async {
            match visible {
                Some(visible) if include.contains(CommunityInclude::Categories) => {
                    app::category::read_communities_categories(state, visible)
                        .await
                        .map(Some)
                }
                _ => Ok(None),
            }
        }
        .await?,
        async {
            if include.contains(CommunityInclude::Members) {
                app::community::read_community_members(state, caller, communities)
                    .await
                    .map(Some)
            } else {
                Ok(None)
            }
        }
        .await?,
        async {
            match visible {
                Some(visible) if include.contains(CommunityInclude::Voice) => {
                    app::voice::read_communities_voice(state, visible)
                        .await
                        .map(Some)
                }
                _ => Ok(None),
            }
        }
        .await?,
        async {
            match visible {
                Some(visible) if include.contains(CommunityInclude::ReadStates) => {
                    app::read_state::read_communities_read_states(state, visible)
                        .await
                        .map(Some)
                }
                _ => Ok(None),
            }
        }
        .await?,
        async {
            match visible {
                Some(visible) if include.contains(CommunityInclude::Mutes) => {
                    app::channel_mute::read_community_mutes(state, visible)
                        .await
                        .map(Some)
                }
                _ => Ok(None),
            }
        }
        .await?,
        async {
            match visible {
                Some(visible) if include.contains(CommunityInclude::Notifications) => {
                    app::notification_setting::read_community_settings(state, visible)
                        .await
                        .map(Some)
                }
                _ => Ok(None),
            }
        }
        .await?,
        async {
            match visible {
                Some(visible) if include.contains(CommunityInclude::Collapses) => {
                    app::category_collapse::read_collapsed(state, visible)
                        .await
                        .map(Some)
                }
                _ => Ok(None),
            }
        }
        .await?,
        async {
            match visible {
                Some(visible) if include.contains(CommunityInclude::Roles) => {
                    let roles = app::role::read_communities_roles(state, communities).await?;
                    let overrides = app::role::read_communities_overrides(state, visible).await?;
                    Ok::<_, app::Error>(Some((roles, overrides)))
                }
                _ => Ok(None),
            }
        }
        .await?,
        async {
            if include.contains(CommunityInclude::Emoji) {
                app::custom_emoji::read_communities_emoji(state, communities)
                    .await
                    .map(Some)
            } else {
                Ok(None)
            }
        }
        .await?,
    );
    let mut included = Included {
        channels: channels.map(|channels| {
            channels
                .into_iter()
                .map(crate::channel::channel_to_api)
                .collect()
        }),
        categories: categories.map(|categories| {
            categories
                .into_iter()
                .map(message_enum::Category::from)
                .collect()
        }),
        read_states: read_states.map(|states| {
            states
                .into_iter()
                .map(crate::read_state::ReadState::from)
                .collect()
        }),
        notification_settings: notifications.map(|settings| {
            settings
                .into_iter()
                .map(crate::notification_setting::NotificationSetting::from)
                .collect()
        }),
        channel_mutes: mutes.map(|mutes| {
            mutes
                .into_iter()
                .map(crate::channel_mute::ChannelMute::from)
                .collect()
        }),
        category_collapses: collapses.map(|collapses| {
            collapses
                .into_iter()
                .map(|category| crate::category_collapse::CategoryCollapse { category })
                .collect()
        }),
        ..Included::default()
    };
    if let Some((roles, (channel_overrides, category_overrides))) = roles {
        included.roles = Some(roles);
        included.channel_overrides = Some(channel_overrides);
        included.category_overrides = Some(category_overrides);
    }
    if let Some(emoji) = emoji {
        included.custom_emoji = Some(emoji);
    }
    if let Some((sessions, participants)) = voice {
        included.voice_sessions = Some(sessions);
        included.voice_participants = Some(participants);
    }
    if let Some(members) = members {
        // A user who belongs to several of the communities is one record in `users` and one
        // membership per community in `userCommunities`.
        let mut seen = HashSet::<UserId>::with_capacity(members.len());
        let mut users = Vec::with_capacity(members.len());
        let mut memberships = Vec::with_capacity(members.len());
        for membership in members {
            let user_id = membership.user.user_pg.id;
            memberships.push(UserCommunity::from(&membership));
            if seen.insert(user_id) {
                users.push(User::from(membership.user));
            }
        }
        included.users = Some(users);
        included.user_communities = Some(memberships);
    }
    Ok(included)
}

/// Creates a community. The caller becomes its first member and a default text and voice channel
/// are created.
#[utoipa::path(
    post,
    path = "/communities",
    tag = TAG_COMMUNITIES,
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, body = message_enum::Community, headers(("Location" = String, description = "URL of the new community"))),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn create_community(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Json(request): Json<CommunityCreateRequest>,
) -> ApiResult<Created<message_enum::Community>> {
    let c = app::community::create_community(state, user.id, &request).await?;
    Ok(Created::new(
        format!("{API_PREFIX}/communities/{}", c.id.0),
        message_enum::Community::from(c),
    ))
}

/// Reads a community. `include` sideloads its channels, categories, and members so a client can
/// render the whole community from one response.
#[utoipa::path(
    get,
    path = "/communities/{community}",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path), CommunityReadQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = CommunityRead),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_community(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Query(query): Query<CommunityReadQuery>,
) -> ApiResult<Json<CommunityRead>> {
    let c = app::community::read_community(&state, user.id, community).await?;
    let included = sideload_communities(&state, user.id, &[c.id], &query.include).await?;
    Ok(Json(Sideloaded::new(
        message_enum::Community::from(c),
        included,
    )))
}

#[utoipa::path(
    patch,
    path = "/communities/{community}",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = message_enum::Community),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_community(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Json(request): Json<CommunityUpdateRequest>,
) -> ApiResult<Json<message_enum::Community>> {
    let c = app::community::update_community(&state, user.id, community, request).await?;
    Ok(Json(message_enum::Community::from(c)))
}

#[utoipa::path(
    delete,
    path = "/communities/{community}",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: a permission this needs is missing", body = Problem),
        (status = NOT_FOUND, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn delete_community(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
) -> ApiResult<NoContent> {
    app::community::delete_community(&state, user.id, community).await?;
    Ok(NoContent)
}

/// Body of a member read: the members, with their memberships (their roles among them) as
/// `included.userCommunities`.
pub type MemberList = SideloadedList<User>;

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MemberListQuery {
    /// Search: members whose username, display name, or nickname contains this, ignoring case,
    /// or for one or two characters starts with it; at most 100 characters.
    #[serde(rename = "filter[name]")]
    #[param(rename = "filter[name]")]
    pub name: Option<String>,
    /// Continue a search after this member, the last of the previous page.
    pub after: Option<UserId>,
    /// How many a search returns, at most 50; 20 when absent.
    pub limit: Option<i64>,
}

/// How many members a search page holds when no `limit` is given.
const DEFAULT_MEMBER_PAGE: i64 = 20;

/// The community's members. Without parameters, the sample every member reads: the 100 most
/// recently seen, the caller always among them. With `filter[name]`, `offset`, or `limit`, a
/// search of every member by name, sorted by name, a page at a time. In a community of more than
/// 100 members only those who act on members may search it (its owner, a deployment moderator,
/// and holders of Assign roles, Remove members, Manage channels, or Manage categories); anyone
/// else is refused with `forbidden`, so no ordinary member can list a large community whole.
#[utoipa::path(
    get,
    path = "/communities/{community}/members",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path), MemberListQuery),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = MemberList),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: searching a large community without acting on members", body = Problem),
        (status = NOT_FOUND, description = "No such community, or the caller is not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_community_members(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Query(query): Query<MemberListQuery>,
) -> ApiResult<Json<MemberList>> {
    let searching = query.name.is_some() || query.after.is_some() || query.limit.is_some();
    let members = if searching {
        app::community::search_community_members(
            &state,
            user.id,
            community,
            query.name.as_deref(),
            query.after,
            query.limit.unwrap_or(DEFAULT_MEMBER_PAGE),
        )
        .await?
    } else {
        app::community::read_community_sample(&state, user.id, community).await?
    };
    let mut users = Vec::with_capacity(members.len());
    let mut memberships = Vec::with_capacity(members.len());
    for membership in members {
        memberships.push(UserCommunity::from(&membership));
        users.push(User::from(membership.user));
    }
    Ok(Json(MemberList::new(
        users,
        Included {
            user_communities: Some(memberships),
            ..Included::default()
        },
    )))
}

/// One member's membership of the community: the roles they hold there besides everyone's,
/// whether or not they are in the member sample. Any member may read it, as a deployment
/// moderator may; `sortIndex` is given only for the caller's own.
#[utoipa::path(
    get,
    path = "/communities/{community}/members/{user}",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path), ("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = UserCommunity),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such community, the caller is not a member, or the user is not", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn get_community_member(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path((community, user)): Path<(CommunityId, UserRef)>,
) -> ApiResult<Json<UserCommunity>> {
    let member = user.resolve(&session);
    let membership =
        app::community::read_community_member(&state, session.user.id, community, member).await?;
    Ok(Json(UserCommunity::from(&membership)))
}

/// Top-level channels of the community (those not filed under a category), in sort order.
#[utoipa::path(
    get,
    path = "/communities/{community}/channels",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = Vec<Channel>),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = NOT_FOUND, description = "No such community, or the caller is not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn list_community_channels(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
) -> ApiResult<Json<Vec<Channel>>> {
    let channels = app::channel::read_community_channels(&state, user.id, community).await?;
    Ok(Json(
        channels
            .into_iter()
            .map(crate::channel::channel_to_api)
            .collect(),
    ))
}

/// Joins the calling user to the community using an invite code. Joining a community the user
/// already belongs to succeeds with `200`.
#[utoipa::path(
    put,
    path = "/communities/{community}/members/@me",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = CREATED, description = "Joined", body = UserCommunity),
        (status = OK, description = "Already a member", body = UserCommunity),
        (status = BAD_REQUEST, description = "`badRequest` or `validation` (invite invalid, expired, or for another community)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn join_community(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Json(request): Json<UserCommunityCreateRequest>,
) -> ApiResult<(StatusCode, Json<UserCommunity>)> {
    match app::community::join_community(&state, user.id, community, request.invite_code).await {
        Ok(membership) => Ok((StatusCode::CREATED, Json(membership))),
        Err(app::Error::Diesel(diesel::result::Error::DatabaseError(
            DatabaseErrorKind::UniqueViolation,
            _,
        ))) => {
            let membership = app::community::read_membership(&state, user.id, community).await?;
            Ok((StatusCode::OK, Json(membership)))
        }
        Err(e) => Err(e.into()),
    }
}

/// Changes the calling user's own membership. Absent fields are unchanged.
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct MembershipUpdateRequest {
    /// The community's new position in the caller's list; lower comes first. Other members'
    /// lists are unaffected.
    #[serde(default)]
    pub sort_index: Option<i32>,
    /// The caller's name in this community, shown there in place of their display name; `null`
    /// clears it. Setting one takes Change nickname; clearing it takes nothing.
    #[serde(default, deserialize_with = "double_option")]
    pub nickname: Option<Option<String>>,
}

#[utoipa::path(
    patch,
    path = "/communities/{community}/members/@me",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = OK, body = UserCommunity),
        (status = BAD_REQUEST, description = "`validation` (a nickname blank or too long)", body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: setting a nickname takes Change nickname", body = Problem),
        (status = NOT_FOUND, description = "Not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn update_membership(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
    Json(request): Json<MembershipUpdateRequest>,
) -> ApiResult<Json<UserCommunity>> {
    let membership = app::community::update_membership(
        &state,
        user.id,
        community,
        request.sort_index,
        request.nickname,
    )
    .await?;
    Ok(Json(membership))
}

/// Clears a member's nickname in the community. Anyone may clear their own (`@me`); clearing
/// someone else's takes Manage nicknames, and they must rank below the caller's highest role, so
/// the owner's is theirs alone. Clearing a nickname that is not there still yields `204`.
#[utoipa::path(
    delete,
    path = "/communities/{community}/members/{user}/nickname",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path), ("user" = inline(UserRef), Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = FORBIDDEN, description = "`forbidden`: Manage nicknames is missing, or they do not rank below the caller", body = Problem),
        (status = NOT_FOUND, description = "No such community, or either person is not a member", body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn clear_nickname(
    State(state): State<GlobalServerContext>,
    session: SessionUser,
    Path((community, member)): Path<(CommunityId, UserRef)>,
) -> ApiResult<NoContent> {
    let member = member.resolve(&session);
    app::role::clear_nickname(&state, session.user.id, community, member).await?;
    Ok(NoContent)
}

/// Removes the calling user from the community. Leaving a community the user is not a member of
/// still yields `204`.
#[utoipa::path(
    delete,
    path = "/communities/{community}/members/@me",
    tag = TAG_COMMUNITIES,
    params(("community" = CommunityId, Path)),
    security(("bearerAuth" = [])),
    responses(
        (status = NO_CONTENT),
        (status = BAD_REQUEST, body = Problem),
        (status = UNAUTHORIZED, body = Problem),
        (status = INTERNAL_SERVER_ERROR, body = Problem),
    )
)]
pub async fn leave_community(
    State(state): State<GlobalServerContext>,
    SessionUser { user, .. }: SessionUser,
    Path(community): Path<CommunityId>,
) -> ApiResult<NoContent> {
    app::community::leave_community(&state, user.id, community).await?;
    Ok(NoContent)
}
