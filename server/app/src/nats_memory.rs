//! How much of NATS's memory the deployment's two in-memory streams reserve, and how much they
//! use: the events of the last minute (`app::event_feed`) and the voice servers' reports not yet
//! applied (`app::voice`).
//!
//! NATS counts a memory stream's `max_bytes` against its memory limit when the stream is made or
//! resized, whatever the stream then holds, and refuses one that does not fit. So each stream's
//! size is chosen to fit ([`sizes`]): the size the deployment's settings give it
//! (`event_stream_mib`, `voice_report_stream_mib`), or, for a stream they leave automatic, a
//! share of everything NATS has that no other stream has reserved. What NATS has is its
//! account's memory limit when the account has one, and the server's `max_memory_store` when
//! `[nats] monitor_url` names its monitoring port ([`Monitor`]), the only place a server says
//! it. Where neither is known an automatic stream takes a fixed size ([`FALLBACK_EVENTS`]).
//!
//! The streams are fitted ([`fit`]) as each server starts, when a size setting changes, inside
//! the change's transaction so that a size NATS refuses is never saved, and every [`FIT_EVERY`]
//! by the recurring job `fitStreams`, which makes them again after NATS restarts without them
//! and follows a limit NATS was restarted with. All three hold the settings row's lock, so none
//! applies sizes another has since changed.

use crate::aspen_config::AspenConfig;
use crate::context::GlobalServerContext;
use crate::deployment_settings::DeploymentSettings;
use crate::jobs::{Claimed, Outcome};
use crate::t;
use async_nats::jetstream::context::{CreateStreamErrorKind, GetStreamErrorKind};
use async_nats::jetstream::stream::{Info, StorageType};
use async_nats::jetstream::{Context, ErrorCode};
use chrono::{DateTime, Utc};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection};
use serde::Deserialize;
use std::time::Duration;
use voice_protocol::control::REPORT_STREAM;

const MIB: u64 = 1 << 20;

/// The least a stream may be set to hold, in MiB, and the least an automatic one is given.
pub const MIN_STREAM_MIB: u32 = 16;

/// What an automatic events stream holds where NATS's limit cannot be read.
pub const FALLBACK_EVENTS: u64 = 1024 * MIB;

/// The part of what the two automatic streams share that the voice reports' takes: one part in
/// this many. Reports wait only while no API server applies them, and a voice server's next
/// snapshot repairs any let go.
const REPORTS_SHARE: u64 = 8;

/// The most an automatic voice report stream takes of its share.
const AUTOMATIC_REPORTS_MAX: u64 = 1024 * MIB;

/// How long the monitoring port has to answer.
const MONITOR_TIMEOUT: Duration = Duration::from_secs(2);

/// How often `fitStreams` runs: how long the streams may be missing after NATS restarts, or
/// sized for a limit it no longer has.
pub const FIT_EVERY: Duration = Duration::from_secs(15);

/// The sizes the settings give the streams, in bytes; `None` for one left automatic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted {
    pub events: Option<u64>,
    pub voice_reports: Option<u64>,
}

impl From<&DeploymentSettings> for Wanted {
    fn from(settings: &DeploymentSettings) -> Self {
        let bytes = |mib: Option<u32>| mib.map(|mib| u64::from(mib) * MIB);
        Self {
            events: bytes(settings.event_stream_mib),
            voice_reports: bytes(settings.voice_report_stream_mib),
        }
    }
}

/// What each stream is to hold, in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sizes {
    pub events: u64,
    pub voice_reports: u64,
}

/// Why no sizes fit what NATS has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shortfall {
    /// The sizes the settings give ask for more than there is.
    Over { asked: u64, available: u64 },
    /// What is left for an automatic stream is less than [`MIN_STREAM_MIB`].
    TooLittle { available: u64 },
}

