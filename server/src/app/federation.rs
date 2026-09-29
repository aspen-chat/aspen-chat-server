//! Federation: this deployment among others. A deployment is known by its domain
//! ([`Domain`]) and proves what it says with an Ed25519 key pair, whose public half it
//! publishes at `https://{domain}/.well-known/aspen` ([`DeploymentDocument`]) with its policy.
//!
//! The policy is two gates each for users and for bots (`[federation]` in aspen.toml): who may
//! go from here to other deployments (emigration) and who may come here from them
//! (immigration). A gate is closed, open, or governed by a list of deployments that it allows or
//! blocks ([`FederationList`]); with `shared_list`, both directions read one list.
//!
//! Every other deployment this one knows is a row of `federated_deployment`, added by an
//! administrator (from the dashboard, under Manage federation, or the terminal) or recorded when
//! it is first contacted. Contacting a deployment ([`contact`]) reads its document and pins the
//! key found there the first time; a different key later is refused, and waits as the
//! deployment's offered key until an administrator accepts it ([`accept_key`]).

pub mod abroad;
pub mod contact;
pub mod fetch;
pub mod jws;
pub mod keys;
pub mod notices;
pub mod protocol;
pub mod received;
pub mod standing;

pub use contact::{ContactOutcome, contact, fetch_document, record_contact};
pub use keys::{
    DeploymentDocument, Gates, Rotation, current_key, document, ensure_key, fingerprint, rotate_key,
};

use crate::app::{self, UserId};
use crate::aspen_config::{FederationConfig, Gate, MigrationRules};
use crate::database::schema::{federated_deployment, federation_list_entry};
use crate::t;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::{AsExpression, FromSqlRow};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;
use utoipa::ToSchema;

/// Where a deployment publishes its [`DeploymentDocument`].
pub const WELL_KNOWN_PATH: &str = "/.well-known/aspen";
/// The longest note an administrator may keep on a deployment, in characters.
pub const MAX_NOTE_CHARS: usize = 200;

/// A deployment's name: a DNS name of at least two labels, in lowercase, with `:port` when it
/// is served on a port other than 443. It is never an IP address, since a deployment's
/// certificate and identity belong to a name.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    ToSchema,
    FromSqlRow,
    AsExpression,
)]
#[serde(try_from = "String", into = "String")]
#[schema(value_type = String, example = "chat.example.org")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub struct Domain(String);

#[derive(Debug, thiserror::Error)]
#[error("not a deployment domain")]
pub struct InvalidDomain;

impl Domain {
    /// Reads a domain as someone might write it: surrounding space and case do not matter, and
    /// `:443` is the same as no port.
    pub fn parse(text: &str) -> Result<Self, InvalidDomain> {
        let text = text.trim().to_ascii_lowercase();
        let (host, port) = match text.rsplit_once(':') {
            Some((host, port)) => {
                // Only digits: `u16::from_str` would take a leading `+`.
                if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(InvalidDomain);
                }
                let port: u16 = port.parse().map_err(|_| InvalidDomain)?;
                if port == 0 {
                    return Err(InvalidDomain);
                }
                (host, (port != 443).then_some(port))
            }
            None => (text.as_str(), None),
        };
        let host = host.strip_suffix('.').unwrap_or(host);
        let labels: Vec<&str> = host.split('.').collect();
        let label_ok = |label: &&str| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        };
        if host.len() > 253
            || labels.len() < 2
            || !labels.iter().all(label_ok)
            // A last label of digits alone would make an IPv4 address a domain.
            || labels.last().is_some_and(|l| l.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(InvalidDomain);
        }
        Ok(Domain(match port {
            Some(port) => format!("{host}:{port}"),
            None => host.to_string(),
        }))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Where the deployment publishes its document.
    pub fn document_url(&self) -> String {
        format!("https://{}{WELL_KNOWN_PATH}", self.0)
    }
}

impl fmt::Display for Domain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for Domain {
    type Err = InvalidDomain;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Self::parse(text)
    }
}

impl TryFrom<String> for Domain {
    type Error = InvalidDomain;
    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::parse(&text)
    }
}

impl From<Domain> for String {
    fn from(domain: Domain) -> Self {
        domain.0
    }
}

app::text_sql_traits!(Domain);

impl JsonSchema for Domain {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Domain".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "A deployment's name: a DNS name of two or more labels, in lowercase, \
                with `:port` when it is not served on 443.",
            "examples": ["chat.example.org"]
        })
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
/// it allows or blocks. Only the lists `[federation]` puts in force are read; the others keep
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

