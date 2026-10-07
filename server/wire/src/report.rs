//! What a moderator's warning is about.

use crate::user::CustomStatus;
use crate::{CommunityId, IconId, MessageId, UserId};
use diesel::{AsExpression, FromSqlRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Something on a profile a report may find objectionable.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    strum::VariantArray,
)]
#[serde(rename_all = "camelCase")]
pub enum ProfileAspect {
    DisplayName,
    Username,
    Picture,
    Status,
    Bio,
    Pronouns,
}

crate::wire_name_traits!(ProfileAspect);

/// A profile as a report found it.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[diesel(sql_type = diesel::sql_types::Jsonb)]
#[serde(rename_all = "camelCase")]
pub struct ProfileSnapshot {
    pub name: String,
    pub display_name: Option<String>,
    /// The picture, which is kept while a report names it.
    pub icon: Option<IconId>,
    pub status: Option<CustomStatus>,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
}

crate::jsonb_sql_traits!(ProfileSnapshot);

/// What a moderator's warning is about: the person warned, and the message reported, or their
/// profile as the reports found it with the aspects they named. The message is shown to the
/// people of the warning's DM even once deleted.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[diesel(sql_type = diesel::sql_types::Jsonb)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    pub subject: UserId,
    pub message: Option<MessageId>,
    pub profile: Option<ProfileSnapshot>,
    #[serde(default)]
    pub aspects: Vec<ProfileAspect>,
    /// For a warning about a nickname, the nickname as the latest report found it.
    #[serde(default)]
    pub nickname: Option<NicknameSnapshot>,
}

crate::jsonb_sql_traits!(Warning);

/// A nickname a warning is about, with the community it was chosen in, named as it was when the
/// warning was sent, so the warning reads the same once the person has left or the community is
/// gone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NicknameSnapshot {
    pub community: CommunityId,
    pub community_name: String,
    pub nickname: String,
}
