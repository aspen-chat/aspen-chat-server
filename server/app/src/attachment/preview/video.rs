//! Previews of videos: a poster, one frame of the video made into a picture's preview, taken
//! by `aspen_previews::video` (see `super`).

use super::{Made, Outcome};
use crate::aspen_config::PreviewConfig;
use crate::context::GlobalServerContext;
use aspen_previews::video::{HDR_TRANSFERS, POSTER_AT, poster_frame, probe};
use tokio::io::AsyncWriteExt;

/// Copies a video from storage to a file of its own and makes its poster.
pub async fn make(
    state: &GlobalServerContext,
    config: &PreviewConfig,
    key: &str,
    original_bytes: u64,
) -> Outcome {
    if original_bytes > config.max_video_bytes {
        return Outcome::NoPreview("larger than max_video_bytes");
    }
    let dir = match tempfile::Builder::new().prefix("aspen-preview-").tempdir() {
        Ok(dir) => dir,
        Err(e) => return Outcome::Failed(format!("could not make a scratch directory: {e}")),
    };
    let original = dir.path().join("original");
    let copied = async {
        let mut file = tokio::fs::File::create(&original).await?;
        let copied = state
            .media_store
            .copy_object_to(key, config.max_video_bytes, &mut file)
            .await?;
        file.flush().await?;
        crate::Result::Ok(copied)
    };
    match copied.await {
        Ok(Some(_)) => {}
        Ok(None) => return Outcome::NoPreview("larger than max_video_bytes"),
        Err(e) => return Outcome::Failed(e.to_string()),
    }

    let probed = match probe(config, &original).await {
        Ok(Some(probed)) => probed,
        Ok(None) => return Outcome::NoPreview("not a video ffprobe reads"),
        Err(e) => return Outcome::Failed(e),
    };
    let Some(picture) = probed.picture() else {
        return Outcome::NoPreview("no moving picture");
    };
    if picture
        .color_transfer
        .as_deref()
        .is_some_and(|transfer| HDR_TRANSFERS.contains(&transfer))
    {
        return Outcome::NoPreview("HDR");
    }
    let at = probed
        .duration()
        .map_or(0.0, |seconds| POSTER_AT.min(seconds / 2.0));
    let frame = match poster_frame(config, &original, at).await {
        Ok(frame) => frame,
        Err(outcome) => return outcome,
    };
    drop(dir);

    let max_pixels = config.max_picture_pixels;
    match tokio::task::spawn_blocking(move || aspen_previews::picture::preview(&frame, max_pixels))
        .await
    {
        Ok(Ok(poster)) => Outcome::Made(Made {
            bytes: poster.webp,
            mime_type: "image/webp",
            width: poster.width,
            height: poster.height,
        }),
        Ok(Err(reason)) => Outcome::NoPreview(reason),
        Err(e) => Outcome::Failed(format!("the poster's maker stopped: {e}")),
    }
}
