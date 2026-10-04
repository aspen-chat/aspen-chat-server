//! This deployment's signing keys, the document that publishes them, and how a key is
//! replaced.
//!
//! The current key is the one not retired. Replacing it as planned signs a handover with the
//! outgoing key, vouching for the new one; the document carries the handovers of the keys made
//! within [`HANDOVER_WINDOW_DAYS`], so a deployment that pinned any of them follows the chain to
//! the current key on its own ([`follow_handovers`]). Replacing a key that may have leaked
//! makes no handover: the old key cannot be trusted to vouch for anything, so every deployment
//! that pinned it refuses the new one until its administrators accept it.

use crate::app::context::GlobalServerContext;
use crate::app::federation::protocol::{Protocol, Software};
use crate::app::federation::{Domain, jws, own_domain};
use crate::app::federation::{Gate, MigrationRules};
use crate::app::{self, FederationKeyId};
use crate::database::schema::federation_key;
use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

/// How long a replaced key's handover stays in the document: a deployment that has not
/// contacted this one for longer must have the new key accepted by its administrators.
pub const HANDOVER_WINDOW_DAYS: i64 = 90;
/// The `typ` of a key handover.
const HANDOVER_TYPE: &str = "aspen-key-handover+jwt";

/// The public half of one of this deployment's keys.
#[derive(Debug, Clone, Queryable, Selectable)]
#[diesel(table_name = federation_key)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct PublicKey {
    pub id: FederationKeyId,
    pub public_key: Vec<u8>,
    pub created_at: DateTime<Utc>,
    pub retired_at: Option<DateTime<Utc>>,
    pub handover: Option<String>,
}

/// The key this deployment signs with now.
pub struct SigningKey {
    pub id: FederationKeyId,
    pub pair: Ed25519KeyPair,
}

/// A key's fingerprint as people compare them: `SHA256:` and the digest of its 32 bytes in
/// base64, as SSH shows keys.
pub fn fingerprint(public_key: &[u8]) -> String {
    format!(
        "SHA256:{}",
        STANDARD_NO_PAD.encode(Sha256::digest(public_key))
    )
}

fn key_error(error: impl std::fmt::Display) -> app::Error {
    app::Error::Config(config::ConfigError::Message(format!(
        "federation key: {error}"
    )))
}

/// A new key: its id, PKCS #8 document, and public key, and the pair to sign with.
fn new_key() -> app::Result<(FederationKeyId, Vec<u8>, Ed25519KeyPair)> {
    let document = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).map_err(key_error)?;
    let pair = Ed25519KeyPair::from_pkcs8(document.as_ref()).map_err(key_error)?;
    Ok((FederationKeyId::new(), document.as_ref().to_vec(), pair))
}

/// Makes this deployment's key when it has none, as its first server starts. Servers starting
/// together make one: the current key is unique.
pub async fn ensure_key(conn: &mut AsyncPgConnection) -> app::Result<()> {
    let has_key = diesel::select(diesel::dsl::exists(
        federation_key::table.filter(federation_key::retired_at.is_null()),
    ))
    .get_result::<bool>(conn)
    .await?;
    if has_key {
        return Ok(());
    }
    let (id, private_key, pair) = new_key()?;
    diesel::insert_into(federation_key::table)
        .values((
            federation_key::id.eq(id),
            federation_key::private_key.eq(private_key),
            federation_key::public_key.eq(pair.public_key().as_ref()),
        ))
        .on_conflict_do_nothing()
        .execute(conn)
        .await?;
    Ok(())
}

/// This deployment's current key.
pub async fn current_key(conn: &mut AsyncPgConnection) -> app::Result<Option<PublicKey>> {
    Ok(federation_key::table
        .select(PublicKey::as_select())
        .filter(federation_key::retired_at.is_null())
        .first(conn)
        .await
        .optional()?)
}

/// The key to sign with; `None` before the first server has made one.
pub async fn signing_key(conn: &mut AsyncPgConnection) -> app::Result<Option<SigningKey>> {
    let found: Option<(FederationKeyId, Vec<u8>)> = federation_key::table
        .select((federation_key::id, federation_key::private_key))
        .filter(federation_key::retired_at.is_null())
        .first(conn)
        .await
        .optional()?;
    found
        .map(|(id, document)| {
            // Accepts version 1 documents, which carry no public key, as well as version 2.
            let pair = Ed25519KeyPair::from_pkcs8_maybe_unchecked(&document).map_err(key_error)?;
            Ok(SigningKey { id, pair })
        })
        .transpose()
}

/// How a key is replaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    /// The outgoing key signs a handover to the new one, which deployments that pinned it
    /// accept on their own.
    Planned,
    /// The outgoing key may be in someone else's hands, so it vouches for nothing: deployments
    /// that pinned it refuse the new key until their administrators accept it.
    Compromised,
}

/// A key handover: `iss` says that its key `key` is `public_key`, signed by the key before it.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Handover {
    iss: Domain,
    key: FederationKeyId,
    public_key: String,
    iat: i64,
}

