//! Wire types and the read endpoint for link-preview thumbnails.
//!
//! The [`LinkPreview`] DTO is what clients see on every
//! [`Message`][crate::api::message_enum::Message] record — both REST responses
//! and the `Create` WebSocket event. The text fields (title, description,
//! site name, theme colour) are the card's final rendered content, and
//! `image_id` points at the server's own copy of the preview thumbnail: the
//! third-party origin URL is deliberately not exposed on the DTO so a client
//! never has to reach off-network to paint a card, and the entire outbound
//! fetch cost falls on the server where it can be cached and rate-limited.
//! The actual image bytes are served by [`read_link_preview_image`] via
//! `GET /link-preview-image`, following the same JSON-body-with-bytes shape
//! as the `read_icon` / `read_attachment` endpoints so a single generated
//! client can speak to all three without a special case.

use crate::api::GlobalServerContext;
use crate::app::{self, LinkPreviewImageId};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use tracing::error;
use utoipa::ToSchema;

/// S3 key prefix under which preview-image blobs live inside the media store.
pub const IMAGE_STORAGE_PREFIX: &str = "link-preview-images";

/// Build the S3 object key for a given preview image id.
pub fn image_storage_key(id: LinkPreviewImageId) -> String {
    format!("{IMAGE_STORAGE_PREFIX}/{}", id.0)
}

/// Wire-level representation of a single link preview.
///
/// This is the only shape clients ever see. The per-row `image_mime_type`
/// column lives in the database alongside [`image_id`](Self::image_id) so the
/// image-read endpoint can hand back an authoritative content type without
/// round-tripping to S3's object metadata; clients don't need it until they
/// actually fetch the image, so we don't publish it on the Message record.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreview {
    pub url: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    pub image_id: Option<LinkPreviewImageId>,
    pub theme_color: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreviewImageReadCommand {
    pub id: LinkPreviewImageId,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum LinkPreviewImageReadCommandResponse {
    Image {
        data: Vec<u8>,
        mime_type: String,
    },
    Error {
        cause: Option<Cow<'static, str>>,
    },
}

#[utoipa::path(get, path = "/link-preview-image", responses((status = OK, body = LinkPreviewImageReadCommandResponse)))]
pub async fn read_link_preview_image(
    State(state): State<GlobalServerContext>,
    Json(command): Json<LinkPreviewImageReadCommand>,
) -> (StatusCode, Json<LinkPreviewImageReadCommandResponse>) {
    match app::link_preview::read_image(&state, command.id).await {
        Ok((data, mime_type)) => (
            StatusCode::OK,
            LinkPreviewImageReadCommandResponse::Image { data, mime_type }.into(),
        ),
        Err(app::Error::Diesel(diesel::result::Error::NotFound)) => (
            StatusCode::NOT_FOUND,
            LinkPreviewImageReadCommandResponse::Error { cause: None }.into(),
        ),
        Err(app::Error::S3GetObject(e)) => {
            // Row is present but S3 lost the bytes. From the caller's
            // perspective the image is missing; surface that as 404 so
            // the client can render the preview without a thumbnail
            // rather than failing the whole card.
            error!(
                error = e.to_string(),
                id = command.id.0.to_string(),
                "link preview image not found in media store"
            );
            (
                StatusCode::NOT_FOUND,
                LinkPreviewImageReadCommandResponse::Error { cause: None }.into(),
            )
        }
        Err(e) => {
            error!(error = e.to_string(), "error reading link preview image");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                LinkPreviewImageReadCommandResponse::Error { cause: None }.into(),
            )
        }
    }
}