app::wire_name_traits!(FederationList);
app::text_sql_traits!(FederationList);

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

    /// The list that decides `subject`'s crossings `direction` under `config`, if a list
    /// decides them.
    pub fn in_force(
        config: &FederationConfig,
        subject: Subject,
        direction: Direction,
    ) -> Option<Self> {
        let rules = rules_for(config, subject);
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

    /// Every list `config` puts in force, each once.
    pub fn all_in_force(config: &FederationConfig) -> Vec<Self> {
        let mut lists = Vec::new();
        for subject in [Subject::Users, Subject::Bots] {
            for direction in [Direction::Emigration, Direction::Immigration] {
                if let Some(list) = Self::in_force(config, subject, direction)
                    && !lists.contains(&list)
                {
                    lists.push(list);
                }
            }
        }
        lists
    }
}

fn rules_for(config: &FederationConfig, subject: Subject) -> &MigrationRules {
    match subject {
        Subject::Users => &config.users,
        Subject::Bots => &config.bots,
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
    config: &FederationConfig,
    subject: Subject,
    direction: Direction,
    lists: &[FederationList],
) -> bool {
    match gate(rules_for(config, subject), direction) {
        Gate::Closed | Gate::Unknown => false,
        Gate::Open => true,
        Gate::AllowList | Gate::BlockList => {
            let list = FederationList::in_force(config, subject, direction)
                .expect("a gate with a list has a list in force");
            let on = lists.contains(&list);
            match list.parts().2 {
                ListKind::Allow => on,
                ListKind::Block => !on,
            }
        }
    }
}

/// This deployment's own domain, if it has one.
pub fn own_domain(config: &FederationConfig) -> Option<Domain> {
    config.domain.as_deref().and_then(|d| Domain::parse(d).ok())
}

// ---------------------------------------------------------------------------
// The directory of deployments
// ---------------------------------------------------------------------------

/// How a deployment came to be known.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, FromSqlRow, AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum Origin {
    /// Added from the Administration Dashboard.
    Administrator,
    /// Added from the terminal.
    Terminal,
    /// Recorded when it was first contacted.
    FirstContact,
}

app::wire_name_traits!(Origin);
app::text_sql_traits!(Origin);

/// Another deployment this one knows.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = federated_deployment)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct FederatedDeployment {
    pub domain: Domain,
    pub origin: Origin,
    pub added_by: Option<UserId>,
    pub created_at: DateTime<Utc>,
    pub note: Option<String>,
    /// The key pinned when it was first contacted; `None` until then.
    pub public_key: Option<Vec<u8>>,
    pub first_contact_at: Option<DateTime<Utc>>,
    pub last_contact_at: Option<DateTime<Utc>>,
    /// A key it presented other than the pinned one, refused until accepted.
    pub offered_key: Option<Vec<u8>>,
    pub offered_key_at: Option<DateTime<Utc>>,
    /// The range of protocol versions it said it speaks when last contacted.
    pub protocol_version: Option<i32>,
    pub protocol_minimum: Option<i32>,
    pub capabilities: Vec<Option<String>>,
    pub software_name: Option<String>,
    pub software_version: Option<String>,
}

impl FederatedDeployment {
    /// The protocol it said it speaks when last contacted; `None` before any contact.
    pub fn protocol(&self) -> Option<protocol::Protocol> {
        Some(protocol::Protocol {
            version: u32::try_from(self.protocol_version?).ok()?,
            minimum: u32::try_from(self.protocol_minimum?).ok()?,
            capabilities: self.capabilities.iter().flatten().cloned().collect(),
        })
    }
}

/// A deployment with the lists it is on.
#[derive(Debug, Clone)]
pub struct Listed {
    pub deployment: FederatedDeployment,
    pub lists: Vec<FederationList>,
}

fn clean_note(note: Option<String>) -> app::Result<Option<String>> {
    let note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    if note
        .as_ref()
        .is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS)
    {
        return Err(app::Error::Validation(t!(
            "federationNoteLength",
            max = MAX_NOTE_CHARS
        )));
    }
    Ok(note)
}

