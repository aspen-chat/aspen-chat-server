//! The engine as the iOS and Android layers hold it: one object whose methods are the call's
//! commands, and whose observer reads each snapshot of the call's state. The call runs on a
//! tokio current-thread runtime on a thread of its own, where the platform's `Http`, `Socket`,
//! and `PeerConnection` implementations are called from; the methods here only queue commands,
//! so they may be called from any thread and never block.

use crate::call::{Call, Command, Options, Platform};
use crate::peer_connection::TrackId;
use crate::state::{CallObserver, CallState, SessionEndReason};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use uuid::Uuid;
use voice_protocol::signal::{MediaSource, WatchedVideo};

/// Keeps the latest snapshot for `Engine::state` and passes each on to the platform's observer.
struct Keeping {
    latest: Arc<Mutex<CallState>>,
    observer: Arc<dyn CallObserver>,
}

impl CallObserver for Keeping {
    fn state_changed(&self, state: &CallState) {
        *self.latest.lock().expect("state lock") = state.clone();
        self.observer.state_changed(state);
    }
}

pub struct Engine {
    commands: mpsc::UnboundedSender<Command>,
    latest: Arc<Mutex<CallState>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Engine {
    /// Starts the engine's thread. `observer` is called on that thread with every change.
    pub fn start(platform: Platform, options: Options, observer: Arc<dyn CallObserver>) -> Self {
        let latest = Arc::new(Mutex::new(CallState::default()));
        let keeping = Arc::new(Keeping {
            latest: latest.clone(),
            observer,
        });
        let (commands, receiver) = mpsc::unbounded_channel();
        let call = Call::new(platform, options, keeping);
        let thread = std::thread::Builder::new()
            .name("aspen-call".to_string())
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("the call's runtime builds")
                    .block_on(call.run(receiver));
            })
            .expect("the call's thread starts");
        Self {
            commands,
            latest,
            thread: Some(thread),
        }
    }

    /// The latest snapshot the observer was given.
    pub fn state(&self) -> CallState {
        self.latest.lock().expect("state lock").clone()
    }

    fn send(&self, command: Command) {
        // Refused only once the thread has ended, when there is no call to command.
        let _ = self.commands.send(command);
    }

    pub fn join(&self, channel: Uuid) {
        self.send(Command::Join { channel });
    }

    pub fn leave(&self) {
        self.send(Command::Leave);
    }

    pub fn set_muted(&self, muted: bool) {
        self.send(Command::SetMuted(muted));
    }

    pub fn set_deafened(&self, deafened: bool) {
        self.send(Command::SetDeafened(deafened));
    }

    pub fn set_server_muted(&self, server_muted: bool) {
        self.send(Command::SetServerMuted(server_muted));
    }

    pub fn set_microphone(&self, track: Option<TrackId>) {
        self.send(Command::SetMicrophone(track));
    }

    pub fn start_source(&self, source: MediaSource, track: TrackId) {
        self.send(Command::StartSource { source, track });
    }

    pub fn stop_source(&self, source: MediaSource) {
        self.send(Command::StopSource(source));
    }

    pub fn set_watched(&self, video: Vec<WatchedVideo>) {
        self.send(Command::SetWatched(video));
    }

    pub fn set_user_volume(&self, user: Uuid, source: MediaSource, gain: f64) {
        self.send(Command::SetUserVolume { user, source, gain });
    }

    pub fn set_user_muted(&self, user: Uuid, source: MediaSource, muted: bool) {
        self.send(Command::SetUserMuted {
            user,
            source,
            muted,
        });
    }

    pub fn session_ended(&self, session: Uuid, channel: Uuid, reason: SessionEndReason) {
        self.send(Command::SessionEnded {
            session,
            channel,
            reason,
        });
    }

    pub fn acknowledge_end(&self) {
        self.send(Command::AcknowledgeEnd);
    }

    pub fn clear_state_refusal(&self) {
        self.send(Command::ClearStateRefusal);
    }

    pub fn clear_source_refusal(&self) {
        self.send(Command::ClearSourceRefusal);
    }
}

/// Leaves any call and stops the thread, waiting for it.
impl Drop for Engine {
    fn drop(&mut self) {
        self.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
