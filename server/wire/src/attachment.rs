//! An attachment's preview.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A smaller copy of an attachment for showing it inline: a picture fitted within
/// `aspen_previews::BOX` and encoded for the web, or the same of a video.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentPreview {
    pub url: String,
    pub mime_type: String,
    /// Its size in pixels, as it is shown upright.
    pub width: u32,
    pub height: u32,
}
