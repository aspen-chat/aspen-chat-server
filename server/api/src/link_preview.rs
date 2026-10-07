//! Wire types for link-preview cards.
//!
//! The [`LinkPreview`] DTO is what clients see on every
//! [`Message`][crate::message_enum::Message] record — both REST
//! responses and the `Create` WebSocket event. The text fields (title,
//! description, site name, theme colour) are the card's final rendered
//! content; the third-party origin URL is deliberately not exposed so a
//! client never has to reach off-network to paint a card.
//!
//! `image_url` (when present) is the anonymous-read URL for the server's
//! own copy of the preview thumbnail. The bytes are uploaded to S3 by
//! [`aspen_app::link_preview`] when the card is materialised, and the
//! URL is templated against the configured `public_base_url` so clients
//! fetch the image directly from object storage with no Aspen API
//! involvement.

pub use aspen_wire::link_preview::LinkPreview;
