//! The link to the API server over NATS: reports go into the report stream in the order they are
//! made, each on its subject (`VoiceReport::subject`), and commands for this server come in on
//! its own subject.

use crate::config::{NatsAuth, NatsUser, VoiceServerConfig};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{Notify, mpsc};
use tracing::{error, warn};
use uuid::Uuid;
use voice_protocol::control::{
    LOAD_REPORT_INTERVAL_SECONDS, SNAPSHOT_INTERVAL_SECONDS, VoiceCommand, VoiceReport,
    command_subject,
};

#[derive(Clone)]
pub struct Reporter {
    client: async_nats::Client,
    /// Reports waiting to be published, which one task publishes in the order they were sent.
    /// Sending under the lock is what gives a report its place, so the reports `report_with`
    /// makes under it go out together, with none between them.
    queue: Arc<Mutex<mpsc::UnboundedSender<VoiceReport>>>,
    /// Told each time the connection to NATS comes back after being lost.
    reconnected: Arc<Notify>,
}

impl Reporter {
    /// What the subjects of this server's replies start with, rather than NATS's shared
    /// `_INBOX`, so a NATS user for the server may be allowed its own replies and no one else's.
    pub fn inbox_prefix(server: Uuid) -> String {
        format!("_INBOX_voice.{server}")
    }

    /// Connects to NATS as `config` says, signing in with `auth`, and warns when the connection
    /// is unencrypted beyond this machine.
    pub async fn connect(config: &VoiceServerConfig, auth: NatsAuth) -> anyhow::Result<Self> {
        let server = config.id;
        let reconnected = Arc::new(Notify::new());
        let connected_before = Arc::new(AtomicBool::new(false));
        let options = match auth {
            NatsAuth::User(NatsUser { user, password }) => {
                async_nats::ConnectOptions::with_user_and_password(user, password)
            }
            NatsAuth::Token(token) => async_nats::ConnectOptions::with_token(token),
        };
        let options = aspen_tls::nats_options(options, config.nats.tls.as_ref())
            .custom_inbox_prefix(Self::inbox_prefix(server))
            .event_callback({
                let reconnected = Arc::clone(&reconnected);
                move |event| {
                    let reconnected = Arc::clone(&reconnected);
                    let connected_before = Arc::clone(&connected_before);
                    async move {
                        // The first connection is not a return.
                        if matches!(event, async_nats::Event::Connected)
                            && connected_before.swap(true, Ordering::Relaxed)
                        {
                            reconnected.notify_one();
                        }
                    }
                }
            });
        let client = async_nats::connect_with_options(&config.nats_url, options).await?;
        aspen_tls::warn_if_nats_unencrypted(&config.nats_url, config.nats.tls.as_ref(), &client);
        let (queue, mut waiting) = mpsc::unbounded_channel::<VoiceReport>();
        let stream = async_nats::jetstream::new(client.clone());
        tokio::spawn(async move {
            while let Some(report) = waiting.recv().await {
                publish(&stream, &report.subject(server), &report).await;
            }
        });
        Ok(Self {
            client,
            queue: Arc::new(Mutex::new(queue)),
            reconnected,
        })
    }

    /// The NATS connection, for anything else this server follows there.
    pub fn client(&self) -> async_nats::Client {
        self.client.clone()
    }

    /// Sends one report, after every report sent before it. A report the stream does not take
    /// is logged rather than returned: the next snapshot repairs what it would have changed,
    /// and the call itself must go on.
    pub fn report(&self, report: VoiceReport) {
        self.report_with(|| vec![report]);
    }

    /// Sends the reports `make` returns, made while no other report can be sent, so they go out
    /// together and each describes things as of its place in the order. A snapshot is made this
    /// way. `make` may lock the rooms and their participants, so nothing reports while holding
    /// either lock.
    pub fn report_with(&self, make: impl FnOnce() -> Vec<VoiceReport>) {
        let queue = self.queue.lock().expect("report queue lock");
        for report in make() {
            // The receiver lives as long as the process.
            let _ = queue.send(report);
        }
    }

    /// Publishes the load every `LOAD_REPORT_INTERVAL_SECONDS`, forever.
    pub fn spawn_load_reports(self, server: Uuid, participants: impl Fn() -> u32 + Send + 'static) {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(LOAD_REPORT_INTERVAL_SECONDS));
            loop {
                interval.tick().await;
                self.report(VoiceReport::Load {
                    server,
                    participants: participants(),
                });
            }
        });
    }

    /// Reports the snapshot `snapshot` makes every `SNAPSHOT_INTERVAL_SECONDS`, and each time
    /// the connection to NATS comes back, forever. The first waits a full interval: a server
    /// that has just started holds no calls, and the calls it held before are ended by their
    /// people rejoining, which carries them on here.
    pub fn spawn_snapshots(self, snapshot: impl Fn() -> Vec<VoiceReport> + Send + 'static) {
        tokio::spawn(async move {
            let period = Duration::from_secs(SNAPSHOT_INTERVAL_SECONDS);
            let mut interval =
                tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    () = self.reconnected.notified() => {}
                }
                self.report_with(&snapshot);
            }
        });
    }

    /// Delivers every command addressed to `server` to `handle`, forever.
    pub async fn spawn_commands(
        &self,
        server: Uuid,
        handle: impl Fn(VoiceCommand) + Send + Sync + 'static,
    ) -> anyhow::Result<()> {
        use futures_util::StreamExt;
        let mut commands = self.client.subscribe(command_subject(server)).await?;
        tokio::spawn(async move {
            while let Some(message) = commands.next().await {
                match serde_json::from_slice::<VoiceCommand>(&message.payload) {
                    Ok(command) => handle(command),
                    Err(e) => warn!(error = e.to_string(), "unreadable voice command ignored"),
                }
            }
            error!("the voice command subscription ended");
        });
        Ok(())
    }
}

/// Publishes one report to the stream. Its place in the stream is fixed once it is sent, so the
/// stream's acknowledgement is awaited apart, keeping the next report from waiting on it.
async fn publish(stream: &async_nats::jetstream::Context, subject: &str, report: &VoiceReport) {
    let payload = match serde_json::to_vec(report) {
        Ok(payload) => payload,
        Err(e) => {
            error!(error = e.to_string(), "voice report did not serialize");
            return;
        }
    };
    match stream.publish(subject.to_string(), payload.into()).await {
        Ok(acknowledgement) => {
            tokio::spawn(async move {
                if let Err(e) = acknowledgement.await {
                    warn!(
                        error = e.to_string(),
                        "the report stream did not take a voice report"
                    );
                }
            });
        }
        Err(e) => warn!(error = e.to_string(), "voice report was not sent"),
    }
}
