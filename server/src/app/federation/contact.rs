//! Contacting another deployment: reading its document and checking the key it presents
//! against the one pinned for it.

use crate::api::GlobalServerContext;
use crate::app;
use crate::app::federation::keys::{DeploymentDocument, current_of, follow_handovers};
use crate::app::federation::{Domain, Listed, Origin, fetch, get, own_domain};
use crate::aspen_config::FederationConfig;
use crate::database::schema::federated_deployment;
use crate::t;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::Serialize;
use utoipa::ToSchema;

/// What contacting a deployment found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub enum ContactOutcome {
    /// It had no pinned key, and the one it presented is now pinned.
    Pinned,
    /// It presented its pinned key.
    Confirmed,
    /// It replaced its key as planned: handovers lead from the pinned key to the one it
    /// presented, which is now pinned.
    HandedOver,
    /// It presented a different key that nothing vouches for, which is refused and kept as its
    /// offered key until an administrator accepts it.
    KeyChanged,
}

/// Reads `domain`'s document and checks its key against the one pinned, pinning it when none
/// is. A deployment not yet known is recorded as first contacted, so callers decide whether
/// policy admits it before contacting it.
pub async fn contact(
    state: &GlobalServerContext,
    domain: &Domain,
) -> app::Result<(Listed, ContactOutcome)> {
    let document =
        fetch_document(&state.config.federation, &state.federation_client, domain).await?;
    // The connection is taken once the other deployment has answered, not held while it might.
    record_contact(
        state.connection_pool.get().await?.as_mut(),
        domain,
        &document,
    )
    .await
}

/// `domain`'s document as it is now, which must name `domain` and present an Ed25519 key.
pub async fn fetch_document(
    config: &FederationConfig,
    client: &reqwest::Client,
    domain: &Domain,
) -> app::Result<DeploymentDocument> {
    if own_domain(config).as_ref() == Some(domain) {
        return Err(app::Error::Validation(t!("federationOwnDomain")));
    }
    let document = fetch::document(client, domain).await?;
    check_document(domain, &document)?;
    Ok(document)
}

/// Whether a document fetched for `domain` is one: it names `domain`, and its current key is
/// an Ed25519 key.
fn check_document(domain: &Domain, document: &DeploymentDocument) -> app::Result<()> {
    if document.domain != *domain {
        return Err(app::Error::DeploymentUnreachable(t!(
            "federationDocumentOtherDomain",
            domain = document.domain.as_str()
        )));
    }
    if current_of(document).is_none() {
        return Err(app::Error::DeploymentUnreachable(t!(
            "federationDocumentInvalid",
            domain = domain.as_str()
        )));
    }
    Ok(())
}

