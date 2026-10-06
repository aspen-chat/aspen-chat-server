//! Previews of videos: a poster, one frame of the video made into a picture's preview, which
//! apps show inline with a play button, playing the original only when asked (see `super`).
//!
//! The frame is taken by the operator's `ffmpeg` in a process of its own, so a video that crashes
//! its decoder, or takes more memory than it should, takes nothing else with it; and it may read
//! only the file it is given ([`PROTOCOLS`]), as one of the containers videos are sent in
//! ([`CONTAINERS`]), so a file made to look like a playlist cannot have it fetch addresses or
//! read other files. Decoding one frame takes about a second, however long the video.

use super::{Made, Outcome};
use crate::app::context::GlobalServerContext;
use crate::aspen_config::PreviewConfig;
use serde::Deserialize;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

/// The only protocol `ffmpeg` and `ffprobe` may read with.
const PROTOCOLS: &str = "file";

/// The demuxers `ffmpeg` and `ffprobe` may read with: the containers phones, cameras, and
/// screen recorders write, and nothing that names other files or addresses.
const CONTAINERS: &str = "mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,avi,mpegts,ogg,flv,asf";

/// How far into a video its poster is taken, in seconds, past the black or faded first frames
/// many begin with; a shorter video's is taken from its middle.
const POSTER_AT: f64 = 1.0;

/// How long `ffprobe` or `ffmpeg` may take over one video.
const TIMEOUT: Duration = Duration::from_secs(60);

/// The transfer characteristics of HDR video, whose frame would look washed out in SDR without
/// tone mapping, which not every `ffmpeg` build has.
const HDR_TRANSFERS: [&str; 2] = ["smpte2084", "arib-std-b67"];

/// Whether this server can make posters: `ffmpeg` and `ffprobe` run.
pub async fn available(config: &PreviewConfig) -> bool {
    if config.ffmpeg.is_empty() || config.ffprobe.is_empty() {
        return false;
    }
    runs(&config.ffmpeg).await && runs(&config.ffprobe).await
}

async fn runs(program: &str) -> bool {
    Command::new(program)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

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
        crate::app::Result::Ok(copied)
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
    match tokio::task::spawn_blocking(move || super::picture::preview(&frame, max_pixels)).await {
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

/// The frame `at` seconds into the video at `path`, upright, as a PNG.
async fn poster_frame(config: &PreviewConfig, path: &Path, at: f64) -> Result<Vec<u8>, Outcome> {
    let at = format!("{at:.3}");
    let output = Command::new(&config.ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error"])
        .args([
            "-protocol_whitelist",
            PROTOCOLS,
            "-format_whitelist",
            CONTAINERS,
        ])
        .args(["-ss", &at, "-i"])
        .arg(path)
        // The first moving picture (never cover art), turned upright as it is decoded.
        .args([
            "-map",
            "0:V:0",
            "-frames:v",
            "1",
            "-c:v",
            "png",
            "-f",
            "image2pipe",
        ])
        .arg("pipe:1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    // Dropping the child on timeout kills it.
    match tokio::time::timeout(TIMEOUT, output).await {
        Err(_) => Err(Outcome::NoPreview("ffmpeg took longer than a minute")),
        Ok(Err(e)) => Err(Outcome::Failed(format!("could not run ffmpeg: {e}"))),
        Ok(Ok(finished)) if !finished.status.success() || finished.stdout.is_empty() => {
            tracing::warn!(
                stderr = %String::from_utf8_lossy(&finished.stderr),
                "ffmpeg could not take a video's poster"
            );
            Err(Outcome::NoPreview("ffmpeg could not take a frame"))
        }
        Ok(Ok(finished)) => Ok(finished.stdout),
    }
}

/// What `ffprobe` says of a file.
#[derive(Debug, Deserialize)]
struct Probe {
    #[serde(default)]
    streams: Vec<VideoStream>,
    format: Option<Format>,
}

#[derive(Debug, Deserialize)]
struct VideoStream {
    codec_type: Option<String>,
    color_transfer: Option<String>,
    #[serde(default)]
    disposition: Disposition,
}

#[derive(Debug, Default, Deserialize)]
struct Disposition {
    #[serde(default)]
    attached_pic: u8,
}

#[derive(Debug, Deserialize)]
struct Format {
    duration: Option<String>,
}

impl Probe {
    /// The first moving picture: a video stream that is not cover art.
    fn picture(&self) -> Option<&VideoStream> {
        self.streams.iter().find(|stream| {
            stream.codec_type.as_deref() == Some("video") && stream.disposition.attached_pic == 0
        })
    }

    fn duration(&self) -> Option<f64> {
        self.format.as_ref()?.duration.as_deref()?.parse().ok()
    }
}

/// Asks `ffprobe` about `path`: `None` when it is not something `ffprobe` reads, an error when
/// `ffprobe` cannot be run.
async fn probe(config: &PreviewConfig, path: &Path) -> Result<Option<Probe>, String> {
    let output = Command::new(&config.ffprobe)
        .args([
            "-v",
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .args([
            "-protocol_whitelist",
            PROTOCOLS,
            "-format_whitelist",
            CONTAINERS,
        ])
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(TIMEOUT, output)
        .await
        .map_err(|_| "ffprobe took longer than a minute".to_string())?
        .map_err(|e| format!("could not run ffprobe: {e}"))?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(serde_json::from_slice(&output.stdout).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cover_art_is_not_the_picture() {
        let probe: Probe = serde_json::from_str(
            r#"{"streams":[
                {"codec_type":"audio"},
                {"codec_type":"video","disposition":{"attached_pic":1}},
                {"codec_type":"video","color_transfer":"bt709"}
            ],"format":{"duration":"12.5"}}"#,
        )
        .unwrap();
        assert_eq!(
            probe.picture().unwrap().color_transfer.as_deref(),
            Some("bt709")
        );
        assert_eq!(probe.duration(), Some(12.5));

        let sound: Probe = serde_json::from_str(
            r#"{"streams":[{"codec_type":"audio"},
                {"codec_type":"video","disposition":{"attached_pic":1}}]}"#,
        )
        .unwrap();
        assert!(sound.picture().is_none());
    }
}
