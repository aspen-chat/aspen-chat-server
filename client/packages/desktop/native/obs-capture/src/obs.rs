//! Game and window capture through libobs, on the platforms that use it (Windows and macOS,
//! and Linux when built with the `libobs` feature, for developing the picture path there).
//!
//! libobs is used for one thing: its capture sources, which reach games that ordinary display
//! capture cannot (`game_capture` hooks Direct3D and OpenGL on Windows). The picture goes
//! through a scene sized to the requested output, an H.264 encoder (`obs_x264`, constrained
//! baseline so every browser can decode it), and an output registered here whose only job is
//! to hand every encoded packet to the SRTP sender in `rtp`, which delivers it straight to
//! the voice server's plain RTP transport. The application's sound goes with the picture where
//! the platform's OBS source can capture it on its own, encoded as Opus by obs-ffmpeg.
//!
//! libobs itself is started once and kept for the life of the process, since it does not
//! support a second startup.

#[cfg(not(target_os = "linux"))]
use crate::AudioStart;
use crate::rtp::{Feedback, RtpSender, StreamKind};
use crate::{AudioOptions, CaptureKind, CaptureTarget, Result, StartOptions};
use std::ffi::{CStr, CString, c_char, c_void};
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

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

// ---------------------------------------------------------------------------------------------
// Startup

/// Starts libobs once: platform display, core, data and module paths, video, audio, the
/// modules the capture needs, and the packet output.
fn ensure_started(plugin_dir: &str, data_dir: &str) -> Result<()> {
    STARTUP
        .get_or_init(|| start_libobs(plugin_dir, data_dir))
        .clone()
}

fn start_libobs(plugin_dir: &str, data_dir: &str) -> Result<()> {
    unsafe {
        logging::install();
        #[cfg(target_os = "linux")]
        {
            // The graphics module needs the session's display: Wayland when there is one, else X11.
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                let display = display::wl_display_connect(ptr::null());
                if display.is_null() {
                    return Err("could not connect to the Wayland display".into());
                }
                ffi::obs_set_nix_platform(ffi::OBS_NIX_PLATFORM_WAYLAND);
                ffi::obs_set_nix_platform_display(display);
            } else {
                let display = display::XOpenDisplay(ptr::null());
                if display.is_null() {
                    return Err("could not open the X display".into());
                }
                ffi::obs_set_nix_platform(ffi::OBS_NIX_PLATFORM_X11_EGL);
                ffi::obs_set_nix_platform_display(display);
            }
        }
        if !ffi::obs_startup(c"en-US".as_ptr(), ptr::null(), ptr::null_mut()) {
            return Err("libobs did not start".into());
        }
        let libobs_data = cstring(&format!("{data_dir}/libobs/"))?;
        ffi::obs_add_data_path(libobs_data.as_ptr());
        reset_video(1280, 720, 30)?;
        let audio = ffi::obs_audio_info {
            samples_per_sec: 48_000,
            speakers: ffi::SPEAKERS_STEREO,
        };
        if !ffi::obs_reset_audio(&audio) {
            return Err("libobs audio did not initialise".into());
        }
        load_modules(plugin_dir, data_dir)?;
        register_output();
    }
    Ok(())
}

#[cfg(target_os = "windows")]
const GRAPHICS_MODULE: &CStr = c"libobs-d3d11";
#[cfg(not(target_os = "windows"))]
const GRAPHICS_MODULE: &CStr = c"libobs-opengl";

unsafe fn reset_video(width: u32, height: u32, fps: u32) -> Result<()> {
    let mut info: ffi::obs_video_info = unsafe { std::mem::zeroed() };
    info.graphics_module = GRAPHICS_MODULE.as_ptr();
    info.fps_num = fps;
    info.fps_den = 1;
    info.base_width = width;
    info.base_height = height;
    info.output_width = width;
    info.output_height = height;
    info.output_format = ffi::VIDEO_FORMAT_NV12;
    info.adapter = 0;
    info.gpu_conversion = true;
    info.colorspace = ffi::VIDEO_CS_709;
    info.range = ffi::VIDEO_RANGE_PARTIAL;
    info.scale_type = ffi::OBS_SCALE_BICUBIC;
    // The result codes are an enum libobs declares with negative members, so they bind as
    // signed while the zero for success binds unsigned.
    let code = unsafe { ffi::obs_reset_video(&mut info) };
    if code == ffi::OBS_VIDEO_SUCCESS as i32 {
        Ok(())
    } else if code == ffi::OBS_VIDEO_CURRENTLY_ACTIVE {
        Err("a capture is already running".into())
    } else if code == ffi::OBS_VIDEO_MODULE_NOT_FOUND {
        Err("the libobs graphics module was not found".into())
    } else if code == ffi::OBS_VIDEO_NOT_SUPPORTED {
        Err("the graphics adapter is not supported by libobs".into())
    } else {
        Err(format!("libobs video did not initialise (code {code})"))
    }
}