impl From<Shortfall> for crate::Error {
    fn from(shortfall: Shortfall) -> Self {
        crate::Error::Validation(match shortfall {
            Shortfall::Over { asked, available } => t!(
                "streamSizesOverLimit",
                asked = asked / MIB,
                available = available / MIB
            ),
            Shortfall::TooLittle { available } => t!(
                "streamMemoryTooLittle",
                available = available / MIB,
                min = MIN_STREAM_MIB
            ),
        })
    }
}

/// Each stream's size: what the settings give it, or for an automatic one its part of
/// `available`, the bytes NATS has that no other stream has reserved, in whole MiB. Both
/// automatic, the voice reports take one part in [`REPORTS_SHARE`], at most
/// [`AUTOMATIC_REPORTS_MAX`], and the events the rest; one automatic, it takes what the other
/// leaves. With `available` unknown an automatic stream takes its fallback.
fn sizes(wanted: Wanted, available: Option<u64>) -> Result<Sizes, Shortfall> {
    let Some(available) = available else {
        return Ok(Sizes {
            events: wanted.events.unwrap_or(FALLBACK_EVENTS),
            voice_reports: wanted
                .voice_reports
                .unwrap_or(FALLBACK_EVENTS / REPORTS_SHARE),
        });
    };
    let asked = wanted.events.unwrap_or(0) + wanted.voice_reports.unwrap_or(0);
    let Some(left) = available.checked_sub(asked) else {
        return Err(Shortfall::Over { asked, available });
    };
    let whole = |bytes: u64| bytes / MIB * MIB;
    let voice_reports = wanted.voice_reports.unwrap_or_else(|| {
        let part = match wanted.events {
            Some(_) => left,
            None => left / REPORTS_SHARE,
        };
        whole(part.min(AUTOMATIC_REPORTS_MAX))
    });
    let events = wanted
        .events
        .unwrap_or_else(|| whole(available - voice_reports));
    if events.min(voice_reports) < u64::from(MIN_STREAM_MIB) * MIB {
        return Err(Shortfall::TooLittle { available });
    }
    Ok(Sizes {
        events,
        voice_reports,
    })
}

/// What a NATS server says of its JetStream memory on its monitoring port (`/jsz`).
#[derive(Debug, Clone, Copy, Deserialize)]
struct ServerMemory {
    /// Bytes its memory streams hold.
    memory: u64,
    /// Bytes its memory streams have reserved.
    reserved_memory: u64,
    config: ServerLimits,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct ServerLimits {
    /// `max_memory_store`.
    max_memory: u64,
}

/// The NATS server's monitoring port, where `[nats] monitor_url` names one.
#[derive(Clone)]
pub struct Monitor {
    /// The address of `/jsz`; `None` when no port is named.
    jsz: Option<String>,
    client: reqwest::Client,
}

impl Monitor {
    /// A reader of `[nats] monitor_url`, which trusts `[nats.tls]`'s authorities, and presents
    /// its certificate, where the address is `https`.
    pub fn new(config: &AspenConfig) -> crate::Result<Self> {
        let mut client = reqwest::Client::builder()
            // The port is on the deployment's own network, where a proxy does not reach.
            .no_proxy()
            .timeout(MONITOR_TIMEOUT);
        if let Some(tls) = &config.nats.tls {
            client =
                client.tls_backend_preconfigured(tls.rustls_client_config("nats.tls").map_err(
                    |message| crate::Error::Config(config::ConfigError::Message(message)),
                )?);
        }
        Ok(Self {
            jsz: config
                .nats
                .monitor_url
                .as_ref()
                .map(|url| format!("{}/jsz", url.trim_end_matches('/'))),
            client: client.build().map_err(|e| {
                crate::Error::Config(config::ConfigError::Message(format!(
                    "[nats] monitor_url: {e}"
                )))
            })?,
        })
    }

