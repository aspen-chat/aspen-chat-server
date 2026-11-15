//! The voice server's media, through str0m: threads that each hold the WebRTC connections of the
//! calls given to them and forward what each participant sends to the others.
//!
//! A call lives on one thread. Every participant holds one connection (`ConnectionKind::Call`),
//! and one more (`Capture`) while the desktop shell's game capture helper sends for them. The
//! server is the only side that offers: what a participant wants changed is a command, the
//! thread makes the change and offers it (`Event::Offer`), and a connection never has two offers
//! outstanding, changes made meanwhile waiting for the next.
//!
//! What a participant receives is bounded whatever the size of the call: `audio_slots` audio
//! sections carrying whichever microphones are loudest (`Event::Slots` says whom each carries), and
//! a video section for each video it watches (at most `max_watched_video`), with a share's sound
//! beside its picture.
//!
//! The rooms decide who may do what; this crate only does what they ask, and tells them what
//! happened (a connection up or failed, a source's first media, speaking).

mod connection;
mod keyframe;
mod levels;
mod net;
mod sdp;
mod thread;

pub use thread::Census;

use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use uuid::Uuid;
use voice_protocol::signal::{ConnectionKind, MediaSource, Section, SlotAssignment};

/// A participant's place on a media thread, chosen by the rooms; one user rejoining from another
/// socket is another participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ParticipantId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CallId(pub Uuid);

/// One interface media is received on, and the address clients are told to send to in its
/// place (resolved: a host name is looked up before the threads start).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaAddress {
    pub ip: IpAddr,
    /// `None` announces `ip` itself.
    pub announced: Option<IpAddr>,
}

#[derive(Debug, Clone)]
pub struct MediaConfig {
    pub addresses: Vec<MediaAddress>,
    /// Thread `i` listens on `first_port + i`, over UDP and TCP, on every address.
    pub first_port: u16,
    pub threads: usize,
    pub audio_slots: u8,
    pub max_watched_video: usize,
}

/// What the rooms ask of a media thread.
#[derive(Debug)]
pub enum Command {
    /// Seats a participant in a call on this thread, every participant of a call on one thread,
    /// and offers its connection: its audio slots, and nothing to send yet.
    Join {
        call: CallId,
        participant: ParticipantId,
        user: Uuid,
        events: Events,
    },
    Leave {
        participant: ParticipantId,
    },
    /// Adds a section the participant sends `source` on, on its call connection or, `external`,
    /// on its capture connection.
    Send {
        participant: ParticipantId,
        source: MediaSource,
        external: bool,
    },
    StopSending {
        participant: ParticipantId,
        source: MediaSource,
    },
    Answer {
        participant: ParticipantId,
        connection: ConnectionKind,
        revision: u64,
        sdp: String,
    },
    /// The video the participant shows, by owner and source; its own (a share it sends from
    /// outside the browser) included when it wants a preview.
    Watch {
        participant: ParticipantId,
        video: Vec<(Uuid, MediaSource)>,
    },
    /// Stops or resumes one section the participant receives, with no renegotiation.
    SetReceiving {
        participant: ParticipantId,
        mid: String,
        receiving: bool,
    },
    /// Stops forwarding what the participant sends of `source`, as a mute does.
    SetSourcePaused {
        participant: ParticipantId,
        source: MediaSource,
        paused: bool,
    },
    /// Stops every audio slot to the participant.
    SetDeafened {
        participant: ParticipantId,
        deafened: bool,
    },
    RestartIce {
        participant: ParticipantId,
        connection: ConnectionKind,
    },
    Census {
        reply: tokio::sync::oneshot::Sender<Census>,
    },
}

/// What a media thread tells the rooms, about one participant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// An offer to send the client; `sections` names every section it holds.
    Offer {
        participant: ParticipantId,
        connection: ConnectionKind,
        revision: u64,
        sdp: String,
        sections: Vec<Section>,
    },
    Connected {
        participant: ParticipantId,
        connection: ConnectionKind,
    },
    Failed {
        participant: ParticipantId,
        connection: ConnectionKind,
        reason: Failure,
    },
    /// The first media of a source the participant sends.
    Live {
        participant: ParticipantId,
        source: MediaSource,
    },
    Speaking {
        participant: ParticipantId,
        speaking: bool,
    },
    /// Whom the participant's audio slots carry from now on, those that changed.
    Slots {
        participant: ParticipantId,
        assignments: Vec<SlotAssignment>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// An offer went unanswered for `ANSWER_TIMEOUT`.
    AnswerTimedOut,
    /// The answer could not be applied.
    BadAnswer,
    /// ICE or DTLS ended the connection.
    Disconnected,
    /// str0m failed an internal check on this connection; it was dropped and nothing else.
    Internal,
}

pub type Events = tokio::sync::mpsc::UnboundedSender<Event>;

/// Every media thread, and which the next call goes to.
pub struct Media {
    threads: Vec<thread::Handle>,
    next: AtomicUsize,
    stopped: std::sync::Arc<thread::Stopped>,
}

impl Media {
    /// Binds every thread's sockets and starts them; any address or port that cannot be bound is
    /// an error, and nothing is left running.
    pub fn start(config: MediaConfig) -> std::io::Result<Self> {
        // str0m's DTLS and SRTP through AWS-LC; installing it again does nothing.
        str0m::crypto::from_feature_flags().install_process_default();
        let stopped = std::sync::Arc::new(thread::Stopped::default());
        let threads = (0..config.threads.max(1))
            .map(|index| thread::Handle::start(&config, index, std::sync::Arc::clone(&stopped)))
            .collect::<std::io::Result<_>>()?;
        Ok(Self {
            threads,
            next: AtomicUsize::new(0),
            stopped,
        })
    }

    pub fn threads(&self) -> usize {
        self.threads.len()
    }

    /// The thread a new call goes to, round robin.
    pub fn assign(&self) -> usize {
        self.next.fetch_add(1, Ordering::Relaxed) % self.threads.len()
    }

    /// Hands `command` to thread `index`; `false` when that thread has stopped.
    pub fn send(&self, index: usize, command: Command) -> bool {
        self.threads[index].send(command)
    }

    /// Resolves once any thread has stopped, which a server treats as fatal.
    pub async fn stopped(&self) {
        self.stopped.wait().await;
    }
}
