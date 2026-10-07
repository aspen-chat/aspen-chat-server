//! A plugin's settings: the fields its manifest declares for the operator (`settings`) and for
//! each community (`communitySettings`), and checking values against them. Clients draw a form
//! from the fields, so they are a small vocabulary of types rather than any JSON Schema.

use crate::{ChannelId, CommunityId, RoleId};
use aspen_schema::{channel, community_role};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use utoipa::ToSchema;
use uuid::Uuid;

/// The longest a `text` or `longText` value may be when its field sets no limit, in characters.
pub const DEFAULT_MAX_LENGTH: u32 = 1000;
/// The most entries of a list when its field sets no limit.
pub const DEFAULT_MAX_ITEMS: u32 = 100;
/// The longest any one value may be, in characters, whatever its field says.
pub const MAX_LENGTH: u32 = 20_000;
/// The most entries any list may have, whatever its field says.
pub const MAX_ITEMS: u32 = 5_000;

/// One setting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SettingField {
    /// What the plugin calls it: letters and digits, starting with a letter.
    pub name: String,
    /// A key of the plugin's messages naming it.
    pub label: String,
    /// A key of the plugin's messages saying more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(flatten)]
    pub kind: FieldKind,
    /// Whether it must be given; a field with a default never needs to be.
    #[serde(default)]
    pub required: bool,
    /// Its value until one is given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    /// For a `text` field, kept out of every read and log: only the plugin receives it.
    #[serde(default)]
    pub secret: bool,
}

/// What a setting holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FieldKind {
    Boolean,
    #[serde(rename_all = "camelCase")]
    Integer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    /// One line of text.
    #[serde(rename_all = "camelCase")]
    Text {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u32>,
    },
    /// Text of several lines.
    #[serde(rename_all = "camelCase")]
    LongText {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u32>,
    },
    /// Lines of text, one entry each.
    #[serde(rename_all = "camelCase")]
    TextList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u32>,
    },
    /// One of `options`.
    #[serde(rename_all = "camelCase")]
    Choice {
        options: Vec<ChoiceOption>,
    },
    /// One of the community's roles (community settings only).
    Role,
    /// Some of the community's roles (community settings only).
    #[serde(rename_all = "camelCase")]
    RoleList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u32>,
    },
    /// One of the community's channels (community settings only).
    Channel,
    /// Some of the community's channels (community settings only).
    #[serde(rename_all = "camelCase")]
    ChannelList {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u32>,
    },
}

/// One option of a `choice`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ChoiceOption {
    pub value: String,
    /// A key of the plugin's messages naming it.
    pub label: String,
}

impl FieldKind {
    /// Whether the field names something of a community, and so belongs only in community
    /// settings.
    pub fn of_community(&self) -> bool {
        matches!(
            self,
            FieldKind::Role
                | FieldKind::RoleList { .. }
                | FieldKind::Channel
                | FieldKind::ChannelList { .. }
        )
    }
}

/// Why a value is not what its field takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A field the plugin does not have.
    Unknown,
    /// Required, and neither given nor defaulted.
    Missing,
    /// Not of the field's type.
    WrongType,
    /// Below `min` or above `max`.
    OutOfRange,
    /// Longer than the field allows.
    TooLong(u32),
    /// More entries than the field allows.
    TooMany(u32),
    /// Not one of a choice's options.
    NotAnOption,
    /// A role or channel that is not the community's.
    NotInCommunity,
}

/// A value that is not what its field takes, by the field's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldProblem {
    pub field: String,
    pub problem: Problem,
}

impl FieldProblem {
    fn new(field: &str, problem: Problem) -> Self {
        FieldProblem {
            field: field.to_string(),
            problem,
        }
    }
}

