//! Game and window capture for the Aspen desktop shell.
//!
//! On Windows the capture is libobs's (`obs`): its game hook reaches games that ordinary
//! display capture cannot, and the picture and the application's sound are encoded and sent
//! as SRTP straight to the voice server's plain RTP transport. The shell's main
//! process spawns this helper and tells it where to send; the media never passes through the
//! shell or the browser.
//!
//! On Linux and macOS the picture comes from the browser's own screen share instead (on Linux
//! only the process owning the requesting window can raise the desktop portal that picks what
//! to show; on macOS the system picker does the same, and libobs's window capture is the same
//! framework the browser uses), and the helper captures only the sound (`startAudio`): one
//! application's streams from the PipeWire graph (`pipewire_audio`) or through
//! ScreenCaptureKit (`sck_audio`), encoded as Opus with libopus (`app_audio`). Those builds
//! link no libobs; the `libobs` feature adds the picture path for developing it.
//!
//! It is a process of its own rather than a Node addon because x264's aligned allocations
//! trip the allocator every Electron process runs on, and because a crash in native capture
//! code should cost the capture, not the app.
//!
//! The protocol: stdin carries one JSON request per line (`Request`), stdout one JSON reply
//! per line (`Reply`). One capture runs at a time.

#![allow(clippy::missing_safety_doc)]

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod app_audio;
#[cfg(obs)]
mod obs;
#[cfg(target_os = "linux")]
mod pipewire_audio;
mod rtp;
#[cfg(target_os = "macos")]
mod sck_audio;

use rtp::RtpTarget;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};

type Result<T> = std::result::Result<T, String>;

/// One thing a capture source can be pointed at, as the source's own window list names it.
#[derive(Serialize)]
pub struct CaptureTarget {
    pub name: String,
    /// The value the source's window setting takes to capture it.
    pub value: String,
}

/// The capture source this platform uses for games, and what it can be pointed at.
#[derive(Serialize)]
pub struct CaptureKind {
    /// The libobs source id, such as `game_capture`.
    pub kind: String,
    /// The setting that names the window, when `targets` is how one is chosen.
    pub property: String,
    pub targets: Vec<CaptureTarget>,
    /// The audio source that captures the window's application, when this platform has one.
    pub audio: Option<AudioKind>,
}

/// An audio source and how it is pointed at an application: through `property`, which takes
/// the same value as the video source's window setting, when `targets` is absent; otherwise
/// by one of `targets`, chosen on its own, because the platform names applications playing
/// sound in a vocabulary of its own (Linux, where a PipeWire node is not a window).
#[derive(Serialize)]
pub struct AudioKind {
    pub kind: String,
    pub property: String,
    pub targets: Option<Vec<AudioTarget>>,
}

