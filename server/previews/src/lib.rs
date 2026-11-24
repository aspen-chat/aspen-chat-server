//! Previews of attachments as `app::attachment::preview` makes them: a picture's decoded,
//! brought into sRGB, fitted within [`BOX`], and encoded as WebP ([`picture`]), an SVG drawn by
//! `aspen-svg-render` and encoded the same way ([`svg`]), and a video's poster, one frame taken
//! by the operator's `ffmpeg` ([`video`]). Icons are stored fitted within a box of their own the
//! same way (`app::icon::as_stored`). What is read from storage, queued, and kept is the
//! server's; this is the work done on the bytes.

use serde::Deserialize;
use smart_default::SmartDefault;

pub mod picture;
mod process;
pub mod svg;
pub mod video;

pub use aspen_svg_render::fit;

/// The box a preview is fitted within, in pixels: width, then height.
pub const BOX: (u32, u32) = (1920, 960);

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
/// `concurrency` at a time. Pictures are made in the server's own process; SVGs by running
/// `svg_renderer`, and videos' posters by running `ffmpeg` and `ffprobe`, so a server without
/// them (or with either naming nothing) makes previews of pictures only, and of whichever of
/// the two it has.
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
    /// The program SVGs are drawn by (`aspen-svg-render`), by path or by name on `PATH`; left
    /// empty, no previews of SVGs are made, and apps offer them as files to save.
    #[default = "aspen-svg-render"]
    pub svg_renderer: String,
    /// The directory of fonts SVGs' text is drawn with, which `scripts/svg_fonts.py` builds.
    #[default = "/usr/share/aspen/svg-fonts"]
    pub svg_fonts: String,
    /// The largest SVG, in bytes, a preview is drawn of, also once a compressed one (SVGZ) is
    /// unpacked (8 MiB).
    #[default = 8_388_608]
    pub max_svg_bytes: u64,
    /// The most memory, in MiB, the SVG renderer may map while drawing one (its address space,
    /// `RLIMIT_AS`), its fonts (about 60 MiB) among it; 1 GiB.
    #[default = 1024]
    pub svg_memory_mib: u64,
}