/// What a field's declaration is wrong about, for the manifest's checks.
pub fn check_declaration(field: &SettingField, community: bool) -> Result<(), String> {
    let name_ok = field
        .name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && field.name.chars().all(|c| c.is_ascii_alphanumeric())
        && field.name.len() <= 64;
    if !name_ok {
        return Err(format!(
            "setting name {:?} must be letters and digits, starting with a letter, at most 64",
            field.name
        ));
    }
    if field.kind.of_community() && !community {
        return Err(format!(
            "setting {} names a community's roles or channels, which only community settings may",
            field.name
        ));
    }
    if field.secret && !matches!(field.kind, FieldKind::Text { .. }) {
        return Err(format!(
            "setting {} is secret, which only a text setting may be",
            field.name
        ));
    }
    match &field.kind {
        FieldKind::Integer {
            min: Some(min),
            max: Some(max),
        } if min > max => {
            return Err(format!("setting {}'s min is above its max", field.name));
        }
        FieldKind::Text {
            max_length: Some(n),
        }
        | FieldKind::LongText {
            max_length: Some(n),
        } if *n == 0 || *n > MAX_LENGTH => {
            return Err(format!(
                "setting {}'s maxLength must be 1 to {MAX_LENGTH}",
                field.name
            ));
        }
        FieldKind::TextList { max_items: Some(n) }
        | FieldKind::RoleList { max_items: Some(n) }
        | FieldKind::ChannelList { max_items: Some(n) }
            if *n == 0 || *n > MAX_ITEMS =>
        {
            return Err(format!(
                "setting {}'s maxItems must be 1 to {MAX_ITEMS}",
                field.name
            ));
        }
        FieldKind::Choice { options } => {
            if options.is_empty() {
                return Err(format!("setting {} offers no options", field.name));
            }
            let mut seen = HashSet::new();
            if !options.iter().all(|o| seen.insert(&o.value)) {
                return Err(format!("setting {} offers an option twice", field.name));
            }
        }
        _ => {}
    }
    if let Some(default) = &field.default {
        if field.kind.of_community() {
            return Err(format!(
                "setting {} names a community's roles or channels, which no default can",
                field.name
            ));
        }
        check_value(field, default)
            .map_err(|p| format!("setting {}'s default is not valid: {:?}", field.name, p))?;
    }
    Ok(())
}

/// Checks `value` against `field`, leaving roles and channels to `check_community`.
fn check_value(field: &SettingField, value: &Value) -> Result<(), Problem> {
    let max_length = |n: Option<u32>| n.unwrap_or(DEFAULT_MAX_LENGTH).min(MAX_LENGTH);
    let max_items = |n: Option<u32>| n.unwrap_or(DEFAULT_MAX_ITEMS).min(MAX_ITEMS);
    let ids = |value: &Value, max: Option<u32>| -> Result<(), Problem> {
        let items = value.as_array().ok_or(Problem::WrongType)?;
        if items.len() > max_items(max) as usize {
            return Err(Problem::TooMany(max_items(max)));
        }
        for item in items {
            item.as_str()
                .and_then(|s| Uuid::parse_str(s).ok())
                .ok_or(Problem::WrongType)?;
        }
        Ok(())
    };
    match &field.kind {
        FieldKind::Boolean => value.as_bool().map(|_| ()).ok_or(Problem::WrongType),
        FieldKind::Integer { min, max } => {
            let n = value.as_i64().ok_or(Problem::WrongType)?;
            if min.is_some_and(|min| n < min) || max.is_some_and(|max| n > max) {
                return Err(Problem::OutOfRange);
            }
            Ok(())
        }
        FieldKind::Text { max_length: n } => {
            let s = value.as_str().ok_or(Problem::WrongType)?;
            if s.contains('\n') {
                return Err(Problem::WrongType);
            }
            if s.chars().count() > max_length(*n) as usize {
                return Err(Problem::TooLong(max_length(*n)));
            }
            Ok(())
        }
        FieldKind::LongText { max_length: n } => {
            let s = value.as_str().ok_or(Problem::WrongType)?;
            if s.chars().count() > max_length(*n) as usize {
                return Err(Problem::TooLong(max_length(*n)));
            }
            Ok(())
        }
        FieldKind::TextList { max_items: n } => {
            let items = value.as_array().ok_or(Problem::WrongType)?;
            if items.len() > max_items(*n) as usize {
                return Err(Problem::TooMany(max_items(*n)));
            }
            for item in items {
                let s = item.as_str().ok_or(Problem::WrongType)?;
                if s.contains('\n') || s.chars().count() > DEFAULT_MAX_LENGTH as usize {
                    return Err(Problem::TooLong(DEFAULT_MAX_LENGTH));
                }
            }
            Ok(())
        }
        FieldKind::Choice { options } => {
            let s = value.as_str().ok_or(Problem::WrongType)?;
            options
                .iter()
                .any(|o| o.value == s)
                .then_some(())
                .ok_or(Problem::NotAnOption)
        }
        FieldKind::Role | FieldKind::Channel => value
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .map(|_| ())
            .ok_or(Problem::WrongType),
        FieldKind::RoleList { max_items: n } | FieldKind::ChannelList { max_items: n } => {
            ids(value, *n)
        }
    }
}

