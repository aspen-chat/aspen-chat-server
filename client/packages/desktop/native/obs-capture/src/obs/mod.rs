//! Game and window capture through libobs, on the platforms that use it: Windows, and Linux
//! or macOS when built with the `libobs` feature, for developing the picture path there.
//!
//! libobs is used for one thing: its capture sources, which reach games that ordinary display
//! capture cannot (`game_capture` hooks Direct3D and OpenGL on Windows). The picture goes
//! through a scene sized to the requested output, an H.264 encoder (`obs_x264`, constrained
//! baseline so every browser can decode it), and an output registered in `startup` whose only
//! job is to hand every encoded packet to the SRTP sender in `rtp`, which delivers it straight to
//! the voice server's plain RTP transport. The application's sound goes with the picture where
//! the platform's OBS source can capture it on its own, encoded as Opus by obs-ffmpeg.
//!
//! libobs itself is started once and kept for the life of the process, since it does not
//! support a second startup.

use crate::Result;
use crate::rtp::{Feedback, RtpSender};
use std::ffi::{CStr, CString};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex, OnceLock};

mod capture;
mod hook;
mod startup;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use capture::start_audio_capture;
pub use capture::{capture_kinds, shutdown, start_capture, stop_capture};

#[allow(
    non_upper_case_globals,
    non_camel_case_types,
    non_snake_case,
    dead_code,
    unsafe_op_in_unsafe_fn
)]
mod ffi {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

#[cfg(target_os = "linux")]
mod display {
    use std::ffi::c_void;
    #[link(name = "wayland-client")]
    unsafe extern "C" {
        pub fn wl_display_connect(name: *const std::ffi::c_char) -> *mut c_void;
    }
    #[link(name = "X11")]
    unsafe extern "C" {
        pub fn XOpenDisplay(name: *const std::ffi::c_char) -> *mut c_void;
    }
}

const OUTPUT_ID: &CStr = c"aspen_packet_output";
/// The same output with an audio encoder attached; libobs requires the flags at registration.
const OUTPUT_AV_ID: &CStr = c"aspen_packet_output_av";
/// The output of a capture of sound alone.
const OUTPUT_AUDIO_ID: &CStr = c"aspen_packet_output_audio";

/// libobs logs everything to stderr by default. Only its warnings and errors are worth the
/// terminal, prefixed so they can be told from the shell's own, unless `ASPEN_OBS_CAPTURE_VERBOSE`
/// is set, which lets its information through too (how much audio buffering it added, which
/// encoder settings it took). Formatting a C `va_list` is platform-specific; where it is not
/// done here, libobs's own logging stands.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod logging {
    use super::ffi;
    use std::ffi::{CStr, c_char, c_int, c_void};
    use std::sync::OnceLock;

    fn verbose() -> bool {
        static VERBOSE: OnceLock<bool> = OnceLock::new();
        *VERBOSE.get_or_init(|| std::env::var_os("ASPEN_OBS_CAPTURE_VERBOSE").is_some())
    }

    unsafe extern "C" {
        fn vsnprintf(
            buffer: *mut c_char,
            size: usize,
            format: *const c_char,
            args: *mut ffi::__va_list_tag,
        ) -> c_int;
    }

    unsafe extern "C" fn handler(
        level: c_int,
        format: *const c_char,
        args: *mut ffi::__va_list_tag,
        _: *mut c_void,
    ) {
        if level > ffi::LOG_WARNING as c_int && !verbose() {
            return;
        }
        let mut buffer = [0 as c_char; 1024];
        unsafe {
            vsnprintf(buffer.as_mut_ptr(), buffer.len(), format, args);
            let message = CStr::from_ptr(buffer.as_ptr()).to_string_lossy();
            let severity = if level <= ffi::LOG_ERROR as c_int {
                "error"
            } else if level <= ffi::LOG_WARNING as c_int {
                "warning"
            } else {
                "info"
            };
            eprintln!("libobs {severity}: {message}");
        }
    }

    pub unsafe fn install() {
        unsafe { ffi::base_set_log_handler(Some(handler), std::ptr::null_mut()) };
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
mod logging {
    pub unsafe fn install() {}
}

const MIN_BITRATE_KBPS: u32 = 500;

/// Applies the receiver's bandwidth estimate to the running encoder.
struct EncoderFeedback;

impl Feedback for EncoderFeedback {
    fn set_bitrate_kbps(&self, kbps: u32) {
        let session = SESSION.lock().expect("session lock");
        let Some(session) = session.as_ref() else {
            return;
        };
        if session.encoder.is_null() {
            return;
        }
        unsafe {
            let settings = ffi::obs_data_create();
            ffi::obs_data_set_int(settings, c"bitrate".as_ptr(), i64::from(kbps));
            ffi::obs_encoder_update(session.encoder, settings);
            ffi::obs_data_release(settings);
        }
    }
}

/// The running capture's libobs objects. Raw pointers, owned here and released in `stop`.
struct Session {
    /// Which capture this is, so a watcher started for it touches no later one.
    serial: u64,
    source: *mut ffi::obs_source,
    scene: *mut ffi::obs_scene,
    item: *mut ffi::obs_scene_item,
    encoder: *mut ffi::obs_encoder,
    output: *mut ffi::obs_output,
    audio_source: *mut ffi::obs_source,
    audio_encoder: *mut ffi::obs_encoder,
}
unsafe impl Send for Session {}

/// The senders: video, with the H.264 parameter sets it puts in front of every keyframe, when
/// the capture has a picture, and audio when it carries sound.
struct Sink {
    video: Option<Arc<RtpSender>>,
    parameter_sets: Option<Vec<u8>>,
    audio: Option<Arc<RtpSender>>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static SINK: Mutex<Option<Sink>> = Mutex::new(None);
static SESSIONS_STARTED: AtomicU64 = AtomicU64::new(0);
static STARTUP: OnceLock<Result<()>> = OnceLock::new();

fn cstring(value: &str) -> Result<CString> {
    CString::new(value).map_err(|_| "string contains a NUL byte".to_string())
}