/// The modules the capture needs, by file name without extension; missing ones are skipped,
/// since each platform ships its own capture plugin.
const MODULES: &[&str] = &[
    "obs-x264",
    "obs-ffmpeg",
    "image-source",
    "win-wasapi",
    "win-capture",
    "mac-capture",
];

#[cfg(target_os = "windows")]
const MODULE_EXTENSION: &str = "dll";
#[cfg(not(target_os = "windows"))]
const MODULE_EXTENSION: &str = "so";

unsafe fn load_modules(plugin_dir: &str, data_dir: &str) -> Result<()> {
    let mut loaded = 0;
    for name in MODULES {
        let path = format!("{plugin_dir}/{name}.{MODULE_EXTENSION}");
        if !std::path::Path::new(&path).exists() {
            continue;
        }
        let path_c = cstring(&path)?;
        let data_c = cstring(&format!("{data_dir}/obs-plugins/{name}"))?;
        let mut module: *mut ffi::obs_module = ptr::null_mut();
        if unsafe { ffi::obs_open_module(&mut module, path_c.as_ptr(), data_c.as_ptr()) }
            == ffi::MODULE_SUCCESS as i32
            && unsafe { ffi::obs_init_module(module) }
        {
            loaded += 1;
        }
    }
    unsafe { ffi::obs_post_load_modules() };
    if loaded == 0 {
        return Err(format!("no OBS plugins found in {plugin_dir}"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// The packet output

unsafe extern "C" fn output_get_name(_: *mut c_void) -> *const c_char {
    c"Aspen packet output".as_ptr()
}

unsafe extern "C" fn output_create(
    _: *mut ffi::obs_data,
    output: *mut ffi::obs_output,
) -> *mut c_void {
    output.cast()
}

unsafe extern "C" fn output_destroy(_: *mut c_void) {}

unsafe extern "C" fn output_start(data: *mut c_void) -> bool {
    let output: *mut ffi::obs_output = data.cast();
    unsafe {
        if !ffi::obs_output_can_begin_data_capture(output, 0) {
            return false;
        }
        if !ffi::obs_output_initialize_encoders(output, 0) {
            return false;
        }
        ffi::obs_output_begin_data_capture(output, 0)
    }
}

unsafe extern "C" fn output_stop(data: *mut c_void, _: u64) {
    let output: *mut ffi::obs_output = data.cast();
    unsafe { ffi::obs_output_end_data_capture(output) };
}

unsafe extern "C" fn output_encoded_packet(data: *mut c_void, packet: *mut ffi::encoder_packet) {
    let output: *mut ffi::obs_output = data.cast();
    let packet = unsafe { &*packet };
    let mut sink = SINK.lock().expect("sink lock");
    let Some(sink) = sink.as_mut() else {
        return;
    };
    if packet.type_ == ffi::OBS_ENCODER_AUDIO {
        if let Some(audio) = &sink.audio {
            let frame = unsafe { std::slice::from_raw_parts(packet.data, packet.size) };
            let timestamp_us = packet.pts as f64 * 1_000_000.0 * f64::from(packet.timebase_num)
                / f64::from(packet.timebase_den.max(1));
            audio.send_frame(frame, timestamp_us);
        }
        return;
    }
    if packet.type_ != ffi::OBS_ENCODER_VIDEO {
        return;
    }
    let Some(video) = sink.video.clone() else {
        return;
    };
    if sink.parameter_sets.is_none() {
        // obs_x264 keeps the SPS and PPS out of its packets; the decoders need them with
        // every keyframe, and the payloader sends them as their own aggregate packet.
        let encoder = unsafe { ffi::obs_output_get_video_encoder(output) };
        let mut extra: *mut u8 = ptr::null_mut();
        let mut size: usize = 0;
        if unsafe { ffi::obs_encoder_get_extra_data(encoder, &mut extra, &mut size) }
            && !extra.is_null()
        {
            sink.parameter_sets = Some(unsafe { std::slice::from_raw_parts(extra, size) }.to_vec());
        }
    }
    let frame = unsafe { std::slice::from_raw_parts(packet.data, packet.size) };
    let timestamp_us = packet.pts as f64 * 1_000_000.0 * f64::from(packet.timebase_num)
        / f64::from(packet.timebase_den.max(1));
    match (&sink.parameter_sets, packet.keyframe) {
        (Some(sets), true) => {
            let mut with_sets = Vec::with_capacity(sets.len() + frame.len());
            with_sets.extend_from_slice(sets);
            with_sets.extend_from_slice(frame);
            video.send_frame(&with_sets, timestamp_us);
        }
        _ => video.send_frame(frame, timestamp_us),
    }
}

unsafe fn register_output() {
    unsafe {
        register_output_kind(OUTPUT_ID, ffi::OBS_OUTPUT_VIDEO | ffi::OBS_OUTPUT_ENCODED);
        register_output_kind(OUTPUT_AV_ID, ffi::OBS_OUTPUT_AV | ffi::OBS_OUTPUT_ENCODED);
        register_output_kind(
            OUTPUT_AUDIO_ID,
            ffi::OBS_OUTPUT_AUDIO | ffi::OBS_OUTPUT_ENCODED,
        );
    }
}

unsafe fn register_output_kind(id: &'static CStr, flags: u32) {
    let mut info: ffi::obs_output_info = unsafe { std::mem::zeroed() };
    info.id = id.as_ptr();
    info.flags = flags;
    info.get_name = Some(output_get_name);
    info.create = Some(output_create);
    info.destroy = Some(output_destroy);
    info.start = Some(output_start);
    info.stop = Some(output_stop);
    info.encoded_packet = Some(output_encoded_packet);
    unsafe { ffi::obs_register_output_s(&info, std::mem::size_of::<ffi::obs_output_info>()) };
}

// ---------------------------------------------------------------------------------------------
// Captures

fn plugin_dir(options: Option<&String>) -> String {
    options
        .cloned()
        .unwrap_or_else(|| env!("OBS_DEFAULT_PLUGIN_DIR").to_string())
}

fn data_dir(options: Option<&String>) -> String {
    options
        .cloned()
        .unwrap_or_else(|| env!("OBS_DEFAULT_DATA_DIR").to_string())
}

/// One capture kind this platform can use for a game: the video source, the setting that
/// names its window, and the audio source that captures the same window's application, with
/// its own application setting, which takes the video source's window value.
struct KindEntry {
    kind: &'static str,
    property: &'static str,
    audio: Option<(&'static str, &'static str)>,
}

/// The kinds, best first. Windows captures a process's audio by the same window string
/// (`wasapi_process_output_capture`, OBS's "Application Audio Capture", a beta feature); macOS
/// captures an application's audio by bundle id (`sck_audio_capture`, likewise beta), so there
/// the video source targets an application too (`screen_capture` type 2). Linux has none: its
/// window and screen capture goes through the desktop portal, which only the process owning
/// the requesting window can raise, so the shell shares the picture through the browser's own
/// screen share and this helper supplies only the sound, through its own PipeWire capture.
#[cfg(target_os = "windows")]
const KINDS: &[KindEntry] = &[
    KindEntry {
        kind: "game_capture",
        property: "window",
        audio: Some(("wasapi_process_output_capture", "window")),
    },
    KindEntry {
        kind: "window_capture",
        property: "window",
        audio: Some(("wasapi_process_output_capture", "window")),
    },
];
#[cfg(target_os = "macos")]
const KINDS: &[KindEntry] = &[KindEntry {
    kind: "screen_capture",
    property: "application",
    audio: Some(("sck_audio_capture", "application")),
}];
#[cfg(target_os = "linux")]
const KINDS: &[KindEntry] = &[];

unsafe fn source_kind_registered(id: &CStr) -> bool {
    let mut index = 0;
    let mut registered: *const c_char = ptr::null();
    while unsafe { ffi::obs_enum_source_types(index, &mut registered) } {
        if !registered.is_null() && unsafe { CStr::from_ptr(registered) } == id {
            return true;
        }
        index += 1;
    }
    false
}

/// The game capture kinds this platform has, with the windows each can be pointed at.
pub fn capture_kinds(
    plugin_dir_override: Option<&String>,
    data_dir_override: Option<&String>,
) -> Result<Vec<CaptureKind>> {
    ensure_started(
        &plugin_dir(plugin_dir_override),
        &data_dir(data_dir_override),
    )?;
    let mut kinds = Vec::new();
    for entry in KINDS {
        let id = cstring(entry.kind)?;
        if !unsafe { source_kind_registered(&id) } {
            continue;
        }
        let audio = match entry.audio {
            Some((kind, property)) => {
                let audio_id = cstring(kind)?;
                unsafe { source_kind_registered(&audio_id) }.then(|| crate::AudioKind {
                    kind: kind.to_string(),
                    property: property.to_string(),
                    targets: None,
                })
            }
            None => None,
        };
        kinds.push(CaptureKind {
            kind: entry.kind.to_string(),
            property: entry.property.to_string(),
            targets: unsafe { window_targets(&id, entry.property)? },
            audio,
        });
    }
    Ok(kinds)
}

/// Asks a throwaway source of `id` for the items of its window list property.
unsafe fn window_targets(id: &CStr, property: &str) -> Result<Vec<CaptureTarget>> {
    let property_c = cstring(property)?;
    let source = unsafe {
        ffi::obs_source_create_private(id.as_ptr(), c"aspen-probe".as_ptr(), ptr::null_mut())
    };
    if source.is_null() {
        return Ok(Vec::new());
    }
    let mut targets = Vec::new();
    unsafe {
        let properties = ffi::obs_source_properties(source);
        if !properties.is_null() {
            let list = ffi::obs_properties_get(properties, property_c.as_ptr());
            if !list.is_null() {
                let count = ffi::obs_property_list_item_count(list);
                for index in 0..count {
                    let name = ffi::obs_property_list_item_name(list, index);
                    let value = ffi::obs_property_list_item_string(list, index);
                    if name.is_null() || value.is_null() {
                        continue;
                    }
                    let value = CStr::from_ptr(value).to_string_lossy().into_owned();
                    if value.is_empty() {
                        continue;
                    }
                    targets.push(CaptureTarget {
                        name: CStr::from_ptr(name).to_string_lossy().into_owned(),
                        value,
                    });
                }
            }
            ffi::obs_properties_destroy(properties);
        }
        ffi::obs_source_release(source);
    }
    Ok(targets)
}

/// Starts capturing: the source, sized into the output, encoded, and sent as SRTP to the
/// target. One capture at a time; a second call while one runs is an error.
pub fn start_capture(options: StartOptions) -> Result<()> {
    ensure_started(
        &plugin_dir(options.plugin_dir.as_ref()),
        &data_dir(options.data_dir.as_ref()),
    )?;
    let mut session = SESSION.lock().expect("session lock");
    if session.is_some() {
        return Err("a capture is already running".into());
    }
    let width = options.width.unwrap_or(1920).clamp(16, 7680);
    let height = options.height.unwrap_or(1080).clamp(16, 4320);
    let fps = options.fps.unwrap_or(60).clamp(1, 240);
    let bitrate_kbps = options
        .bitrate_kbps
        .unwrap_or(6000)
        .clamp(MIN_BITRATE_KBPS, 100_000);
    let bitrate = i64::from(bitrate_kbps);
    let sender = RtpSender::start(
        &options.rtp,
        StreamKind::Video,
        Arc::new(EncoderFeedback),
        MIN_BITRATE_KBPS,
        bitrate_kbps,
    )?;
    let audio_sender = match &options.audio {
        Some(audio) => Some(RtpSender::start(
            &audio.rtp,
            StreamKind::Audio,
            Arc::new(EncoderFeedback),
            MIN_BITRATE_KBPS,
            bitrate_kbps,
        )?),
        None => None,
    };

    unsafe {
        reset_video(width, height, fps)?;

        let kind = cstring(&options.kind)?;
        let settings = match &options.settings {
            Some(json) => {
                let json_c = cstring(json)?;
                let data = ffi::obs_data_create_from_json(json_c.as_ptr());
                if data.is_null() {
                    return Err("settings are not a JSON object".into());
                }
                data
            }
            None => ffi::obs_data_create(),
        };
        // Names are unique per capture: libobs destroys a released source a little later, on
        // its graphics thread, and renames a newcomer that collides with one still going.
        let serial = SESSIONS_STARTED.fetch_add(1, Ordering::SeqCst);
        let source_name = cstring(&format!("aspen-capture-{serial}"))?;
        let scene_name = cstring(&format!("aspen-scene-{serial}"))?;
        let encoder_name = cstring(&format!("aspen-h264-{serial}"))?;
        let output_name = cstring(&format!("aspen-output-{serial}"))?;
        let source = ffi::obs_source_create(
            kind.as_ptr(),
            source_name.as_ptr(),
            settings,
            ptr::null_mut(),
        );
        ffi::obs_data_release(settings);
        if source.is_null() {
            return Err(format!("libobs has no source of kind {}", options.kind));
        }

        let scene = ffi::obs_scene_create(scene_name.as_ptr());
        let item = ffi::obs_scene_add(scene, source);
        // The picture fits inside the output, centred, keeping its shape.
        ffi::obs_sceneitem_set_bounds_type(item, ffi::OBS_BOUNDS_SCALE_INNER);
        ffi::obs_sceneitem_set_bounds_alignment(item, 0);
        let bounds = ffi::vec2 {
            __bindgen_anon_1: ffi::vec2__bindgen_ty_1 {
                __bindgen_anon_1: ffi::vec2__bindgen_ty_1__bindgen_ty_1 {
                    x: width as f32,
                    y: height as f32,
                },
            },
        };
        ffi::obs_sceneitem_set_bounds(item, &bounds);
        ffi::obs_set_output_source(0, ffi::obs_scene_get_source(scene));

        let encoder_settings = ffi::obs_data_create();
        ffi::obs_data_set_int(encoder_settings, c"bitrate".as_ptr(), bitrate);
        ffi::obs_data_set_string(encoder_settings, c"rate_control".as_ptr(), c"CBR".as_ptr());
        ffi::obs_data_set_string(encoder_settings, c"preset".as_ptr(), c"veryfast".as_ptr());
        ffi::obs_data_set_string(encoder_settings, c"tune".as_ptr(), c"zerolatency".as_ptr());
        // Constrained baseline decodes everywhere, Firefox's OpenH264 included; with the
        // zero-latency tune there are no B-frames to lose. A keyframe every second is how
        // long a new viewer waits, since libobs cannot be asked for one.
        ffi::obs_data_set_string(encoder_settings, c"profile".as_ptr(), c"baseline".as_ptr());
        ffi::obs_data_set_int(encoder_settings, c"keyint_sec".as_ptr(), 1);
        let encoder = ffi::obs_video_encoder_create(
            c"obs_x264".as_ptr(),
            encoder_name.as_ptr(),
            encoder_settings,
            ptr::null_mut(),
        );
        ffi::obs_data_release(encoder_settings);
        if encoder.is_null() {
            release(Session {
                source,
                scene,
                item,
                encoder: ptr::null_mut(),
                output: ptr::null_mut(),
                audio_source: ptr::null_mut(),
                audio_encoder: ptr::null_mut(),
            });
            return Err("the obs_x264 encoder is not available".into());
        }
        ffi::obs_encoder_set_video(encoder, ffi::obs_get_video());

        let (audio_source, audio_encoder) = match &options.audio {
            Some(audio) => match create_audio(audio, serial) {
                Ok(created) => created,
                Err(reason) => {
                    release(Session {
                        source,
                        scene,
                        item,
                        encoder,
                        output: ptr::null_mut(),
                        audio_source: ptr::null_mut(),
                        audio_encoder: ptr::null_mut(),
                    });
                    return Err(reason);
                }
            },
            None => (ptr::null_mut(), ptr::null_mut()),
        };

        let output = ffi::obs_output_create(
            if audio_encoder.is_null() {
                OUTPUT_ID.as_ptr()
            } else {
                OUTPUT_AV_ID.as_ptr()
            },
            output_name.as_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if output.is_null() {
            release(Session {
                source,
                scene,
                item,
                encoder,
                output: ptr::null_mut(),
                audio_source,
                audio_encoder,
            });
            return Err("the packet output could not be created".into());
        }
        // An encoded output takes its media from its encoders.
        ffi::obs_output_set_video_encoder(output, encoder);
        if !audio_encoder.is_null() {
            ffi::obs_output_set_audio_encoder(output, audio_encoder, 0);
        }

        *SINK.lock().expect("sink lock") = Some(Sink {
            video: Some(Arc::clone(&sender)),
            parameter_sets: None,
            audio: audio_sender.clone(),
        });
        if !ffi::obs_output_start(output) {
            let reason = ffi::obs_output_get_last_error(output);
            let reason = if reason.is_null() {
                "the output did not start".to_string()
            } else {
                CStr::from_ptr(reason).to_string_lossy().into_owned()
            };
            *SINK.lock().expect("sink lock") = None;
            sender.stop();
            if let Some(audio) = &audio_sender {
                audio.stop();
            }
            release(Session {
                source,
                scene,
                item,
                encoder,
                output,
                audio_source,
                audio_encoder,
            });
            return Err(reason);
        }
        *session = Some(Session {
            source,
            scene,
            item,
            encoder,
            output,
            audio_source,
            audio_encoder,
        });
    }
    Ok(())
}

/// The application's audio: its source on output channel 1 (a video capture's scene holds
/// channel 0), encoded as Opus by obs-ffmpeg. An empty kind means no source of its own: the mix
/// already carries what the video source produces, as a media file's sound. On failure what was
/// made here is released and the reason returned.
unsafe fn create_audio(
    audio: &AudioOptions,
    serial: u64,
) -> Result<(*mut ffi::obs_source, *mut ffi::obs_encoder)> {
    unsafe {
        let audio_source = if audio.kind.is_empty() {
            ptr::null_mut()
        } else {
            let audio_kind = cstring(&audio.kind)?;
            let audio_settings = match &audio.settings {
                Some(json) => {
                    let json_c = cstring(json)?;
                    let data = ffi::obs_data_create_from_json(json_c.as_ptr());
                    if data.is_null() {
                        return Err("audio settings are not a JSON object".into());
                    }
                    data
                }
                None => ffi::obs_data_create(),
            };
            let audio_name = cstring(&format!("aspen-audio-{serial}"))?;
            let audio_source = ffi::obs_source_create(
                audio_kind.as_ptr(),
                audio_name.as_ptr(),
                audio_settings,
                ptr::null_mut(),
            );
            ffi::obs_data_release(audio_settings);
            if audio_source.is_null() {
                return Err(format!("libobs has no audio source of kind {}", audio.kind));
            }
            ffi::obs_set_output_source(1, audio_source);
            audio_source
        };
        // 128 kbps stereo carries music and a game's transients cleanly; lower rates smear
        // cymbals and other dense high-frequency sound.
        let opus_settings = ffi::obs_data_create();
        ffi::obs_data_set_int(
            opus_settings,
            c"bitrate".as_ptr(),
            i64::from(audio.bitrate_kbps.unwrap_or(128).clamp(32, 320)),
        );
        let opus_name = cstring(&format!("aspen-opus-{serial}"))?;
        let audio_encoder = ffi::obs_audio_encoder_create(
            c"ffmpeg_opus".as_ptr(),
            opus_name.as_ptr(),
            opus_settings,
            0,
            ptr::null_mut(),
        );
        ffi::obs_data_release(opus_settings);
        if audio_encoder.is_null() {
            if !audio_source.is_null() {
                ffi::obs_set_output_source(1, ptr::null_mut());
                ffi::obs_source_release(audio_source);
            }
            return Err("the ffmpeg_opus encoder is not available".into());
        }
        ffi::obs_encoder_set_audio(audio_encoder, ffi::obs_get_audio());
        Ok((audio_source, audio_encoder))
    }
}

/// Starts capturing the application's sound alone, encoded as Opus and sent as SRTP to its
/// target. One capture at a time, with or without a picture.
#[cfg(not(target_os = "linux"))]
pub fn start_audio_capture(options: AudioStart) -> Result<()> {
    ensure_started(
        &plugin_dir(options.plugin_dir.as_ref()),
        &data_dir(options.data_dir.as_ref()),
    )?;
    let mut session = SESSION.lock().expect("session lock");
    if session.is_some() {
        return Err("a capture is already running".into());
    }
    if options.audio.kind.is_empty() {
        return Err("a capture of sound alone needs an audio source".into());
    }
    let serial = SESSIONS_STARTED.fetch_add(1, Ordering::SeqCst);
    let output_name = cstring(&format!("aspen-output-{serial}"))?;
    let sender = RtpSender::start(
        &options.audio.rtp,
        StreamKind::Audio,
        Arc::new(EncoderFeedback),
        MIN_BITRATE_KBPS,
        MIN_BITRATE_KBPS,
    )?;
    unsafe {
        let (audio_source, audio_encoder) = match create_audio(&options.audio, serial) {
            Ok(created) => created,
            Err(reason) => {
                sender.stop();
                return Err(reason);
            }
        };
        let sound_only = |output| Session {
            source: ptr::null_mut(),
            scene: ptr::null_mut(),
            item: ptr::null_mut(),
            encoder: ptr::null_mut(),
            output,
            audio_source,
            audio_encoder,
        };
        let output = ffi::obs_output_create(
            OUTPUT_AUDIO_ID.as_ptr(),
            output_name.as_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if output.is_null() {
            sender.stop();
            release(sound_only(ptr::null_mut()));
            return Err("the packet output could not be created".into());
        }
        ffi::obs_output_set_audio_encoder(output, audio_encoder, 0);
        *SINK.lock().expect("sink lock") = Some(Sink {
            video: None,
            parameter_sets: None,
            audio: Some(Arc::clone(&sender)),
        });
        if !ffi::obs_output_start(output) {
            let reason = ffi::obs_output_get_last_error(output);
            let reason = if reason.is_null() {
                "the output did not start".to_string()
            } else {
                CStr::from_ptr(reason).to_string_lossy().into_owned()
            };
            *SINK.lock().expect("sink lock") = None;
            sender.stop();
            release(sound_only(output));
            return Err(reason);
        }
        *session = Some(sound_only(output));
    }
    Ok(())
}

/// Stops the capture, if one runs. Safe to call at any time.
pub fn stop_capture() {
    let taken = SESSION.lock().expect("session lock").take();
    if let Some(session) = taken {
        unsafe {
            if !session.output.is_null() {
                ffi::obs_output_stop(session.output);
            }
            release(session);
        }
    }
    if let Some(sink) = SINK.lock().expect("sink lock").take() {
        if let Some(video) = &sink.video {
            video.stop();
        }
        if let Some(audio) = &sink.audio {
            audio.stop();
        }
    }
    // libobs destroys released sources on its graphics thread; waiting for that keeps a capture
    // started right after from colliding with the old one's objects. Before libobs has started
    // there is no queue to wait on, and asking crashes.
    if STARTUP.get().is_some_and(|started| started.is_ok()) {
        unsafe { ffi::obs_wait_for_destroy_queue() };
    }
}

/// Stops any capture and shuts libobs down. For the end of the process only: libobs cannot
/// be started again afterwards, and its graphics thread complains at exit if it is skipped.
pub fn shutdown() {
    stop_capture();
    if STARTUP.get().is_some_and(|started| started.is_ok()) {
        unsafe {
            ffi::obs_wait_for_destroy_queue();
            ffi::obs_shutdown();
        }
    }
}

unsafe fn release(session: Session) {
    unsafe {
        if !session.output.is_null() {
            ffi::obs_output_release(session.output);
        }
        if !session.encoder.is_null() {
            ffi::obs_encoder_release(session.encoder);
        }
        if !session.audio_encoder.is_null() {
            ffi::obs_encoder_release(session.audio_encoder);
        }
        ffi::obs_set_output_source(0, ptr::null_mut());
        ffi::obs_set_output_source(1, ptr::null_mut());
        if !session.audio_source.is_null() {
            ffi::obs_source_release(session.audio_source);
        }
        // The item's hold on the source is dropped now, on this thread. libobs destroys the
        // scene itself later, on its graphics thread, and a shutdown in between would tear the
        // source down from under the item.
        if !session.item.is_null() {
            ffi::obs_sceneitem_remove(session.item);
        }
        if !session.scene.is_null() {
            ffi::obs_scene_release(session.scene);
        }
        if !session.source.is_null() {
            ffi::obs_source_release(session.source);
        }
    }
}