pub(crate) async fn lists_of(
    conn: &mut AsyncPgConnection,
    domains: &[Domain],
) -> app::Result<HashMap<Domain, Vec<FederationList>>> {
    let rows: Vec<(Domain, FederationList)> = federation_list_entry::table
        .select((federation_list_entry::domain, federation_list_entry::list))
        .filter(federation_list_entry::domain.eq_any(domains))
        .order((federation_list_entry::domain, federation_list_entry::list))
        .load(conn)
        .await?;
    let mut lists: HashMap<Domain, Vec<FederationList>> = HashMap::new();
    for (domain, list) in rows {
        lists.entry(domain).or_default().push(list);
    }
    for found in lists.values_mut() {
        found.sort();
    }
    Ok(lists)
}

/// One page of the deployments whose domain contains `search`, alphabetically: `limit` of them
/// (at most `app::admin::MAX_PAGE`) from `offset` (at most `app::admin::MAX_OFFSET`).
pub async fn list(
    conn: &mut AsyncPgConnection,
    search: Option<&str>,
    offset: i64,
    limit: i64,
) -> app::Result<Vec<Listed>> {
    let mut query = federated_deployment::table
        .select(FederatedDeployment::as_select())
        .order(federated_deployment::domain)
        .offset(offset.clamp(0, app::admin::MAX_OFFSET))
        .limit(limit.clamp(1, app::admin::MAX_PAGE))
        .into_boxed();
    if let Some(pattern) = app::admin::contains_pattern(search) {
        query = query.filter(federated_deployment::domain.like(pattern));
    }
    let deployments: Vec<FederatedDeployment> = query.load(conn).await?;
    let domains: Vec<Domain> = deployments.iter().map(|d| d.domain.clone()).collect();
    let mut lists = lists_of(conn, &domains).await?;
    Ok(deployments
        .into_iter()
        .map(|deployment| Listed {
            lists: lists.remove(&deployment.domain).unwrap_or_default(),
            deployment,
        })
        .collect())
}

/// Every deployment known, with its lists, for the terminal.
pub async fn list_all(conn: &mut AsyncPgConnection) -> app::Result<Vec<Listed>> {
    let deployments: Vec<FederatedDeployment> = federated_deployment::table
        .select(FederatedDeployment::as_select())
        .order(federated_deployment::domain)
        .load(conn)
        .await?;
    let domains: Vec<Domain> = deployments.iter().map(|d| d.domain.clone()).collect();
    let mut lists = lists_of(conn, &domains).await?;
    Ok(deployments
        .into_iter()
        .map(|deployment| Listed {
            lists: lists.remove(&deployment.domain).unwrap_or_default(),
            deployment,
        })
        .collect())
}

pub async fn get(conn: &mut AsyncPgConnection, domain: &Domain) -> app::Result<Listed> {
    let deployment: FederatedDeployment = federated_deployment::table
        .select(FederatedDeployment::as_select())
        .find(domain)
        .first(conn)
        .await?;
    let lists = lists_of(conn, std::slice::from_ref(domain))
        .await?
        .remove(domain)
        .unwrap_or_default();
    Ok(Listed { deployment, lists })
}

/// Adds a deployment to the directory, not yet contacted. A deployment already known is a
/// conflict. This deployment's own domain is refused.
pub async fn add(
    config: &FederationConfig,
    conn: &mut AsyncPgConnection,
    domain: &Domain,
    origin: Origin,
    added_by: Option<UserId>,
    note: Option<String>,
) -> app::Result<Listed> {
    if own_domain(config).as_ref() == Some(domain) {
        return Err(app::Error::Validation(t!("federationOwnDomain")));
    }
    let note = clean_note(note)?;
    let inserted = diesel::insert_into(federated_deployment::table)
        .values((
            federated_deployment::domain.eq(domain),
            federated_deployment::origin.eq(origin),
            federated_deployment::added_by.eq(added_by),
            federated_deployment::note.eq(note),
        ))
        .on_conflict_do_nothing()
        .returning(FederatedDeployment::as_returning())
        .get_result(conn)
        .await
        .optional()?;
    match inserted {
        Some(deployment) => Ok(Listed {
            deployment,
            lists: Vec::new(),
        }),
        None => Err(app::Error::Conflict(t!("federationAlreadyKnown"))),
    }
}

/// Changes the note an administrator keeps on a deployment.
pub async fn set_note(
    conn: &mut AsyncPgConnection,
    domain: &Domain,
    note: Option<String>,
) -> app::Result<Listed> {
    let note = clean_note(note)?;
    let updated = diesel::update(federated_deployment::table.find(domain))
        .set(federated_deployment::note.eq(note))
        .execute(conn)
        .await?;
    if updated == 0 {
        return Err(diesel::result::Error::NotFound.into());
    }
    get(conn, domain).await
}

