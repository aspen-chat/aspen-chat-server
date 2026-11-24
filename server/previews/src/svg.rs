//! Previews of SVGs: the SVG drawn at its own size, fitted within [`crate::BOX`], by
//! `aspen-svg-render` (`svg_renderer`) in a process of its own, and encoded as a picture's
//! preview is.
//!
//! An SVG is served to be saved, never shown in place (`app::attachment::INLINE_TYPES`), since
//! a browser opening one runs its scripts; its preview is all apps show of it. Drawing one can
//! ask for any amount of memory and time, so the renderer runs under limits set as it starts:
//! `svg_memory_mib` of address space, [`CPU_TIME`], no file written, and `MAX_OPEN_FILES`,
//! and is killed past [`DEADLINE`]; and it draws behind a seccomp filter of its own, which lets
//! it open no file or connection once its fonts (`svg_fonts`) are mapped
//! (`aspen_svg_render::sandbox`). Only its pixels are read back, at most the box's worth.

use crate::process::{Limits, confined, limit};
use crate::{Made, Outcome, PreviewConfig};
use aspen_svg_render::protocol::{self, Exit};
use std::process::{ExitStatus, Stdio};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;

/// The CPU time the renderer may take over one SVG. A detailed map draws in well under a
/// second; an SVG still drawing past this asks for more than any preview is worth.
pub const CPU_TIME: Duration = Duration::from_secs(10);

/// How long the renderer may run over one SVG, its startup and the pipes included.
pub const DEADLINE: Duration = Duration::from_secs(20);

/// The most files the renderer may hold open: its pipes, and one font at a time as it maps
/// them.
const MAX_OPEN_FILES: u64 = 16;

/// The most of the renderer's complaints kept for the log.
const STDERR_BYTES: u64 = 16 * 1024;

/// The renderer, run with `config`'s fonts and limits.
fn renderer(config: &PreviewConfig) -> Command {
    let mut command = confined(&config.svg_renderer);
    limit(
        &mut command,
        Limits {
            memory: config.svg_memory_mib.saturating_mul(1024 * 1024),
            cpu: CPU_TIME,
            open_files: MAX_OPEN_FILES,
        },
    );
    command
        .arg(protocol::FONTS)
        .arg(&config.svg_fonts)
        .args([protocol::BOX, &protocol::box_arg(crate::BOX)])
        .args([protocol::MAX_PIXELS, &config.max_picture_pixels.to_string()])
        .args([protocol::MAX_BYTES, &config.max_svg_bytes.to_string()])
        .kill_on_drop(true);
    command
}

