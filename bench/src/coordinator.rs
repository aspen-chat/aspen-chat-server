//! The coordinator: gathers agents, splits the online users among them, starts them together,
//! runs the profile's commands, samples the deployment's metrics, and merges what comes back.
//! With no remote agents it runs one agent in its own process.

use crate::agent;
use crate::clock::Clock;
use crate::profile::{EventKind, Profile};
use crate::report::{self, Aggregate, CapacityStep, CommandRun, PhaseStats, Point, Report};
use crate::scrape::{self, Sample};
use crate::suspension::{resume_limits, suspend_limits};
use aspen_bench_protocol::Manifest;
use aspen_bench_protocol::coordination::{AgentMessage, Assignment, CoordinatorMessage, Phase};
use aspen_limits::suspension;
use axum::extract::State;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::response::Response;
use axum::serve::ListenerExt as _;
use futures_util::{SinkExt, StreamExt};
use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::sync::mpsc;

/// How long after assignments go out the first user connects: time for every agent to set up.
const START_MARGIN: Duration = Duration::from_secs(3);
/// How long past the planned end the coordinator waits for agents to finish.
const AGENT_GRACE: Duration = Duration::from_secs(90);

/// One agent, as the coordinator sees it.
pub struct AgentLink {
    pub name: String,
    tx: mpsc::UnboundedSender<CoordinatorMessage>,
    rx: mpsc::UnboundedReceiver<AgentMessage>,
}

/// Starts an agent inside this process.
pub fn local_agent() -> AgentLink {
    let (to_agent, agent_rx) = mpsc::unbounded_channel();
    let (agent_tx, from_agent) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        if let Err(e) = agent::serve("local".into(), agent_tx, agent_rx).await {
            tracing::error!(error = e, "the local agent failed");
        }
    });
    AgentLink {
        name: "local".into(),
        tx: to_agent,
        rx: from_agent,
    }
}

/// Answers an agent's hello and clock pings until it is ready for work.
async fn handshake(link: &mut AgentLink) -> Result<(), String> {
    loop {
        match link.rx.recv().await {
            Some(AgentMessage::Hello { name, .. }) => link.name = name,
            Some(AgentMessage::ClockPing { agent_ns }) => {
                let _ = link.tx.send(CoordinatorMessage::ClockPong {
                    agent_ns,
                    coordinator_ns: Clock::local_ns(),
                });
            }
            Some(AgentMessage::Ready) => return Ok(()),
            Some(AgentMessage::Failed { error }) => return Err(error),
            Some(other) => {
                return Err(format!(
                    "unexpected message during the handshake: {other:?}"
                ));
            }
            None => return Err("an agent went away during the handshake".into()),
        }
    }
}

/// Accepts `count` agents on `listen` at `/agent`.
pub async fn remote_agents(listen: SocketAddr, count: u32) -> Result<Vec<AgentLink>, String> {
    let (arrivals_tx, mut arrivals) = mpsc::unbounded_channel::<AgentLink>();
    let app = axum::Router::new()
        .route("/agent", axum::routing::get(accept))
        .with_state(arrivals_tx);
    // Clock synchronisation times round trips over these connections, which Nagle's algorithm
    // would stretch by the agent's delayed ACK.
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .map_err(|e| format!("could not listen on {listen}: {e}"))?
        .tap_io(|stream| {
            let _ = stream.set_nodelay(true);
        });
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    eprintln!("waiting for {count} agents on ws://{listen}/agent");
    let mut agents = Vec::new();
    while agents.len() < count as usize {
        let link = arrivals.recv().await.ok_or("the agent listener stopped")?;
        eprintln!(
            "agent {} joined ({} of {count})",
            link.name,
            agents.len() + 1
        );
        agents.push(link);
    }
    Ok(agents)
}

async fn accept(
    State(arrivals): State<mpsc::UnboundedSender<AgentLink>>,
    ws: WebSocketUpgrade,
) -> Response {
    ws.on_upgrade(move |socket| bridge(socket, arrivals))
}

