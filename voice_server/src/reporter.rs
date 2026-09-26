//! The link to the API server over NATS: reports go out on the shared report subject, and
//! commands for this server come in on its own subject.

use std::time::Duration;
use tracing::{error, warn};
use uuid::Uuid;
use voice_protocol::control::{
    LOAD_REPORT_INTERVAL_SECONDS, REPORT_SUBJECT, VoiceCommand, VoiceReport, command_subject,
};

#[derive(Clone)]
pub struct Reporter {
    client: async_nats::Client,
}

impl Reporter {
    pub async fn connect(url: &str, token: &str) -> anyhow::Result<Self> {
        let client = async_nats::connect_with_options(
            url,
            async_nats::ConnectOptions::new().token(token.to_string()),
        )
        .await?;
        Ok(Self { client })
    }

    /// Sends one report. A failure is logged rather than returned: the API server's reaper
    /// covers a report that never arrives, and the call itself must go on.
    pub async fn report(&self, report: VoiceReport) {
        let payload = match serde_json::to_vec(&report) {
            Ok(payload) => payload,
            Err(e) => {
                error!(error = e.to_string(), "voice report did not serialize");
                return;
            }
        };
        if let Err(e) = self.client.publish(REPORT_SUBJECT, payload.into()).await {
            warn!(error = e.to_string(), "voice report was not delivered");
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
                })
                .await;
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