/// Whether this server can draw SVGs: the renderer runs, its fonts open, it enters its sandbox,
/// and it draws text there. When it cannot, why.
pub async fn available(config: &PreviewConfig) -> Result<(), String> {
    if config.svg_renderer.is_empty() {
        return Err("svg_renderer is empty".into());
    }
    let checked = renderer(config)
        .arg(protocol::CHECK)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output();
    let output = tokio::time::timeout(DEADLINE, checked)
        .await
        .map_err(|_| format!("its check took longer than {DEADLINE:?}"))?
        .map_err(|e| format!("it cannot be run: {e}"))?;
    if output.status.code().and_then(Exit::of) == Some(Exit::Drawn) {
        Ok(())
    } else {
        Err(format!(
            "its check ended {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Makes the preview of the SVG in `svg`, of at most `max_svg_bytes`.
pub async fn preview(config: &PreviewConfig, svg: Vec<u8>) -> Outcome {
    if svg.len() as u64 > config.max_svg_bytes {
        return Outcome::NoPreview("larger than max_svg_bytes");
    }
    let mut child = match renderer(config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => return Outcome::Failed(format!("could not run the SVG renderer: {e}")),
    };
    let (Some(mut stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Outcome::Failed("the SVG renderer's pipes were not made".into());
    };
    // Written, and its complaints read, beside its pixels, so no pipe fills while another is
    // read. A renderer that stops reading early (it refused the SVG) closes the pipe, which
    // ends the writing; what it says of that is in its exit.
    let writing = tokio::spawn(async move {
        let _ = stdin.write_all(&svg).await;
    });
    let complaints = tokio::spawn(async move {
        let mut said = Vec::new();
        let _ = stderr.take(STDERR_BYTES).read_to_end(&mut said).await;
        said
    });
    let (max_width, max_height) = crate::BOX;
    let most = protocol::HEADER_BYTES as u64 + u64::from(max_width) * u64::from(max_height) * 4;
    let finished = async {
        let mut drawn = Vec::new();
        stdout.take(most + 1).read_to_end(&mut drawn).await?;
        // Past what any SVG draws it is killed rather than waited for.
        if drawn.len() as u64 > most {
            return Ok((None, drawn));
        }
        Ok::<_, std::io::Error>((Some(child.wait().await?), drawn))
    };
    // Dropping the child, past the deadline or the most it may write, kills it.
    let ended = tokio::time::timeout(DEADLINE, finished).await;
    writing.abort();
    let (status, drawn) = match ended {
        Err(_) => return Outcome::NoPreview("the SVG renderer took longer than DEADLINE"),
        Ok(Err(e)) => {
            return Outcome::Failed(format!("could not read the SVG renderer's pixels: {e}"));
        }
        Ok(Ok((None, _))) => {
            return Outcome::NoPreview("the SVG renderer wrote more than the box holds");
        }
        Ok(Ok((Some(status), drawn))) => (status, drawn),
    };
    let said = || async {
        String::from_utf8_lossy(&complaints.await.unwrap_or_default())
            .trim()
            .to_owned()
    };
    match status.code().and_then(Exit::of) {
        Some(Exit::Drawn) => {}
        Some(Exit::Unreadable) => return Outcome::NoPreview("not an SVG the renderer reads"),
        Some(Exit::Refused) => {
            let why = said().await;
            tracing::debug!(why, "the SVG renderer refused an SVG");
            return Outcome::NoPreview("an SVG asking for more than the renderer's limits");
        }
        Some(Exit::Unusable) => {
            return Outcome::Failed(format!("the SVG renderer cannot draw: {}", said().await));
        }
        None => return ended_otherwise(status, said().await),
    }
    let Some((width, height, rgba)) = pixels(drawn) else {
        return Outcome::Failed("the SVG renderer wrote pixels of another size".into());
    };
    match tokio::task::spawn_blocking(move || crate::picture::from_rgba(width, height, rgba)).await
    {
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

/// The pixels the renderer wrote, when they are as many as their header says and fit the box.
fn pixels(mut drawn: Vec<u8>) -> Option<(u32, u32, Vec<u8>)> {
    let header = drawn.get(..protocol::HEADER_BYTES)?;
    let width = u32::from_le_bytes(header[..4].try_into().ok()?);
    let height = u32::from_le_bytes(header[4..].try_into().ok()?);
    let (max_width, max_height) = crate::BOX;
    let fits = (1..=max_width).contains(&width) && (1..=max_height).contains(&height);
    let expected = protocol::HEADER_BYTES + width as usize * height as usize * 4;
    if !fits || drawn.len() != expected {
        return None;
    }
    drawn.drain(..protocol::HEADER_BYTES);
    Some((width, height, drawn))
}

/// What a renderer that exited with no code of the protocol's means: killed by a signal (one of
/// its limits, or its sandbox), or an exit no version of it gives.
#[cfg(unix)]
fn ended_otherwise(status: ExitStatus, said: String) -> Outcome {
    use std::os::unix::process::ExitStatusExt;
    match status.signal() {
        Some(libc::SIGXCPU | libc::SIGKILL) => {
            Outcome::NoPreview("the SVG renderer took longer than its CPU time")
        }
        Some(libc::SIGSYS) => {
            // Its sandbox stops a system call drawing needs: the renderer's fault, not the SVG's.
            tracing::warn!(said, "the SVG renderer was stopped by its sandbox");
            Outcome::NoPreview("the SVG renderer was stopped by its sandbox")
        }
        Some(_) => {
            tracing::debug!(%status, said, "the SVG renderer stopped");
            Outcome::NoPreview("the SVG renderer ran out of memory or stopped")
        }
        None => Outcome::Failed(format!("the SVG renderer ended {status}: {said}")),
    }
}

#[cfg(not(unix))]
fn ended_otherwise(status: ExitStatus, said: String) -> Outcome {
    Outcome::Failed(format!("the SVG renderer ended {status}: {said}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(width: u32, height: u32) -> Vec<u8> {
        [width.to_le_bytes(), height.to_le_bytes()].concat()
    }

    #[test]
    fn only_pixels_as_many_as_their_header_says_and_within_the_box_are_taken() {
        let mut drawn = header(2, 3);
        drawn.extend([7; 24]);
        assert_eq!(pixels(drawn), Some((2, 3, vec![7; 24])));

        let mut short = header(2, 3);
        short.extend([7; 23]);
        assert_eq!(pixels(short), None);
        let mut long = header(2, 3);
        long.extend([7; 25]);
        assert_eq!(pixels(long), None);

        assert_eq!(pixels(header(0, 1)), None);
        let (max_width, max_height) = crate::BOX;
        assert_eq!(pixels(header(max_width + 1, 1)), None);
        assert_eq!(pixels(header(1, max_height + 1)), None);
        assert_eq!(pixels(vec![1, 2, 3]), None);
    }
}
