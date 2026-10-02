//! Game audio on macOS: a capture of one application's sound through ScreenCaptureKit, handed
//! to whoever started it buffer by buffer (`app_audio` encodes and sends it).
//!
//! ScreenCaptureKit captures the sound of the applications a content filter includes, apart
//! from everything else playing, which is how OBS's own application audio source works on
//! macOS. The stream must also carry a picture, so it is asked for the smallest and slowest
//! one the framework allows, which is never read. Capturing needs the Screen & System Audio
//! Recording permission (System Settings, Privacy & Security), granted to the app that runs
//! this helper; without it the framework lists nothing and the error says so.
//!
//! A `Target` is `pid`, `bundle`, and `application` (the name), as the settings of a
//! `startAudio` request carry it; `running_applications` lists what can be captured in the
//! same vocabulary: every running application with a window, since the framework does not say
//! which are playing sound.

use crate::AudioTarget;
use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_core_audio_types::{
    AudioBuffer, AudioBufferList, kAudioFormatFlagIsFloat, kAudioFormatFlagIsNonInterleaved,
};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{
    CMAudioFormatDescriptionGetStreamBasicDescription, CMBlockBuffer, CMSampleBuffer, CMTime,
};
use objc2_foundation::{NSArray, NSError, NSObject, NSObjectProtocol};
use objc2_screen_capture_kit::{
    SCContentFilter, SCRunningApplication, SCShareableContent, SCStream, SCStreamConfiguration,
    SCStreamOutput, SCStreamOutputType,
};
use serde::Deserialize;
use std::ptr::{self, NonNull};
use std::sync::Mutex;
use std::sync::mpsc;
use std::time::Duration;

const RATE: i64 = 48_000;
const CHANNELS: usize = 2;
/// How long the framework may take to list what can be captured, or to start.
const TIMEOUT: Duration = Duration::from_secs(10);

/// What the capture captures, as the settings of a `startAudio` request name it.
#[derive(Clone, Debug, PartialEq, Default, Deserialize)]
#[serde(default)]
pub struct Target {
    pid: Option<u32>,
    bundle: Option<String>,
    application: String,
}

impl Target {
    fn settings(&self) -> serde_json::Value {
        let mut settings = serde_json::Map::new();
        settings.insert("application".into(), self.application.clone().into());
        if let Some(pid) = self.pid {
            settings.insert("pid".into(), pid.into());
        }
        if let Some(bundle) = &self.bundle {
            settings.insert("bundle".into(), bundle.clone().into());
        }
        serde_json::Value::Object(settings)
    }

    /// The target `settings` name, a JSON object with `application`, `pid`, and `bundle`.
    pub fn from_settings(settings: Option<&str>) -> Result<Target, String> {
        let Some(json) = settings else {
            return Ok(Target::default());
        };
        let mut target: Target =
            serde_json::from_str(json).map_err(|e| format!("audio settings: {e}"))?;
        target.pid = target.pid.filter(|pid| *pid != 0);
        Ok(target)
    }
}

/// An application the framework lists, as this module reads it.
#[derive(Clone, Debug, PartialEq)]
struct Application {
    name: String,
    pid: Option<u32>,
    bundle: String,
}

impl Application {
    fn target(&self) -> AudioTarget {
        AudioTarget {
            name: self.name.clone(),
            pid: self.pid,
            settings: Target {
                pid: self.pid,
                bundle: Some(self.bundle.clone()),
                application: self.name.clone(),
            }
            .settings(),
        }
    }
}

fn read_application(application: &SCRunningApplication) -> Application {
    unsafe {
        Application {
            name: application.applicationName().to_string(),
            pid: u32::try_from(application.processID())
                .ok()
                .filter(|pid| *pid != 0),
            bundle: application.bundleIdentifier().to_string(),
        }
    }
}

