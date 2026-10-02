//! The directory of deployments: every other deployment this one knows, as rows of
//! `federated_deployment`, and the lists each is on, as rows of `federation_list_entry`.

use super::{Domain, FederationList, MAX_NOTE_CHARS, own_domain, protocol};
use crate::app::{self, UserId};
use crate::aspen_config::FederationConfig;
use crate::database::schema::{federated_deployment, federation_list_entry};
use crate::t;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::{AsExpression, FromSqlRow};
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

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
