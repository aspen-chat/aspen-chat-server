//! Workload profiles: the job an operator wants a deployment to do, written in TOML.
//!
//! A profile names the deployment (`[target]`), the population to seed (`[population]`), how many
//! of those people are online and for how long (`[load]`), what they do (`[behaviours.*]`),
//! anything that happens at a set moment (`[[events]]`), and what counts as keeping up
//! (`[slo]`). `profiles/` holds the built-in scenarios, each documented.

use aspen_bench_protocol::{CommunityPlan, SeedPlan};
use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};
use serde::{Deserialize, Serialize};
use smart_default::SmartDefault;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub target: Target,
    pub population: Population,
    pub load: Load,
    pub behaviours: BTreeMap<String, Behaviour>,
    #[serde(default)]
    pub events: Vec<TimedEvent>,
    #[serde(default)]
    pub slo: Slo,
    #[serde(default)]
    pub limits: LimitsOptions,
    #[serde(default)]
    pub capacity: CapacityOptions,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// The API server's origin, `https://chat.example.org`.
    pub api: String,
    /// Prometheus endpoints to sample during the run: the API servers', the voice servers',
    /// and any exporters (Postgres, NATS, Valkey, node). Read to say what ran out first.
    #[serde(default)]
    pub metrics: Vec<String>,
    /// Local addresses to open connections from, users spread over them in turn; empty for the
    /// system's choice. One address has only the system's ephemeral port range (about 28,000
    /// ports on Linux) for connections to one server address, and a machine playing many
    /// users runs out, so a generator that plays more than about ten thousand needs several
    /// (on Linux every `127.0.0.x` is local, for a server on the same machine).
    #[serde(default)]
    pub source_addresses: Vec<std::net::IpAddr>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Population {
    pub users: u32,
    pub communities: u32,
    /// Community sizes fall off by rank, `largest / rank^size_exponent`, never below
    /// `smallest_community`: a few big communities and many small ones.
    pub largest_community: u32,
    #[serde(default = "default_smallest_community")]
    pub smallest_community: u32,
    #[serde(default = "default_size_exponent")]
    pub size_exponent: f64,
    #[serde(default = "default_text_channels")]
    pub text_channels: u32,
    #[serde(default = "default_voice_channels")]
    pub voice_channels: u32,
    #[serde(default = "default_history")]
    pub history_per_channel: u32,
    /// Every seeded user's password; left out, the seeder draws one for the run.
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Load {
    /// The share of the population online during the run.
    pub online: f64,
    /// Online users connect evenly over this long; 0 connects them all at once.
    pub ramp_up_seconds: f64,
    /// How long the steady phase, which the verdict is about, lasts.
    pub duration_seconds: f64,
    /// Seconds between the figures the report charts.
    #[serde(default = "default_report_interval")]
    pub report_interval_seconds: f64,
    /// Chooses who is online and who behaves how, so runs of one profile compare.
    #[serde(default = "default_seed")]
    pub seed: u64,
    /// When the generator starts actions later than planned by more than this at the 99th
    /// percentile, it was the bottleneck, and the run says so rather than blaming the server.
    #[serde(default = "default_lag_limit")]
    pub generator_lag_limit_ms: f64,
}

/// What one kind of user does, per hour online.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Behaviour {
    /// The share of online users who behave this way; shares add up to 1.
    pub share: f64,
    #[serde(default)]
    pub messages_per_hour: f64,
    #[serde(default)]
    pub dm_messages_per_hour: f64,
    #[serde(default)]
    pub reactions_per_hour: f64,
    #[serde(default)]
    pub edits_per_hour: f64,
    #[serde(default)]
    pub deletes_per_hour: f64,
    /// Opening a channel: its latest page of history.
    #[serde(default)]
    pub history_reads_per_hour: f64,
    /// Dropping the event stream and resuming it, as a phone changing networks does.
    #[serde(default)]
    pub reconnects_per_hour: f64,
    #[serde(default)]
    pub attachments_per_hour: f64,
    #[serde(default = "default_attachment_bytes")]
    pub attachment_bytes: u64,
    /// Share of messages that go to each community's first channel, the busy one.
    #[serde(default)]
    pub hot_channel_share: f64,
    #[serde(default)]
    pub voice: Option<VoiceBehaviour>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VoiceBehaviour {
    /// Calls joined per hour online.
    pub calls_per_hour: f64,
    /// How long each call lasts.
    pub call_minutes: f64,
    /// Share of calls in which the user also shares a screen.
    #[serde(default)]
    pub screen_share: f64,
    /// Bitrate of the microphone and of a shared screen, bits per second.
    #[serde(default = "default_audio_bitrate")]
    pub audio_bitrate: u32,
    #[serde(default = "default_screen_bitrate")]
    pub screen_bitrate: u32,
    /// How many voice channels calls gather in, per community: fewer means bigger calls.
    #[serde(default = "default_call_channels")]
    pub channels_used: u32,
}

/// Unknown fields are refused by `EventKind`: serde cannot also refuse them here, beside a
/// flattened field.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TimedEvent {
    /// Seconds after the steady phase starts.
    pub at_seconds: f64,
    #[serde(flatten)]
    pub kind: EventKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventKind {
    /// This share of online users lose their event stream at once and resume it, as after an
    /// outage.
    ReconnectStorm { fraction: f64 },
    /// Every action happens `factor` times as often for `seconds`: an announcement, a match
    /// ending.
    Spike { factor: f64, seconds: f64 },
    /// A shell command the coordinator runs, to restart or break part of the deployment.
    Command { run: String },
}

/// What counts as keeping up. Every figure is over the steady phase.
#[derive(Clone, Debug, Serialize, Deserialize, SmartDefault)]
#[serde(default, deny_unknown_fields)]
pub struct Slo {
    /// A message reaching the other people in its channel, sender to receivers.
    pub delivery_p99_ms: Option<f64>,
    pub delivery_p50_ms: Option<f64>,
    /// Any request but signing in (slow on purpose, and judged by `connect_p99_ms`), and
    /// particular routes (`"POST /channels/{channel}/messages" = 300`).
    pub request_p99_ms: Option<f64>,
    pub route_p99_ms: BTreeMap<String, f64>,
    /// Signing in, starting up, and opening the event stream.
    pub connect_p99_ms: Option<f64>,
    /// From the server dropping an event stream to its resumption: outages, restarts.
    pub recovery_p99_ms: Option<f64>,
    /// Failed requests over all requests.
    #[default = 0.001]
    pub error_rate: f64,
    /// Voice: share of media packets lost, and jitter.
    pub voice_loss: Option<f64>,
    pub voice_jitter_ms: Option<f64>,
    /// The least share of the users meant to be online that must be connected in the steady
    /// phase: users who could not connect are failures, however quick everyone else was.
    #[default = 0.99]
    pub connected_share: f64,
}

/// Suspending the deployment's rate limits for the run (`aspen_limits::suspension`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsOptions {
    #[serde(default)]
    pub suspend: bool,
    /// `networks` (the default) exempts `networks` from the limits by address, leaving every
    /// other limit in force; `all` lifts them all.
    #[serde(default)]
    pub scope: SuspendScope,
    #[serde(default)]
    pub networks: Vec<String>,
    /// The deployment's NATS, where the suspension is written.
    pub nats_url: Option<String>,
    pub nats_token: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuspendScope {
    #[default]
    Networks,
    All,
}

/// Capacity mode: the steady phase repeats with more users online each time until a service
/// level breaks.
#[derive(Clone, Debug, Serialize, Deserialize, SmartDefault)]
#[serde(default, deny_unknown_fields)]
pub struct CapacityOptions {
    /// Online share of the first step, of each step's increase, and the most tried.
    #[default = 0.1]
    pub start: f64,
    #[default = 0.1]
    pub step: f64,
    #[default = 1.0]
    pub max: f64,
    /// How long each step's steady phase lasts.
    #[default = 120.0]
    pub step_seconds: f64,
}

fn default_smallest_community() -> u32 {
    10
}
fn default_size_exponent() -> f64 {
    1.0
}
fn default_text_channels() -> u32 {
    3
}
fn default_voice_channels() -> u32 {
    1
}
fn default_history() -> u32 {
    100
}
fn default_report_interval() -> f64 {
    5.0
}
fn default_seed() -> u64 {
    1
}
fn default_lag_limit() -> f64 {
    100.0
}
fn default_attachment_bytes() -> u64 {
    50_000
}
fn default_audio_bitrate() -> u32 {
    48_000
}
fn default_screen_bitrate() -> u32 {
    2_500_000
}
fn default_call_channels() -> u32 {
    1
}
impl Profile {
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let profile: Profile = toml::from_str(text).map_err(|e| e.to_string())?;
        profile.validate()?;
        Ok(profile)
    }

    /// Checks what serde cannot; the error says what is wrong.
    pub fn validate(&self) -> Result<(), String> {
        let p = &self.population;
        if p.users == 0 || p.communities == 0 {
            return Err("population.users and population.communities must be positive".into());
        }
        if p.largest_community > p.users || p.smallest_community > p.largest_community {
            return Err(
                "communities must satisfy smallest_community <= largest_community <= users".into(),
            );
        }
        if !(0.0..=1.0).contains(&self.load.online) {
            return Err("load.online is a share between 0 and 1".into());
        }
        if self.load.duration_seconds <= 0.0 || self.load.ramp_up_seconds < 0.0 {
            return Err(
                "load.duration_seconds must be positive and ramp_up_seconds not negative".into(),
            );
        }
        if self.behaviours.is_empty() {
            return Err("at least one behaviour is needed".into());
        }
        let shares: f64 = self.behaviours.values().map(|b| b.share).sum();
        if (shares - 1.0).abs() > 0.001 {
            return Err(format!("behaviour shares add up to {shares}, not 1"));
        }
        for (name, behaviour) in &self.behaviours {
            if !(0.0..=1.0).contains(&behaviour.hot_channel_share) {
                return Err(format!(
                    "behaviours.{name}.hot_channel_share is a share between 0 and 1"
                ));
            }
        }
        for event in &self.events {
            if let EventKind::ReconnectStorm { fraction } = event.kind
                && !(0.0..=1.0).contains(&fraction)
            {
                return Err("a reconnect storm's fraction is a share between 0 and 1".into());
            }
        }
        if self.limits.suspend {
            if self.limits.nats_url.is_none() {
                return Err("limits.suspend needs limits.nats_url".into());
            }
            if self.limits.scope == SuspendScope::Networks && self.limits.networks.is_empty() {
                return Err("limits.scope = \"networks\" needs limits.networks: the load generators' addresses".into());
            }
        }
        let c = &self.capacity;
        if !(c.start > 0.0
            && c.step > 0.0
            && c.max <= 1.0
            && c.start <= c.max
            && c.step_seconds > 0.0)
        {
            return Err(
                "capacity needs 0 < start <= max <= 1, a positive step, and positive step_seconds"
                    .into(),
            );
        }
        Ok(())
    }

    /// Community sizes, largest first.
    pub fn community_sizes(&self) -> Vec<u32> {
        let p = &self.population;
        (0..p.communities)
            .map(|rank| {
                let size =
                    f64::from(p.largest_community) / f64::from(rank + 1).powf(p.size_exponent);
                (size.round() as u32).clamp(p.smallest_community, p.largest_community)
            })
            .collect()
    }

    /// The population to seed. Members of each community are a run of consecutive users that
    /// starts where the previous community's ended, wrapping around, so communities overlap
    /// and everyone belongs somewhere once the sizes add up to the population.
    pub fn seed_plan(&self, run: &str) -> SeedPlan {
        let p = &self.population;
        let mut start = 0u64;
        let communities = self
            .community_sizes()
            .into_iter()
            .map(|size| {
                let members = (0..u64::from(size))
                    .map(|offset| ((start + offset) % u64::from(p.users)) as u32)
                    .collect();
                start = (start + u64::from(size)) % u64::from(p.users);
                CommunityPlan {
                    members,
                    text_channels: p.text_channels,
                    voice_channels: p.voice_channels,
                    history_per_channel: p.history_per_channel,
                }
            })
            .collect();
        SeedPlan {
            run: run.to_string(),
            password: p.password.clone(),
            users: p.users,
            communities,
        }
    }

    /// Who is online, in the order they connect, each with the name of their behaviour, for
    /// an online share of `online`.
    pub fn roster(&self, online: f64) -> Vec<(u32, String)> {
        let users = self.population.users;
        let mut order: Vec<u32> = (0..users).collect();
        let mut rng = StdRng::seed_from_u64(self.load.seed);
        // Fisher-Yates, so the choice depends only on the seed.
        for i in (1..order.len()).rev() {
            let j = rng.random_range(0..=i);
            order.swap(i, j);
        }
        let count = ((f64::from(users) * online).round() as usize).min(order.len());
        let names: Vec<(&String, f64)> =
            self.behaviours.iter().map(|(n, b)| (n, b.share)).collect();
        order
            .into_iter()
            .take(count)
            .enumerate()
            .map(|(position, user)| {
                // Behaviours are dealt by position, so every share is honoured to within one
                // user whatever the count.
                let point = (position as f64 + 0.5) / count as f64;
                let mut cumulative = 0.0;
                let name = names
                    .iter()
                    .find(|(_, share)| {
                        cumulative += share;
                        point <= cumulative
                    })
                    .or(names.last())
                    .map(|(n, _)| (*n).clone())
                    .unwrap_or_default();
                (user, name)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> Profile {
        Profile::from_toml(
            r#"
name = "test"
[target]
api = "http://localhost:8000"
[population]
users = 1000
communities = 5
largest_community = 600
smallest_community = 50
[load]
online = 0.5
ramp_up_seconds = 10
duration_seconds = 60
[behaviours.lurker]
share = 0.75
history_reads_per_hour = 6
[behaviours.chatter]
share = 0.25
messages_per_hour = 30
[[events]]
at_seconds = 30
kind = "reconnect_storm"
fraction = 0.5
[slo]
delivery_p99_ms = 250
"#,
        )
        .unwrap()
    }

    #[test]
    fn community_sizes_fall_off_by_rank() {
        assert_eq!(profile().community_sizes(), vec![600, 300, 200, 150, 120]);
    }

    #[test]
    fn the_seed_plan_places_everyone() {
        let plan = profile().seed_plan("r1");
        assert_eq!(plan.users, 1000);
        let mut members = std::collections::HashSet::new();
        for community in &plan.communities {
            members.extend(community.members.iter().copied());
        }
        assert_eq!(
            members.len(),
            1000,
            "sizes add up to 1370, so everyone is somewhere"
        );
        assert!(plan.validate(500).is_ok());
    }

    #[test]
    fn the_roster_honours_shares_and_the_seed() {
        let p = profile();
        let roster = p.roster(0.5);
        assert_eq!(roster.len(), 500);
        let chatters = roster.iter().filter(|(_, b)| b == "chatter").count();
        assert!((124..=126).contains(&chatters), "{chatters}");
        assert_eq!(roster, p.roster(0.5));
        let distinct: std::collections::HashSet<u32> = roster.iter().map(|(u, _)| *u).collect();
        assert_eq!(distinct.len(), 500);
    }

    #[test]
    fn mistakes_are_explained() {
        let bad = |edit: &str| {
            let text = format!(
                "name = \"t\"\n[target]\napi = \"x\"\n[population]\nusers = 10\ncommunities = 1\nlargest_community = 10\nsmallest_community = 1\n[load]\nonline = 1\nramp_up_seconds = 0\nduration_seconds = 1\n{edit}"
            );
            Profile::from_toml(&text).unwrap_err()
        };
        assert!(bad("[behaviours.a]\nshare = 0.5\n").contains("add up"));
        assert!(
            bad("[behaviours.a]\nshare = 1\nmessages_per_minute = 3\n").contains("unknown field")
        );
        assert!(
            bad("[behaviours.a]\nshare = 1\n[limits]\nsuspend = true\nnats_url = \"nats://x\"\n")
                .contains("networks")
        );
    }

    #[test]
    fn events_read_from_toml() {
        assert_eq!(
            profile().events[0].kind,
            EventKind::ReconnectStorm { fraction: 0.5 }
        );
    }
}