/// Forgets a deployment: its pinned key and every list it is on. Contacted again, it is a
/// stranger whose key is pinned afresh.
pub async fn remove(conn: &mut AsyncPgConnection, domain: &Domain) -> app::Result<()> {
    let removed = diesel::delete(federated_deployment::table.find(domain))
        .execute(conn)
        .await?;
    if removed == 0 {
        return Err(diesel::result::Error::NotFound.into());
    }
    Ok(())
}

/// Puts a known deployment on `list`, or takes it off. Returns whether anything changed.
pub async fn set_listed(
    conn: &mut AsyncPgConnection,
    domain: &Domain,
    list: FederationList,
    listed: bool,
    by: Option<UserId>,
) -> app::Result<bool> {
    if listed {
        let known = diesel::select(diesel::dsl::exists(
            federated_deployment::table.find(domain),
        ))
        .get_result::<bool>(conn)
        .await?;
        if !known {
            return Err(diesel::result::Error::NotFound.into());
        }
        let added = diesel::insert_into(federation_list_entry::table)
            .values((
                federation_list_entry::domain.eq(domain),
                federation_list_entry::list.eq(list),
                federation_list_entry::added_by.eq(by),
            ))
            .on_conflict_do_nothing()
            .execute(conn)
            .await?;
        Ok(added > 0)
    } else {
        let removed = diesel::delete(
            federation_list_entry::table
                .filter(federation_list_entry::domain.eq(domain))
                .filter(federation_list_entry::list.eq(list)),
        )
        .execute(conn)
        .await?;
        Ok(removed > 0)
    }
}

/// Accepts the key a deployment offered in place of its pinned one. `offered` must be the key
/// the administrator was shown, so a key offered since is not accepted unseen.
pub async fn accept_key(
    conn: &mut AsyncPgConnection,
    domain: &Domain,
    offered: &[u8],
) -> app::Result<Listed> {
    let accepted = diesel::update(
        federated_deployment::table
            .find(domain)
            .filter(federated_deployment::offered_key.eq(offered)),
    )
    .set((
        federated_deployment::public_key.eq(offered),
        federated_deployment::offered_key.eq(None::<Vec<u8>>),
        federated_deployment::offered_key_at.eq(None::<DateTime<Utc>>),
    ))
    .execute(conn)
    .await?;
    if accepted == 0 {
        let known = diesel::select(diesel::dsl::exists(
            federated_deployment::table.find(domain),
        ))
        .get_result::<bool>(conn)
        .await?;
        return Err(if known {
            app::Error::Conflict(t!("federationOfferChanged"))
        } else {
            diesel::result::Error::NotFound.into()
        });
    }
    get(conn, domain).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aspen_config::FederationDevelopment;

    #[test]
    fn domains_are_read_as_people_write_them() {
        let d = |text: &str| Domain::parse(text).map(String::from).ok();
        assert_eq!(d(" Chat.Example.org "), Some("chat.example.org".into()));
        assert_eq!(d("chat.example.org:443"), Some("chat.example.org".into()));
        assert_eq!(
            d("alpha.localhost:8443"),
            Some("alpha.localhost:8443".into())
        );
        assert_eq!(d("chat.example.org."), Some("chat.example.org".into()));
        for bad in [
            "localhost",
            "10.0.0.1",
            "https://chat.example.org",
            "chat.example.org/path",
            "chat..example.org",
            "-chat.example.org",
            "chat.example.org:0",
            "chat.example.org:+80",
            "chat.example.org:99999",
            "chat_example.org",
            "[::1]:443",
            "",
        ] {
            assert_eq!(d(bad), None, "{bad}");
        }
    }

    #[test]
    fn list_names_round_trip() {
        for list in FederationList::ALL {
            let (subject, direction, kind) = list.parts();
            assert_eq!(FederationList::of(subject, direction, kind), *list);
            assert_eq!(list.to_string().parse::<FederationList>().unwrap(), *list);
        }
    }

    fn config(users: MigrationRules) -> FederationConfig {
        FederationConfig {
            domain: Some("a.example".into()),
            users,
            bots: MigrationRules::default(),
            development: FederationDevelopment::default(),
            ..FederationConfig::default()
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
