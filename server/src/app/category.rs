use crate::api::message_enum;
use crate::api::message_enum::request::CategoryUpdateRequest;
use crate::api::message_enum::server_event::{
    CategoryEvent, CategoryOverrideEvent, ChannelEvent, ServerEvent,
};
use crate::app;
use crate::app::channel::Channel;
use crate::app::community::Community;
use crate::app::context::GlobalServerContext;
use crate::app::permissions::{CommunityAccess, Permissions, in_category, missing, require_member};
use crate::app::{
    CategoryId, ChannelId, CommunityId, EventScope, Loadable, MaybeLoaded, RoleId, UserId,
    publish_event,
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

/// A live category for `caller` to manage, which takes Manage categories and viewing what the
/// category's own overrides let them view: nobody changes or removes the overrides that hide its
/// channels from them. Not found for anyone who is not a member.
pub(crate) async fn managed_category(
    conn: &mut AsyncPgConnection,
    caller: UserId,
    id: CategoryId,
) -> app::Result<(Category, CommunityAccess)> {
    let (category, access) = member_category(conn, caller, id).await?;
    access.require(Permissions::MANAGE_CATEGORIES)?;
    if !in_category(conn, &access, id)
        .await?
        .contains(Permissions::VIEW_CHANNEL)
    {
        return Err(missing(Permissions::VIEW_CHANNEL));
    }
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
    let (row, _) = member_category(conn.as_mut(), caller, category).await?;
    let visibility =
        crate::app::visibility::Visibility::load(state, caller, &[*row.community.id()]).await?;
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
/// channels are left in no category and its overrides are cleared.
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
            let cleared: Vec<RoleId> =
                diesel::delete(category_override::table.filter(category_override::category.eq(id)))
                    .returning(category_override::role)
                    .get_results(conn.as_mut())
                    .await?;
            for role in cleared {
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::CommunityOfCategory(id),
                    &ServerEvent::CategoryOverride(CategoryOverrideEvent::Delete {
                        category: id,
                        role,
                    }),
                )
                .await?;
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
