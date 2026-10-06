use crate::api::message_enum;
use crate::api::message_enum::request::CategoryUpdateRequest;
use crate::api::message_enum::server_event::{CategoryEvent, ChannelEvent, ServerEvent};
use crate::app;
use crate::app::channel::Channel;
use crate::app::community::Community;
use crate::app::context::GlobalServerContext;
use crate::app::permissions::{CommunityAccess, Permissions, in_category, require_member};
use crate::app::visibility::Visibility;
use crate::app::{
    CategoryId, ChannelId, CommunityId, EventScope, Loadable, MaybeLoaded, UserId, publish_event,
};
use crate::database::schema::{category, category_override, channel};
use crate::t;
use diesel::{
    AsChangeset, BoolExpressionMethods, ExpressionMethods, Insertable, QueryDsl, Queryable,
    Selectable, SelectableHelper,
};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

#[derive(Debug, Clone, Selectable, Insertable, Queryable)]
#[diesel(table_name=category)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Category {
    pub id: CategoryId,
    pub community: MaybeLoaded<Community>,
    pub name: String,
    pub sort_index: i32,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Loadable for Category {
    type Id = CategoryId;

    async fn load_from_db(state: &GlobalServerContext, id: Self::Id) -> crate::app::Result<Self> {
        category::table
            .select(Category::as_select())
            .filter(category::id.eq(id).and(category::deleted_at.is_null()))
            .first(&mut state.connection_pool.get().await?)
            .await
            .map_err(Into::into)
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

/// A category's name as given, trimmed and within `app::community::MAX_NAME_CHARS`.
fn category_name(name: &str) -> app::Result<String> {
    app::community::trimmed_name(name, |max| t!("categoryNameLength", max = max))
}

/// Makes a category, which takes Manage categories.
pub(crate) async fn create_category(
    state: &GlobalServerContext,
    caller: UserId,
    name: String,
    sort_index: i32,
    community: CommunityId,
) -> app::error::Result<Category> {
    let name = category_name(&name)?;
    let id = CategoryId::new();
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            require_member(conn.as_mut(), caller, community)
                .await?
                .require(Permissions::MANAGE_CATEGORIES)?;
            let category = Category {
                id,
                community: MaybeLoaded::NotLoaded(community),
                name: name.clone(),
                sort_index,
                deleted_at: None,
            };
            diesel::insert_into(category::table)
                .values(&category)
                .execute(conn.as_mut())
                .await?;
            let event = ServerEvent::Category(CategoryEvent::Create(message_enum::Category {
                id,
                community,
                name,
                sort_index,
            }));
            app::publish_event(
                state,
                conn.as_mut(),
                EventScope::Community(community),
                &event,
            )
            .await?;
            Ok(category)
        }
        .scope_boxed()
    })
    .await
}

/// The community of a live category, when `caller` is a member of it; not found otherwise.
async fn member_category(
    conn: &mut AsyncPgConnection,
    caller: UserId,
    id: CategoryId,
) -> app::Result<(Category, CommunityAccess)> {
    let category: Category = category::table
        .select(Category::as_select())
        .filter(category::id.eq(id).and(category::deleted_at.is_null()))
        .first(conn)
        .await?;
    let access = require_member(conn, caller, *category.community.id()).await?;
    Ok((category, access))
}

/// A live category `caller` may learn of: a member (or deployment moderator) whom the category's
/// own overrides leave View channel there (`in_category`, as
/// `app::visibility::CommunityModel::can_view_category` decides it for the event stream). Not
/// found for anyone else, so a hidden category's name and overrides stay hidden.
pub(crate) async fn viewed_category(
    conn: &mut AsyncPgConnection,
    caller: UserId,
    id: CategoryId,
) -> app::Result<(Category, CommunityAccess)> {
    let (category, access) = member_category(conn, caller, id).await?;
    if !in_category(conn, &access, id)
        .await?
        .contains(Permissions::VIEW_CHANNEL)
    {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    }
    Ok((category, access))
}

/// A live category for `caller` to manage, which takes viewing it (`viewed_category`), so
/// nobody changes or removes the overrides that hide it and its channels from them, and Manage
/// categories.
pub(crate) async fn managed_category(
    conn: &mut AsyncPgConnection,
    caller: UserId,
    id: CategoryId,
) -> app::Result<(Category, CommunityAccess)> {
    let (category, access) = viewed_category(conn, caller, id).await?;
    access.require(Permissions::MANAGE_CATEGORIES)?;
    Ok((category, access))
}

/// A category the caller may learn of (`viewed_category`).
pub(crate) async fn read_category(
    state: &GlobalServerContext,
    caller: UserId,
    id: CategoryId,
) -> app::error::Result<Category> {
    let mut conn = state.connection_pool.get().await?;
    Ok(viewed_category(conn.as_mut(), caller, id).await?.0)
}

