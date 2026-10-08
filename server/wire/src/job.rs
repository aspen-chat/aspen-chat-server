//! Background jobs as the dashboard and the terminal name them: what kind of work each is, and
//! how soon it must be done (`app::jobs`).

use diesel::{AsExpression, FromSqlRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// How soon a job must be done. Classes are taken in this order, each always keeping one place
/// to run in, so later classes are never starved by earlier ones.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    strum::VariantArray,
)]
#[serde(rename_all = "camelCase")]
pub enum JobClass {
    /// Taking access away from what is already open, which someone is waiting on.
    Urgent,
    /// What someone is looking at a screen for: a code by mail, a preview of what they posted.
    Interactive,
    /// Cleaning up after a decision already made: a banned user's messages, a role's holders.
    Normal,
    /// Work for many at once: newsletters, digests, purges.
    Bulk,
    /// Upkeep no one waits on: sweeping what has expired.
    Maintenance,
}

crate::wire_name_traits!(JobClass);

impl JobClass {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    /// The number the `job` table stores, which orders the classes.
    pub fn rank(self) -> i16 {
        match self {
            Self::Urgent => 0,
            Self::Interactive => 1,
            Self::Normal => 2,
            Self::Bulk => 3,
            Self::Maintenance => 4,
        }
    }

    /// The class a stored rank names; an unknown one is taken as the least pressing.
    pub fn of_rank(rank: i16) -> Self {
        Self::ALL
            .iter()
            .copied()
            .find(|class| class.rank() == rank)
            .unwrap_or(Self::Maintenance)
    }
}

/// A kind of job: what it does, which decides how it is run.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
    strum::VariantArray,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum JobKind {
    /// Deleting the messages a ban's deletion window covers.
    DeleteMessagesBy,
    /// Sweeping jobs given up long enough ago.
    PruneFailedJobs,
    /// Sweeping sessions and sign-ins that have ended.
    SweepSignIns,
    /// Closing a poll at its deadline.
    ClosePoll,
    /// Ending the calls of voice servers that went silent, and calls left alone too long.
    ReapVoice,
    /// Taking a deleted role off its holders and the tags of it, then deleting it.
    PurgeRole,
    /// Taking a deleted custom emoji's reactions off, then deleting it and its picture.
    PurgeCustomEmoji,
    /// Deleting what plugins kept in a deleted community, channel, or account.
    ForgetPluginScope,
    /// Taking a removed plugin's notes away and its account out of its communities.
    RetirePlugin,
    /// Deleting everything a removed plugin kept.
    PurgePlugin,
    /// Signing out users from elsewhere whose homes the gates no longer admit.
    ShutOut,
    /// Rechecking every call on the deployment, for a change that touches them all.
    RecheckAllCalls,
}

crate::wire_name_traits!(JobKind);
crate::text_sql_traits!(JobKind);

impl JobKind {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;
}