/// `current` with `patch` laid over it as a JSON Merge Patch (a field given `null` goes back to
/// its default), checked against `fields`. Roles and channels are checked by `check_community`.
pub fn apply(
    fields: &[SettingField],
    current: &Map<String, Value>,
    patch: &Map<String, Value>,
) -> Result<Map<String, Value>, FieldProblem> {
    let mut next = current.clone();
    for (name, value) in patch {
        let field = fields
            .iter()
            .find(|f| &f.name == name)
            .ok_or_else(|| FieldProblem::new(name, Problem::Unknown))?;
        if value.is_null() {
            next.remove(name);
            continue;
        }
        check_value(field, value).map_err(|p| FieldProblem::new(name, p))?;
        next.insert(name.clone(), value.clone());
    }
    // Fields the plugin no longer has, after an upgrade, are dropped.
    next.retain(|name, _| fields.iter().any(|f| &f.name == name));
    for field in fields {
        if field.required && field.default.is_none() && !next.contains_key(&field.name) {
            return Err(FieldProblem::new(&field.name, Problem::Missing));
        }
    }
    Ok(next)
}

/// What the plugin receives: `stored` with each missing field's default.
pub fn effective(fields: &[SettingField], stored: &Map<String, Value>) -> Map<String, Value> {
    let mut out = stored.clone();
    for field in fields {
        if !out.contains_key(&field.name)
            && let Some(default) = &field.default
        {
            out.insert(field.name.clone(), default.clone());
        }
    }
    out
}

/// What people with the right to configure it read: `stored` without its secrets, and the
/// names of the secret fields that are set.
pub fn readable(
    fields: &[SettingField],
    stored: &Map<String, Value>,
) -> (Map<String, Value>, Vec<String>) {
    let mut shown = stored.clone();
    let mut set = Vec::new();
    for field in fields.iter().filter(|f| f.secret) {
        if shown.remove(&field.name).is_some() {
            set.push(field.name.clone());
        }
    }
    (shown, set)
}

