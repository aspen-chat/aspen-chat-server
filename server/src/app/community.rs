use crate::app::icon::Icon;
use crate::app::{CommunityId, Loadable, MaybeLoaded};
use diesel_async::AsyncPgConnection;

pub struct Community {
    pub id: CommunityId,
    pub name: String,
    pub icon: MaybeLoaded<Icon>,
}

impl Loadable for Community {
    type Id = CommunityId;

    fn load_from_db(
        pg_connection: &AsyncPgConnection,
        id: CommunityId,
    ) -> Result<Self, diesel::result::Error> {
        todo!()
    }

    fn id(&self) -> Self::Id {
        self.id
    }
}