/// What the framework says can be captured right now, or why it will not say.
fn shareable_content() -> Result<Retained<SCShareableContent>, String> {
    let (send, receive) = mpsc::channel::<Result<Retained<SCShareableContent>, String>>();
    let handler = RcBlock::new(
        move |content: *mut SCShareableContent, error: *mut NSError| {
            let outcome = match unsafe { Retained::retain(content) } {
                Some(content) => Ok(content),
                None => Err(describe(error)),
            };
            let _ = send.send(outcome);
        },
    );
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    receive
        .recv_timeout(TIMEOUT)
        .map_err(|_| "ScreenCaptureKit did not answer".to_string())?
}

/// What an error of the framework says, with the permission named where it is the cause.
fn describe(error: *mut NSError) -> String {
    let text = unsafe { Retained::retain(error) }
        .map(|error| error.localizedDescription().to_string())
        .unwrap_or_else(|| "ScreenCaptureKit gave no reason".to_string());
    format!(
        "{text} (capturing an application's sound needs the Screen & System Audio Recording permission, granted to Aspen in System Settings, Privacy & Security)"
    )
}

// ---------------------------------------------------------------------------------------------
// Listing

/// The running applications with a window, one entry each, sorted by name.
pub fn running_applications() -> Result<Vec<AudioTarget>, String> {
    let content = shareable_content()?;
    let applications = unsafe { content.applications() };
    let mut listed: Vec<Application> = applications.iter().map(|a| read_application(&a)).collect();
    Ok(sort_applications(&mut listed))
}

/// Sorted by name and then process id so the list is stable between refreshes; applications
/// with neither a name nor a process id are left out.
fn sort_applications(applications: &mut [Application]) -> Vec<AudioTarget> {
    applications.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.pid.cmp(&b.pid))
    });
    applications
        .iter()
        .filter(|a| !a.name.is_empty() || a.pid.is_some())
        .map(Application::target)
        .collect()
}

/// The application the target names: by process id while it runs, else by bundle id, else by
/// name, which follows an application across a restart.
fn select_application(applications: &[Application], target: &Target) -> Option<usize> {
    if let Some(pid) = target.pid
        && let Some(index) = applications.iter().position(|a| a.pid == Some(pid))
    {
        return Some(index);
    }
    if let Some(bundle) = target.bundle.as_deref().filter(|b| !b.is_empty())
        && let Some(index) = applications.iter().position(|a| a.bundle == bundle)
    {
        return Some(index);
    }
    if target.application.is_empty() {
        return None;
    }
    applications
        .iter()
        .position(|a| a.name == target.application)
}

// ---------------------------------------------------------------------------------------------
// The stream

/// Where a capture's samples go, called on the stream's own queue.
type AudioSink = Box<dyn FnMut(&[f32]) + Send>;

struct Ivars {
    sink: Mutex<AudioSink>,
    /// Interleaved stereo float, reused across buffers.
    scratch: Mutex<Vec<f32>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class implements no Drop.
    #[unsafe(super(NSObject))]
    #[name = "AspenApplicationAudioOutput"]
    #[ivars = Ivars]
    struct AudioOutput;

    unsafe impl NSObjectProtocol for AudioOutput {}

    unsafe impl SCStreamOutput for AudioOutput {
        // The protocol's method, named as the framework's bindings name it.
        #[allow(non_snake_case)]
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        unsafe fn stream_didOutputSampleBuffer_ofType(
            &self,
            _stream: &SCStream,
            sample_buffer: &CMSampleBuffer,
            kind: SCStreamOutputType,
        ) {
            if kind != SCStreamOutputType::Audio {
                return;
            }
            let mut scratch = self.ivars().scratch.lock().expect("scratch lock");
            scratch.clear();
            if unsafe { interleave(sample_buffer, &mut scratch) } && !scratch.is_empty() {
                (self.ivars().sink.lock().expect("sink lock"))(&scratch);
            }
        }
    }
);

impl AudioOutput {
    fn new(sink: AudioSink) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars {
            sink: Mutex::new(sink),
            scratch: Mutex::new(Vec::new()),
        });
        unsafe { msg_send![super(this), init] }
    }
}

