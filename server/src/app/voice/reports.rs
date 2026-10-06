//! The voice servers' reports, read from the report stream (`voice_protocol::control`).
//!
//! Every lane, of reports and of speaking changes, has one durable consumer, shared by every
//! API server and allowing one unacknowledged report at a time, so the reports about a channel
//! are applied once each, in the order they were sent, by whichever API server takes each one,
//! while the lanes are worked through side by side. Every API server reads every lane, applying
//! at most half as many reports at once as it has database connections, so requests always have
//! the other half. A report is acknowledged once it has been applied. The stream keeps reports until they are
//! applied, so a report sent while every API server is away waits for one; what is lost anyway
//! (the stream is in memory, and a report that keeps failing is dropped) is repaired by the
//! voice server's next snapshot.

use super::sessions::apply_report;
use crate::app;
use crate::app::VoiceServerId;
use crate::app::context::GlobalServerContext;
use crate::app::events::Publishing;
use async_nats::jetstream;
use async_nats::jetstream::AckKind;
use async_nats::jetstream::consumer::pull::MessagesErrorKind;
use async_nats::jetstream::consumer::{AckPolicy, DeliverPolicy, pull};
use async_nats::jetstream::stream::{DiscardPolicy, RetentionPolicy, StorageType};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{error, warn};
use voice_protocol::control::{
    REPORT_PARTITIONS, REPORT_STREAM, REPORT_SUBJECT_ROOT, SPEAKING_SUBJECT_ROOT, VoiceReport,
    subject_server,
};

/// How long a report waits in the stream for an API server before it is let go. Long enough
/// for any outage of the API servers to be over; a snapshot repairs whatever is let go.
const REPORT_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// The most the stream holds, its oldest reports going first past it.
const REPORT_MAX_BYTES: i64 = 1024 * 1024 * 1024;

/// How long a report may go unacknowledged before it is handed out again: an API server that
/// stopped while applying it holds up its lane this long.
const ACK_WAIT: Duration = Duration::from_secs(10);

/// How many times a report that failed for a reason that may pass is tried before it is
/// dropped, and how long it waits between tries. The later reports of its lane wait too.
const MAX_ATTEMPTS: i64 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(1);

/// The two kinds of lane, read the same way.
#[derive(Clone, Copy)]
enum Kind {
    Reports,
    Speaking,
}

impl Kind {
    fn root(self) -> &'static str {
        match self {
            Kind::Reports => REPORT_SUBJECT_ROOT,
            Kind::Speaking => SPEAKING_SUBJECT_ROOT,
        }
    }

    /// The durable consumer lane `partition` of this kind is read through.
    fn consumer(self, partition: u8) -> String {
        match self {
            Kind::Reports => format!("voice-reports-{partition}"),
            Kind::Speaking => format!("voice-speaking-{partition}"),
        }
    }
}

/// Makes the report stream, and starts reading every lane.
pub async fn spawn_report_listener(state: GlobalServerContext) -> app::Result<()> {
    state
        .nats_context
        .create_or_update_stream(jetstream::stream::Config {
            name: REPORT_STREAM.to_string(),
            subjects: vec![
                format!("{REPORT_SUBJECT_ROOT}.*.*"),
                format!("{SPEAKING_SUBJECT_ROOT}.*.*"),
            ],
            retention: RetentionPolicy::WorkQueue,
            storage: StorageType::Memory,
            max_age: REPORT_MAX_AGE,
            max_bytes: REPORT_MAX_BYTES,
            discard: DiscardPolicy::Old,
            ..Default::default()
        })
        .await?;
    let stream = state.nats_context.get_stream(REPORT_STREAM).await?;
    let applying = Arc::new(Semaphore::new(
        (state.connection_pool.status().max_size / 2).max(1),
    ));
    for kind in [Kind::Reports, Kind::Speaking] {
        for partition in 0..REPORT_PARTITIONS {
            tokio::spawn(read_lane(
                state.clone(),
                stream.clone(),
                Arc::clone(&applying),
                kind,
                partition,
            ));
        }
    }
    Ok(())
}

