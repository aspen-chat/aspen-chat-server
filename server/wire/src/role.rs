//! A role's override as a request names it.

use crate::RoleId;
use crate::permissions::Permission;

/// One role's override as a request names it before its channel exists (`overrides` on a new
/// channel): what the role is allowed and denied there besides its own permissions.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
    schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct RoleOverride {
    pub role: RoleId,
    pub allow: Vec<Permission>,
    pub deny: Vec<Permission>,
}