/// Reads a sample buffer's float samples into `out` as interleaved stereo, whatever the
/// framework's layout (interleaved or one buffer per channel, mono or stereo); false when
/// the buffer is not float audio.
unsafe fn interleave(sample_buffer: &CMSampleBuffer, out: &mut Vec<f32>) -> bool {
    unsafe {
        let Some(description) = sample_buffer.format_description() else {
            return false;
        };
        let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(&description);
        if asbd.is_null() {
            return false;
        }
        let asbd = &*asbd;
        if asbd.mFormatFlags & kAudioFormatFlagIsFloat == 0 || asbd.mBitsPerChannel != 32 {
            return false;
        }
        let channels = asbd.mChannelsPerFrame as usize;
        let non_interleaved = asbd.mFormatFlags & kAudioFormatFlagIsNonInterleaved != 0;

        let mut needed = 0usize;
        sample_buffer.audio_buffer_list_with_retained_block_buffer(
            &mut needed,
            ptr::null_mut(),
            0,
            None,
            None,
            0,
            ptr::null_mut(),
        );
        if needed < std::mem::size_of::<AudioBufferList>() {
            return false;
        }
        // The list's buffers follow its count; room for them all, aligned as the struct is.
        let mut storage = vec![0u64; needed.div_ceil(8)];
        let list = storage.as_mut_ptr().cast::<AudioBufferList>();
        let mut block: *mut CMBlockBuffer = ptr::null_mut();
        let status = sample_buffer.audio_buffer_list_with_retained_block_buffer(
            ptr::null_mut(),
            list,
            needed,
            None,
            None,
            0,
            &mut block,
        );
        if status != 0 {
            return false;
        }
        // The block buffer holds the samples alive while they are read.
        let _block = NonNull::new(block).map(|block| CFRetained::from_raw(block));
        let count = (*list).mNumberBuffers as usize;
        let buffers = std::slice::from_raw_parts((*list).mBuffers.as_ptr(), count);
        let samples = |buffer: &AudioBuffer| {
            std::slice::from_raw_parts(
                buffer.mData.cast::<f32>(),
                buffer.mDataByteSize as usize / std::mem::size_of::<f32>(),
            )
        };
        if non_interleaved {
            let Some(left) = buffers.first() else {
                return false;
            };
            let left = samples(left);
            let right = buffers.get(1).map(samples).unwrap_or(left);
            for (l, r) in left.iter().zip(right.iter()) {
                out.push(*l);
                out.push(*r);
            }
        } else {
            let Some(buffer) = buffers.first() else {
                return false;
            };
            let data = samples(buffer);
            match channels {
                0 => return false,
                1 => {
                    for sample in data {
                        out.push(*sample);
                        out.push(*sample);
                    }
                }
                _ => {
                    for frame in data.chunks_exact(channels) {
                        out.push(frame[0]);
                        out.push(frame[1]);
                    }
                }
            }
        }
        true
    }
}

/// A running capture: the stream, its output, and the queue the output is called on.
/// Dropping it stops the stream.
pub struct Capture {
    stream: Retained<SCStream>,
    _output: Retained<AudioOutput>,
    _queue: dispatch2::DispatchRetained<DispatchQueue>,
}

// SAFETY: the stream and output are used from this thread only, through calls the framework
// makes thread safe; the output's own state is behind mutexes.
unsafe impl Send for Capture {}