/// An application playing sound: its name (empty when it gave none), its process id, and the
/// settings that make the audio source capture it.
#[derive(Serialize, Debug, PartialEq)]
pub struct AudioTarget {
    pub name: String,
    pub pid: Option<u32>,
    pub settings: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartOptions {
    /// The libobs source id to create.
    pub kind: String,
    /// The source's settings as a JSON object, in the source's own vocabulary.
    #[serde(default)]
    pub settings: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub fps: Option<u32>,
    #[serde(default)]
    pub bitrate_kbps: Option<u32>,
    /// Where OBS's plugins are, when not the build-time default.
    #[serde(default)]
    pub plugin_dir: Option<String>,
    /// Where OBS's data (`libobs/` effects, `obs-plugins/`) is, when not the build-time default.
    #[serde(default)]
    pub data_dir: Option<String>,
    /// Where to send the video SRTP.
    pub rtp: RtpTarget,
    /// The application's audio: the source that captures it, when this platform has one for
    /// the target, and where to send it as Opus. Both or neither.
    #[serde(default)]
    pub audio: Option<AudioOptions>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioOptions {
    /// The audio source id: a libobs source such as `wasapi_process_output_capture`, or on
    /// Linux and macOS `APPLICATION_AUDIO`.
    pub kind: String,
    /// Its settings as a JSON object.
    #[serde(default)]
    pub settings: Option<String>,
    pub rtp: RtpTarget,
    #[serde(default)]
    pub bitrate_kbps: Option<u32>,
}

/// A capture of sound alone, for where the picture comes from elsewhere: on Linux the browser's
/// own screen share, since only the process owning the requesting window can raise the desktop
/// portal that picks what to show.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioStart {
    pub audio: AudioOptions,
    #[serde(default)]
    pub plugin_dir: Option<String>,
    #[serde(default)]
    pub data_dir: Option<String>,
}

/// The audio source that captures one application chosen on its own, rather than the
/// application behind a captured window: the PipeWire capture on Linux and the
/// ScreenCaptureKit one on macOS, named as the catalogue and `startAudio` name it.
#[cfg(target_os = "linux")]
const APPLICATION_AUDIO: Option<&str> = Some("aspen_pipewire_app_audio");
#[cfg(target_os = "macos")]
const APPLICATION_AUDIO: Option<&str> = Some("aspen_sck_app_audio");
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const APPLICATION_AUDIO: Option<&str> = None;

/// The applications whose sound can be captured right now (on Linux those playing sound, on
/// macOS those with a window); `None` when they cannot be listed (no PipeWire socket, no
/// permission), which leaves the platform without application audio.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn playing_applications() -> Option<Vec<AudioTarget>> {
    #[cfg(target_os = "linux")]
    let listed = pipewire_audio::playing_applications();
    #[cfg(target_os = "macos")]
    let listed = sck_audio::running_applications();
    match listed {
        Ok(targets) => Some(targets),
        Err(error) => {
            eprintln!("aspen-obs-capture: application audio unavailable: {error}");
            None
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn playing_applications() -> Option<Vec<AudioTarget>> {
    None
}

/// What this platform can capture right now: the game capture kinds with the windows each can
/// be pointed at, and the applications whose sound can be captured on its own.
struct Catalogue {
    kinds: Vec<CaptureKind>,
    application_audio: Option<AudioKind>,
}

fn catalogue(plugin_dir: Option<&String>, data_dir: Option<&String>) -> Result<Catalogue> {
    #[cfg(obs)]
    let kinds = obs::capture_kinds(plugin_dir, data_dir)?;
    #[cfg(not(obs))]
    let kinds = {
        let _ = (plugin_dir, data_dir);
        Vec::new()
    };
    let application_audio = APPLICATION_AUDIO.and_then(|kind| {
        playing_applications().map(|targets| AudioKind {
            kind: kind.to_string(),
            property: String::new(),
            targets: Some(targets),
        })
    });
    Ok(Catalogue {
        kinds,
        application_audio,
    })
}

/// Starts capturing a picture, with the application's sound when asked.
fn start_capture(options: StartOptions) -> Result<()> {
    #[cfg(obs)]
    {
        obs::start_capture(options)
    }
    #[cfg(not(obs))]
    {
        let _ = options;
        Err("this build captures only sound; the picture is the browser's screen share".into())
    }
}

/// Starts capturing one application's sound alone.
fn start_audio_capture(options: AudioStart) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        if Some(options.audio.kind.as_str()) != APPLICATION_AUDIO {
            return Err(format!(
                "this platform has no audio source of kind {}",
                options.audio.kind
            ));
        }
        app_audio::start(&options.audio)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        obs::start_audio_capture(options)
    }
}

/// Stops the capture, if one runs. Safe to call at any time.
fn stop_capture() {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    app_audio::stop();
    #[cfg(obs)]
    obs::stop_capture();
}

/// Stops any capture and whatever must be shut down at the end of the process.
fn shutdown() {
    stop_capture();
    #[cfg(obs)]
    obs::shutdown();
}

// ---------------------------------------------------------------------------------------------
// The process

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Request {
    Kinds {
        id: u64,
        #[serde(default)]
        plugin_dir: Option<String>,
        #[serde(default)]
        data_dir: Option<String>,
    },
    Start {
        id: u64,
        options: Box<StartOptions>,
    },
    /// A capture of the application's sound alone, the picture coming from elsewhere.
    StartAudio {
        id: u64,
        options: Box<AudioStart>,
    },
    Stop {
        id: u64,
    },
    Shutdown,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Reply {
    Kinds {
        id: u64,
        kinds: Vec<CaptureKind>,
        #[serde(rename = "applicationAudio")]
        application_audio: Option<AudioKind>,
        /// Whether this helper can capture a picture at all (a build with libobs).
        pictures: bool,
    },
    Done {
        id: u64,
    },
    Failed {
        id: u64,
        reason: String,
    },
}

fn main() {
    let stdout = std::io::stdout();
    let mut out = std::io::BufWriter::new(stdout.lock());
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let request: Request = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                eprintln!("aspen-obs-capture: unreadable request: {error}");
                continue;
            }
        };
        let reply = match request {
            Request::Kinds {
                id,
                plugin_dir,
                data_dir,
            } => match catalogue(plugin_dir.as_ref(), data_dir.as_ref()) {
                Ok(catalogue) => Reply::Kinds {
                    id,
                    kinds: catalogue.kinds,
                    application_audio: catalogue.application_audio,
                    pictures: cfg!(obs),
                },
                Err(reason) => Reply::Failed { id, reason },
            },
            Request::Start { id, options } => {
                stop_capture();
                match start_capture(*options) {
                    Ok(()) => Reply::Done { id },
                    Err(reason) => Reply::Failed { id, reason },
                }
            }
            Request::StartAudio { id, options } => {
                stop_capture();
                match start_audio_capture(*options) {
                    Ok(()) => Reply::Done { id },
                    Err(reason) => Reply::Failed { id, reason },
                }
            }
            Request::Stop { id } => {
                stop_capture();
                Reply::Done { id }
            }
            Request::Shutdown => break,
        };
        let json = serde_json::to_string(&reply).expect("reply serialises");
        if writeln!(out, "{json}").and_then(|()| out.flush()).is_err() {
            break;
        }
    }
    // stdin closed or a shutdown asked for: end the capture and whatever it needed.
    shutdown();
}