/// Turns an agent's socket into an `AgentLink`, answering clock pings on the spot so the
/// measured trip is the network's alone.
async fn bridge(socket: WebSocket, arrivals: mpsc::UnboundedSender<AgentLink>) {
    let (mut sink, mut source) = socket.split();
    let (to_agent, mut to_agent_rx) = mpsc::unbounded_channel::<CoordinatorMessage>();
    let (from_agent_tx, from_agent) = mpsc::unbounded_channel::<AgentMessage>();
    let mut link = Some(AgentLink {
        name: String::new(),
        tx: to_agent.clone(),
        rx: from_agent,
    });
    loop {
        tokio::select! {
            frame = source.next() => {
                let Some(Ok(WsMessage::Text(text))) = frame else {
                    if matches!(frame, Some(Ok(_))) { continue; }
                    break;
                };
                let Ok(message) = serde_json::from_str::<AgentMessage>(&text) else { continue };
                match message {
                    AgentMessage::ClockPing { agent_ns } => {
                        let pong = CoordinatorMessage::ClockPong { agent_ns, coordinator_ns: Clock::local_ns() };
                        let text = serde_json::to_string(&pong).expect("serializes");
                        if sink.send(WsMessage::Text(text.into())).await.is_err() { break; }
                    }
                    AgentMessage::Hello { name, version } => {
                        if let Some(mut ready) = link.take() {
                            ready.name = name;
                            let _ = version;
                            if arrivals.send(ready).is_err() { break; }
                        }
                    }
                    other => { if from_agent_tx.send(other).is_err() { break; } }
                }
            }
            Some(message) = to_agent_rx.recv() => {
                let text = serde_json::to_string(&message).expect("serializes");
                if sink.send(WsMessage::Text(text.into())).await.is_err() { break; }
            }
        }
    }
}

/// What one session (a ramp and steady phase) needs beyond the profile.
pub struct Session {
    pub online: f64,
    pub ramp_up_seconds: f64,
    pub duration_seconds: f64,
    pub rate_factor: f64,
    /// Whether to run the profile's commands.
    pub commands: bool,
}

pub struct SessionResult {
    pub phases: BTreeMap<Phase, PhaseStats>,
    pub timeline: Vec<Point>,
    pub samples: Vec<Sample>,
    pub commands: Vec<CommandRun>,
    pub online_users: usize,
}