impl Capture {
    /// Starts capturing `target`, calling `on_audio` with each buffer of interleaved 48 kHz
    /// stereo float on the stream's queue. Fails when the application is not running, the
    /// permission is missing, or the framework refuses.
    pub fn start(
        target: Target,
        on_audio: impl FnMut(&[f32]) + Send + 'static,
    ) -> Result<Capture, String> {
        let content = shareable_content()?;
        let listed = unsafe { content.applications() };
        let applications: Vec<Application> = listed.iter().map(|a| read_application(&a)).collect();
        let Some(index) = select_application(&applications, &target) else {
            return Err(format!(
                "{} is not running, or has no window to capture",
                if target.application.is_empty() {
                    "the application"
                } else {
                    &target.application
                }
            ));
        };
        let application = listed.objectAtIndex(index);
        let displays = unsafe { content.displays() };
        let Some(display) = displays.firstObject() else {
            return Err("no display to capture on".into());
        };
        unsafe {
            let filter = SCContentFilter::initWithDisplay_includingApplications_exceptingWindows(
                SCContentFilter::alloc(),
                &display,
                &NSArray::from_retained_slice(&[application]),
                &NSArray::new(),
            );
            let configuration = SCStreamConfiguration::new();
            configuration.setCapturesAudio(true);
            configuration.setSampleRate(RATE as isize);
            configuration.setChannelCount(CHANNELS as isize);
            configuration.setExcludesCurrentProcessAudio(true);
            // The smallest, slowest picture the stream can carry, since it must carry one.
            configuration.setWidth(2);
            configuration.setHeight(2);
            configuration.setMinimumFrameInterval(CMTime::with_seconds(1.0, 1));
            let stream = SCStream::initWithFilter_configuration_delegate(
                SCStream::alloc(),
                &filter,
                &configuration,
                None,
            );
            let output = AudioOutput::new(Box::new(on_audio));
            let queue = DispatchQueue::new("aspen-sck-audio", None);
            stream
                .addStreamOutput_type_sampleHandlerQueue_error(
                    ProtocolObject::from_ref(&*output),
                    SCStreamOutputType::Audio,
                    Some(&queue),
                )
                .map_err(|error| error.localizedDescription().to_string())?;
            let (send, receive) = mpsc::channel::<Result<(), String>>();
            let started = RcBlock::new(move |error: *mut NSError| {
                let _ = send.send(if error.is_null() {
                    Ok(())
                } else {
                    Err(describe(error))
                });
            });
            stream.startCaptureWithCompletionHandler(Some(&started));
            receive
                .recv_timeout(TIMEOUT)
                .map_err(|_| "ScreenCaptureKit did not start".to_string())??;
            Ok(Capture {
                stream,
                _output: output,
                _queue: queue,
            })
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        unsafe { self.stream.stopCaptureWithCompletionHandler(None) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn application(name: &str, pid: Option<u32>, bundle: &str) -> Application {
        Application {
            name: name.into(),
            pid,
            bundle: bundle.into(),
        }
    }

    #[test]
    fn applications_are_sorted_by_name_and_name_their_settings() {
        let mut listed = vec![
            application("Firefox", Some(10), "org.mozilla.firefox"),
            application("doom", Some(20), "com.id.doom"),
            application("", None, ""),
        ];
        let targets = sort_applications(&mut listed);
        let names: Vec<&str> = targets.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["doom", "Firefox"]);
        assert_eq!(
            targets[0].settings,
            serde_json::json!({ "application": "doom", "pid": 20, "bundle": "com.id.doom" })
        );
    }

    #[test]
    fn the_process_is_the_target_while_it_runs_then_its_bundle_then_its_name() {
        let applications = vec![
            application("doom", Some(20), "com.id.doom"),
            application("doom", Some(99), "com.id.doom"),
            application("Firefox", Some(10), "org.mozilla.firefox"),
        ];
        let target = Target::from_settings(Some(
            r#"{"application":"doom","pid":99,"bundle":"com.id.doom"}"#,
        ))
        .unwrap();
        assert_eq!(select_application(&applications, &target), Some(1));
        let restarted = Target {
            pid: Some(7),
            ..target.clone()
        };
        assert_eq!(select_application(&applications, &restarted), Some(0));
        let by_name = Target {
            pid: None,
            bundle: None,
            application: "Firefox".into(),
        };
        assert_eq!(select_application(&applications, &by_name), Some(2));
        assert_eq!(select_application(&applications, &Target::default()), None);
        assert_eq!(Target::from_settings(None).unwrap(), Target::default());
    }
}
