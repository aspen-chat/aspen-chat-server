//! Starting libobs once for the life of the process: the platform display, the core, its data
//! and module paths, video and audio, the capture modules, and the packet output registered
//! here, which hands every encoded packet to the senders in the session's sink.

#[cfg(target_os = "linux")]
use super::display;
use super::{OUTPUT_AUDIO_ID, OUTPUT_AV_ID, OUTPUT_ID, SINK, STARTUP, cstring, ffi, logging};
use crate::Result;
use std::ffi::{CStr, c_char, c_void};
use std::ptr;

/// Starts libobs once: platform display, core, data and module paths, video, audio, the
/// modules the capture needs, and the packet output.
pub(super) fn ensure_started(plugin_dir: &str, data_dir: &str) -> Result<()> {
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

pub(super) unsafe fn reset_video(width: u32, height: u32, fps: u32) -> Result<()> {
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