pub async fn run_session(
    profile: &Profile,
    profile_text: &str,
    manifest: &Manifest,
    agents: &mut [AgentLink],
    session: &Session,
) -> Result<SessionResult, String> {
    let roster = profile.roster(session.online);
    let online_users = roster.len();
    let count = agents.len() as u32;
    let mut shares: Vec<Vec<(u32, String)>> = vec![Vec::new(); agents.len()];
    for (position, entry) in roster.into_iter().enumerate() {
        shares[position % agents.len()].push(entry);
    }
    let clock = Clock::default();
    let start_ns = clock.now_ns() + START_MARGIN.as_nanos() as i64;
    for (index, (agent, users)) in agents.iter_mut().zip(shares).enumerate() {
        agent
            .tx
            .send(CoordinatorMessage::Assign(Box::new(Assignment {
                agent: index as u32,
                agents: count,
                profile: profile_text.to_string(),
                manifest: manifest.clone(),
                users,
                start_ns,
                ramp_up_seconds: session.ramp_up_seconds,
                duration_seconds: session.duration_seconds,
                rate_factor: session.rate_factor,
            })))
            .map_err(|_| format!("agent {} went away", agent.name))?;
    }
    let steady_start = start_ns + (session.ramp_up_seconds * 1e9) as i64;
    let interval = Duration::from_secs_f64(profile.load.report_interval_seconds);

    // Sample the deployment's metrics for the whole session.
    let (samples_tx, mut samples_rx) = mpsc::unbounded_channel::<Sample>();
    let endpoints = profile.target.metrics.clone();
    let scraper = tokio::spawn(async move {
        let client = reqwest::Client::new();
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            let t = (Clock::default().now_ns() - start_ns) as f64 / 1e9;
            for endpoint in &endpoints {
                if let Some(sample) = scrape::sample(&client, endpoint, t).await {
                    let _ = samples_tx.send(sample);
                }
            }
        }
    });

    // Commands at their times.
    let (commands_tx, mut commands_rx) = mpsc::unbounded_channel::<CommandRun>();
    if session.commands {
        for event in &profile.events {
            if let EventKind::Command { run } = &event.kind {
                let at = steady_start + (event.at_seconds * 1e9) as i64;
                let run = run.clone();
                let tx = commands_tx.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Clock::default().until(at)).await;
                    let output = tokio::process::Command::new("sh")
                        .arg("-c")
                        .arg(&run)
                        .output()
                        .await;
                    let t = (Clock::default().now_ns() - start_ns) as f64 / 1e9;
                    let (status, output) = match output {
                        Ok(o) => (
                            o.status.code(),
                            format!(
                                "{}{}",
                                String::from_utf8_lossy(&o.stdout),
                                String::from_utf8_lossy(&o.stderr)
                            ),
                        ),
                        Err(e) => (None, e.to_string()),
                    };
                    eprintln!(
                        "[{t:>7.1}s] ran `{run}`: {}",
                        status.map_or("failed to start".into(), |s| format!("exit {s}"))
                    );
                    let _ = tx.send(CommandRun {
                        t,
                        command: run,
                        status,
                        output,
                    });
                });
            }
        }
    }
    drop(commands_tx);

    let mut aggregate = Aggregate::new(start_ns, profile.load.report_interval_seconds);
    let mut finished = 0;
    let mut progress: BTreeMap<u64, (u64, u64, u64)> = BTreeMap::new();
    // Agents drain for a few seconds after the steady phase; one silent long after that is
    // stuck, and the run goes on without it.
    let give_up = tokio::time::Instant::now()
        + START_MARGIN
        + Duration::from_secs_f64(session.ramp_up_seconds + session.duration_seconds)
        + AGENT_GRACE;
    while finished < agents.len() {
        if tokio::time::Instant::now() > give_up {
            eprintln!(
                "{} agents did not finish; reporting what arrived",
                agents.len() - finished
            );
            break;
        }
        let mut next = None;
        for (i, agent) in agents.iter_mut().enumerate() {
            if let Ok(message) = agent.rx.try_recv() {
                next = Some((i, message));
                break;
            }
        }
        let (i, message) = match next {
            Some(found) => found,
            None => {
                tokio::time::sleep(Duration::from_millis(20)).await;
                continue;
            }
        };
        match message {
            AgentMessage::Snapshot(snapshot) => {
                aggregate.add(&snapshot)?;
                // One line per interval once every agent has reported it.
                let entry = progress.entry(snapshot.interval).or_default();
                entry.0 += 1;
                entry.1 += snapshot.connected;
                entry.2 += snapshot.counters.get("requests").copied().unwrap_or(0);
                if entry.0 == agents.len() as u64 {
                    let t = (snapshot.ended_ns - start_ns) as f64 / 1e9;
                    eprintln!(
                        "[{t:>7.1}s] {:<6} {:>7} connected {:>8.0} requests/s",
                        snapshot
                            .phase
                            .map_or("-".into(), |p| format!("{p:?}").to_lowercase()),
                        entry.1,
                        entry.2 as f64 / interval.as_secs_f64()
                    );
                }
            }
            AgentMessage::Finished { .. } => finished += 1,
            AgentMessage::Failed { error } => {
                return Err(format!("agent {} failed: {error}", agents[i].name));
            }
            _ => {}
        }
    }
    scraper.abort();
    let mut samples = Vec::new();
    while let Ok(sample) = samples_rx.try_recv() {
        samples.push(sample);
    }
    let mut commands = Vec::new();
    while let Some(command) = commands_rx.recv().await {
        commands.push(command);
    }
    let (phases, timeline) = aggregate.finish(session.duration_seconds);
    Ok(SessionResult {
        phases,
        timeline,
        samples,
        commands,
        online_users,
    })
}

/// The whole of a check run: one session at the profile's load.
pub async fn check(
    profile: &Profile,
    profile_text: &str,
    manifest: &Manifest,
    agents: &mut [AgentLink],
) -> Result<Report, String> {
    for agent in agents.iter_mut() {
        handshake(agent).await?;
    }
    let planned =
        Duration::from_secs_f64(profile.load.ramp_up_seconds + profile.load.duration_seconds)
            + START_MARGIN;
    let suspension = suspend_limits(profile, planned).await?;
    let started_at = suspension::now_ms() as i64 / 1000;
    let result = run_session(
        profile,
        profile_text,
        manifest,
        agents,
        &Session {
            online: profile.load.online,
            ramp_up_seconds: profile.load.ramp_up_seconds,
            duration_seconds: profile.load.duration_seconds,
            rate_factor: 1.0,
            commands: true,
        },
    )
    .await;
    if let Some((store, record)) = &suspension {
        resume_limits(store, record).await;
    }
    let result = result?;
    let verdict = report::judge(
        profile,
        &result.phases,
        result.online_users,
        &result.timeline,
    );
    let mut report = Report {
        profile: profile.name.clone(),
        description: profile.description.clone(),
        run: manifest.run.clone(),
        started_at,
        online_users: result.online_users,
        agents: agents.len() as u32,
        phases: result.phases,
        timeline: result.timeline,
        verdict,
        server_samples: result.samples,
        findings: Vec::new(),
        memory_growth_per_hour: BTreeMap::new(),
        heap_growth_per_hour: BTreeMap::new(),
        capacity: Vec::new(),
        commands: result.commands,
    };
    report.analyse_server(profile.load.ramp_up_seconds, profile.load.duration_seconds);
    Ok(report)
}

