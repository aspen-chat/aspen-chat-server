use diesel::PgConnection;
use serde::{Deserialize, Serialize};

pub mod login;
pub mod community;
pub mod category;
pub mod channel;
pub mod icon;
pub mod message;
pub mod react;
pub mod user;

macro_rules! id_type {
    ($type_name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Copy, Deserialize, Serialize, Hash)]
        #[serde(transparent)]
        pub struct $type_name(pub uuid::Uuid);

        impl $type_name {
            pub fn new() -> Self {
                Self(uuid::Uuid::now_v7())
            }
        }

        impl From<uuid::Uuid> for $type_name {
            fn from(value: uuid::Uuid) -> Self {
                Self(value)
            }
        }
    };
}

id_type!(CommunityId);

id_type!(UserId);

id_type!(ChannelId);

id_type!(MessageId);

id_type!(CategoryId);

id_type!(AttachmentId);

id_type!(IconId);

pub enum MaybeLoaded<T: Loadable> {
    Loaded(T),
    NotLoaded(T::Id),
}

pub trait Loadable {
    type Id;
    fn load_from_db(pg_connection: &PgConnection, id: Self::Id) -> Self;
}