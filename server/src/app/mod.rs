use diesel::deserialize::{FromSql, FromSqlRow};
use diesel::expression::{AsExpression, TypedExpressionType};
use diesel::pg::sql_types::Uuid as PgUuid;
use diesel::pg::{Pg, PgValue};
use diesel::serialize::ToSql;
use diesel::sql_types::{SqlType, Uuid as DieselUuid};
use diesel::{QueryId, Queryable};
use heck::ToKebabCase;
use serde::{Deserialize, Serialize};
use std::error::Error as StdError;
use std::fmt::{Debug, Display, Formatter};
use std::result::Result as StdResult;

pub mod admin;
pub mod attachment;
pub mod ban;
pub mod benchmark;
pub mod block;
pub mod bot;
pub mod bot_command;
pub mod category;
pub mod category_collapse;
pub mod channel;
pub mod channel_mute;
pub mod community;
pub mod custom_emoji;
pub mod deployment;
pub mod dm;
mod error;
pub mod event_feed;
pub mod everyone_limit;
pub mod federation;
pub mod file_transfer;
pub mod fleet;
pub mod icon;
pub mod invite;
pub mod link_preview;
pub mod locale;
pub mod login;
pub mod markdown;
pub mod media_store;
pub mod mention;
pub mod message;
pub mod notification_setting;
pub mod outbound;
pub mod passkey;
pub mod permissions;
pub mod poll;
pub mod preferences;
pub mod push;
pub mod rate_limit;
pub mod react;
pub mod read_state;
pub mod registration_invite;
pub mod role;
pub mod search;
pub mod system_account;
pub mod thread;
pub mod two_factor;
pub mod user;
pub mod user_status;
pub mod visibility;
pub mod voice;
use crate::api::GlobalServerContext;
pub use error::Error;
pub use error::Result;

/// `Display` and `FromStr` for a unit-variant enum through its serde names, so the names it
/// has on the wire are the only ones it has anywhere.
macro_rules! wire_name_traits {
    ($type_name:ty) => {
        impl std::fmt::Display for $type_name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match serde_json::to_value(self) {
                    Ok(serde_json::Value::String(name)) => f.write_str(&name),
                    _ => Err(std::fmt::Error),
                }
            }
        }

        impl std::str::FromStr for $type_name {
            type Err = serde_json::Error;
            fn from_str(name: &str) -> Result<Self, Self::Err> {
                serde_json::from_value(serde_json::Value::String(name.to_string()))
            }
        }
    };
}
pub(crate) use wire_name_traits;

/// Diesel's `FromSql` and `ToSql` for a `bitflags` set stored as a `BIGINT`, so rows load and
/// store the set itself. Unknown bits are kept, as the database holds them.
macro_rules! bigint_sql_traits {
    ($type_name:ty) => {
        impl diesel::deserialize::FromSql<diesel::sql_types::BigInt, diesel::pg::Pg>
            for $type_name
        {
            fn from_sql(
                bytes: diesel::pg::PgValue<'_>,
            ) -> diesel::deserialize::Result<Self> {
                <i64 as diesel::deserialize::FromSql<diesel::sql_types::BigInt, diesel::pg::Pg>>::from_sql(bytes)
                    .map(Self::from_bits_retain)
            }
        }

        impl diesel::serialize::ToSql<diesel::sql_types::BigInt, diesel::pg::Pg> for $type_name {
            fn to_sql<'b>(
                &'b self,
                out: &mut diesel::serialize::Output<'b, '_, diesel::pg::Pg>,
            ) -> diesel::serialize::Result {
                use std::io::Write;
                out.write_all(&self.bits().to_be_bytes())?;
                Ok(diesel::serialize::IsNull::No)
            }
        }
    };
}
pub(crate) use bigint_sql_traits;

/// Diesel's `FromSql` and `ToSql` for a type stored as `TEXT` in its `Display` form and read
/// back through `FromStr`, such as an enum with `wire_name_traits!`.
macro_rules! text_sql_traits {
    ($type_name:ty) => {
        impl diesel::deserialize::FromSql<diesel::sql_types::Text, diesel::pg::Pg> for $type_name {
            fn from_sql(bytes: diesel::pg::PgValue<'_>) -> diesel::deserialize::Result<Self> {
                let text = <String as diesel::deserialize::FromSql<
                    diesel::sql_types::Text,
                    diesel::pg::Pg,
                >>::from_sql(bytes)?;
                Ok(text.parse::<$type_name>()?)
            }
        }

        impl diesel::serialize::ToSql<diesel::sql_types::Text, diesel::pg::Pg> for $type_name {
            fn to_sql<'b>(
                &'b self,
                out: &mut diesel::serialize::Output<'b, '_, diesel::pg::Pg>,
            ) -> diesel::serialize::Result {
                use std::io::Write;
                write!(out, "{self}")?;
                Ok(diesel::serialize::IsNull::No)
            }
        }
    };
}
pub(crate) use text_sql_traits;