/// The live categories of `community` the caller may learn of, by sort index.
pub(crate) async fn read_community_categories(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
) -> app::error::Result<Vec<Category>> {
    {
        let mut conn = state.connection_pool.get().await?;
        require_member(conn.as_mut(), caller, community).await?;
    }
    let visible = Visibility::load(state, caller, &[community]).await?;
    read_communities_categories(state, &visible).await
}

/// Every live category of each of the communities `visible` covers that its user may learn of
/// (`Visibility::can_view_category`), ordered by community and then sort index.
pub(crate) async fn read_communities_categories(
    state: &GlobalServerContext,
    visible: &Visibility,
) -> app::error::Result<Vec<Category>> {
    let communities = visible.communities();
    if communities.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let categories: Vec<Category> = category::table
        .select(Category::as_select())
        .filter(
            category::community
                .eq_any(communities)
                .and(category::deleted_at.is_null()),
        )
        .order_by((category::community.asc(), category::sort_index.asc()))
        .load(conn.as_mut())
        .await?;
    Ok(categories
        .into_iter()
        .filter(|category| visible.can_view_category(*category.community.id(), category.id))
        .collect())
}

/// The channels the caller may view in a category they may learn of.
pub(crate) async fn read_category_channels(
    state: &GlobalServerContext,
    caller: UserId,
    category: CategoryId,
) -> app::error::Result<Vec<Channel>> {
    let mut conn = state.connection_pool.get().await?;
    let (row, _) = viewed_category(conn.as_mut(), caller, category).await?;
    let visibility = Visibility::load(state, caller, &[*row.community.id()]).await?;
    let channels: Vec<Channel> = channel::table
        .select(Channel::as_select())
        .filter(
            channel::parent_category
                .eq(category)
                .and(channel::deleted_at.is_null()),
        )
        .order_by(channel::sort_index.asc())
        .load(conn.as_mut())
        .await?;
    Ok(channels
        .into_iter()
        .filter(|c| visibility.can_view(c.id))
        .collect())
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = category)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CategoryChangeset {
    pub name: Option<String>,
    pub sort_index: Option<i32>,
}

/// Renames or moves a category, which takes Manage categories and viewing it
/// (`managed_category`).
pub(crate) async fn update_category(
    state: &GlobalServerContext,
    caller: UserId,
    id: CategoryId,
    mut command: CategoryUpdateRequest,
) -> app::error::Result<Category> {
    command.name = command.name.as_deref().map(category_name).transpose()?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            managed_category(conn.as_mut(), caller, id).await?;
            let Some(category) = diesel::update(category::table)
                .set(CategoryChangeset {
                    name: command.name.clone(),
                    sort_index: command.sort_index,
                })
                .filter(category::id.eq(id).and(category::deleted_at.is_null()))
                .returning(Category::as_select())
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
                EventScope::CommunityOfCategory(id),
                &ServerEvent::Category(CategoryEvent::Update {
                    id,
                    name: command.name,
                    sort_index: command.sort_index,
                }),
            )
            .await?;
            Ok(category)
        }
        .scope_boxed()
    })
    .await
}

/// Deletes a category, which takes Manage categories and viewing it (`managed_category`). Its
/// channels are left in no category, each move announced, and its overrides are cleared, which
/// its deletion's event tells of (`app::visibility::ModelChange::CategoryDeleted`): announcing
/// each would show the category, once its last override went, to everyone.
pub(crate) async fn delete_category(
    state: &GlobalServerContext,
    caller: UserId,
    id: CategoryId,
) -> app::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            managed_category(conn.as_mut(), caller, id).await?;
            let deleted = diesel::update(category::table)
                .set(category::deleted_at.eq(diesel::dsl::now))
                .filter(category::id.eq(id).and(category::deleted_at.is_null()))
                .execute(conn.as_mut())
                .await?;
            if deleted == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            // Its channels leave it, each move announced to those who may view the channel on
            // either side of it, and its overrides go with it, so nothing it decided outlives it.
            let moved: Vec<ChannelId> = diesel::update(channel::table)
                .set(channel::parent_category.eq(None::<CategoryId>))
                .filter(
                    channel::parent_category
                        .eq(id)
                        .and(channel::deleted_at.is_null()),
                )
                .returning(channel::id)
                .get_results(conn.as_mut())
                .await?;
            for channel in moved {
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::ChannelDefinition {
                        channel,
                        departed: None,
                    },
                    &ServerEvent::Channel(ChannelEvent::Update {
                        id: channel,
                        parent_category: Some(None),
                        community: None,
                        name: None,
                        sort_index: None,
                        reply_count: None,
                        last_reply_at: None,
                        recipients: None,
                    }),
                )
                .await?;
            }
            diesel::delete(category_override::table.filter(category_override::category.eq(id)))
                .execute(conn.as_mut())
                .await?;
            publish_event(
                state,
                conn.as_mut(),
                EventScope::CommunityOfCategory(id),
                &ServerEvent::Category(CategoryEvent::Delete { id }),
            )
            .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}
