//! Federation policy: the gates, which are deployment settings (`app::deployment_settings`), the
//! lists a deployment may be on ([`FederationList`]), and whether the gates admit a crossing
//! ([`admits`]).

use diesel::{AsExpression, FromSqlRow};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// One direction's gate.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    Hash,
    Deserialize,
    Serialize,
    ToSchema,
    JsonSchema,
    FromSqlRow,
    AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum Gate {
    /// No one crosses.
    #[default]
    Closed,
    /// Anyone crosses, to or from any deployment.
    Open,
    /// Only to or from the deployments on this direction's allow list.
    AllowList,
    /// To or from any deployment but those on this direction's block list.
    BlockList,
    /// A gate another deployment publishes that this one does not know, as a newer one may.
    /// Never set here: the settings hold only the gates above.
    #[serde(other)]
    Unknown,
}

aspen_wire::wire_name_traits!(Gate);
aspen_wire::text_sql_traits!(Gate);

/// Who may cross between this deployment and others, one direction at a time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MigrationRules {
    /// This deployment's accounts using other deployments.
    pub emigration: Gate,
    /// Other deployments' accounts using this one.
    pub immigration: Gate,
    /// Both directions read one list instead of a list each. Both gates then use a list, and
    /// the same kind of list.
    pub shared_list: bool,
    /// Whether an account of another deployment arriving here for the first time needs a
    /// registration invite (`app::registration_invite`), as the deployment setting
    /// `registration_invite_required` asks of accounts made here.
    pub immigration_invite_required: bool,
}

/// The deployment's federation policy: the gates of its users and, separately, its bots.
///
/// The policy, in the terms operators use, is the gates of its users: none (both closed, the
/// default), emigration (only `emigration` open or on a list), immigration (only `immigration`),
/// and full (both), each either open or with a list. Bots' gates work the same way.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FederationPolicy {
    pub users: MigrationRules,
    pub bots: MigrationRules,
}

impl FederationPolicy {
    /// Whether any gate lets anyone cross.
    pub fn enabled(&self) -> bool {
        [&self.users, &self.bots]
            .iter()
            .any(|rules| rules.emigration != Gate::Closed || rules.immigration != Gate::Closed)
    }

    /// Whether an immigration gate lets accounts of other deployments in.
    pub fn admits_anyone(&self) -> bool {
        self.users.immigration != Gate::Closed || self.bots.immigration != Gate::Closed
    }

    pub fn rules(&self, subject: Subject) -> &MigrationRules {
        match subject {
            Subject::Users => &self.users,
            Subject::Bots => &self.bots,
        }
    }
}

/// Whose crossing a gate governs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Subject {
    Users,
    Bots,
}

/// Which way a crossing goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// From this deployment to another.
    Emigration,
    /// From another deployment to this one.
    Immigration,
}

/// Which way a list governs: one direction, or both through the one list they share.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListDirection {
    Emigration,
    Immigration,
    Shared,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListKind {
    Allow,
    Block,
}

/// A list a deployment may be on, named by whose crossings it governs, which way, and whether
/// it allows or blocks. Only the lists the gates put in force are read; the others keep
/// their entries, so switching a gate from an allow list to a block list never turns the
/// deployments allowed into ones blocked.
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
    FromSqlRow,
    AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum FederationList {
    UsersEmigrationAllow,
    UsersEmigrationBlock,
    UsersImmigrationAllow,
    UsersImmigrationBlock,
    UsersSharedAllow,
    UsersSharedBlock,
    BotsEmigrationAllow,
    BotsEmigrationBlock,
    BotsImmigrationAllow,
    BotsImmigrationBlock,
    BotsSharedAllow,
    BotsSharedBlock,
}

aspen_wire::wire_name_traits!(FederationList);
aspen_wire::text_sql_traits!(FederationList);

impl FederationList {
    pub const ALL: &'static [Self] = <Self as strum::VariantArray>::VARIANTS;

    pub fn parts(self) -> (Subject, ListDirection, ListKind) {
        use ListDirection as D;
        use ListKind as K;
        use Subject as S;
        match self {
            Self::UsersEmigrationAllow => (S::Users, D::Emigration, K::Allow),
            Self::UsersEmigrationBlock => (S::Users, D::Emigration, K::Block),
            Self::UsersImmigrationAllow => (S::Users, D::Immigration, K::Allow),
            Self::UsersImmigrationBlock => (S::Users, D::Immigration, K::Block),
            Self::UsersSharedAllow => (S::Users, D::Shared, K::Allow),
            Self::UsersSharedBlock => (S::Users, D::Shared, K::Block),
            Self::BotsEmigrationAllow => (S::Bots, D::Emigration, K::Allow),
            Self::BotsEmigrationBlock => (S::Bots, D::Emigration, K::Block),
            Self::BotsImmigrationAllow => (S::Bots, D::Immigration, K::Allow),
            Self::BotsImmigrationBlock => (S::Bots, D::Immigration, K::Block),
            Self::BotsSharedAllow => (S::Bots, D::Shared, K::Allow),
            Self::BotsSharedBlock => (S::Bots, D::Shared, K::Block),
        }
    }

