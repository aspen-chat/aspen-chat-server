//! Statements other deployments sign and send here: an assertion that one of their users is who
//! they say, a notice about one of this deployment's users. Each is believed only once it
//! verifies against the key pinned for its issuer, which has offered no other key since, is
//! addressed to this deployment, is within its lifetime, has not been seen before, and comes
//! from a deployment this one federates with in the direction concerned (or, for a statement
//! that only takes away, one whose key is pinned here) and shares a protocol version with.

use crate::context::GlobalServerContext;
use crate::federation::protocol::Protocol;
use crate::federation::{
    ContactOutcome, Direction, Domain, FederatedDeployment, FederationList, Subject, admits,
    contact_verifying, jws, lists_of, own_domain,
};
use crate::t;
use aspen_schema::federated_deployment;
use chrono::{Duration, Utc};
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use fred::prelude::KeysInterface as _;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use uuid::Uuid;

/// How far another deployment's clock may run ahead of this one's.
pub const CLOCK_SKEW: Duration = Duration::seconds(60);
/// The longest lifetime a statement from elsewhere may claim.
pub const MAX_LIFETIME: Duration = Duration::minutes(5);
/// How long after a statement had its sender contacted no other does so again.
pub const CONTACT_INTERVAL_SECONDS: i64 = 15;
/// How long after a statement's sender could not be reached no statement contacts it again.
pub const UNREACHED_INTERVAL_SECONDS: i64 = 60;

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

/// Refuses a statement for `reason`, which says what is wrong and what to do about it: nothing
/// in a statement's refusal helps anyone forge one, so its sender is told as plainly as the log.
pub fn invalid(domain: Option<&Domain>, reason: std::borrow::Cow<'static, str>) -> crate::Error {
    tracing::info!(
        domain = domain.map(Domain::as_str),
        %reason,
        "refused a statement"
    );
    crate::Error::AssertionInvalid(reason)
}

