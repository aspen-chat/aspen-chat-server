//! Previews of videos: a poster, one frame of the video made into a picture's preview, which
//! apps show inline with a play button, playing the original only when asked (see `super`).
//!
//! The frame is taken by the operator's `ffmpeg` in a process of its own, so a video that crashes
//! its decoder takes nothing else with it; and it may read only the file it is given
//! ([`PROTOCOLS`]), as one of the containers videos are sent in ([`CONTAINERS`]), so a file made
//! to look like a playlist cannot have it fetch addresses or read other files. It decodes only
//! the codecs phones, cameras, and screen recorders write ([`DECODERS`]), pictures of at most
//! `max_picture_pixels`, on one thread, and runs with nothing of the server's environment
//! (its configuration may be there) and under limits set as it starts ([`limit`]): its memory
//! (`ffmpeg_memory_mib`), its CPU time ([`TIMEOUT`]), no file written, and few open. What it
//! writes is read only up to `max_picture_bytes`. Decoding one frame takes about a second,
//! however long the video.

use super::{Made, Outcome};
use crate::app::context::GlobalServerContext;
use crate::aspen_config::PreviewConfig;
use serde::Deserialize;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

/// The only protocol `ffmpeg` and `ffprobe` may read with.
const PROTOCOLS: &str = "file";

/// The demuxers `ffmpeg` and `ffprobe` may read with: the containers phones, cameras, and
/// screen recorders write, and nothing that names other files or addresses.
const CONTAINERS: &str = "mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,avi,mpegts,ogg,flv,asf";

/// The decoders `ffmpeg` and `ffprobe` may use: the video codecs phones, cameras, and screen
/// recorders write, and the sound and subtitle codecs that come with them, which reading a file's
/// streams opens too (a file with any other stream gets no poster).
const DECODERS: &str = "h264,hevc,vp8,vp9,av1,libdav1d,mpeg4,mpeg2video,mpeg1video,mjpeg,\
    prores,theora,h263,flv,vc1,wmv3,png,aac,aac_fixed,mp3float,mp3,mp2float,mp2,opus,libopus,\
    vorbis,libvorbis,flac,alac,ac3,eac3,pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le,\
    pcm_f32le,pcm_mulaw,pcm_alaw,amrnb,amrwb,wmav2,mov_text";

/// The most files `ffmpeg` or `ffprobe` may hold open: the video, its pipes, and its own.
const MAX_OPEN_FILES: u64 = 64;

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
    confined(program)
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

/// A command for `program` with nothing of this server's environment but where to find
/// programs, which a `program` named without a path needs.
fn confined(program: &str) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    command
}

/// [`confined`], also under the limits [`limit`] sets, and taking only what [`DECODERS`]
/// decode, of at most `max_picture_pixels`, on one thread.
fn decoding(program: &str, config: &PreviewConfig) -> Command {
    let mut command = confined(program);
    limit(
        &mut command,
        config.ffmpeg_memory_mib.saturating_mul(1024 * 1024),
    );
    command
        .args(["-codec_whitelist", DECODERS])
        .args([
            "-max_pixels",
            &config.max_picture_pixels.min(i32::MAX as u64).to_string(),
        ])
        .args(["-threads", "1"]);
    command
}