    /// What the server says, `None` when no port is named, and why not when it does not answer.
    async fn read(&self) -> Option<Result<ServerMemory, reqwest::Error>> {
        let jsz = self.jsz.as_ref()?;
        Some(
            async {
                self.client
                    .get(jsz)
                    .send()
                    .await?
                    .error_for_status()?
                    .json()
                    .await
            }
            .await,
        )
    }
}

/// NATS's memory and the two streams as they are now.
struct Reading {
    account: async_nats::jetstream::account::Account,
    /// The server's own figures, where its monitoring port is named and answered.
    server: Option<ServerMemory>,
    /// Whether a monitoring port is named and did not answer.
    monitor_failed: bool,
    events: Option<Info>,
    voice_reports: Option<Info>,
}

/// The stream `name` as NATS holds it; `None` when it has none.
async fn stream_info(nats: &Context, name: &str) -> crate::Result<Option<Info>> {
    match nats.get_stream(name).await {
        Ok(stream) => Ok(Some(stream.cached_info().clone())),
        Err(e) => match e.kind() {
            GetStreamErrorKind::JetStream(e) if e.error_code() == ErrorCode::STREAM_NOT_FOUND => {
                Ok(None)
            }
            _ => Err(e.into()),
        },
    }
}

/// What a stream has reserved of NATS's memory.
fn reserved(stream: Option<&Info>) -> u64 {
    stream
        .filter(|info| info.config.storage == StorageType::Memory)
        .map_or(0, |info| u64::try_from(info.config.max_bytes).unwrap_or(0))
}

impl Reading {
    async fn take(nats: &Context, monitor: &Monitor) -> crate::Result<Self> {
        let (account, server, events, voice_reports) = tokio::join!(
            nats.query_account(),
            monitor.read(),
            stream_info(nats, crate::ASPEN_NATS_STREAM_NAME),
            stream_info(nats, REPORT_STREAM),
        );
        let monitor_failed = matches!(server, Some(Err(_)));
        if let Some(Err(e)) = &server {
            tracing::warn!(error = %e, "NATS's monitoring port ([nats] monitor_url) did not answer");
        }
        Ok(Self {
            account: account?,
            server: server.and_then(Result::ok),
            monitor_failed,
            events: events?,
            voice_reports: voice_reports?,
        })
    }

