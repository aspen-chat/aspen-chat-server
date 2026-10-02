//! Suspending the deployment's rate limits for a run, through the NATS KV record that
//! `aspen_limits::suspension` defines and every API and voice server watches.

use crate::profile::{Profile, SuspendScope};
use aspen_limits::suspension::{self, Scope, Suspension};
use std::time::Duration;

/// Extra time a suspension of the limits covers beyond the planned run.
const SUSPENSION_MARGIN: Duration = Duration::from_secs(600);

/// Suspends the deployment's rate limits for the run, if the profile asks, and returns the
/// record written so it can be withdrawn afterwards.
pub async fn suspend_limits(
    profile: &Profile,
    planned: Duration,
) -> Result<Option<(async_nats::jetstream::kv::Store, Suspension)>, String> {
    let limits = &profile.limits;
    if !limits.suspend {
        return Ok(None);
    }
    let url = limits
        .nats_url
        .as_deref()
        .ok_or("limits.nats_url is required")?;
    let mut options = async_nats::ConnectOptions::new();
    if let Some(token) = &limits.nats_token {
        options = options.token(token.clone());
    }
    let client = async_nats::connect_with_options(url, options)
        .await
        .map_err(|e| format!("could not connect to NATS at {url}: {e}"))?;
    let store = suspension::bucket(client).await?;
    let started_at = suspension::now_ms();
    let record = Suspension {
        started_at,
        until: started_at
            + u64::try_from((planned + SUSPENSION_MARGIN).as_millis()).unwrap_or(u64::MAX),
        scope: match limits.scope {
            SuspendScope::All => Scope::All,
            SuspendScope::Networks => Scope::Networks {
                networks: limits.networks.clone(),
            },
        },
        reason: format!("aspen-bench: {}", profile.name),
        by: "aspen-bench".into(),
    };
    suspension::write(&store, &record).await?;
    eprintln!(
        "rate limits suspended until the run ends (at most {}s)",
        (planned + SUSPENSION_MARGIN).as_secs()
    );
    // Give every server a moment to see it.
    tokio::time::sleep(Duration::from_secs(2)).await;
    Ok(Some((store, record)))
}

/// Withdraws the suspension, unless someone has replaced it meanwhile.
pub async fn resume_limits(store: &async_nats::jetstream::kv::Store, ours: &Suspension) {
    match suspension::read(store).await {
        Ok(Some(current)) if &current == ours => match suspension::clear(store).await {
            Ok(()) => eprintln!("rate limits are in force again"),
            Err(e) => eprintln!("could not withdraw the suspension ({e}); it ends by itself"),
        },
        Ok(_) => eprintln!("the suspension was changed by someone else; leaving it"),
        Err(e) => eprintln!("could not read the suspension ({e}); it ends by itself"),
    }
}
