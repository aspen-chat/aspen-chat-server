//! Wire types for link-preview cards.
//!
//! The [`LinkPreview`] DTO is what clients see on every
//! [`Message`][crate::api::message_enum::Message] record — both REST
//! responses and the `Create` WebSocket event. The text fields (title,
//! description, site name, theme colour) are the card's final rendered
//! content; the third-party origin URL is deliberately not exposed so a
//! client never has to reach off-network to paint a card.
//!
//! `image_url` (when present) is the anonymous-read URL for the server's
//! own copy of the preview thumbnail. The bytes are uploaded to S3 by
//! [`crate::app::link_preview`] when the card is materialised, and the
//! URL is templated against the configured `public_base_url` so clients
//! fetch the image directly from object storage with no Aspen API
//! involvement.

use crate::app::LinkPreviewImageId;
pub use aspen_wire::link_preview::{LinkPreview, VideoEmbed};

/// S3 key prefix under which preview-image blobs live inside the media store.
pub const IMAGE_STORAGE_PREFIX: &str = "link-preview-images";

/// Build the S3 object key for a given preview image id.
pub fn image_storage_key(id: LinkPreviewImageId) -> String {
    format!("{IMAGE_STORAGE_PREFIX}/{}", id.0)
}
