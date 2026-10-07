//! Cards: what a message a plugin's account posts shows beneath its text.

use super::PluginText;
use crate::UserId;
use chrono::{DateTime, Utc};
use diesel::{AsExpression, FromSqlRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What a message of a plugin's account shows beneath its text.
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
pub struct Card {
    /// The plugin whose card it is, whose catalogue its text is drawn from.
    pub plugin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<PluginText>,
    pub fields: Vec<CardField>,
    pub buttons: Vec<CardButton>,
}

crate::jsonb_sql_traits!(Card);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CardField {
    pub label: PluginText,
    pub value: CardValue,
}

/// What a card's field shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CardValue {
    /// Text as it is, not translated.
    Plain {
        text: String,
    },
    /// A time, which each client shows in its reader's time zone and language.
    Time {
        at: DateTime<Utc>,
    },
    Count {
        count: u64,
    },
    /// A person, whom clients name.
    Person {
        user: UserId,
    },
    /// An `https` link, and the text it shows.
    Link {
        url: String,
        text: PluginText,
    },
}

/// A button, which calls the card's plugin as whoever presses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CardButton {
    pub id: String,
    pub label: PluginText,
    pub style: ButtonStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Danger,
}
