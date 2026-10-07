//! The directory of deployments: every other deployment this one knows, as rows of
//! `federated_deployment`, and the lists each is on, as rows of `federation_list_entry`.

use super::policy::ListKind;
use super::{Domain, FederationList, MAX_NOTE_CHARS, own_domain, protocol};
use super::{FederationPolicy, standing};
use crate::UserId;
use crate::aspen_config::FederationConfig;
use crate::events::Publishing;
use crate::t;
use aspen_schema::{federated_deployment, federation_list_entry};
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::{AsExpression, FromSqlRow};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

/// Whether the `federated_deployment` row in a query is in use: added by an administrator,
/// noted, on a list, the home of a user here, or used by one of this deployment's own users.
/// Only deployments in use have their documents read again at each standing pass, and a
/// deployment recorded on first contact that is not in use is forgotten once it has gone
/// uncontacted for [`UNUSED_FORGOTTEN_AFTER_DAYS`] ([`prune_unused`]).
pub const IN_USE_SQL: &str = "(federated_deployment.origin <> 'firstContact' \
     OR federated_deployment.note IS NOT NULL \
     OR EXISTS (SELECT 1 FROM federation_list_entry l \
         WHERE l.domain = federated_deployment.domain) \
     OR EXISTS (SELECT 1 FROM \"user\" u \
         WHERE u.home_domain = federated_deployment.domain AND u.deleted_at IS NULL) \
     OR EXISTS (SELECT 1 FROM user_foreign_deployment f \
         WHERE f.domain = federated_deployment.domain))";

/// The SQL condition that the `federated_deployment` row in a query is in use.
pub fn in_use() -> diesel::expression::SqlLiteral<diesel::sql_types::Bool> {
    diesel::dsl::sql::<diesel::sql_types::Bool>(IN_USE_SQL)
}

/// How long a deployment recorded on first contact and not in use is kept after it was last
/// contacted.
pub const UNUSED_FORGOTTEN_AFTER_DAYS: i64 = 30;

/// Forgets the deployments recorded on first contact that are not in use ([`IN_USE_SQL`]) and
/// were last contacted more than [`UNUSED_FORGOTTEN_AFTER_DAYS`] ago, with their pins, so names
/// that were contacted once and never used do not pile up. One waiting on an administrator to
/// accept a key it offered is kept, since forgetting it would lift its suspension. Returns how
/// many were forgotten. A deployment on no list and with no users here admits or refuses no one
/// differently once forgotten, so no one's sessions change.
pub async fn prune_unused(conn: &mut AsyncPgConnection) -> crate::Result<usize> {
    let before = Utc::now() - chrono::Duration::days(UNUSED_FORGOTTEN_AFTER_DAYS);
    let pruned = diesel::delete(federated_deployment::table)
        .filter(federated_deployment::origin.eq(Origin::FirstContact))
        .filter(federated_deployment::offered_key.is_null())
        .filter(
            diesel::dsl::sql::<diesel::sql_types::Timestamptz>(
                "coalesce(federated_deployment.last_contact_at, federated_deployment.created_at)",
            )
            .lt(before),
        )
        .filter(diesel::dsl::not(in_use()))
        .execute(conn)
        .await?;
    Ok(pruned)
}

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

crate::wire_name_traits!(Origin);
crate::text_sql_traits!(Origin);

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
    /// The lists it is on itself.
    pub lists: Vec<FederationList>,
    /// The lists that decide whether it is admitted ([`lists_of`]): its own, and the block
    /// lists of every deployment whose host is its host or a parent of it.
    pub deciding: Vec<FederationList>,
}

fn clean_note(note: Option<String>) -> crate::Result<Option<String>> {
    let note = note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
    if note
        .as_ref()
        .is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS)
    {
        return Err(crate::Error::Validation(t!(
            "federationNoteLength",
            max = MAX_NOTE_CHARS
        )));
    }
    Ok(note)
}

diesel::define_sql_function! {
    /// PostgreSQL's `split_part`, which takes a domain's host from before its port.
    fn split_part(text: diesel::sql_types::Text, delimiter: diesel::sql_types::Text, field: diesel::sql_types::Integer) -> diesel::sql_types::Text;
}

