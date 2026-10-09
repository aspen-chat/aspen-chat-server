//! An agent's side of a run: plays its assigned users through the phases and the profile's
//! timed events, reporting snapshots as it goes.

use crate::clock::Clock;
use crate::profile::Behaviour;
use crate::profile::{EventKind, Profile};
use crate::stats;
use crate::user::{User, World, memberships, text_channels};
use aspen_bench_protocol::coordination::{AgentSummary, Assignment, Phase, Snapshot};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::Instant;

/// How long users get to leave once the steady phase ends.
const DRAIN: Duration = Duration::from_secs(10);

/// Runs an assignment. Snapshots arrive on the returned receiver as the run goes; the handle
/// resolves once every user has stopped. `stop` ends the run early.
pub fn run(
    assignment: Assignment,
    clock: Clock,
    stop: watch::Receiver<bool>,
) -> Result<
    (
        mpsc::UnboundedReceiver<Snapshot>,
        tokio::task::JoinHandle<AgentSummary>,
    ),
    String,
> {
    let profile = Profile::from_toml(&assignment.profile)?;
    let interval = Duration::from_secs_f64(profile.load.report_interval_seconds);
    let (recorder, snapshots) = stats::start(assignment.agent, interval, clock);
    let api = profile.target.api.trim_end_matches('/').to_string();
    let events_url = format!(
        "{}/api/v1/events",
        api.replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1)
    );
    let (users_stop_tx, users_stop) = watch::channel(false);
    let (storms, _) = broadcast::channel(16);
    let world = Arc::new(World {
        memberships: memberships(&assignment.manifest),
        text_channels: text_channels(&assignment.manifest),
        images: std::sync::Mutex::new(std::collections::HashMap::new()),
        profile: profile.clone(),
        manifest: assignment.manifest.clone(),
        api,
        events_url,
        events_addr: tokio::sync::OnceCell::new(),
        clock,
        recorder: recorder.clone(),
        rate_factor: AtomicU64::new(assignment.rate_factor.to_bits()),
        storms,
        stop: users_stop,
    });
    let handle = tokio::spawn(async move {
        let start = Instant::now() + clock.until(assignment.start_ns);
        let ramp = Duration::from_secs_f64(assignment.ramp_up_seconds);
        let steady_start = start + ramp;
        let end = steady_start + Duration::from_secs_f64(assignment.duration_seconds);
        let mut users = JoinSet::new();
        let count = assignment.users.len().max(1);
        for (position, (index, behaviour)) in assignment.users.iter().enumerate() {
            let Some(behaviour) = profile.behaviours.get(behaviour).cloned() else {
                continue;
            };
            let connect_at = start + ramp.mul_f64(position as f64 / count as f64);
            let user = User::new(Arc::clone(&world), *index, behaviour);
            users.spawn(user.run(connect_at));
        }
        tokio::time::sleep_until(start).await;
        recorder.phase(Phase::Ramp);
        let mut stop = stop;
        let timeline = async {
            tokio::time::sleep_until(steady_start).await;
            recorder.phase(Phase::Steady);
            let mut events = profile.events.clone();
            events.sort_by(|a, b| a.at_seconds.total_cmp(&b.at_seconds));
            let base = assignment.rate_factor;
            for event in events {
                let at = steady_start + Duration::from_secs_f64(event.at_seconds.max(0.0));
                if at >= end {
                    break;
                }
                tokio::time::sleep_until(at).await;
                match event.kind {
                    EventKind::ReconnectStorm { fraction } => {
                        recorder.count("storms", 1);
                        let _ = world.storms.send(fraction);
                    }
                    EventKind::Spike { factor, seconds } => {
                        world.set_rate_factor(base * factor);
                        let world = Arc::clone(&world);
                        tokio::spawn(async move {
                            tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
                            world.set_rate_factor(base);
                        });
                    }
                    // Made once, by the first agent.
                    EventKind::Announcement { community } if assignment.agent == 0 => {
                        let community = community as usize;
                        if let Some(owner) = world
                            .manifest
                            .communities
                            .get(community)
                            .and_then(|c| c.members.first())
                        {
                            let owner = User::new(Arc::clone(&world), *owner, Behaviour::default());
                            tokio::spawn(Arc::new(owner).announce(community));
                        }
                    }
                    EventKind::Announcement { .. } => {}
                    // Run by the coordinator.
                    EventKind::Command { .. } => {}
                }
            }
            tokio::time::sleep_until(end).await;
        };
        tokio::select! {
            () = timeline => {}
            _ = stop.changed() => {}
        }
        recorder.phase(Phase::Drain);
        let _ = users_stop_tx.send(true);
        let _ =
            tokio::time::timeout(DRAIN, async { while users.join_next().await.is_some() {} }).await;
        users.abort_all();
        let connected_at_end = recorder.connected().load(Ordering::Relaxed);
        drop(world);
        drop(recorder);
        AgentSummary {
            users: count as u64,
            connected_at_end,
        }
    });
    Ok((snapshots, handle))
}
