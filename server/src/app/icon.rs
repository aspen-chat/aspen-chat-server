use diesel::PgConnection;
use crate::app::{IconId, Loadable};

pub struct Icon {
    pub id: IconId,
    pub data: Vec<u8>,
    pub mime_type: String,
}

impl Loadable for Icon {
    type Id = IconId;

    fn load_from_db(pg_connection: &PgConnection, id: IconId) -> Self {
        todo!()
    }
}