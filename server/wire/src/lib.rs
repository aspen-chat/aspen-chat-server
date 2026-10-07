//! What the API server and its clients exchange, as `message_gen` makes it from
//! `message_enum`: the records, the request bodies, and the server events, with the IDs and the
//! types the records hold, and how each is stored in the database.

// The catalogue `t!` reads, which `aspen_locale` holds.
use aspen_locale::{_rust_i18n_try_translate, t};
use diesel::deserialize::FromSql;
use diesel::pg::sql_types::Uuid as PgUuid;
use diesel::pg::{Pg, PgValue};
use diesel::serialize::ToSql;
use diesel::sql_types::Uuid as DieselUuid;
use diesel::{AsExpression, FromSqlRow, QueryId};
use heck::ToKebabCase;
use serde::{Deserialize, Deserializer, Serialize};
use std::error::Error as StdError;
use std::fmt::{Display, Formatter};
use std::result::Result as StdResult;

/// Every REST route and the event stream live under this prefix. Bumping the version means a
/// breaking change to the wire contract; additive changes stay within `v1`.
pub const API_PREFIX: &str = "/api/v1";

pub mod attachment;
pub mod bot_command;
pub mod channel;
pub mod deployment;
pub mod link_preview;
pub mod mention;
pub mod message;
pub mod message_enum;
pub mod notification_setting;
pub mod permissions;
pub mod plugin;
pub mod poll;
pub mod report;
pub mod role;
pub mod user;
pub mod voice;

/// `Display` and `FromStr` for a unit-variant enum through its serde names, so the names it
/// has on the wire are the only ones it has anywhere.
#[macro_export]
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

/// Diesel's `FromSql` and `ToSql` for a `bitflags` set stored as a `BIGINT`, so rows load and
/// store the set itself. Unknown bits are kept, as the database holds them.
#[macro_export]
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

/// Diesel's `FromSql` and `ToSql` for a type stored as `TEXT` in its `Display` form and read
/// back through `FromStr`, such as an enum with `wire_name_traits!`.
#[macro_export]
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

/// Diesel's `FromSql` and `ToSql` for a serde type stored as `JSONB`, which Postgres sends and
/// takes as a version byte, 1, followed by the JSON text.
#[macro_export]
macro_rules! jsonb_sql_traits {
    ($type_name:ty) => {
        impl diesel::deserialize::FromSql<diesel::sql_types::Jsonb, diesel::pg::Pg> for $type_name {
            fn from_sql(value: diesel::pg::PgValue<'_>) -> diesel::deserialize::Result<Self> {
                match value.as_bytes().split_first() {
                    Some((1, json)) => Ok(serde_json::from_slice(json)?),
                    _ => Err("unsupported jsonb encoding".into()),
                }
            }
        }

        impl diesel::serialize::ToSql<diesel::sql_types::Jsonb, diesel::pg::Pg> for $type_name {
            fn to_sql<'b>(
                &'b self,
                out: &mut diesel::serialize::Output<'b, '_, diesel::pg::Pg>,
            ) -> diesel::serialize::Result {
                use std::io::Write;
                out.write_all(&[1])?;
                serde_json::to_writer(out, self)?;
                Ok(diesel::serialize::IsNull::No)
            }
        }
    };
}

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

        // A new ID is a fresh one, never a default.
        #[allow(clippy::new_without_default)]
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
// A message waiting for its attachments' previews before it is posted (`app::message::held`).
id_type!(HeldMessageId);

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
id_type!(ReportCaseId);
id_type!(ReportId);
id_type!(ReportCategoryId);
id_type!(AnnotationId);
id_type!(PluginNoticeId);
id_type!(NewsletterPostId);

/// Deserializes `Option<Option<T>>` for JSON Merge Patch fields. Plain serde folds a JSON `null`
/// into the outer `None`, which would make "clear this field" indistinguishable from "leave it
/// alone". Routing the field through this function (together with `#[serde(default)]` for the
/// absent case) maps a present `null` to `Some(None)` and a present value to `Some(Some(v))`.
pub fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
