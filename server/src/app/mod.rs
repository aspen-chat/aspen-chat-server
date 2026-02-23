use diesel::QueryId;
use diesel::deserialize::{FromSql, FromSqlRow};
use diesel::expression::AsExpression;
use diesel::pg::sql_types::Uuid;
use diesel::pg::{Pg, PgValue};
use diesel::serialize::ToSql;
use diesel::sql_types::Uuid as DieselUuid;
use diesel_async::AsyncPgConnection;
use heck::ToKebabCase;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use std::error::Error as StdError;
use std::fmt::{Display, Formatter};

pub mod attachment;
pub mod category;
pub mod channel;
pub mod community;
mod error;
pub mod icon;
pub mod login;
pub mod message;
pub mod react;
pub mod user;
pub use error::Error;

macro_rules! id_type {
    ($type_name:ident) => {
        #[derive(
            Debug,
            Clone,
            PartialEq,
            Eq,
            Copy,
            Deserialize,
            Serialize,
            Hash,
            FromSqlRow,
            QueryId,
            AsExpression,
            utoipa::ToSchema,
        )]
        #[serde(transparent)]
        #[diesel(sql_type = Uuid)]
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

        impl Display for $type_name {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}-{}", stringify!($type_name).to_kebab_case(), self.0)
            }
        }

        impl FromSql<DieselUuid, Pg> for $type_name {
            fn from_sql(v: PgValue) -> Result<Self, Box<dyn StdError + Send + Sync + 'static>> {
                uuid::Uuid::from_sql(v).map(|u| $type_name(u))
            }
        }

        impl ToSql<DieselUuid, Pg> for $type_name {
            fn to_sql<'b>(
                &'b self,
                out: &mut diesel::serialize::Output<'b, '_, Pg>,
            ) -> diesel::serialize::Result {
                <uuid::Uuid as ToSql<diesel::sql_types::Uuid, Pg>>::to_sql(&self.0, out)
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

impl<T: Loadable> MaybeLoaded<T> {
    pub fn id(&self) -> T::Id {
        match self {
            MaybeLoaded::Loaded(l) => l.id(),
            MaybeLoaded::NotLoaded(id) => id.clone(),
        }
    }

    pub fn get(
        &mut self,
        pg_connection: &AsyncPgConnection,
    ) -> Result<&mut T, diesel::result::Error> {
        match self {
            MaybeLoaded::Loaded(v) => Ok(v),
            MaybeLoaded::NotLoaded(id) => {
                let v = T::load_from_db(pg_connection, id.clone())?;
                *self = MaybeLoaded::Loaded(v);
                self.get(pg_connection)
            }
        }
    }
}

pub trait Loadable: Sized {
    type Id: Clone;
    fn load_from_db(
        pg_connection: &AsyncPgConnection,
        id: Self::Id,
    ) -> Result<Self, diesel::result::Error>;

    fn id(&self) -> Self::Id;
}

impl<T: Loadable> Loadable for Vec<T> {
    type Id = Vec<T::Id>;

    fn load_from_db(
        pg_connection: &AsyncPgConnection,
        ids: Self::Id,
    ) -> Result<Self, diesel::result::Error> {
        // Probably a pretty inefficient implementation, but we can address it later if it matters.
        ids.into_iter()
            .map(|id| T::load_from_db(pg_connection, id))
            .try_collect()
    }

    fn id(&self) -> Self::Id {
        self.iter().map(|v| v.id()).collect()
    }
}