/// Capacity mode: steps the online share up until a service level breaks, and reports the
/// last step that held. The report's phases and timeline are the last passing step's (or the
/// first step's, if none passed).
pub async fn capacity(
    profile: &Profile,
    profile_text: &str,
    manifest: &Manifest,
    agents: &mut [AgentLink],
) -> Result<Report, String> {
    for agent in agents.iter_mut() {
        handshake(agent).await?;
    }
    let c = &profile.capacity;
    let steps: Vec<f64> = {
        let mut steps = Vec::new();
        let mut online = c.start;
        while online <= c.max + 1e-9 {
            steps.push(online.min(1.0));
            online += c.step;
        }
        steps
    };
    let per_step = profile.load.ramp_up_seconds + c.step_seconds + 20.0;
    let planned = Duration::from_secs_f64(per_step * steps.len() as f64);
    let suspension = suspend_limits(profile, planned).await?;
    let started_at = suspension::now_ms() as i64 / 1000;
    let mut results = Vec::new();
    let mut outcome = Ok(());
    // Interrupting the search (Ctrl-C, or a guard that stops the generator before its machine
    // runs out of memory) ends the step under way and reports the ones that finished.
    let interrupted = tokio::signal::ctrl_c();
    tokio::pin!(interrupted);
    for online in steps {
        eprintln!("capacity step: {:.0}% online", online * 100.0);
        let session = Session {
            online,
            ramp_up_seconds: profile.load.ramp_up_seconds,
            duration_seconds: c.step_seconds,
            rate_factor: 1.0,
            commands: false,
        };
        let step = tokio::select! {
            step = run_session(profile, profile_text, manifest, agents, &session) => step,
            _ = &mut interrupted => {
                eprintln!("  interrupted; reporting the steps that finished");
                break;
            }
        };
        match step {
            Ok(result) => {
                let verdict = report::judge(
                    profile,
                    &result.phases,
                    result.online_users,
                    &result.timeline,
                );
                let findings = scrape::findings(&result.samples);
                let pass = verdict.pass;
                eprintln!(
                    "  {} users online ({} connected): {}",
                    result.online_users,
                    report::connected_in_steady(&result.timeline).unwrap_or(0),
                    if pass { "held" } else { "did not hold" }
                );
                results.push((result, verdict, findings));
                if !pass {
                    break;
                }
                // Let the deployment settle between steps.
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
            Err(e) => {
                outcome = Err(e);
                break;
            }
        }
    }
    if let Some((store, record)) = &suspension {
        resume_limits(store, record).await;
    }
    outcome?;
    let capacity: Vec<CapacityStep> = results
        .iter()
        .map(|(result, verdict, findings)| CapacityStep {
            online_users: result.online_users,
            connected: report::connected_in_steady(&result.timeline).unwrap_or(0),
            pass: verdict.pass,
            checks: verdict.checks.clone(),
            findings: findings.clone(),
            steady: result.phases.get(&Phase::Steady).cloned(),
        })
        .collect();
    let chosen = results
        .iter()
        .rposition(|(_, verdict, _)| verdict.pass)
        .unwrap_or(0);
    let mut all_samples: Vec<Sample> = Vec::new();
    for (result, _, _) in &results {
        all_samples.extend(result.samples.iter().cloned());
    }
    let (result, verdict, _) = results
        .into_iter()
        .nth(chosen)
        .ok_or("no capacity step ran")?;
    let mut report = Report {
        profile: profile.name.clone(),
        description: profile.description.clone(),
        run: manifest.run.clone(),
        started_at,
        online_users: result.online_users,
        agents: agents.len() as u32,
        phases: result.phases,
        timeline: result.timeline,
        verdict,
        server_samples: all_samples,
        findings: Vec::new(),
        memory_growth_per_hour: BTreeMap::new(),
        heap_growth_per_hour: BTreeMap::new(),
        capacity,
        commands: Vec::new(),
    };
    // Steps restart the clock, so no one window covers a steady phase; growth is not read.
    report.analyse_server(0.0, 0.0);
    Ok(report)
}
