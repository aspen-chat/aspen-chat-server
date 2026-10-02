//! The watchdog of a game capture's hook: a game capture whose hook delivers no frame in time
//! is replaced in the scene by a window capture of the same window through
//! Windows.Graphics.Capture.

use super::{SESSION, Session, cstring, ffi};
use crate::Result;
use std::ptr;
use std::time::{Duration, Instant};

/// How long a game capture may go without a frame from its hook before the window is captured
/// another way: the game may never present, an anti-cheat may refuse the hook (after which
/// the source never tries again), or the window may not be a game at all. The source's first
/// injection is immediate and a hook that takes delivers a frame within the game's next
/// presents, and at the fastest hook rate (`HOOK_RATE_FASTEST`) an injection that ran but
/// captured nothing is tried again every 0.4 s, so three seconds covers several attempts.
const HOOK_TIMEOUT: Duration = Duration::from_secs(3);
const HOOK_POLL: Duration = Duration::from_millis(250);
/// `game_capture`'s `hook_rate` that retries a hook every 0.4 s rather than every 4 s.
pub(super) const HOOK_RATE_FASTEST: i64 = 3;
/// `window_capture`'s capture method that goes through Windows.Graphics.Capture.
const WINDOW_CAPTURE_WGC: i64 = 2;

/// Watches a game capture's hook from a thread of its own: a source that still has no size
/// after `HOOK_TIMEOUT` has had no frame from its hook, and is replaced in the scene by a
/// window capture of the same window through Windows.Graphics.Capture, the sound unchanged.
pub(super) fn watch_hook(serial: u64, settings: Option<String>) {
    let _ = std::thread::Builder::new()
        .name("aspen-hook-watch".into())
        .spawn(move || {
            let started = Instant::now();
            loop {
                std::thread::sleep(HOOK_POLL);
                let mut session = SESSION.lock().expect("session lock");
                let Some(current) = session.as_mut().filter(|s| s.serial == serial) else {
                    return;
                };
                if unsafe { ffi::obs_source_get_width(current.source) } > 0 {
                    return;
                }
                if started.elapsed() < HOOK_TIMEOUT {
                    continue;
                }
                match unsafe { capture_window_instead(current, settings.as_deref()) } {
                    Ok(()) => eprintln!(
                        "aspen-obs-capture: the game capture hook delivered no frame in {} s; capturing the window through Windows.Graphics.Capture instead",
                        HOOK_TIMEOUT.as_secs()
                    ),
                    Err(reason) => eprintln!(
                        "aspen-obs-capture: the game capture hook delivered no frame, and the window could not be captured instead: {reason}"
                    ),
                }
                return;
            }
        });
}

/// Swaps the session's source for a `window_capture` of the window its settings name, in
/// the scene item's place with its bounds.
unsafe fn capture_window_instead(session: &mut Session, settings: Option<&str>) -> Result<()> {
    unsafe {
        let data = match settings {
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
        ffi::obs_data_set_int(data, c"method".as_ptr(), WINDOW_CAPTURE_WGC);
        let name = cstring(&format!("aspen-window-{}", session.serial))?;
        let source = ffi::obs_source_create(
            c"window_capture".as_ptr(),
            name.as_ptr(),
            data,
            ptr::null_mut(),
        );
        ffi::obs_data_release(data);
        if source.is_null() {
            return Err("libobs has no source of kind window_capture".into());
        }
        let item = ffi::obs_scene_add(session.scene, source);
        let mut bounds = ffi::vec2::default();
        ffi::obs_sceneitem_get_bounds(session.item, &mut bounds);
        ffi::obs_sceneitem_set_bounds_type(item, ffi::obs_sceneitem_get_bounds_type(session.item));
        ffi::obs_sceneitem_set_bounds_alignment(
            item,
            ffi::obs_sceneitem_get_bounds_alignment(session.item),
        );
        ffi::obs_sceneitem_set_bounds(item, &bounds);
        ffi::obs_sceneitem_remove(session.item);
        ffi::obs_source_release(session.source);
        session.item = item;
        session.source = source;
    }
    Ok(())
}
