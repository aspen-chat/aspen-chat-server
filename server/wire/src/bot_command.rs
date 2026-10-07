//! A bot command's arguments as the bot receives them.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What a parameter's value must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum ParameterType {
    /// Someone's id; a client completes it from the people there.
    UserId,
    /// A channel's id, one the person invoking may view.
    ChannelId,
    /// A message's id, one the person invoking may read.
    MessageId,
    /// A community's id, one the person invoking belongs to.
    CommunityId,
    /// A role's id, of the community the command is invoked in.
    RoleId,
    /// A file the person invoking attaches to the command.
    AttachmentId,
    /// A deployment's domain, as federation names deployments.
    DeploymentHost,
    /// One emoji.
    React,
    /// Any text; as the last parameter it takes the rest of what was typed.
    Any,
    /// Text that matches the parameter's `pattern` whole.
    Regex,
}

/// One argument as the bot receives it: named, typed, and checked against its type, an emoji
/// in its fully qualified form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Argument {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: ParameterType,
    pub value: String,
}