    pub fn of(subject: Subject, direction: ListDirection, kind: ListKind) -> Self {
        *Self::ALL
            .iter()
            .find(|list| list.parts() == (subject, direction, kind))
            .expect("every combination of parts names a list")
    }

    /// The list that decides `subject`'s crossings `direction` under `policy`, if a list
    /// decides them.
    pub fn in_force(
        policy: &FederationPolicy,
        subject: Subject,
        direction: Direction,
    ) -> Option<Self> {
        let rules = policy.rules(subject);
        let kind = match gate(rules, direction) {
            Gate::AllowList => ListKind::Allow,
            Gate::BlockList => ListKind::Block,
            Gate::Closed | Gate::Open | Gate::Unknown => return None,
        };
        let direction = match (rules.shared_list, direction) {
            (true, _) => ListDirection::Shared,
            (false, Direction::Emigration) => ListDirection::Emigration,
            (false, Direction::Immigration) => ListDirection::Immigration,
        };
        Some(Self::of(subject, direction, kind))
    }

    /// Every list `policy` puts in force, each once.
    pub fn all_in_force(policy: &FederationPolicy) -> Vec<Self> {
        let mut lists = Vec::new();
        for subject in [Subject::Users, Subject::Bots] {
            for direction in [Direction::Emigration, Direction::Immigration] {
                if let Some(list) = Self::in_force(policy, subject, direction)
                    && !lists.contains(&list)
                {
                    lists.push(list);
                }
            }
        }
        lists
    }
}

fn gate(rules: &MigrationRules, direction: Direction) -> Gate {
    match direction {
        Direction::Emigration => rules.emigration,
        Direction::Immigration => rules.immigration,
    }
}

/// Whether `subject` may cross `direction` between this deployment and one that is on `lists`.
pub fn admits(
    policy: &FederationPolicy,
    subject: Subject,
    direction: Direction,
    lists: &[FederationList],
) -> bool {
    match gate(policy.rules(subject), direction) {
        Gate::Closed | Gate::Unknown => false,
        Gate::Open => true,
        Gate::AllowList | Gate::BlockList => {
            let list = FederationList::in_force(policy, subject, direction)
                .expect("a gate with a list has a list in force");
            let on = lists.contains(&list);
            match list.parts().2 {
                ListKind::Allow => on,
                ListKind::Block => !on,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_names_round_trip() {
        for list in FederationList::ALL {
            let (subject, direction, kind) = list.parts();
            assert_eq!(FederationList::of(subject, direction, kind), *list);
            assert_eq!(list.to_string().parse::<FederationList>().unwrap(), *list);
        }
    }

    fn config(users: MigrationRules) -> FederationPolicy {
        FederationPolicy {
            users,
            bots: MigrationRules::default(),
        }
    }

    fn rules(emigration: Gate, immigration: Gate, shared_list: bool) -> MigrationRules {
        MigrationRules {
            emigration,
            immigration,
            shared_list,
            immigration_invite_required: false,
        }
    }

    #[test]
    fn gates_decide_by_their_own_lists() {
        use Direction::{Emigration, Immigration};
        use FederationList as L;
        let users = Subject::Users;
        let c = config(rules(Gate::AllowList, Gate::BlockList, false));
        assert!(!admits(&c, users, Emigration, &[]));
        assert!(admits(&c, users, Emigration, &[L::UsersEmigrationAllow]));
        // A list not in force is not read.
        assert!(!admits(&c, users, Emigration, &[L::UsersSharedAllow]));
        assert!(admits(&c, users, Immigration, &[L::UsersEmigrationAllow]));
        assert!(!admits(&c, users, Immigration, &[L::UsersImmigrationBlock]));
        // Bots have gates of their own, closed here.
        assert!(!admits(
            &c,
            Subject::Bots,
            Emigration,
            &[L::BotsEmigrationAllow]
        ));

        let shared = config(rules(Gate::BlockList, Gate::BlockList, true));
        assert!(admits(
            &shared,
            users,
            Emigration,
            &[L::UsersEmigrationBlock]
        ));
        assert!(!admits(&shared, users, Emigration, &[L::UsersSharedBlock]));
        assert!(!admits(&shared, users, Immigration, &[L::UsersSharedBlock]));
        assert_eq!(
            FederationList::all_in_force(&shared),
            vec![L::UsersSharedBlock]
        );

        let open = config(rules(Gate::Open, Gate::Closed, false));
        assert!(admits(&open, users, Emigration, &[L::UsersEmigrationBlock]));
        assert!(!admits(
            &open,
            users,
            Immigration,
            &[L::UsersImmigrationAllow]
        ));
        assert!(FederationList::all_in_force(&open).is_empty());
    }
}
