//! What the benchmark tool (`bench/`) exchanges with others.
//!
//! The tool turns a workload profile into a `SeedPlan`, which the API server's
//! `bench seed` command writes straight into the database, answering with a `Manifest` of what
//! it made. Everything seeded carries the plan's `run` tag, and `bench purge --run` removes it
//! with everything that came to depend on it.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub mod coordination;
pub mod words;

/// The longest run tag, which also names seeded users (`bench-{run}-{index}`).
pub const MAX_RUN_TAG: usize = 24;

/// Whether `run` can tag a benchmark run: 1 to `MAX_RUN_TAG` lowercase letters, digits, or
/// dashes.
pub fn valid_run_tag(run: &str) -> bool {
    !run.is_empty()
        && run.len() <= MAX_RUN_TAG
        && run
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn user_name(run: &str, index: u32) -> String {
    format!("bench-{run}-{index}")
}

/// A population to write into the database.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeedPlan {
    pub run: String,
    /// Every seeded user's password; left out, the seeder draws a random one for the run and
    /// gives it in the manifest, so no deployment's benchmark users share a published password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Users `0..users`.
    pub users: u32,
    pub communities: Vec<CommunityPlan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityPlan {
    /// Indices of the users who belong to it.
    pub members: Vec<u32>,
    pub text_channels: u32,
    pub voice_channels: u32,
    /// Messages already in each text channel, spread over the past week.
    pub history_per_channel: u32,
    /// Of those, how many start a thread, and how many replies each thread holds.
    #[serde(default)]
    pub threads_per_channel: u32,
    #[serde(default)]
    pub replies_per_thread: u32,
    /// Polls in each text channel besides its history, about half of them still open, and how
    /// many members have voted in each (at most every member).
    #[serde(default)]
    pub polls_per_channel: u32,
    #[serde(default)]
    pub votes_per_poll: u32,
    /// History messages in each text channel that tag a member.
    #[serde(default)]
    pub tagged_per_channel: u32,
}

/// What was seeded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub run: String,
    pub password: String,
    /// By index.
    pub users: Vec<SeededUser>,
    pub communities: Vec<SeededCommunity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeededUser {
    pub id: Uuid,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeededCommunity {
    pub id: Uuid,
    pub members: Vec<u32>,
    pub text_channels: Vec<Uuid>,
    pub voice_channels: Vec<Uuid>,
    /// The threads seeded in its text channels.
    #[serde(default)]
    pub threads: Vec<Uuid>,
    /// Its seeded polls still open, to vote in.
    #[serde(default)]
    pub open_polls: Vec<SeededPoll>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeededPoll {
    pub id: Uuid,
    pub options: u32,
}

impl SeedPlan {
    /// Checks the plan against the server's `max_communities_per_user`; the error says what is
    /// wrong.
    pub fn validate(&self, max_communities_per_user: u32) -> Result<(), String> {
        if !valid_run_tag(&self.run) {
            return Err(format!(
                "run tag {:?} must be 1 to {MAX_RUN_TAG} lowercase letters, digits, or dashes",
                self.run
            ));
        }
        if self
            .password
            .as_ref()
            .is_some_and(|password| password.len() < 8)
        {
            return Err("the password must be at least 8 characters".into());
        }
        let mut memberships = vec![0u32; self.users as usize];
        for (index, community) in self.communities.iter().enumerate() {
            if community.members.is_empty() {
                return Err(format!("community {index} has no members"));
            }
            if community.threads_per_channel > community.history_per_channel
                || community.tagged_per_channel > community.history_per_channel
            {
                return Err(format!(
                    "community {index} starts threads from, or tags in, more messages than its {} of history per channel",
                    community.history_per_channel
                ));
            }
            for member in &community.members {
                let count = memberships.get_mut(*member as usize).ok_or_else(|| {
                    format!(
                        "community {index} names user {member}, beyond the {} users",
                        self.users
                    )
                })?;
                *count += 1;
            }
        }
        if let Some((user, count)) = memberships
            .iter()
            .enumerate()
            .find(|(_, count)| **count > max_communities_per_user)
        {
            return Err(format!(
                "user {user} would belong to {count} communities, over the server's limit of {max_communities_per_user}"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> SeedPlan {
        SeedPlan {
            run: "r1".into(),
            password: Some("benchmark password".into()),
            users: 3,
            communities: vec![CommunityPlan {
                members: vec![0, 1, 2],
                text_channels: 1,
                voice_channels: 1,
                history_per_channel: 10,
                threads_per_channel: 1,
                replies_per_thread: 3,
                polls_per_channel: 1,
                votes_per_poll: 2,
                tagged_per_channel: 1,
            }],
        }
    }

    #[test]
    fn run_tags_are_short_and_plain() {
        assert!(valid_run_tag("nightly-42"));
        assert!(!valid_run_tag(""));
        assert!(!valid_run_tag("Nightly"));
        assert!(!valid_run_tag("a b"));
        assert!(!valid_run_tag(&"x".repeat(MAX_RUN_TAG + 1)));
    }

    #[test]
    fn plans_are_checked() {
        assert!(plan().validate(500).is_ok());
        let mut bad = plan();
        bad.communities[0].members.push(3);
        assert!(bad.validate(500).unwrap_err().contains("beyond"));
        let mut crowded = plan();
        crowded.communities.push(crowded.communities[0].clone());
        assert!(
            crowded
                .validate(1)
                .unwrap_err()
                .contains("over the server's limit")
        );
        let mut threaded = plan();
        threaded.communities[0].threads_per_channel = 11;
        assert!(threaded.validate(500).unwrap_err().contains("history"));
        let mut untagged = plan();
        untagged.run = "Has Spaces".into();
        assert!(untagged.validate(500).is_err());
    }
}
