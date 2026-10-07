//! What kind a channel is.

use diesel::deserialize::FromSql;
use diesel::pg::Pg;
use diesel::serialize::{IsNull, Output, ToSql};
use diesel::{AsExpression, FromSqlRow};
use serde::{Deserialize, Serialize};
use std::io::Write;

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

impl ToSql<aspen_schema::sql_types::ChannelType, Pg> for ChannelType {
    fn to_sql<'b>(&'b self, out: &mut Output<'b, '_, Pg>) -> diesel::serialize::Result {
        out.write_all(match self {
            ChannelType::Text => b"text",
            ChannelType::Voice => b"voice",
            ChannelType::Thread => b"thread",
            ChannelType::Dm => b"dm",
            ChannelType::GroupDm => b"group_dm",
            ChannelType::Plugin => b"plugin",
        })?;
        Ok(IsNull::No)
    }
}

impl FromSql<aspen_schema::sql_types::ChannelType, Pg> for ChannelType {
    fn from_sql(
        bytes: <Pg as diesel::backend::Backend>::RawValue<'_>,
    ) -> diesel::deserialize::Result<Self> {
        match bytes.as_bytes() {
            b"voice" => Ok(ChannelType::Voice),
            b"text" => Ok(ChannelType::Text),
            b"thread" => Ok(ChannelType::Thread),
            b"dm" => Ok(ChannelType::Dm),
            b"group_dm" => Ok(ChannelType::GroupDm),
            b"plugin" => Ok(ChannelType::Plugin),
            _ => Err(format!(
                "Unrecognized enum variant: {:?}",
                String::from_utf8_lossy(bytes.as_bytes())
            )
            .into()),
        }
    }
}
