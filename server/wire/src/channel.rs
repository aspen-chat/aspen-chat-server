//! What kind a channel is.

use diesel::deserialize::FromSql;
use diesel::pg::Pg;
use diesel::serialize::{IsNull, Output, ToSql};
use diesel::{AsExpression, FromSqlRow};
use serde::{Deserialize, Serialize};
use std::io::Write;
use strum::VariantArray as _;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Deserialize,
    Serialize,
    utoipa::ToSchema,
    schemars::JsonSchema,
    FromSqlRow,
    AsExpression,
    strum::VariantArray,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = aspen_schema::sql_types::ChannelType)]
pub enum ChannelType {
    Text,
    Voice,
    /// Replies to one message of a text channel, DM, or group DM; see `Channel.parentChannel`.
    Thread,
    /// A conversation between two people, outside any community.
    Dm,
    /// A conversation among up to `app::dm::MAX_RECIPIENTS` people, outside any community.
    GroupDm,
    /// A channel of a kind a plugin adds, which `pluginType` names; its contents are the
    /// plugin's, shown by its view (`app::plugin::channel_type`).
    Plugin,
}

crate::wire_name_traits!(ChannelType);

impl ChannelType {
    /// The variant's label in the database's `channel_type` enum, read in both directions.
    const fn sql_name(self) -> &'static str {
        match self {
            ChannelType::Text => "text",
            ChannelType::Voice => "voice",
            ChannelType::Thread => "thread",
            ChannelType::Dm => "dm",
            ChannelType::GroupDm => "group_dm",
            ChannelType::Plugin => "plugin",
        }
    }
}

impl ToSql<aspen_schema::sql_types::ChannelType, Pg> for ChannelType {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        out.write_all(self.sql_name().as_bytes())?;
        Ok(IsNull::No)
    }
}

impl FromSql<aspen_schema::sql_types::ChannelType, Pg> for ChannelType {
    fn from_sql(
        bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
    ) -> diesel::deserialize::Result<Self> {
        Self::VARIANTS
            .iter()
            .copied()
            .find(|variant| variant.sql_name().as_bytes() == bytes.as_bytes())
            .ok_or_else(|| {
                format!(
                    "Unrecognized enum variant: {:?}",
                    String::from_utf8_lossy(bytes.as_bytes())
                )
                .into()
            })
    }
}
