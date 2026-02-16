use diesel::PgConnection;
use crate::app::{CommunityId, Loadable, MaybeLoaded};
use crate::app::icon::Icon;

pub struct Community {
    pub id: CommunityId,
    pub name: String,
    pub icon: MaybeLoaded<Icon>,
}

impl Loadable for Community {
    type Id = CommunityId;

    fn load_from_db(pg_connection: &PgConnection, id: CommunityId) -> Self {
        todo!()
    }
}