/// Retires the current key and makes another, handing over to it when `rotation` is planned.
/// A planned rotation needs this deployment's domain, which the handover names.
pub async fn rotate_key(
    conn: &mut AsyncPgConnection,
    domain: Option<&Domain>,
    rotation: Rotation,
) -> app::Result<PublicKey> {
    let (id, private_key, pair) = new_key()?;
    let public_key = pair.public_key().as_ref().to_vec();
    let handover = match rotation {
        Rotation::Compromised => None,
        Rotation::Planned => {
            let domain = domain.ok_or_else(|| key_error("a planned rotation needs a domain"))?;
            let outgoing = signing_key(conn)
                .await?
                .ok_or_else(|| key_error("there is no key to hand over from"))?;
            Some(jws::sign(
                HANDOVER_TYPE,
                outgoing.id,
                &outgoing.pair,
                &Handover {
                    iss: domain.clone(),
                    key: id,
                    public_key: URL_SAFE_NO_PAD.encode(&public_key),
                    iat: Utc::now().timestamp(),
                },
            ))
        }
    };
    conn.transaction(|conn| {
        async move {
            diesel::update(federation_key::table.filter(federation_key::retired_at.is_null()))
                .set(federation_key::retired_at.eq(diesel::dsl::now))
                .execute(conn)
                .await?;
            Ok(diesel::insert_into(federation_key::table)
                .values((
                    federation_key::id.eq(id),
                    federation_key::private_key.eq(private_key),
                    federation_key::public_key.eq(public_key),
                    federation_key::handover.eq(handover),
                ))
                .returning(PublicKey::as_returning())
                .get_result(conn)
                .await?)
        }
        .scope_boxed()
    })
    .await
}

// ---------------------------------------------------------------------------
// The published document
// ---------------------------------------------------------------------------

/// What a deployment publishes at [`super::WELL_KNOWN_PATH`]: its name, its keys, and its
/// gates.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeploymentDocument {
    pub domain: Domain,
    /// Its current key first, then the keys it replaced within the handover window, newest
    /// first.
    pub keys: Vec<DocumentKey>,
    pub users: Gates,
    pub bots: Gates,
    /// The protocol it speaks; a document that does not say speaks the first version.
    #[serde(default)]
    pub protocol: Protocol,
    /// The software it runs, for people to read.
    #[serde(default)]
    pub software: Option<Software>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentKey {
    pub id: FederationKeyId,
    pub algorithm: KeyAlgorithm,
    /// The public key's bytes in unpadded base64url.
    pub public_key: String,
    pub created_at: DateTime<Utc>,
    /// When it stopped signing; `null` for the current key.
    #[serde(default)]
    pub retired_at: Option<DateTime<Utc>>,
    /// The key before it vouching for it, a compact JWS; `null` when none did.
    #[serde(default)]
    pub handover: Option<String>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub enum KeyAlgorithm {
    Ed25519,
    /// An algorithm another deployment publishes that this one does not know, as a newer one
    /// may; such a key is never used.
    #[serde(other)]
    Unknown,
}

/// One kind of account's gates, as a deployment publishes them.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, schemars::JsonSchema,
)]
#[serde(rename_all = "camelCase")]
pub struct Gates {
    pub emigration: Gate,
    pub immigration: Gate,
}

impl From<&MigrationRules> for Gates {
    fn from(rules: &MigrationRules) -> Self {
        Gates {
            emigration: rules.emigration,
            immigration: rules.immigration,
        }
    }
}

impl From<PublicKey> for DocumentKey {
    fn from(key: PublicKey) -> Self {
        DocumentKey {
            id: key.id,
            algorithm: KeyAlgorithm::Ed25519,
            public_key: URL_SAFE_NO_PAD.encode(&key.public_key),
            created_at: key.created_at,
            retired_at: key.retired_at,
            handover: key.handover,
        }
    }
}

/// This deployment's document; `None` when it has no domain, and so takes no part in
/// federation.
pub async fn document(state: &GlobalServerContext) -> app::Result<Option<DeploymentDocument>> {
    let Some(domain) = own_domain(&state.config.federation) else {
        return Ok(None);
    };
    let policy = state.settings().federation;
    let mut conn = state.connection_pool.get().await?;
    let since = Utc::now() - Duration::days(HANDOVER_WINDOW_DAYS);
    let keys: Vec<PublicKey> = federation_key::table
        .select(PublicKey::as_select())
        .filter(
            federation_key::retired_at
                .is_null()
                .or(federation_key::created_at.ge(since)),
        )
        // The current key, which has no retirement, sorts first.
        .order((
            federation_key::retired_at.desc().nulls_first(),
            federation_key::created_at.desc(),
        ))
        .load(&mut conn)
        .await?;
    Ok(Some(DeploymentDocument {
        domain,
        keys: keys.into_iter().map(DocumentKey::from).collect(),
        users: (&policy.users).into(),
        bots: (&policy.bots).into(),
        protocol: Protocol::ours(),
        software: Some(Software::ours()),
    }))
}

/// The current key a document presents: its first, which must be Ed25519 and 32 bytes.
pub fn current_of(document: &DeploymentDocument) -> Option<Vec<u8>> {
    // Called as a slice method: diesel's query traits in scope also have a `first`.
    <[DocumentKey]>::first(&document.keys).and_then(decode_key)
}

