//! Previews of SVGs, drawn by `aspen_previews::svg` (see `super`).

use super::Outcome;
use crate::aspen_config::PreviewConfig;
use crate::context::GlobalServerContext;

/// Reads an SVG from storage and draws its preview.
pub async fn make(
    state: &GlobalServerContext,
    config: &PreviewConfig,
    key: &str,
    original_bytes: u64,
) -> Outcome {
    if original_bytes > config.max_svg_bytes {
        return Outcome::NoPreview("larger than max_svg_bytes");
    }
    let mut bytes = Vec::with_capacity(usize::try_from(original_bytes).unwrap_or_default());
    match state
        .media_store
        .copy_object_to(key, config.max_svg_bytes, &mut bytes)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return Outcome::NoPreview("larger than max_svg_bytes"),
        Err(e) => return Outcome::Failed(e.to_string()),
    }
    aspen_previews::svg::preview(config, bytes).await
}
