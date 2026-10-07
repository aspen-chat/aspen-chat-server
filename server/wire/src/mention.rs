//! Who a message tags.

use crate::{RoleId, UserId};
use diesel::sql_types::Jsonb;
use diesel::{AsExpression, FromSqlRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Who a message tags, as far as its author was allowed to: these are the tags that count.
#[derive(
    Debug,
    Clone,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[diesel(sql_type = Jsonb)]
#[serde(rename_all = "camelCase")]
pub struct Mentions {
    /// Members tagged by name, in the order first tagged.
    pub users: Vec<UserId>,
    /// Roles tagged, in the order first tagged; everyone's is tagged as `everyone` instead.
    pub roles: Vec<RoleId>,
    /// Whether the message tags everyone who can see the channel.
    pub everyone: bool,
}

impl Mentions {
    pub fn is_empty(&self) -> bool {
        self.users.is_empty() && self.roles.is_empty() && !self.everyone
    }
}

crate::jsonb_sql_traits!(Mentions);
