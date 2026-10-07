//! Previews of pictures, made in the server's own process (see `super`) by
//! `aspen_previews::picture`.

use super::{Made, Outcome};
use crate::aspen_config::PreviewConfig;
use crate::context::GlobalServerContext;
use aspen_previews::picture::preview;

/// Reads a picture from storage and makes its preview.
pub async fn make(
    state: &GlobalServerContext,
    config: &PreviewConfig,
    key: &str,
    original_bytes: u64,
) -> Outcome {
    if original_bytes > config.max_picture_bytes {
        return Outcome::NoPreview("larger than max_picture_bytes");
    }
    let mut bytes = Vec::with_capacity(usize::try_from(original_bytes).unwrap_or_default());
    match state
        .media_store
        .copy_object_to(key, config.max_picture_bytes, &mut bytes)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return Outcome::NoPreview("larger than max_picture_bytes"),
        Err(e) => return Outcome::Failed(e.to_string()),
    }
    let max_pixels = config.max_picture_pixels;
    match tokio::task::spawn_blocking(move || preview(&bytes, max_pixels)).await {
        Ok(Ok(picture)) => Outcome::Made(Made {
            bytes: picture.webp,
            mime_type: "image/webp",
            width: picture.width,
            height: picture.height,
        }),
        Ok(Err(reason)) => Outcome::NoPreview(reason),
        Err(e) => Outcome::Failed(format!("the preview's maker stopped: {e}")),
    }
}
