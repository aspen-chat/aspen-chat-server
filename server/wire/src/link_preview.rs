//! Previews of links in messages.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// An embeddable player for a link to a video on an allowlisted provider. `src` is the
/// provider's own player page, always `https`, on a host the server allowlists; clients frame
/// it only after the reader asks to play.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct VideoEmbed {
    pub src: String,
    pub width: u32,
    pub height: u32,
}

/// Wire-level representation of a single link preview.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub image_url: Option<String>,
    /// The picture's size in pixels, both or neither, so readers can make room for it before
    /// it loads.
    pub image_width: Option<u32>,
    pub image_height: Option<u32>,
    pub theme_color: Option<String>,
    /// Present when the link is a video on a provider the server embeds players from.
    pub video: Option<VideoEmbed>,
}
