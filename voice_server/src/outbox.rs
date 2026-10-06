//! A signalling socket's outbox: the frames waiting to be written to the client. The rooms put
//! frames in without waiting, so no call waits on a slow socket, and the socket's writer takes
//! them out. What may wait is bounded by bytes rather than frames, since one frame may be a
//! 256 KiB transfer signal and the next a few bytes: a client that stops reading cannot make
//! the server hold an unbounded pile of everyone else's frames for it. A frame that would pass
//! the bound hangs the socket up instead, and the client, finding its socket closed, rejoins.

use axum::extract::ws::Utf8Bytes;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{mpsc, watch};
use tracing::warn;
use voice_protocol::signal::ServerMessage;

/// The most bytes of frames one socket may have waiting: thirty-odd of the largest transfer
/// signals, or many thousands of everyday frames, far more than a client that is reading ever
/// lets build up.
pub const OUTBOX_BYTES: usize = 8 * 1024 * 1024;

struct Shared {
    /// Bytes of frames sent and not yet taken by the writer.
    queued: AtomicUsize,
    limit: usize,
    /// Becomes true when the socket is to close.
    hung_up: watch::Sender<bool>,
}

/// The sending side, cloned wherever the socket's participant is reached.
#[derive(Clone)]
pub struct Outbox {
    frames: mpsc::UnboundedSender<Utf8Bytes>,
    shared: Arc<Shared>,
}

/// The writer's side.
pub struct OutboxReader {
    frames: mpsc::UnboundedReceiver<Utf8Bytes>,
    shared: Arc<Shared>,
}

/// A frame as it is written to every socket it goes to, so a frame for many is serialized once.
pub fn frame_text(message: &ServerMessage) -> Utf8Bytes {
    serde_json::to_string(message)
        .expect("frames serialize")
        .into()
}

impl Outbox {
    /// An outbox holding at most `limit` bytes of frames.
    pub fn new(limit: usize) -> (Outbox, OutboxReader) {
        let (frames, receiver) = mpsc::unbounded_channel();
        let shared = Arc::new(Shared {
            queued: AtomicUsize::new(0),
            limit,
            hung_up: watch::Sender::new(false),
        });
        (
            Outbox {
                frames,
                shared: Arc::clone(&shared),
            },
            OutboxReader {
                frames: receiver,
                shared,
            },
        )
    }

    pub fn send(&self, message: &ServerMessage) {
        self.send_text(frame_text(message));
    }

    /// Queues a frame already serialized. Nothing is queued once the socket is hung up; a frame
    /// that would pass the bound hangs it up.
    pub fn send_text(&self, text: Utf8Bytes) {
        if *self.shared.hung_up.borrow() {
            return;
        }
        let len = text.len();
        let queued = self.shared.queued.fetch_add(len, Ordering::AcqRel) + len;
        if queued > self.shared.limit {
            self.shared.queued.fetch_sub(len, Ordering::AcqRel);
            warn!(
                queued,
                limit = self.shared.limit,
                "a signalling socket fell too far behind; closing it"
            );
            self.hang_up();
            return;
        }
        if self.frames.send(text).is_err() {
            // The writer is gone, so the socket is too.
            self.shared.queued.fetch_sub(len, Ordering::AcqRel);
        }
    }

    /// Asks the socket to close: what is already queued is still written, if the client reads
    /// it soon, and nothing more is queued.
    pub fn hang_up(&self) {
        self.shared.hung_up.send_replace(true);
    }

    /// Resolves once the socket has been hung up.
    pub async fn hung_up(&self) {
        let mut hung_up = self.shared.hung_up.subscribe();
        let _ = hung_up.wait_for(|hung_up| *hung_up).await;
    }
}

impl OutboxReader {
    /// The next frame to write, or `None` once every sender is gone and the queue is empty.
    pub async fn recv(&mut self) -> Option<Utf8Bytes> {
        let text = self.frames.recv().await?;
        self.shared.queued.fetch_sub(text.len(), Ordering::AcqRel);
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_reader_that_falls_too_far_behind_is_hung_up() {
        let (outbox, mut reader) = Outbox::new(64);
        let frame = |n: usize| Utf8Bytes::from("x".repeat(n));
        outbox.send_text(frame(40));
        outbox.send_text(frame(20));
        // Taking a frame out makes room again.
        assert_eq!(reader.recv().await.map(|t| t.len()), Some(40));
        outbox.send_text(frame(40));
        assert!(!*outbox.shared.hung_up.borrow());
        // 20 + 40 queued; 10 more passes the bound.
        outbox.send_text(frame(10));
        assert!(*outbox.shared.hung_up.borrow());
        outbox.hung_up().await;
        // Nothing more is queued once hung up, and what was queued is still there.
        outbox.send_text(frame(1));
        drop(outbox);
        assert_eq!(reader.recv().await.map(|t| t.len()), Some(20));
        assert_eq!(reader.recv().await.map(|t| t.len()), Some(40));
        assert_eq!(reader.recv().await, None);
    }
}
