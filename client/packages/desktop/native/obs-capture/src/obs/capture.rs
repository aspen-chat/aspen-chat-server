//! The captures: the kinds this platform has, starting a capture of a picture with its sound or
//! of sound alone, and stopping it, releasing its libobs objects, and shutting libobs down.

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use super::OUTPUT_AUDIO_ID;
use super::hook::{HOOK_RATE_FASTEST, watch_hook};
use super::startup::{ensure_started, reset_video};
use super::{
    EncoderFeedback, MIN_BITRATE_KBPS, OUTPUT_AV_ID, OUTPUT_ID, SESSION, SESSIONS_STARTED, SINK,
    STARTUP, Session, Sink, cstring, ffi,
};
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use crate::AudioStart;
use crate::rtp::{RtpSender, StreamKind};
use crate::{AudioOptions, CaptureKind, CaptureTarget, Result, StartOptions};
use std::ffi::{CStr, c_char};
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::Ordering;

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
        if options.kind == "game_capture"
            && !ffi::obs_data_has_user_value(settings, c"hook_rate".as_ptr())
        {
            ffi::obs_data_set_int(settings, c"hook_rate".as_ptr(), HOOK_RATE_FASTEST);
        }
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
                serial,
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
                        serial,
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
                serial,
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
                serial,
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
            serial,
            source,
            scene,
            item,
            encoder,
            output,
            audio_source,
            audio_encoder,
        });
        if options.kind == "game_capture" {
            watch_hook(serial, options.settings.clone());
        }
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
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
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