fn decode_key(key: &DocumentKey) -> Option<Vec<u8>> {
    if key.algorithm != KeyAlgorithm::Ed25519 {
        return None;
    }
    URL_SAFE_NO_PAD
        .decode(&key.public_key)
        .ok()
        .filter(|bytes| bytes.len() == 32)
}

/// The document's current key, when a chain of handovers in it leads there from `pinned`, each
/// signed by the key before and naming the next as the document lists it.
pub fn follow_handovers(document: &DeploymentDocument, pinned: &[u8]) -> Option<Vec<u8>> {
    let current = current_of(document)?;
    let mut trusted = pinned.to_vec();
    // Each step moves to another key of the document, so a chain is never longer than it.
    for _ in 0..document.keys.len() {
        if trusted == current {
            return Some(current);
        }
        trusted = document.keys.iter().find_map(|key| {
            let claims: Handover = jws::parse(key.handover.as_deref()?)
                .ok()?
                .verify(HANDOVER_TYPE, &trusted)
                .ok()?;
            (claims.iss == document.domain
                && claims.key == key.id
                && claims.public_key == key.public_key)
                .then(|| decode_key(key))
                .flatten()
        })?;
    }
    (trusted == current).then_some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A document whose keys were made in turn, each handed over from the one before when
    /// `planned` says so for it.
    fn chain(planned: &[bool]) -> (DeploymentDocument, Vec<Vec<u8>>) {
        let domain = Domain::parse("b.example").unwrap();
        let mut pairs: Vec<(FederationKeyId, Ed25519KeyPair)> = Vec::new();
        let mut keys = Vec::new();
        for (index, handed_over) in std::iter::once(false)
            .chain(planned.iter().copied())
            .enumerate()
        {
            let (id, _, pair) = new_key().unwrap();
            let public_key = URL_SAFE_NO_PAD.encode(pair.public_key().as_ref());
            let handover = (handed_over && index > 0).then(|| {
                let (outgoing_id, outgoing) = &pairs[index - 1];
                jws::sign(
                    HANDOVER_TYPE,
                    *outgoing_id,
                    outgoing,
                    &Handover {
                        iss: domain.clone(),
                        key: id,
                        public_key: public_key.clone(),
                        iat: 0,
                    },
                )
            });
            keys.push(DocumentKey {
                id,
                algorithm: KeyAlgorithm::Ed25519,
                public_key,
                created_at: Utc::now(),
                retired_at: None,
                handover,
            });
            pairs.push((id, pair));
        }
        let publics = pairs
            .iter()
            .map(|(_, pair)| pair.public_key().as_ref().to_vec())
            .collect();
        keys.reverse();
        let open = Gates {
            emigration: Gate::Open,
            immigration: Gate::Open,
        };
        let document = DeploymentDocument {
            domain,
            keys,
            users: open,
            bots: open,
            protocol: Protocol::default(),
            software: None,
        };
        (document, publics)
    }

    #[test]
    fn planned_handovers_lead_from_any_pinned_key_to_the_current_one() {
        let (document, keys) = chain(&[true, true]);
        for pinned in &keys {
            assert_eq!(follow_handovers(&document, pinned), Some(keys[2].clone()));
        }
    }

    #[test]
    fn a_compromise_breaks_the_chain() {
        let (document, keys) = chain(&[true, false]);
        assert_eq!(follow_handovers(&document, &keys[0]), None);
        assert_eq!(follow_handovers(&document, &keys[1]), None);
        assert_eq!(follow_handovers(&document, &keys[2]), Some(keys[2].clone()));
    }

    #[test]
    fn a_handover_for_another_domain_is_not_followed() {
        let (mut document, keys) = chain(&[true]);
        document.domain = Domain::parse("c.example").unwrap();
        assert_eq!(follow_handovers(&document, &keys[0]), None);
    }

    /// A deployment newer than this one may publish gates, algorithms, and fields this one does
    /// not know; its document still reads, and a key of an unknown algorithm is never used.
    #[test]
    fn a_document_from_a_newer_deployment_reads() {
        let document: DeploymentDocument = serde_json::from_str(include_str!(
            "../../../../spec/fixtures/federation/document-from-newer.json"
        ))
        .unwrap();
        assert_eq!(document.users.immigration, Gate::Unknown);
        assert_eq!(document.keys[0].algorithm, KeyAlgorithm::Unknown);
        assert_eq!(current_of(&document), None);
        assert_eq!(document.protocol.version, 7);
    }

    #[test]
    fn the_first_documents_still_read() {
        let document: DeploymentDocument = serde_json::from_str(include_str!(
            "../../../../spec/fixtures/federation/document-v1.json"
        ))
        .unwrap();
        assert_eq!(document.protocol, Protocol::default());
        assert!(current_of(&document).is_some());
    }

    #[test]
    fn fingerprints_look_like_ssh_ones() {
        let print = fingerprint(&[0u8; 32]);
        assert!(print.starts_with("SHA256:"));
        assert_eq!(print.len(), "SHA256:".len() + 43);
    }
}
