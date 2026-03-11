use crate::app::{IconId, Loadable};
use diesel_async::AsyncPgConnection;

#[derive(Debug, Clone)]
pub struct Icon {
    pub id: IconId,
    pub data: Vec<u8>,
    pub mime_type: String,
}

impl Loadable for Icon {
    type Id = IconId;

    async fn load_from_db(
        _pg_connection: &mut AsyncPgConnection,
        _id: IconId,
    ) -> Result<Self, diesel::result::Error> {
        todo!()
    }

    fn id(&self) -> &Self::Id {
        &self.id
    }
}
