//! Game audio on Linux, from the PipeWire capture (`pipewire_audio`) to the voice server: the
//! application's sound, 48 kHz stereo float as the capture delivers it, cut into 20 ms frames,
//! encoded as Opus with libopus, and sent as SRTP by an `RtpSender` of its own.
//!
//! The capture hands buffers over on PipeWire's real-time thread, where nothing may block, so
//! they are passed through a bounded channel to a thread of this module's that frames,
//! encodes, and sends; a buffer that finds the channel full is dropped, which costs a few
//! milliseconds of sound rather than a late one on the real-time thread.

use crate::AudioOptions;
use crate::pipewire_audio::{Capture, Target};
use crate::rtp::{Feedback, RtpSender, StreamKind};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

const RATE: u32 = 48_000;
const CHANNELS: usize = 2;
/// Opus frames of 20 ms, the size every WebRTC receiver takes without negotiation.
const FRAME_MS: u32 = 20;
const FRAME_SAMPLES: usize = (RATE / 1000 * FRAME_MS) as usize * CHANNELS;
/// How many capture buffers may wait for the encoder before the newest are dropped.
const QUEUE_BUFFERS: usize = 64;
/// 128 kbps stereo carries music and a game's transients cleanly; lower rates smear cymbals
/// and other dense high-frequency sound.
const DEFAULT_BITRATE_KBPS: u32 = 128;
/// The largest Opus packet asked for, well above what the bitrates allow.
const MAX_PACKET_BYTES: usize = 4000;

/// The running capture of sound alone.
struct Running {
    capture: Capture,
    sender: Arc<RtpSender>,
    encoder: Option<JoinHandle<()>>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);

/// Audio runs at its set rate; the receiver's estimate steers nothing here.
struct NoFeedback;

impl Feedback for NoFeedback {
    fn set_bitrate_kbps(&self, _: u32) {}
}

/// Starts capturing the application `options` name, encoding and sending its sound. One
/// capture at a time.
pub fn start(options: &AudioOptions) -> Result<(), String> {
    let mut running = RUNNING.lock().expect("audio lock");
    if running.is_some() {
        return Err("a capture is already running".into());
    }
    let target = Target::from_settings(options.settings.as_deref())?;
    let bitrate_kbps = options
        .bitrate_kbps
        .unwrap_or(DEFAULT_BITRATE_KBPS)
        .clamp(32, 320);
    let sender = RtpSender::start(
        &options.rtp,
        StreamKind::Audio,
        Arc::new(NoFeedback),
        bitrate_kbps,
        bitrate_kbps,
    )?;
    let mut encoder = opus::Encoder::new(RATE, opus::Channels::Stereo, opus::Application::Audio)
        .map_err(|e| format!("Opus encoder: {e}"))?;
    encoder
        .set_bitrate(opus::Bitrate::Bits((bitrate_kbps * 1000) as i32))
        .map_err(|e| format!("Opus bitrate: {e}"))?;
    let (queue, buffers) = sync_channel::<Vec<f32>>(QUEUE_BUFFERS);
    let capture = match Capture::start(target, deliver(queue)) {
        Ok(capture) => capture,
        Err(reason) => {
            sender.stop();
            return Err(reason);
        }
    };
    let thread = std::thread::Builder::new()
        .name("aspen-opus".into())
        .spawn({
            let sender = Arc::clone(&sender);
            move || encode(buffers, encoder, &sender)
        })
        .map_err(|e| format!("could not start the encoder thread: {e}"))?;
    *running = Some(Running {
        capture,
        sender,
        encoder: Some(thread),
    });
    Ok(())
}

/// Stops the capture, if one runs.
pub fn stop() {
    let taken = RUNNING.lock().expect("audio lock").take();
    if let Some(mut running) = taken {
        // Ending the capture closes the queue, which ends the encoder thread.
        drop(running.capture);
        if let Some(thread) = running.encoder.take() {
            let _ = thread.join();
        }
        running.sender.stop();
    }
}

/// What the capture calls with each buffer: a copy onto the queue, or nothing when the queue
/// is full.
fn deliver(queue: SyncSender<Vec<f32>>) -> impl FnMut(&[f32]) + Send + 'static {
    move |samples| {
        let _ = queue.try_send(samples.to_vec());
    }
}

/// The encoder thread: frames what arrives and sends each Opus packet, until the queue closes.
fn encode(buffers: Receiver<Vec<f32>>, mut encoder: opus::Encoder, sender: &RtpSender) {
    let mut framer = Framer::new();
    let mut packet = vec![0u8; MAX_PACKET_BYTES];
    let mut frames_sent: u64 = 0;
    while let Ok(buffer) = buffers.recv() {
        framer.push(&buffer, |frame| {
            let Ok(length) = encoder.encode_float(frame, &mut packet) else {
                return;
            };
            // Presentation time counts frames, so the RTP clock advances exactly 20 ms a
            // packet whatever the capture's buffer sizes.
            let pts_us = frames_sent as f64 * f64::from(FRAME_MS) * 1000.0;
            sender.send_frame(&packet[..length], pts_us);
            frames_sent += 1;
        });
    }
}

/// Cuts a stream of interleaved samples into whole frames of `FRAME_SAMPLES`, carrying what
/// is left over to the next buffer.
struct Framer {
    pending: Vec<f32>,
}

impl Framer {
    fn new() -> Self {
        Framer {
            pending: Vec::with_capacity(FRAME_SAMPLES * 2),
        }
    }

    /// Takes `samples`, calling `emit` with each whole frame they complete, in order.
    fn push(&mut self, samples: &[f32], mut emit: impl FnMut(&[f32])) {
        self.pending.extend_from_slice(samples);
        let (frames, _) = self.pending.as_chunks::<FRAME_SAMPLES>();
        for frame in frames {
            emit(frame);
        }
        let whole = frames.len() * FRAME_SAMPLES;
        self.pending.drain(..whole);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_whole_and_in_order_whatever_the_buffer_sizes() {
        let mut framer = Framer::new();
        let mut frames: Vec<Vec<f32>> = Vec::new();
        let mut next = 0u32;
        // Buffers of odd sizes around and across the frame size, numbered so order shows.
        for size in [1000, 3000, 1920, 17, 1903, 5760] {
            let buffer: Vec<f32> = (0..size)
                .map(|_| {
                    next += 1;
                    next as f32
                })
                .collect();
            framer.push(&buffer, |frame| frames.push(frame.to_vec()));
        }
        assert_eq!(frames.len(), 13600 / FRAME_SAMPLES);
        for (index, frame) in frames.iter().enumerate() {
            assert_eq!(frame.len(), FRAME_SAMPLES);
            assert_eq!(frame[0], (index * FRAME_SAMPLES + 1) as f32);
            assert_eq!(
                frame[FRAME_SAMPLES - 1],
                ((index + 1) * FRAME_SAMPLES) as f32
            );
        }
        assert_eq!(framer.pending.len(), 13600 % FRAME_SAMPLES);
    }

    #[test]
    fn a_frame_is_twenty_milliseconds_of_stereo() {
        assert_eq!(FRAME_SAMPLES, 960 * 2);
    }
}
