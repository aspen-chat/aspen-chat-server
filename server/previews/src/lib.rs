//! Previews of attachments as `app::attachment::preview` makes them: a picture's decoded,
//! brought into sRGB, fitted within [`BOX`], and encoded as WebP ([`picture`]), and a video's
//! poster, one frame taken by the operator's `ffmpeg` ([`video`]). What is read from storage,
//! queued, and kept is the server's; this is the work done on the bytes.

use serde::Deserialize;
use smart_default::SmartDefault;

pub mod picture;
pub mod video;

/// The box a preview is fitted within, in pixels: width, then height.
pub const BOX: (u32, u32) = (1920, 960);

/// Fits `size` within `bounds`, keeping its aspect ratio and never enlarging it; neither side
/// is less than a pixel.
pub fn fit(size: (u32, u32), bounds: (u32, u32)) -> (u32, u32) {
    let (width, height) = size;
    let (max_width, max_height) = bounds;
    if width <= max_width && height <= max_height {
        return size;
    }
    // The tighter of the two ratios, compared without division: max_width / width against
    // max_height / height.
    let (scaled, other, limit) =
        if u64::from(max_width) * u64::from(height) <= u64::from(max_height) * u64::from(width) {
            (max_width, height, width)
        } else {
            (max_height, width, height)
        };
    let other = ((u64::from(other) * u64::from(scaled) + u64::from(limit) / 2) / u64::from(limit))
        .max(1) as u32;
    if scaled == max_width {
        (scaled, other)
    } else {
        (other, scaled)
    }
}

/// What a maker made of an attachment.
pub struct Made {
    pub bytes: Vec<u8>,
    pub mime_type: &'static str,
    pub width: u32,
    pub height: u32,
}

/// What became of making one attachment's preview.
pub enum Outcome {
    /// A preview, not yet weighed against the original.
    Made(Made),
    /// None is to be made: the original is not something a preview is made of, or is beyond
    /// what this deployment makes previews of. The reason is for the log.
    NoPreview(&'static str),
    /// It could not be made now, and is tried again later.
    Failed(String),
}

/// Making what readers' apps show inline in place of pictures and videos
/// (`app::attachment::preview`): smaller copies of pictures, and videos' posters.
///
/// Every server queues the work as jobs; those with `make` and `[jobs] run` on do it,
/// `concurrency` at a time. Pictures are made in the server's own process; videos' posters by
/// running `ffmpeg` and `ffprobe`, so a server without them (or with `ffmpeg` naming nothing)
/// makes previews of pictures only.
#[derive(Clone, Debug, Deserialize, SmartDefault)]
#[serde(default)]
pub struct PreviewConfig {
    /// Whether this server makes previews. Off, it only queues them for the servers that do.
    #[default = true]
    pub make: bool,
    /// How many previews this server makes at once. Each picture may hold several hundred
    /// megabytes while it is made, and each video is copied whole to a scratch file (under
    /// `TMPDIR`) to take its poster.
    #[default = 2]
    pub concurrency: usize,
    /// The largest original, in bytes, a picture's preview is made from (64 MiB).
    #[default = 67_108_864]
    pub max_picture_bytes: u64,
    /// The most pixels a picture may have for a preview to be made from it (100 megapixels).
    #[default = 100_000_000]
    pub max_picture_pixels: u64,
    /// The `ffmpeg` and `ffprobe` to run, by path or by name on `PATH`; `ffmpeg` left empty
    /// makes no previews of videos.
    #[default = "ffmpeg"]
    pub ffmpeg: String,
    #[default = "ffprobe"]
    pub ffprobe: String,
    /// The largest video, in bytes, a poster is taken of (2 GiB).
    #[default = 2_147_483_648]
    pub max_video_bytes: u64,
    /// The most memory, in MiB, `ffmpeg` or `ffprobe` may map while taking one poster (its
    /// address space, `RLIMIT_AS`); 3 GiB, room for a frame of 8K video.
    #[default = 3072]
    pub ffmpeg_memory_mib: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_preview_fits_its_box_and_keeps_its_shape() {
        assert_eq!(fit((800, 600), BOX), (800, 600));
        assert_eq!(fit((1920, 960), BOX), (1920, 960));
        assert_eq!(fit((4032, 3024), BOX), (1280, 960));
        assert_eq!(fit((3024, 4032), BOX), (720, 960));
        assert_eq!(fit((8000, 1000), BOX), (1920, 240));
        assert_eq!(fit((100_000, 1), BOX), (1920, 1));
        assert_eq!(fit((1, 100_000), BOX), (1, 960));
    }
}