/// Applies one lane's reports for as long as the server runs, starting again after a failure.
async fn read_lane(
    state: GlobalServerContext,
    stream: jetstream::stream::Stream,
    applying: Arc<Semaphore>,
    kind: Kind,
    partition: u8,
) {
    loop {
        if let Err(e) = read(&state, &stream, &applying, kind, partition).await {
            warn!(
                lane = kind.consumer(partition),
                error = e.to_string(),
                "reading a lane of voice reports stopped; starting again"
            );
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn read(
    state: &GlobalServerContext,
    stream: &jetstream::stream::Stream,
    applying: &Semaphore,
    kind: Kind,
    partition: u8,
) -> anyhow::Result<()> {
    let name = kind.consumer(partition);
    let consumer = stream
        .get_or_create_consumer(
            &name,
            pull::Config {
                durable_name: Some(name.clone()),
                filter_subject: format!("{}.{partition}.*", kind.root()),
                deliver_policy: DeliverPolicy::All,
                ack_policy: AckPolicy::Explicit,
                ack_wait: ACK_WAIT,
                // One at a time across every API server: the next report is handed out only
                // once this one is acknowledged, which is what keeps the order.
                max_ack_pending: 1,
                ..Default::default()
            },
        )
        .await?;
    let mut messages = consumer.messages().await?;
    while let Some(message) = messages.next().await {
        match message {
            Ok(message) => {
                // The report is out while it waits its turn, so the stream is told it is being
                // worked on, or it would hand the report to another API server meanwhile.
                let _permit = loop {
                    tokio::select! {
                        permit = applying.acquire() => break permit?,
                        () = tokio::time::sleep(ACK_WAIT / 2) => {
                            message.ack_with(AckKind::Progress).await.map_err(|e| anyhow::anyhow!(e))?;
                        }
                    }
                };
                handle(state, &message).await;
            }
            // A consumer that was removed, or one that is not what this reads, ends the
            // reading; anything else (a missed heartbeat, a refused pull while another API
            // server holds the one report out) passes.
            Err(e)
                if matches!(
                    e.kind(),
                    MessagesErrorKind::ConsumerDeleted | MessagesErrorKind::PushBasedConsumer
                ) =>
            {
                return Err(e.into());
            }
            Err(e) => tracing::debug!(
                lane = name,
                error = e.to_string(),
                "a lane of voice reports was interrupted"
            ),
        }
    }
    Ok(())
}

/// Applies one report and tells the stream what became of it.
async fn handle(state: &GlobalServerContext, message: &jetstream::Message) {
    let (attempt, published) = match message.info() {
        Ok(info) => (
            info.delivered,
            DateTime::from_timestamp(info.published.unix_timestamp(), info.published.nanosecond())
                .unwrap_or_else(Utc::now),
        ),
        Err(e) => {
            warn!(
                error = e.to_string(),
                "a voice report came without its details"
            );
            (1, Utc::now())
        }
    };
    let outcome = match serde_json::from_slice::<VoiceReport>(&message.payload) {
        Err(e) => {
            warn!(error = e.to_string(), "unreadable voice report dropped");
            AckKind::Ack
        }
        Ok(report) => {
            let kind: &'static str = (&report).into();
            // Where it came from is the subject's server, which a voice server's NATS user is
            // limited to, and the report must be on the subject that server would send it on.
            let from = subject_server(&message.subject)
                .filter(|server| report.subject(*server) == message.subject.as_str());
            let applied = match from {
                Some(from) => {
                    apply_report(state, report, VoiceServerId::from(from), published).await
                }
                None => {
                    warn!(
                        subject = message.subject.as_str(),
                        "a voice report on a subject it does not belong on was dropped"
                    );
                    Ok(())
                }
            };
            match applied {
                Ok(()) => {
                    metrics::counter!(aspen_metrics::api::VOICE_REPORTS_APPLIED, "report" => kind)
                        .increment(1);
                    metrics::histogram!(aspen_metrics::api::VOICE_REPORT_WAIT_DURATION, "report" => kind)
                    .record((Utc::now() - published).as_seconds_f64().max(0.0));
                    AckKind::Ack
                }
                Err(e) if worth_retrying(&e) && attempt < MAX_ATTEMPTS => {
                    warn!(
                        error = e.to_string(),
                        attempt, "applying a voice report failed; trying it again"
                    );
                    AckKind::Nak(Some(RETRY_DELAY))
                }
                Err(e) => {
                    error!(
                        error = e.to_string(),
                        attempt,
                        "applying a voice report failed; it is dropped, and the server's next snapshot repairs what it would have changed"
                    );
                    AckKind::Ack
                }
            }
        }
    };
    if let Err(e) = message.ack_with(outcome).await {
        warn!(
            error = e.to_string(),
            "could not tell the report stream what became of a voice report"
        );
    }
}

/// Whether a report that failed this way may succeed if tried again: the database or NATS
/// was unreachable or busy, rather than the report being one that can never apply.
fn worth_retrying(error: &app::Error) -> bool {
    use diesel::result::{DatabaseErrorKind, Error as Diesel};
    match error {
        app::Error::Diesel(Diesel::DatabaseError(kind, _)) => matches!(
            kind,
            DatabaseErrorKind::SerializationFailure
                | DatabaseErrorKind::ClosedConnection
                | DatabaseErrorKind::UnableToSendCommand
                | DatabaseErrorKind::ReadOnlyTransaction
        ),
        app::Error::Diesel(Diesel::BrokenTransactionManager) => true,
        app::Error::Diesel(_)
        | app::Error::EventRouting(_)
        | app::Error::SerdeJson(_)
        | app::Error::Validation(_) => false,
        _ => true,
    }
}

/// Removes every report of a voice server still waiting, once the server is no longer
/// registered. A failure is logged: the server is gone either way, and what is left of its
/// reports ages out of the stream.
pub(super) async fn forget_server(state: &impl Publishing, server: VoiceServerId) {
    let stream = match state.nats().get_stream(REPORT_STREAM).await {
        Ok(stream) => stream,
        Err(e) => {
            warn!(
                error = e.to_string(),
                "could not open the voice report stream"
            );
            return;
        }
    };
    for root in [REPORT_SUBJECT_ROOT, SPEAKING_SUBJECT_ROOT] {
        if let Err(e) = stream
            .purge()
            .filter(format!("{root}.*.{}", server.0))
            .await
        {
            warn!(
                server = server.0.to_string(),
                error = e.to_string(),
                "could not clear a removed voice server's waiting reports"
            );
        }
    }
}
