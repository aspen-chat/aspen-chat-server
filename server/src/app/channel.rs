use crate::api::{ChannelPermissions, ChannelType};
use crate::app::category::Category;
use crate::app::community::Community;
use crate::app::{ChannelId, Loadable, MaybeLoaded};
use diesel_async::AsyncPgConnection;

pub struct Channel {
    pub id: ChannelId,
    pub community: MaybeLoaded<Community>,
    pub parent_category: MaybeLoaded<Category>,
    pub name: String,
    pub permissions: ChannelPermissions,
    pub ty: ChannelType,
    pub sort_index: u32,
}

impl Loadable for Channel {
    type Id = ChannelId;

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
