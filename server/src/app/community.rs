use crate::api::GlobalServerContext;
use crate::api::message_enum::command::CommunityCreateCommand;
use crate::app;
use crate::app::icon::Icon;
use crate::app::{CommunityId, Loadable, MaybeLoaded};
use crate::database::schema::{self, community};
use diesel::{ExpressionMethods, Insertable, QueryDsl, Queryable, Selectable, SelectableHelper};
use diesel_async::{AsyncPgConnection, RunQueryDsl};

#[derive(Debug, Clone, Queryable, Selectable, Insertable)]
#[diesel(table_name = community)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Community {
    pub id: CommunityId,
    pub name: String,
    pub icon: Option<MaybeLoaded<Icon>>,
}

impl Loadable for Community {
    type Id = CommunityId;

    async fn load_from_db(
        pg_connection: &mut AsyncPgConnection,
        id: CommunityId,
    ) -> Result<Self, diesel::result::Error> {
        Ok(schema::community::table
            .select(Community::as_select())
            .filter(schema::community::dsl::id.eq(id))
            .first(pg_connection)
            .await?)
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

pub(crate) async fn create_community(
    state: GlobalServerContext,
    command: &CommunityCreateCommand,
) -> Result<Community, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    let community = Community {
        id: CommunityId::new(),
        icon: command.icon.map(MaybeLoaded::NotLoaded),
        name: command.name.clone(),
    };
    diesel::insert_into(schema::community::table)
        .values(community.clone())
        .execute(conn.as_mut())
        .await?;
    Ok(community)
}
