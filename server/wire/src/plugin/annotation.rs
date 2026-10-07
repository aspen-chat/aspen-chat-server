//! How much a plugin's annotation matters.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// How much an annotation matters, which decides how a client draws it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Info,
    Notice,
    Warning,
}

crate::wire_name_traits!(Severity);
