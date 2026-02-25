use crate::api::GlobalServerContext;
use crate::api::message_enum::command::CommunityCreateCommand;
use crate::app;
use crate::app::icon::Icon;
use crate::app::{CommunityId, Loadable, MaybeLoaded};
use crate::database::schema::community;
use diesel::{Insertable, Queryable, Selectable};
use diesel_async::AsyncPgConnection;

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
        todo!()
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}

pub(crate) async fn create_community(
    state: GlobalServerContext,
    command: &CommunityCreateCommand,
) -> Result<Community, app::Error> {
    todo!()
}