macro_rules! id_type {
    ($type_name:ident) => {
        #[derive(
            Debug,
            Clone,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Copy,
            Deserialize,
            Serialize,
            Hash,
            FromSqlRow,
            QueryId,
            AsExpression,
            utoipa::ToSchema,
            schemars::JsonSchema,
        )]
        #[serde(transparent)]
        #[diesel(sql_type = PgUuid)]
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
            fn from_sql(v: PgValue) -> StdResult<Self, Box<dyn StdError + Send + Sync + 'static>> {
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

id_type!(PollId);

id_type!(VoiceServerId);

id_type!(VoiceSessionId);

id_type!(CategoryId);

id_type!(AttachmentId);

id_type!(IconId);
id_type!(CustomEmojiId);

id_type!(LinkPreviewImageId);

id_type!(PasskeyId);
id_type!(RoleId);
id_type!(DeploymentRoleId);
id_type!(FederationKeyId);
id_type!(PushKeyId);
id_type!(PushSubscriptionId);

#[derive(Debug, Clone)]
pub enum MaybeLoaded<T: Loadable> {
    Loaded(T),
    NotLoaded(T::Id),
}

impl<T: Loadable> MaybeLoaded<T> {
    pub fn id(&self) -> &T::Id {
        match self {
            MaybeLoaded::Loaded(l) => l.id(),
            MaybeLoaded::NotLoaded(id) => id,
        }
    }

    pub fn from_id(id: T::Id) -> Self {
        MaybeLoaded::NotLoaded(id)
    }

    pub async fn get(&mut self, state: &GlobalServerContext) -> crate::Result<&mut T> {
        match self {
            MaybeLoaded::Loaded(v) => Ok(v),
            MaybeLoaded::NotLoaded(id) => {
                let v = T::load_from_db(state, id.clone()).await?;
                *self = MaybeLoaded::Loaded(v);
                self.get(state).await
            }
        }
    }
}

pub trait Loadable: Sized {
    type Id: Clone;
    fn load_from_db(
        state: &GlobalServerContext,
        id: Self::Id,
    ) -> impl Future<Output = crate::app::Result<Self>> + Send;

    fn id(&self) -> &Self::Id;
}

impl<T: Loadable> Queryable<PgUuid, Pg> for MaybeLoaded<T>
where
    T::Id: From<uuid::Uuid>,
{
    type Row = uuid::Uuid;

    fn build(row: Self::Row) -> diesel::deserialize::Result<Self> {
        Ok(MaybeLoaded::NotLoaded(row.into()))
    }
}

impl<SqlType, T: Loadable> FromSql<SqlType, Pg> for MaybeLoaded<T>
where
    T::Id: FromSql<SqlType, Pg>,
{
    fn from_sql(v: PgValue) -> StdResult<Self, Box<dyn StdError + Send + Sync + 'static>> {
        Ok(Self::NotLoaded(T::Id::from_sql(v)?))
    }
}

impl<SqlType, T: Loadable + Debug> ToSql<SqlType, Pg> for MaybeLoaded<T>
where
    T::Id: Debug + ToSql<SqlType, Pg>,
{
    fn to_sql<'b>(
        &'b self,
        out: &mut diesel::serialize::Output<'b, '_, Pg>,
    ) -> diesel::serialize::Result {
        self.id().to_sql(out)
    }
}

impl<SqlTy: SqlType + TypedExpressionType, T: Loadable> AsExpression<SqlTy> for MaybeLoaded<T>
where
    T::Id: AsExpression<SqlTy>,
{
    type Expression = <<T as Loadable>::Id as AsExpression<SqlTy>>::Expression;

    fn as_expression(self) -> Self::Expression {
        self.id().clone().as_expression()
    }
}

impl<SqlTy: SqlType + TypedExpressionType, T: Loadable> AsExpression<SqlTy> for &MaybeLoaded<T>
where
    T::Id: AsExpression<SqlTy>,
{
    type Expression = <<T as Loadable>::Id as AsExpression<SqlTy>>::Expression;

    fn as_expression(self) -> Self::Expression {
        self.id().clone().as_expression()
    }
}

impl<SqlTy: SqlType + TypedExpressionType, T: Loadable> AsExpression<SqlTy> for &mut MaybeLoaded<T>
where
    T::Id: AsExpression<SqlTy>,
{
    type Expression = <<T as Loadable>::Id as AsExpression<SqlTy>>::Expression;

    fn as_expression(self) -> Self::Expression {
        self.id().clone().as_expression()
    }
}

pub const ASPEN_NATS_STREAM_NAME: &str = "aspen_omni_stream";

pub mod events;
pub use events::{EventScope, publish_event};