    /// The bytes NATS has that no stream but these two has reserved: the least that the
    /// server's limit and the account's leave, `None` when neither is known.
    fn available(&self) -> Option<u64> {
        let ours = reserved(self.events.as_ref()) + reserved(self.voice_reports.as_ref());
        let room = |limit: u64, reserved: u64| limit.saturating_sub(reserved.saturating_sub(ours));
        let server = self
            .server
            .map(|server| room(server.config.max_memory, server.reserved_memory));
        let account = self
            .account
            .limits
            .max_memory
            .and_then(|limit| u64::try_from(limit).ok())
            .map(|limit| room(limit, self.account.reserved_memory));
        [server, account].into_iter().flatten().min()
    }
}

/// Makes the two streams the sizes `wanted` and NATS's memory give them, where they are not
/// already (or whatever they are, with `always`, as a starting server does so that the rest of
/// their configuration is this version's). A stream made smaller lets its oldest go at once.
///
/// While a monitoring port is named and does not answer, an automatic stream keeps the size it
/// has rather than falling back to a smaller one.
pub async fn fit(
    nats: &Context,
    monitor: &Monitor,
    mut wanted: Wanted,
    always: bool,
) -> crate::Result<Sizes> {
    let reading = Reading::take(nats, monitor).await?;
    if reading.monitor_failed {
        let held = |stream: &Option<Info>| Some(reserved(stream.as_ref())).filter(|held| *held > 0);
        wanted.events = wanted.events.or_else(|| held(&reading.events));
        wanted.voice_reports = wanted
            .voice_reports
            .or_else(|| held(&reading.voice_reports));
    }
    let available = reading.available();
    let sizes = sizes(wanted, available)?;
    let mut streams = [
        (
            &reading.events,
            crate::event_feed::stream_config(sizes.events),
        ),
        (
            &reading.voice_reports,
            crate::voice::report_stream_config(sizes.voice_reports),
        ),
    ];
    // One that shrinks goes first, so that the room it gives up is there for one that grows.
    streams.sort_by_key(|(held, config)| {
        held.as_ref()
            .is_none_or(|held| held.config.max_bytes <= config.max_bytes)
    });
    for (held, config) in streams {
        let fits = held
            .as_ref()
            .is_some_and(|held| held.config.max_bytes == config.max_bytes);
        if fits && !always {
            continue;
        }
        let (name, max_bytes) = (config.name.clone(), config.max_bytes);
        nats.create_or_update_stream(config)
            .await
            .map_err(|e| match e.kind() {
                CreateStreamErrorKind::JetStream(refusal)
                    if refusal.error_code() == ErrorCode::MEMORY_RESOURCES_EXCEEDED =>
                {
                    crate::Error::Validation(t!(
                        "streamSizesRefused",
                        asked = (sizes.events + sizes.voice_reports) / MIB
                    ))
                }
                _ => e.into(),
            })?;
        if !fits {
            tracing::info!(stream = name, max_bytes, "sized a NATS stream");
        }
    }
    Ok(sizes)
}

/// [`fit`], for the sizes the settings row holds, while holding its lock.
pub async fn fit_locked(
    conn: &mut AsyncPgConnection,
    nats: &Context,
    monitor: &Monitor,
    always: bool,
) -> crate::Result<Sizes> {
    conn.transaction(|conn| {
        async move {
            let settings = crate::deployment_settings::load_locked(conn).await?;
            fit(nats, monitor, (&settings).into(), always).await
        }
        .scope_boxed()
    })
    .await
}

/// The recurring job `fitStreams`. A pass that fails is logged and left to the next, a recurring
/// job's backoff being longer than the streams should go without.
pub async fn fit_step(state: &GlobalServerContext, _job: &Claimed) -> crate::Result<Outcome> {
    let mut conn = state.connection_pool.get().await?;
    if let Err(e) = fit_locked(
        conn.as_mut(),
        &state.nats_context,
        &state.nats_monitor,
        false,
    )
    .await
    {
        tracing::warn!(error = %e, "could not fit the NATS streams to its memory");
    }
    Ok(Outcome::Done)
}

/// Whose memory a [`Usage`] counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The whole NATS server's, read from its monitoring port.
    Server,
    /// This deployment's NATS account's.
    Account,
}

/// One stream's use of NATS's memory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamUsage {
    /// Bytes it holds.
    pub bytes: u64,
    /// The most it may hold, which is what it has reserved.
    pub max_bytes: u64,
    pub messages: u64,
    /// When the oldest message it holds was published; `None` when it holds none.
    pub oldest_at: Option<DateTime<Utc>>,
    /// Whether its size follows NATS's memory rather than a setting.
    pub automatic: bool,
}

/// NATS's memory as the dashboard shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    pub scope: Scope,
    /// Bytes every memory stream in the scope holds.
    pub used: u64,
    /// Bytes every memory stream in the scope has reserved.
    pub reserved: u64,
    /// The most the scope's streams may reserve; `None` when NATS does not say.
    pub limit: Option<u64>,
    /// Whether `[nats] monitor_url` is set and did not answer.
    pub monitor_unreachable: bool,
    /// `None` while NATS is without the stream, until the next `fitStreams`.
    pub events: Option<StreamUsage>,
    pub voice_reports: Option<StreamUsage>,
}

