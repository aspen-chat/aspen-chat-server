use crate::api::message_enum::request::CategoryUpdateRequest;
use crate::api::message_enum::server_event::{CategoryEvent, ServerEvent};
use crate::api::{GlobalServerContext, message_enum};
use crate::app;
use crate::app::channel::Channel;
use crate::app::community::Community;
use crate::app::permissions::{Permissions, require_member};
use crate::app::{
    CategoryId, CommunityId, EventScope, Loadable, MaybeLoaded, UserId, publish_event,
};
use crate::database::schema::{category, channel};
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

/// Makes a category, which takes Manage categories.
pub(crate) async fn create_category(
    state: &GlobalServerContext,
    caller: UserId,
    name: String,
    sort_index: i32,
    community: CommunityId,
) -> app::error::Result<Category> {
    let id = CategoryId::new();
    let mut conn = state.connection_pool.get().await?;
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

/// The community of a live category, when `caller` is a member of it; not found otherwise.
async fn member_category(
    conn: &mut AsyncPgConnection,
    caller: UserId,
    id: CategoryId,
) -> app::Result<(Category, crate::app::permissions::CommunityAccess)> {
    let category: Category = category::table
        .select(Category::as_select())
        .filter(category::id.eq(id).and(category::deleted_at.is_null()))
        .first(conn)
        .await?;
    let access = require_member(conn, caller, *category.community.id()).await?;
    Ok((category, access))
}

pub(crate) async fn read_category(
    state: &GlobalServerContext,
    caller: UserId,
    id: CategoryId,
) -> app::error::Result<Category> {
    let mut conn = state.connection_pool.get().await?;
    Ok(member_category(conn.as_mut(), caller, id).await?.0)
}

pub(crate) async fn read_community_categories(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
) -> app::error::Result<Vec<Category>> {
    let mut conn = state.connection_pool.get().await?;
    require_member(conn.as_mut(), caller, community).await?;
    let categories = category::table
        .select(Category::as_select())
        .filter(
            category::community
                .eq(community)
                .and(category::deleted_at.is_null()),
        )
        .order_by(category::sort_index.asc())
        .load(conn.as_mut())
        .await?;
    Ok(categories)
}

/// Every live category of each of `communities`, ordered by community and then sort index.
pub(crate) async fn read_communities_categories(
    state: &GlobalServerContext,
    communities: &[CommunityId],
) -> app::error::Result<Vec<Category>> {
    if communities.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let categories = category::table
        .select(Category::as_select())
        .filter(
            category::community
                .eq_any(communities)
                .and(category::deleted_at.is_null()),
        )
        .order_by((category::community.asc(), category::sort_index.asc()))
        .load(conn.as_mut())
        .await?;
    Ok(categories)
}

pub(crate) async fn read_category_channels(
    state: &GlobalServerContext,
    caller: UserId,
    category: CategoryId,
) -> app::error::Result<Vec<Channel>> {
    let mut conn = state.connection_pool.get().await?;
    member_category(conn.as_mut(), caller, category).await?;
    let channels = channel::table
        .select(Channel::as_select())
        .filter(
            channel::parent_category
                .eq(category)
                .and(channel::deleted_at.is_null()),
        )
        .order_by(channel::sort_index.asc())
        .load(conn.as_mut())
        .await?;
    Ok(channels)
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = category)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct CategoryChangeset {
    pub name: Option<String>,
    pub sort_index: Option<i32>,
}

/// Renames or moves a category, which takes Manage categories.
pub(crate) async fn update_category(
    state: &GlobalServerContext,
    caller: UserId,
    id: CategoryId,
    command: CategoryUpdateRequest,
) -> app::error::Result<Category> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            member_category(conn.as_mut(), caller, id)
                .await?
                .1
                .require(Permissions::MANAGE_CATEGORIES)?;
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

/// Deletes a category, which takes Manage categories.
pub(crate) async fn delete_category(
    state: &GlobalServerContext,
    caller: UserId,
    id: CategoryId,
) -> app::error::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            member_category(conn.as_mut(), caller, id)
                .await?
                .1
                .require(Permissions::MANAGE_CATEGORIES)?;
            let deleted = diesel::update(category::table)
                .set(category::deleted_at.eq(diesel::dsl::now))
                .filter(category::id.eq(id).and(category::deleted_at.is_null()))
                .execute(conn.as_mut())
                .await?;
            if deleted == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
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
