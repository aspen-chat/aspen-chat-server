use crate::api::GlobalServerContext;
use crate::app;
use crate::app::community::Community;
use crate::app::{CategoryId, CommunityId, Loadable, MaybeLoaded};
use crate::database::schema::category;
use diesel::{ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable, SelectableHelper};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

#[derive(Debug, Clone, Selectable, Insertable, Queryable)]
#[diesel(table_name=category)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Category {
    pub id: CategoryId,
    pub community: MaybeLoaded<Community>,
    pub name: String,
    pub sort_index: i32,
}

impl Loadable for Category {
    type Id = CategoryId;

    async fn load_from_db(
        _pg_connection: &mut AsyncPgConnection,
        _id: Self::Id,
    ) -> Result<Self, diesel::result::Error> {
        todo!()
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

pub(crate) async fn read_community_categories(
    state: &GlobalServerContext,
    community: CommunityId,
) -> app::error::Result<Vec<Category>> {
    let mut conn = state.connection_pool.get().await?;
    let categories = category::table
        .select(Category::as_select())
        .filter(category::community.eq(community))
        .order_by(category::sort_index.asc())
        .load(conn.as_mut())
        .await?;
    Ok(categories)
}