/// The lists each of `domains` is on itself, as the directory shows and edits them.
pub async fn entries_of(
    conn: &mut AsyncPgConnection,
    domains: &[Domain],
) -> crate::Result<HashMap<Domain, Vec<FederationList>>> {
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

/// The lists that decide whether each of `domains` is admitted, which is what [`admits`]
/// (`super::admits`) reads: the lists it is on itself, and every block list a deployment is on
/// whose host is its host or a parent of it, on any port ([`Domain::blocked_by`]), so blocking
/// `evil.org` blocks `a.evil.org` and `evil.org:8443` too. An allow list admits only the
/// deployments on it.
pub async fn lists_of(
    conn: &mut AsyncPgConnection,
    domains: &[Domain],
) -> crate::Result<HashMap<Domain, Vec<FederationList>>> {
    let mut lists = entries_of(conn, domains).await?;
    let hosts: Vec<&str> = domains
        .iter()
        .flat_map(Domain::covering_hosts)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let blocks: Vec<FederationList> = FederationList::ALL
        .iter()
        .copied()
        .filter(|list| list.parts().2 == ListKind::Block)
        .collect();
    let covering: Vec<(Domain, FederationList)> = federation_list_entry::table
        .select((federation_list_entry::domain, federation_list_entry::list))
        .filter(federation_list_entry::list.eq_any(&blocks))
        .filter(split_part(federation_list_entry::domain, ":", 1).eq_any(&hosts))
        .load(conn)
        .await?;
    for domain in domains {
        for (entry, list) in &covering {
            if domain.blocked_by(entry) {
                lists.entry(domain.clone()).or_default().push(*list);
            }
        }
    }
    for found in lists.values_mut() {
        found.sort();
        found.dedup();
    }
    Ok(lists)
}

/// `deployments` with the lists each is on and the lists that decide it.
async fn listed(
    conn: &mut AsyncPgConnection,
    deployments: Vec<FederatedDeployment>,
) -> crate::Result<Vec<Listed>> {
    let domains: Vec<Domain> = deployments.iter().map(|d| d.domain.clone()).collect();
    let mut lists = entries_of(conn, &domains).await?;
    let mut deciding = lists_of(conn, &domains).await?;
    Ok(deployments
        .into_iter()
        .map(|deployment| Listed {
            lists: lists.remove(&deployment.domain).unwrap_or_default(),
            deciding: deciding.remove(&deployment.domain).unwrap_or_default(),
            deployment,
        })
        .collect())
}

/// One page of the deployments whose domain contains `search`, alphabetically: `limit` of them
/// (at most `app::admin::MAX_PAGE`) from `offset` (at most `app::admin::MAX_OFFSET`).
pub async fn list(
    conn: &mut AsyncPgConnection,
    search: Option<&str>,
    offset: i64,
    limit: i64,
) -> crate::Result<Vec<Listed>> {
    let mut query = federated_deployment::table
        .select(FederatedDeployment::as_select())
        .order(federated_deployment::domain)
        .offset(offset.clamp(0, crate::admin::MAX_OFFSET))
        .limit(limit.clamp(1, crate::admin::MAX_PAGE))
        .into_boxed();
    if let Some(pattern) = crate::admin::contains_pattern(search)? {
        query = query.filter(federated_deployment::domain.like(pattern));
    }
    let deployments: Vec<FederatedDeployment> = query.load(conn).await?;
    listed(conn, deployments).await
}

/// Every deployment known, with its lists, for the terminal.
pub async fn list_all(conn: &mut AsyncPgConnection) -> crate::Result<Vec<Listed>> {
    let deployments: Vec<FederatedDeployment> = federated_deployment::table
        .select(FederatedDeployment::as_select())
        .order(federated_deployment::domain)
        .load(conn)
        .await?;
    listed(conn, deployments).await
}

pub async fn get(conn: &mut AsyncPgConnection, domain: &Domain) -> crate::Result<Listed> {
    let deployment: FederatedDeployment = federated_deployment::table
        .select(FederatedDeployment::as_select())
        .find(domain)
        .first(conn)
        .await?;
    Ok(listed(conn, vec![deployment])
        .await?
        .pop()
        .expect("one deployment is listed as one"))
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
) -> crate::Result<Listed> {
    if own_domain(config).as_ref() == Some(domain) {
        return Err(crate::Error::Validation(t!("federationOwnDomain")));
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
        Some(deployment) => Ok(listed(conn, vec![deployment])
            .await?
            .pop()
            .expect("one deployment is listed as one")),
        None => Err(crate::Error::Conflict(t!("federationAlreadyKnown"))),
    }
}

/// Changes the note an administrator keeps on a deployment.
pub async fn set_note(
    conn: &mut AsyncPgConnection,
    domain: &Domain,
    note: Option<String>,
) -> crate::Result<Listed> {
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
/// stranger whose key is pinned afresh. One on a block list is not forgotten, since forgetting
/// it would take it off the list and admit it wherever a gate blocks only those listed: it must
/// be taken off its block lists first. Users of its own signed in here whom the gates no longer
/// admit once it is forgotten, as when it was on an allow list, are signed out at once.
pub async fn remove(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    policy: &FederationPolicy,
    domain: &Domain,
) -> crate::Result<()> {
    let blocked = entries_of(conn, std::slice::from_ref(domain))
        .await?
        .remove(domain)
        .unwrap_or_default()
        .into_iter()
        .find(|list| list.parts().2 == ListKind::Block);
    if let Some(list) = blocked {
        return Err(crate::Error::Conflict(t!(
            "federationRemoveBlocked",
            domain = domain.as_str(),
            list = list.to_string()
        )));
    }
    let removed = diesel::delete(federated_deployment::table.find(domain))
        .execute(conn)
        .await?;
    if removed == 0 {
        return Err(diesel::result::Error::NotFound.into());
    }
    standing::shut_out(state, conn, policy).await
}

/// Puts a known deployment on `list`, or takes it off. Returns whether anything changed. Users
/// from elsewhere signed in here whom the gates no longer admit, as when a deployment is put on
/// a block list or taken off an allow list, are signed out at once, whether or not this call
/// changed anything, so trying again after a failure finishes the job.
pub async fn set_listed(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    policy: &FederationPolicy,
    domain: &Domain,
    list: FederationList,
    listed: bool,
    by: Option<UserId>,
) -> crate::Result<bool> {
    let changed = if listed {
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
        added > 0
    } else {
        let removed = diesel::delete(
            federation_list_entry::table
                .filter(federation_list_entry::domain.eq(domain))
                .filter(federation_list_entry::list.eq(list)),
        )
        .execute(conn)
        .await?;
        removed > 0
    };
    standing::shut_out(state, conn, policy).await?;
    Ok(changed)
}

/// Accepts the key a deployment offered in place of its pinned one. `offered` must be the key
/// the administrator was shown, so a key offered since is not accepted unseen.
pub async fn accept_key(
    conn: &mut AsyncPgConnection,
    domain: &Domain,
    offered: &[u8],
) -> crate::Result<Listed> {
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
            crate::Error::Conflict(t!("federationOfferChanged"))
        } else {
            diesel::result::Error::NotFound.into()
        });
    }
    get(conn, domain).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel_async::{AsyncConnection, SimpleAsyncConnection};

    /// Runs against the database `DATABASE_URL` names, inside a transaction that is never
    /// committed; without one it checks nothing.
    #[tokio::test]
    async fn only_unused_deployments_long_uncontacted_are_forgotten() {
        let Ok(url) = std::env::var("DATABASE_URL") else {
            eprintln!("DATABASE_URL is not set; skipped");
            return;
        };
        let mut conn = AsyncPgConnection::establish(&url).await.unwrap();
        conn.begin_test_transaction().await.unwrap();
        conn.batch_execute(
            "INSERT INTO federated_deployment (domain, origin, created_at, note) VALUES
             ('unused.prune.test', 'firstContact', now() - interval '60 days', NULL),
             ('never.prune.test', 'firstContact', now() - interval '60 days', NULL),
             ('recent.prune.test', 'firstContact', now() - interval '60 days', NULL),
             ('noted.prune.test', 'firstContact', now() - interval '60 days', 'kept'),
             ('offered.prune.test', 'firstContact', now() - interval '60 days', NULL),
             ('added.prune.test', 'administrator', now() - interval '60 days', NULL),
             ('listed.prune.test', 'firstContact', now() - interval '60 days', NULL);
             UPDATE federated_deployment SET public_key = decode(repeat('01', 32), 'hex'),
                 first_contact_at = now() - interval '60 days',
                 last_contact_at = now() - interval '40 days'
                 WHERE domain LIKE '%.prune.test' AND domain <> 'never.prune.test';
             UPDATE federated_deployment SET last_contact_at = now() - interval '2 days'
                 WHERE domain = 'recent.prune.test';
             UPDATE federated_deployment SET offered_key = decode(repeat('02', 32), 'hex'), offered_key_at = now()
                 WHERE domain = 'offered.prune.test';
             INSERT INTO federation_list_entry (domain, list) VALUES ('listed.prune.test', 'usersImmigrationAllow');",
        )
        .await
        .unwrap();
        assert!(prune_unused(&mut conn).await.unwrap() >= 2);
        let left: Vec<String> = federated_deployment::table
            .select(federated_deployment::domain)
            .filter(federated_deployment::domain.like("%.prune.test"))
            .order(federated_deployment::domain)
            .load(&mut conn)
            .await
            .unwrap();
        assert_eq!(
            left,
            [
                "added.prune.test",
                "listed.prune.test",
                "noted.prune.test",
                "offered.prune.test",
                "recent.prune.test",
            ]
        );
    }
}