/// Which deployments a statement is taken from.
#[derive(Debug, Clone, Copy)]
pub enum Senders<'a> {
    /// Those a gate of one of these directions admits, for users or for bots.
    Admitted(&'a [Direction]),
    /// Those, and besides them any deployment whose key is pinned here, its statement verified
    /// against that key alone: for statements that only take away what the sender's own users
    /// have here, which a deployment may make whatever the gates now say.
    AdmittedOrPinned(&'a [Direction]),
}

/// Verifies `token` as a statement of kind `T` from one of `senders`; the caller then checks the
/// gate for what the statement is about. Nothing is fetched from, or recorded about, a
/// deployment no gate of the directions named admits.
pub async fn receive<T: Statement>(
    state: &GlobalServerContext,
    token: &str,
    senders: Senders<'_>,
) -> crate::Result<Received<T>> {
    let (Senders::Admitted(directions) | Senders::AdmittedOrPinned(directions)) = senders;
    let config = &state.config.federation;
    let here = own_domain(config).ok_or(crate::Error::FederationRefused(t!("federationOff")))?;
    let unverified = jws::parse(token).map_err(|_| invalid(None, t!("statementMalformed")))?;
    #[derive(Deserialize)]
    struct Issuer {
        iss: Domain,
    }
    // Only who claims to have signed it is read before the signature is checked, to know
    // which key to check it with.
    let Issuer { iss: from } = unverified
        .peek::<Issuer>()
        .map_err(|_| invalid(None, t!("statementMalformed")))?;
    if from == here {
        return Err(invalid(Some(&from), t!("statementFromHere")));
    }
    let mut conn = state.connection_pool.get().await?;
    let lists = lists_of(&mut conn, std::slice::from_ref(&from))
        .await?
        .remove(&from)
        .unwrap_or_default();
    let policy = state.settings().federation;
    let admitted = directions.iter().any(|direction| {
        admits(&policy, Subject::Users, *direction, &lists)
            || admits(&policy, Subject::Bots, *direction, &lists)
    });
    let refusal = || {
        refused(
            &from,
            directions
                .first()
                .copied()
                .unwrap_or(Direction::Immigration),
        )
    };
    if !admitted && !matches!(senders, Senders::AdmittedOrPinned(_)) {
        return Err(refusal());
    }
    let known: Option<FederatedDeployment> = federated_deployment::table
        .find(&from)
        .select(FederatedDeployment::as_select())
        .first(&mut conn)
        .await
        .optional()?;
    drop(conn);
    // A deployment that presented a key nothing vouches for is suspended until an
    // administrator accepts it: the key pinned may be the one that leaked, so nothing it signs
    // is believed any more, and the new key is not believed yet.
    if known.as_ref().is_some_and(|d| d.offered_key.is_some()) {
        return Err(invalid(
            Some(&from),
            t!("statementKeyPending", domain = from.as_str()),
        ));
    }
    let verified = known
        .as_ref()
        .and_then(|d| d.public_key.as_deref())
        .and_then(|key| unverified.verify::<T>(T::TYPE, key).ok());
    let (claims, sender) = match (verified, known) {
        (Some(claims), Some(known)) => (claims, known),
        // A sender no gate admits is not contacted, so only the key already pinned will do.
        _ if !admitted => return Err(refusal()),
        _ => contact_sender::<T>(state, &from, &unverified).await?,
    };
    // A deployment last contacted before it said which protocol it speaks speaks the first.
    let theirs = sender.protocol().unwrap_or_default();
    if theirs.common_version(&Protocol::ours()).is_none() {
        return Err(crate::Error::FederationRefused(t!(
            "federationIncompatible",
            domain = from.as_str(),
            theirs = theirs.range(),
            ours = Protocol::ours().range()
        )));
    }
    let now = Utc::now().timestamp();
    if *claims.issuer() != from {
        return Err(invalid(
            Some(&from),
            t!("statementSignature", domain = from.as_str()),
        ));
    }
    if *claims.audience() != here {
        return Err(invalid(
            Some(&from),
            t!(
                "statementWrongAudience",
                audience = claims.audience().as_str(),
                here = here.as_str()
            ),
        ));
    }
    let (issued_at, expires_at) = (claims.issued_at(), claims.expires_at());
    if let Err(problem) = check_times(now, issued_at, expires_at) {
        return Err(invalid(
            Some(&from),
            match problem {
                TimeProblem::Expired { seconds } => {
                    t!(
                        "statementExpired",
                        domain = from.as_str(),
                        seconds = seconds
                    )
                }
                TimeProblem::FromFuture { seconds } => {
                    t!(
                        "statementFromFuture",
                        domain = from.as_str(),
                        seconds = seconds
                    )
                }
                TimeProblem::TooLong { seconds } => t!(
                    "statementTooLong",
                    seconds = seconds,
                    max = MAX_LIFETIME.num_seconds()
                ),
            },
        ));
    }
    // Used once: the id is remembered until the statement could no longer be accepted anyway.
    let remembered: Option<String> = state
        .valkey
        .set(
            format!("federation:statement:{}:{from}:{}", T::TYPE, claims.id()),
            1,
            Some(fred::types::Expiration::EX(
                expires_at
                    .saturating_sub(now)
                    .saturating_add(CLOCK_SKEW.num_seconds())
                    .max(1),
            )),
            Some(fred::types::SetOptions::NX),
            false,
        )
        .await?;
    if remembered.is_none() {
        return Err(invalid(
            Some(&from),
            t!("statementReused", domain = from.as_str()),
        ));
    }
    Ok(Received {
        claims,
        from,
        lists,
    })
}

/// Contacts the sender of a statement that did not verify against a key pinned for it (none is,
/// or it has since handed over to another), and verifies the statement against the key it
/// presents, as `contact::contact_verifying` does.
///
/// Anyone can name any domain as a statement's issuer, and whoever does makes this server call
/// it, so the caller learns nothing from the call: a sender that cannot be reached is refused
/// with the same detail however it failed, which goes to the log instead (administrators see it
/// by contacting the deployment from the dashboard). And one sender is contacted this way at
/// most once every [`CONTACT_INTERVAL_SECONDS`], or [`UNREACHED_INTERVAL_SECONDS`] after it
/// could not be reached: a statement in between is checked against whatever key that contact
/// left pinned, and refused when it does not verify.
async fn contact_sender<T: Statement>(
    state: &GlobalServerContext,
    from: &Domain,
    unverified: &jws::Unverified<'_>,
) -> crate::Result<(T, FederatedDeployment)> {
    let signature = || invalid(Some(from), t!("statementSignature", domain = from.as_str()));
    let gate = format!("federation:statement-contact:{from}");
    let first: Option<String> = state
        .valkey
        .set(
            &gate,
            1,
            Some(fred::types::Expiration::EX(CONTACT_INTERVAL_SECONDS)),
            Some(fred::types::SetOptions::NX),
            false,
        )
        .await?;
    if first.is_none() {
        let mut conn = state.connection_pool.get().await?;
        let known: Option<FederatedDeployment> = federated_deployment::table
            .find(from)
            .select(FederatedDeployment::as_select())
            .first(&mut conn)
            .await
            .optional()?;
        drop(conn);
        let Some(known) = known.filter(|d| d.offered_key.is_none()) else {
            return Err(invalid(
                Some(from),
                t!(
                    "statementSenderContactedRecently",
                    domain = from.as_str(),
                    seconds = UNREACHED_INTERVAL_SECONDS
                ),
            ));
        };
        let claims = known
            .public_key
            .as_deref()
            .and_then(|key| unverified.verify::<T>(T::TYPE, key).ok())
            .ok_or_else(|| {
                invalid(
                    Some(from),
                    t!(
                        "statementSenderContactedRecently",
                        domain = from.as_str(),
                        seconds = UNREACHED_INTERVAL_SECONDS
                    ),
                )
            })?;
        return Ok((claims, known));
    }
    // A key never pinned, or one the sender has since handed over from: contacting it pins or
    // follows the handover, and a key that changed unannounced stays refused. Nothing is
    // recorded unless the statement verifies against the key it presents.
    let contacted = contact_verifying(state, from, |key| {
        unverified
            .verify::<T>(T::TYPE, key)
            .map_err(|_| signature())
    })
    .await;
    let (listed, outcome, claims) = match contacted {
        Err(crate::Error::DeploymentUnreachable(detail)) => {
            tracing::info!(domain = %from, %detail, "could not contact the sender of a statement");
            let backed_off: Result<(), _> = state
                .valkey
                .set(
                    &gate,
                    1,
                    Some(fred::types::Expiration::EX(UNREACHED_INTERVAL_SECONDS)),
                    None,
                    false,
                )
                .await;
            if let Err(error) = backed_off {
                tracing::warn!(domain = %from, %error, "could not note an unreached deployment");
            }
            return Err(crate::Error::DeploymentUnreachable(t!(
                "statementSenderUnreachable",
                domain = from.as_str()
            )));
        }
        contacted => contacted?,
    };
    if outcome == ContactOutcome::KeyChanged {
        return Err(invalid(
            Some(from),
            t!("statementKeyChanged", domain = from.as_str()),
        ));
    }
    Ok((claims, listed.deployment))
}

/// What is wrong with the times a statement claims.
#[derive(Debug, PartialEq, Eq)]
enum TimeProblem {
    Expired { seconds: i64 },
    FromFuture { seconds: i64 },
    TooLong { seconds: i64 },
}

/// Checks a statement's `iat` and `exp` against `now`: it has not expired, was not signed
/// further ahead than [`CLOCK_SKEW`], and neither the lifetime it claims nor how far ahead it
/// expires passes [`MAX_LIFETIME`] (with the skew), so its id is remembered no longer than that.
/// Every sum and difference saturates, since the claims are the sender's and may be anything.
fn check_times(now: i64, issued_at: i64, expires_at: i64) -> Result<(), TimeProblem> {
    if expires_at <= now {
        return Err(TimeProblem::Expired {
            seconds: now.saturating_sub(expires_at),
        });
    }
    if issued_at > now.saturating_add(CLOCK_SKEW.num_seconds()) {
        return Err(TimeProblem::FromFuture {
            seconds: issued_at.saturating_sub(now),
        });
    }
    let lifetime = expires_at.saturating_sub(issued_at);
    let latest = now
        .saturating_add(MAX_LIFETIME.num_seconds())
        .saturating_add(CLOCK_SKEW.num_seconds());
    if lifetime > MAX_LIFETIME.num_seconds() || expires_at > latest {
        return Err(TimeProblem::TooLong {
            seconds: lifetime.max(expires_at.saturating_sub(now)),
        });
    }
    Ok(())
}

/// The refusal of a deployment a gate of `direction` does not admit.
pub fn refused(from: &Domain, direction: Direction) -> crate::Error {
    crate::Error::FederationRefused(match direction {
        Direction::Immigration => t!("federationImmigrationClosed", domain = from.as_str()),
        Direction::Emigration => t!("federationEmigrationClosed", domain = from.as_str()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_checked_without_overflow() {
        let now = 1_800_000_000;
        assert_eq!(check_times(now, now, now + 120), Ok(()));
        assert_eq!(check_times(now, now + 30, now + 150), Ok(()));
        assert!(matches!(
            check_times(now, now - 10, now),
            Err(TimeProblem::Expired { seconds: 0 })
        ));
        assert!(matches!(
            check_times(now, 0, i64::MIN),
            Err(TimeProblem::Expired { .. })
        ));
        assert!(matches!(
            check_times(now, i64::MAX, i64::MAX),
            Err(TimeProblem::FromFuture { .. })
        ));
        // A lifetime that would overflow, and an expiry far ahead with a lifetime that looks
        // short, are both too long.
        assert!(matches!(
            check_times(now, i64::MIN, now + 10),
            Err(TimeProblem::TooLong { seconds: i64::MAX })
        ));
        assert!(matches!(check_times(now, now + 60, now + 60 + 300), Ok(())));
        assert!(matches!(
            check_times(now, now - 10_000, i64::MAX),
            Err(TimeProblem::TooLong { .. })
        ));
        assert!(matches!(
            check_times(now, now, now + 301),
            Err(TimeProblem::TooLong { seconds: 301 })
        ));
    }
}
