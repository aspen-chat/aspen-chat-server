use crate::app::community::Community;
use crate::app::{CategoryId, Loadable, MaybeLoaded};
use diesel_async::AsyncPgConnection;

pub struct Category {
    pub id: CategoryId,
    pub community: MaybeLoaded<Community>,
    pub name: String,
    pub sort_index: u32,
}

impl Loadable for Category {
    type Id = CategoryId;

    fn load_from_db(
        pg_connection: &AsyncPgConnection,
        id: Self::Id,
    ) -> Result<Self, diesel::result::Error> {
        todo!()
    }

    fn id(&self) -> Self::Id {
        self.id
    }
}