/// Checks that every role and channel `settings` names is `community`'s own: a role of it, and
/// a live channel of it (a thread is not offered).
pub async fn check_community(
    conn: &mut AsyncPgConnection,
    fields: &[SettingField],
    community: CommunityId,
    settings: &Map<String, Value>,
) -> Result<(), FieldProblem> {
    for field in fields.iter().filter(|f| f.kind.of_community()) {
        let Some(value) = settings.get(&field.name) else {
            continue;
        };
        let named: Vec<Uuid> = match value {
            Value::String(s) => Uuid::parse_str(s).into_iter().collect(),
            Value::Array(items) => items
                .iter()
                .filter_map(|i| i.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                .collect(),
            _ => Vec::new(),
        };
        if named.is_empty() {
            continue;
        }
        let found: i64 = match field.kind {
            FieldKind::Role | FieldKind::RoleList { .. } => {
                let ids: Vec<RoleId> = named.iter().copied().map(RoleId).collect();
                community_role::table
                    .filter(
                        community_role::id
                            .eq_any(&ids)
                            .and(community_role::community.eq(community)),
                    )
                    .count()
                    .get_result(conn)
                    .await
                    .map_err(|_| FieldProblem::new(&field.name, Problem::NotInCommunity))?
            }
            _ => {
                let ids: Vec<ChannelId> = named.iter().copied().map(ChannelId).collect();
                channel::table
                    .filter(
                        channel::id
                            .eq_any(&ids)
                            .and(channel::community.eq(community))
                            .and(channel::deleted_at.is_null()),
                    )
                    .count()
                    .get_result(conn)
                    .await
                    .map_err(|_| FieldProblem::new(&field.name, Problem::NotInCommunity))?
            }
        };
        let distinct = named.iter().collect::<HashSet<_>>().len();
        if found != distinct as i64 {
            return Err(FieldProblem::new(&field.name, Problem::NotInCommunity));
        }
    }
    Ok(())
}

/// The localized refusal for a value that is not what its field takes, naming the field by its
/// label in the reader's language.
pub fn refusal(problem: &FieldProblem, label: &str) -> crate::Error {
    use crate::t;
    crate::Error::Validation(match &problem.problem {
        Problem::Unknown => t!("pluginSettingUnknown", field = problem.field),
        Problem::Missing => t!("pluginSettingMissing", field = label),
        Problem::WrongType => t!("pluginSettingWrongType", field = label),
        Problem::OutOfRange => t!("pluginSettingOutOfRange", field = label),
        Problem::TooLong(max) => t!("pluginSettingTooLong", field = label, max = max),
        Problem::TooMany(max) => t!("pluginSettingTooMany", field = label, max = max),
        Problem::NotAnOption => t!("pluginSettingNotAnOption", field = label),
        Problem::NotInCommunity => t!("pluginSettingNotInCommunity", field = label),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fields() -> Vec<SettingField> {
        serde_json::from_value(json!([
            {"name": "words", "type": "textList", "label": "l", "maxItems": 2},
            {"name": "action", "type": "choice", "label": "l", "default": "mask",
             "options": [{"value": "mask", "label": "m"}, {"value": "refuse", "label": "r"}]},
            {"name": "limit", "type": "integer", "label": "l", "min": 1, "max": 10, "required": true},
            {"name": "key", "type": "text", "label": "l", "secret": true}
        ]))
        .unwrap()
    }

    fn object(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn values_are_checked_against_their_fields() {
        let fields = fields();
        let ok = apply(
            &fields,
            &Map::new(),
            &object(json!({"limit": 3, "words": ["a"]})),
        );
        assert!(ok.is_ok());
        assert_eq!(
            apply(&fields, &Map::new(), &object(json!({"limit": 11})))
                .unwrap_err()
                .problem,
            Problem::OutOfRange
        );
        assert_eq!(
            apply(
                &fields,
                &Map::new(),
                &object(json!({"limit": 1, "words": ["a", "b", "c"]}))
            )
            .unwrap_err()
            .problem,
            Problem::TooMany(2)
        );
        assert_eq!(
            apply(
                &fields,
                &Map::new(),
                &object(json!({"limit": 1, "action": "ban"}))
            )
            .unwrap_err()
            .problem,
            Problem::NotAnOption
        );
        assert_eq!(
            apply(&fields, &Map::new(), &object(json!({"words": []})))
                .unwrap_err()
                .problem,
            Problem::Missing
        );
        assert_eq!(
            apply(
                &fields,
                &Map::new(),
                &object(json!({"limit": 1, "other": 1}))
            )
            .unwrap_err()
            .problem,
            Problem::Unknown
        );
    }

    #[test]
    fn null_restores_the_default_and_secrets_are_not_read_back() {
        let fields = fields();
        let set = apply(
            &fields,
            &Map::new(),
            &object(json!({"limit": 2, "action": "refuse", "key": "hunter2"})),
        )
        .unwrap();
        let cleared = apply(&fields, &set, &object(json!({"action": null}))).unwrap();
        assert_eq!(effective(&fields, &cleared)["action"], json!("mask"));
        let (shown, secrets) = readable(&fields, &cleared);
        assert!(shown.get("key").is_none());
        assert_eq!(secrets, vec!["key".to_string()]);
    }

    #[test]
    fn declarations_are_checked() {
        let mut field = fields().remove(0);
        assert!(check_declaration(&field, false).is_ok());
        field.name = "9lives".into();
        assert!(check_declaration(&field, false).is_err());
        let role: SettingField =
            serde_json::from_value(json!({"name": "r", "type": "role", "label": "l"})).unwrap();
        assert!(check_declaration(&role, false).is_err());
        assert!(check_declaration(&role, true).is_ok());
    }
}