/// Checks the key `document` presents against the one pinned for `domain`, as [`contact`]
/// describes.
pub async fn record_contact(
    conn: &mut AsyncPgConnection,
    domain: &Domain,
    document: &DeploymentDocument,
) -> app::Result<(Listed, ContactOutcome)> {
    let presented = current_of(document).ok_or_else(|| {
        app::Error::DeploymentUnreachable(t!("federationDocumentInvalid", domain = domain.as_str()))
    })?;
    let domain = domain.clone();
    conn.transaction(|conn| {
        async move {
            diesel::insert_into(federated_deployment::table)
                .values((
                    federated_deployment::domain.eq(&domain),
                    federated_deployment::origin.eq(Origin::FirstContact),
                ))
                .on_conflict_do_nothing()
                .execute(conn)
                .await?;
            let (pinned, offered): (Option<Vec<u8>>, Option<Vec<u8>>) = federated_deployment::table
                .find(&domain)
                .select((
                    federated_deployment::public_key,
                    federated_deployment::offered_key,
                ))
                .for_update()
                .first(conn)
                .await?;
            let row = federated_deployment::table.find(&domain);
            let now = diesel::dsl::now;
            // What it says of itself is recorded whatever its key: it is only read, never
            // trusted with anything.
            let software = document.software.clone();
            diesel::update(row)
                .set((
                    federated_deployment::protocol_version
                        .eq(i32::try_from(document.protocol.version).unwrap_or(i32::MAX)),
                    federated_deployment::protocol_minimum
                        .eq(i32::try_from(document.protocol.minimum).unwrap_or(i32::MAX)),
                    federated_deployment::capabilities.eq(document
                        .protocol
                        .capabilities
                        .iter()
                        .map(|c| Some(c.clone()))
                        .collect::<Vec<_>>()),
                    federated_deployment::software_name
                        .eq(software.as_ref().map(|s| s.name.clone())),
                    federated_deployment::software_version
                        .eq(software.as_ref().map(|s| s.version.clone())),
                ))
                .execute(conn)
                .await?;
            let outcome = match pinned {
                None => {
                    diesel::update(row)
                        .set((
                            federated_deployment::public_key.eq(&presented),
                            federated_deployment::first_contact_at.eq(now),
                            federated_deployment::last_contact_at.eq(now),
                        ))
                        .execute(conn)
                        .await?;
                    ContactOutcome::Pinned
                }
                Some(pinned)
                    if pinned == presented
                        || follow_handovers(document, &pinned).as_ref() == Some(&presented) =>
                {
                    diesel::update(row)
                        .set((
                            federated_deployment::public_key.eq(&presented),
                            federated_deployment::last_contact_at.eq(now),
                            federated_deployment::offered_key.eq(None::<Vec<u8>>),
                            federated_deployment::offered_key_at.eq(None::<DateTime<Utc>>),
                        ))
                        .execute(conn)
                        .await?;
                    if pinned == presented {
                        ContactOutcome::Confirmed
                    } else {
                        tracing::info!(%domain, "a deployment handed over to a new key");
                        ContactOutcome::HandedOver
                    }
                }
                Some(_) => {
                    // The time a key was first offered is kept while the same key is.
                    if offered.as_deref() != Some(presented.as_slice()) {
                        diesel::update(row)
                            .set((
                                federated_deployment::offered_key.eq(&presented),
                                federated_deployment::offered_key_at.eq(now),
                            ))
                            .execute(conn)
                            .await?;
                    }
                    tracing::warn!(
                        %domain,
                        "a deployment presented a key other than the one pinned, with no handover"
                    );
                    ContactOutcome::KeyChanged
                }
            };
            Ok::<_, app::Error>((get(conn, &domain).await?, outcome))
        }
        .scope_boxed()
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::FederationKeyId;
    use crate::app::federation::keys::{DocumentKey, Gates, KeyAlgorithm};
    use crate::aspen_config::Gate;
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    #[test]
    fn a_document_must_name_its_own_domain_and_an_ed25519_key() {
        let domain = Domain::parse("b.example").unwrap();
        let document = |named: &str, key: &str| DeploymentDocument {
            domain: Domain::parse(named).unwrap(),
            keys: vec![DocumentKey {
                id: FederationKeyId::new(),
                algorithm: KeyAlgorithm::Ed25519,
                public_key: key.to_string(),
                created_at: Utc::now(),
                retired_at: None,
                handover: None,
            }],
            users: Gates {
                emigration: Gate::Open,
                immigration: Gate::Open,
            },
            bots: Gates {
                emigration: Gate::Closed,
                immigration: Gate::Closed,
            },
            protocol: Default::default(),
            software: None,
        };
        let key = URL_SAFE_NO_PAD.encode([7u8; 32]);
        assert!(check_document(&domain, &document("b.example", &key)).is_ok());
        assert!(check_document(&domain, &document("c.example", &key)).is_err());
        let short = URL_SAFE_NO_PAD.encode([7u8; 31]);
        assert!(check_document(&domain, &document("b.example", &short)).is_err());
    }
}