/// NATS's memory now: the server's where its monitoring port answers, else the account's.
pub async fn usage(state: &GlobalServerContext) -> crate::Result<Usage> {
    let reading = Reading::take(&state.nats_context, &state.nats_monitor).await?;
    let wanted = Wanted::from(&*state.settings());
    let stream = |info: Option<&Info>, wanted: Option<u64>| {
        info.map(|info| StreamUsage {
            bytes: info.state.bytes,
            max_bytes: u64::try_from(info.config.max_bytes).unwrap_or(0),
            messages: info.state.messages,
            oldest_at: (info.state.messages > 0)
                .then(|| {
                    let at = info.state.first_timestamp;
                    DateTime::from_timestamp(at.unix_timestamp(), at.nanosecond())
                })
                .flatten(),
            automatic: wanted.is_none(),
        })
    };
    let events = stream(reading.events.as_ref(), wanted.events);
    let voice_reports = stream(reading.voice_reports.as_ref(), wanted.voice_reports);
    let account = &reading.account;
    let (scope, used, reserved, limit) = match reading.server {
        Some(server) => (
            Scope::Server,
            server.memory,
            server.reserved_memory,
            Some(server.config.max_memory),
        ),
        None => (
            Scope::Account,
            account.memory,
            account.reserved_memory,
            account
                .limits
                .max_memory
                .and_then(|limit| u64::try_from(limit).ok()),
        ),
    };
    Ok(Usage {
        scope,
        used,
        reserved,
        limit,
        monitor_unreachable: reading.monitor_failed,
        events,
        voice_reports,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * MIB;

    fn automatic() -> Wanted {
        Wanted {
            events: None,
            voice_reports: None,
        }
    }

    #[test]
    fn automatic_streams_share_everything_available() {
        assert_eq!(
            sizes(automatic(), Some(4 * GIB)),
            Ok(Sizes {
                events: 4 * GIB - 512 * MIB,
                voice_reports: 512 * MIB,
            })
        );
    }

    #[test]
    fn automatic_voice_reports_stop_at_their_most() {
        assert_eq!(
            sizes(automatic(), Some(48 * GIB)),
            Ok(Sizes {
                events: 47 * GIB,
                voice_reports: GIB,
            })
        );
    }

    #[test]
    fn automatic_sizes_are_whole_mebibytes_within_what_is_available() {
        let available = 300 * MIB + 12_345;
        let sizes = sizes(automatic(), Some(available)).unwrap();
        assert_eq!(sizes.events % MIB, 0);
        assert_eq!(sizes.voice_reports % MIB, 0);
        assert!(sizes.events + sizes.voice_reports <= available);
    }

    #[test]
    fn an_unknown_limit_gives_the_fallbacks() {
        assert_eq!(
            sizes(automatic(), None),
            Ok(Sizes {
                events: GIB,
                voice_reports: 128 * MIB,
            })
        );
    }

    #[test]
    fn a_set_size_is_kept_and_the_automatic_stream_takes_what_it_leaves() {
        let events_set = Wanted {
            events: Some(GIB),
            voice_reports: None,
        };
        assert_eq!(
            sizes(events_set, Some(GIB + 100 * MIB)),
            Ok(Sizes {
                events: GIB,
                voice_reports: 100 * MIB,
            })
        );
        let reports_set = Wanted {
            events: None,
            voice_reports: Some(64 * MIB),
        };
        assert_eq!(
            sizes(reports_set, Some(GIB)),
            Ok(Sizes {
                events: GIB - 64 * MIB,
                voice_reports: 64 * MIB,
            })
        );
        assert_eq!(
            sizes(events_set, None),
            Ok(Sizes {
                events: GIB,
                voice_reports: 128 * MIB,
            })
        );
    }

    #[test]
    fn set_sizes_past_what_is_available_are_refused() {
        let wanted = Wanted {
            events: Some(GIB),
            voice_reports: Some(GIB),
        };
        assert_eq!(
            sizes(wanted, Some(GIB)),
            Err(Shortfall::Over {
                asked: 2 * GIB,
                available: GIB,
            })
        );
        assert_eq!(
            sizes(wanted, Some(2 * GIB)),
            Ok(Sizes {
                events: GIB,
                voice_reports: GIB,
            })
        );
    }

    #[test]
    fn too_little_for_an_automatic_stream_is_refused() {
        assert_eq!(
            sizes(automatic(), Some(64 * MIB)),
            Err(Shortfall::TooLittle {
                available: 64 * MIB,
            })
        );
        let events_take_all = Wanted {
            events: Some(GIB),
            voice_reports: None,
        };
        assert_eq!(
            sizes(events_take_all, Some(GIB)),
            Err(Shortfall::TooLittle { available: GIB })
        );
    }
}