/// Sets resource limits on `command`'s process as it starts: `memory` bytes of address space,
/// [`TIMEOUT`] of CPU time (it is killed past it), no file written (its output is a pipe), no
/// core dump, and [`MAX_OPEN_FILES`] open files.
#[cfg(unix)]
fn limit(command: &mut Command, memory: u64) {
    let cpu = TIMEOUT.as_secs();
    let limits = [
        (libc::RLIMIT_AS, memory, memory),
        (libc::RLIMIT_CPU, cpu, cpu + 1),
        (libc::RLIMIT_FSIZE, 0, 0),
        (libc::RLIMIT_CORE, 0, 0),
        (libc::RLIMIT_NOFILE, MAX_OPEN_FILES, MAX_OPEN_FILES),
    ];
    // SAFETY: the closure runs in the child between fork and exec, where only async-signal-safe
    // calls are allowed; it calls only setrlimit, which is, on values made before the fork.
    unsafe {
        command.pre_exec(move || {
            for (resource, soft, hard) in limits {
                let limit = libc::rlimit {
                    rlim_cur: soft as libc::rlim_t,
                    rlim_max: hard as libc::rlim_t,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn limit(_command: &mut Command, _memory: u64) {}

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

/// The frame `at` seconds into the video at `path`, upright, as a PNG of at most
/// `max_picture_bytes`.
async fn poster_frame(config: &PreviewConfig, path: &Path, at: f64) -> Result<Vec<u8>, Outcome> {
    let at = format!("{at:.3}");
    let mut child = decoding(&config.ffmpeg, config)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error"])
        .args([
            "-protocol_whitelist",
            PROTOCOLS,
            "-format_whitelist",
            CONTAINERS,
        ])
        .args(["-ss", &at, "-i"])
        .arg(path)
        // The first moving picture (never cover art), turned upright as it is decoded, encoded
        // on one thread.
        .args([
            "-map",
            "0:V:0",
            "-frames:v",
            "1",
            "-threads",
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
        .spawn()
        .map_err(|e| Outcome::Failed(format!("could not run ffmpeg: {e}")))?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        return Err(Outcome::Failed("ffmpeg's output was not piped".to_string()));
    };
    let limit = config.max_picture_bytes;
    // Its complaints are read beside its frame, so neither pipe fills while the other is read.
    let complaints = tokio::spawn(async move {
        let mut errors = Vec::new();
        let _ = stderr.take(STDERR_BYTES).read_to_end(&mut errors).await;
        errors
    });
    let finished = async {
        let mut frame = Vec::new();
        stdout
            .take(limit.saturating_add(1))
            .read_to_end(&mut frame)
            .await?;
        // Past the limit it is killed rather than waited for.
        if frame.len() as u64 > limit {
            return Ok((None, frame));
        }
        Ok::<_, std::io::Error>((Some(child.wait().await?), frame))
    };
    // Dropping the child, on timeout or past the limit, kills it.
    match tokio::time::timeout(TIMEOUT, finished).await {
        Err(_) => Err(Outcome::NoPreview("ffmpeg took longer than a minute")),
        Ok(Err(e)) => Err(Outcome::Failed(format!(
            "could not read ffmpeg's frame: {e}"
        ))),
        Ok(Ok((None, _))) => Err(Outcome::NoPreview(
            "ffmpeg's frame was larger than max_picture_bytes",
        )),
        Ok(Ok((Some(status), frame))) if !status.success() || frame.is_empty() => {
            let errors = complaints.await.unwrap_or_default();
            tracing::warn!(
                stderr = %String::from_utf8_lossy(&errors),
                "ffmpeg could not take a video's poster"
            );
            Err(Outcome::NoPreview("ffmpeg could not take a frame"))
        }
        Ok(Ok((Some(_), frame))) => Ok(frame),
    }
}

/// The most of `ffmpeg`'s complaints kept for the log.
const STDERR_BYTES: u64 = 16 * 1024;

/// The most `ffprobe` may say of one file.
const PROBE_BYTES: u64 = 1024 * 1024;

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
    let mut child = decoding(&config.ffprobe, config)
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
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("could not run ffprobe: {e}"))?;
    let Some(stdout) = child.stdout.take() else {
        return Err("ffprobe's output was not piped".to_string());
    };
    let finished = async {
        let mut said = Vec::new();
        stdout
            .take(PROBE_BYTES.saturating_add(1))
            .read_to_end(&mut said)
            .await?;
        if said.len() as u64 > PROBE_BYTES {
            return Ok(None);
        }
        Ok::<_, std::io::Error>(Some((child.wait().await?, said)))
    };
    // Dropping the child, on timeout or past the limit, kills it.
    let finished = tokio::time::timeout(TIMEOUT, finished)
        .await
        .map_err(|_| "ffprobe took longer than a minute".to_string())?
        .map_err(|e| format!("could not read ffprobe's answer: {e}"))?;
    match finished {
        Some((status, said)) if status.success() => Ok(serde_json::from_slice(&said).ok()),
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_poster_is_taken_within_the_limits() {
        let config = PreviewConfig::default();
        if !available(&config).await {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("video.mp4");
        let made = confined(&config.ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=320x240:rate=10",
            ])
            .args(["-t", "2", "-c:v", "mpeg4"])
            .arg(&path)
            .status()
            .await
            .unwrap();
        // A build of ffmpeg without its test sources cannot make the video.
        if !made.success() {
            return;
        }
        let probed = probe(&config, &path).await.unwrap().unwrap();
        assert!(probed.picture().is_some());
        let Ok(frame) = poster_frame(&config, &path, 0.5).await else {
            panic!("no poster");
        };
        assert!(frame.starts_with(b"\x89PNG"));
        let too_large = PreviewConfig {
            max_picture_bytes: 100,
            ..config.clone()
        };
        assert!(matches!(
            poster_frame(&too_large, &path, 0.5).await,
            Err(Outcome::NoPreview(_))
        ));
        let too_many_pixels = PreviewConfig {
            max_picture_pixels: 1000,
            ..config
        };
        assert!(poster_frame(&too_many_pixels, &path, 0.5).await.is_err());
    }

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
