//! Statements other deployments sign and send here: an assertion that one of their users is who
//! they say, a notice about one of this deployment's users. Each is believed only once it
//! verifies against the key pinned for its issuer, is addressed to this deployment, is within
//! its lifetime, has not been seen before, and comes from a deployment this one federates with
//! in the direction concerned and shares a protocol version with.

use crate::api::GlobalServerContext;
use crate::app;
use crate::app::federation::protocol::Protocol;
use crate::app::federation::{
    ContactOutcome, Direction, Domain, FederatedDeployment, FederationList, Subject, admits,
    contact, jws, lists_of, own_domain,
};
use crate::database::schema::federated_deployment;
use chrono::{Duration, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use fred::prelude::KeysInterface as _;
use rust_i18n::t;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// How far another deployment's clock may run ahead of this one's.
pub const CLOCK_SKEW: Duration = Duration::seconds(60);
/// The longest lifetime a statement from elsewhere may claim.
pub const MAX_LIFETIME: Duration = Duration::minutes(5);

/// A kind of signed statement: its `typ`, and the claims every kind carries.
pub trait Statement: DeserializeOwned {
    const TYPE: &'static str;
    fn issuer(&self) -> &Domain;
    fn audience(&self) -> &Domain;
    fn issued_at(&self) -> i64;
    fn expires_at(&self) -> i64;
    /// Its id, remembered until it expires so it is accepted once.
    fn id(&self) -> Uuid;
}

/// A statement that verified, with who sent it and what this deployment knows of them.
pub struct Received<T> {
    pub claims: T,
    pub from: Domain,
    /// The lists the sender is on here.
    pub lists: Vec<FederationList>,
}

/// Why a statement is refused, for the server's log; the sender is told only that it was.
pub fn invalid(domain: Option<&Domain>, why: &str) -> app::Error {
    tracing::info!(
        domain = domain.map(Domain::as_str),
        why,
        "refused a statement"
    );
    app::Error::AssertionInvalid
}

/// Verifies `token` as a statement of kind `T` from a deployment that a gate of one of
/// `directions` admits, for users or for bots; the caller then checks the gate for what the
/// statement is about. Nothing is fetched from, or recorded about, a deployment no such gate
/// admits.
pub async fn receive<T: Statement>(
    state: &GlobalServerContext,
    token: &str,
    directions: &[Direction],
) -> app::Result<Received<T>> {
    let config = &state.config.federation;
    let here = own_domain(config).ok_or(app::Error::FederationRefused(t!("federationOff")))?;
    let unverified = jws::parse(token).map_err(|_| invalid(None, "malformed"))?;
    #[derive(Deserialize)]
    struct Issuer {
        iss: Domain,
    }
    // Only who claims to have signed it is read before the signature is checked, to know
    // which key to check it with.
    let Issuer { iss: from } = unverified
        .peek::<Issuer>()
        .map_err(|_| invalid(None, "no issuer"))?;
    if from == here {
        return Err(invalid(Some(&from), "issued here"));
    }
    let mut conn = state.connection_pool.get().await?;
    let lists = lists_of(&mut conn, std::slice::from_ref(&from))
        .await?
        .remove(&from)
        .unwrap_or_default();
    let admitted = directions.iter().any(|direction| {
        admits(config, Subject::Users, *direction, &lists)
            || admits(config, Subject::Bots, *direction, &lists)
    });
    if !admitted {
        return Err(refused(
            &from,
            directions
                .first()
                .copied()
                .unwrap_or(Direction::Immigration),
        ));
    }
    let known: Option<FederatedDeployment> = federated_deployment::table
        .find(&from)
        .select(FederatedDeployment::as_select())
        .first(&mut conn)
        .await
        .optional()?;
    drop(conn);
    let verified = known
        .as_ref()
        .and_then(|d| d.public_key.as_deref())
        .and_then(|key| unverified.verify::<T>(T::TYPE, key).ok());
    let (claims, sender) = match (verified, known) {
        (Some(claims), Some(known)) => (claims, known),
        _ => {
            // A key never pinned, or one the sender has since handed over from: contacting it
            // pins or follows the handover, and a key that changed unannounced stays refused.
            let (listed, outcome) = contact(state, &from).await?;
            if outcome == ContactOutcome::KeyChanged {
                return Err(invalid(Some(&from), "its key changed without a handover"));
            }
            let key = listed
                .deployment
                .public_key
                .clone()
                .ok_or_else(|| invalid(Some(&from), "no key"))?;
            let claims = unverified
                .verify::<T>(T::TYPE, &key)
                .map_err(|_| invalid(Some(&from), "signature"))?;
            (claims, listed.deployment)
        }
    };
    // A deployment last contacted before it said which protocol it speaks speaks the first.
    if sender
        .protocol()
        .unwrap_or_default()
        .common_version(&Protocol::ours())
        .is_none()
    {
        return Err(app::Error::FederationRefused(t!(
            "federationIncompatible",
            domain = from.as_str()
        )));
    }
    let now = Utc::now().timestamp();
    if *claims.issuer() != from || *claims.audience() != here {
        return Err(invalid(Some(&from), "issuer or audience"));
    }
    if claims.expires_at() <= now
        || claims.issued_at() > now + CLOCK_SKEW.num_seconds()
        || claims.expires_at() - claims.issued_at() > MAX_LIFETIME.num_seconds()
    {
        return Err(invalid(Some(&from), "expired or too long-lived"));
    }
    // Used once: the id is remembered until the statement could no longer be accepted anyway.
    let remembered: Option<String> = state
        .valkey
        .set(
            format!("federation:statement:{}:{from}:{}", T::TYPE, claims.id()),
            1,
            Some(fred::types::Expiration::EX(
                (claims.expires_at() - now + CLOCK_SKEW.num_seconds()).max(1),
            )),
            Some(fred::types::SetOptions::NX),
            false,
        )
        .await?;
    if remembered.is_none() {
        return Err(invalid(Some(&from), "used before"));
    }
    Ok(Received {
        claims,
        from,
        lists,
    })
}

/// The refusal of a deployment a gate of `direction` does not admit.
pub fn refused(from: &Domain, direction: Direction) -> app::Error {
    app::Error::FederationRefused(match direction {
        Direction::Immigration => t!("federationImmigrationClosed", domain = from.as_str()),
        Direction::Emigration => t!("federationEmigrationClosed", domain = from.as_str()),
    })
}
