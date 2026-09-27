//! Messages between the benchmark coordinator and its agents, as JSON over a WebSocket.
//!
//! An agent connects and says `Hello`, measures the offset between its clock and the
//! coordinator's with `ClockPing`/`ClockPong` rounds, and says `Ready`; the coordinator then
//! sends each agent an `Assignment`: the profile, the
//! seeded population, which users it plays, and when, on the coordinator's clock, to begin.
//! Agents time everything on the coordinator's clock (their own plus the measured offset), so a
//! message sent from one agent and received on another has a meaningful delivery time. Every
//! report interval each agent sends a `Snapshot` of what it measured, histograms serialized in
//! HdrHistogram's V2 format; the coordinator merges them.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// From an agent.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentMessage {
    #[serde(rename_all = "camelCase")]
    Hello {
        name: String,
        version: String,
    },
    /// `agent_ns` is the agent's clock when it sent this.
    #[serde(rename_all = "camelCase")]
    ClockPing {
        agent_ns: i64,
    },
    Snapshot(Snapshot),
    /// The agent has measured its clock offset and waits for an assignment.
    Ready,
    /// The agent has stopped its users.
    #[serde(rename_all = "camelCase")]
    Finished {
        summary: AgentSummary,
    },
    Failed {
        error: String,
    },
}

/// From the coordinator.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CoordinatorMessage {
    /// Echoes the ping, with the coordinator's clock when it answered.
    #[serde(rename_all = "camelCase")]
    ClockPong {
        agent_ns: i64,
        coordinator_ns: i64,
    },
    Assign(Box<Assignment>),
    /// Stop every user now.
    Stop,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assignment {
    /// This agent's number among `agents`.
    pub agent: u32,
    pub agents: u32,
    /// The profile, as TOML.
    pub profile: String,
    /// The seeded population (`crate::Manifest`).
    pub manifest: crate::Manifest,
    /// The users this agent plays: `(user index, behaviour name)`, in connecting order.
    pub users: Vec<(u32, String)>,
    /// Coordinator-clock nanoseconds at which the first user connects.
    pub start_ns: i64,
    /// How long ramp-up and the steady phase last, which may differ from the profile's in
    /// capacity mode.
    pub ramp_up_seconds: f64,
    pub duration_seconds: f64,
    /// Multiplies every rate: 1 for a normal run.
    #[serde(default = "one")]
    pub rate_factor: f64,
}

fn one() -> f64 {
    1.0
}

/// The phase a measurement belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    /// Users signing in, starting up, and connecting.
    Ramp,
    /// Everyone online; what the verdict is about.
    Steady,
    /// Users leaving.
    Drain,
}

/// One report interval of one agent.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub agent: u32,
    /// The interval's number from the start; intervals of different agents with the same
    /// number cover the same time.
    pub interval: u64,
    pub phase: Option<Phase>,
    /// Latencies in microseconds, by metric name, as base64 HdrHistogram V2.
    pub histograms: BTreeMap<String, String>,
    /// Counts, by name.
    pub counters: BTreeMap<String, u64>,
    /// Users connected at the interval's end.
    pub connected: u64,
    /// Coordinator-clock nanoseconds at the interval's end.
    pub ended_ns: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSummary {
    pub users: u64,
    pub connected_at_end: u64,
}
