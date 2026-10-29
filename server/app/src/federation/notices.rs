//! Notices: what one deployment tells another about that deployment's users, server to server.
//! A notice is a signed statement (`aspen-notice+jwt`) addressed to one user's home, POSTed to
//! `/api/v1/federation/notices` there, naming the user by their id at home. What it says is its
//! `kind`; a kind the receiver does not know it accepts and ignores (`spec/federation.md`).
//!
//! The kinds so far:
//! - `dmJoined`, from a host to a home: a deployment that hosts a DM tells each of its
//!   participants' homes that they are in it, when it is started with them or they are added to
//!   it, and the home tells their devices (`foreignDmJoined`), which sign in there if they are
//!   not and read it. A home passes it on only from deployments its user still uses, so one they
//!   left stays left.
//! - `accountDeleted`, from a home to the deployments its user used: the account is gone, and
//!   each retires its user from there (`app::user::retire`).
//!
//! A notice is delivered by a job (`deliverNotice`, [`deliver_step`]), one per receiver, saved
//! in the transaction of the change it tells of ([`queue_dm`], [`queue_deleted`]), so a notice
//! is never lost to a restart or a receiver that is down for a while: each attempt signs it
//! afresh, and the job keeps trying with the jobs' backoff for about a day ([`MAX_ATTEMPTS`])
//! before giving up with a warning.

use crate::context::GlobalServerContext;
use crate::federation::keys::signing_key;
use crate::federation::received::{Received, Senders, Statement, receive};
use crate::federation::{Direction, Domain, FederationList, Subject, admits, jws, own_domain};
use crate::jobs::{Claimed, JobClass, JobKind, NewJob, Outcome, enqueue_many, last_attempt};
use crate::t;
use crate::{ChannelId, EventScope, UserId, publish_event};
use aspen_schema::{user, user_foreign_deployment};
use aspen_wire::message_enum::request::UserUpdateRequest;
use aspen_wire::message_enum::server_event::ServerEvent;
use chrono::{Duration, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The `typ` of a notice.
const NOTICE_TYPE: &str = "aspen-notice+jwt";
/// How long a notice is good for once signed. Each delivery attempt signs it afresh, so the
/// lifetime need only cover the request.
const NOTICE_LIFETIME: Duration = Duration::minutes(2);
/// Where a home receives notices, under the API.
pub const NOTICES_PATH: &str = "/federation/notices";
/// How many times a notice is tried before it is given up: with the jobs' backoff (ten seconds
/// doubling to an hour, `jobs::backoff`), about a day, which is as long as a receiver's outage
/// is worth waiting out. The standing pass covers what a lost `accountDeleted` would have done
/// for users with a session there, and a DM its user will find on signing in there.
pub const MAX_ATTEMPTS: i32 = 32;

/// A notice about one user, to their home.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// The deployment telling.
    pub iss: Domain,
    /// The user's home.
    pub aud: Domain,
    /// The user's id at home.
    pub sub: Uuid,
    pub iat: i64,
    pub exp: i64,
    pub jti: Uuid,
    #[serde(flatten)]
    pub about: About,
}

/// What a notice says.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum About {
    /// The user is in a DM on the deployment telling: started with them, or they were added.
    DmJoined {
        /// The DM, by the telling deployment's id.
        channel: ChannelId,
        /// Who started it or added them.
        by: Person,
    },
    /// The user deleted their account at home.
    AccountDeleted,
    /// A kind this deployment does not know, sent by a newer one; accepted and ignored.
    #[serde(other)]
    Unknown,
}

/// Someone a notice names, as the telling deployment shows them.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub name: String,
    pub display_name: Option<String>,
}

impl Person {
    /// The same checks a name and display name chosen here pass, since a home shows them to
    /// its user as it would its own users'.
    fn check(&self) -> crate::Result<()> {
        crate::user::validate_username(&self.name)?;
        crate::user::validate_profile(&UserUpdateRequest {
            display_name: Some(self.display_name.clone()),
            ..UserUpdateRequest::default()
        })
    }
}

impl Statement for Notice {
    const TYPE: &'static str = NOTICE_TYPE;
    fn issuer(&self) -> &Domain {
        &self.iss
    }
    fn audience(&self) -> &Domain {
        &self.aud
    }
    fn issued_at(&self) -> i64 {
        self.iat
    }
    fn expires_at(&self) -> i64 {
        self.exp
    }
    fn id(&self) -> Uuid {
        self.jti
    }
}

/// One notice to deliver: what a `deliverNotice` job is given. The notice itself is signed when
/// the job runs, since a signature is good for minutes and a delivery may wait hours.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    /// The receiver: the user's home, or a deployment they used.
    pub to: Domain,
    /// The user's id at home.
    pub sub: Uuid,
    #[serde(flatten)]
    pub about: About,
}

/// Saves one `deliverNotice` job per receiver on `conn`, in the transaction of what they tell
/// of. A `dmJoined` is interactive, since the user's devices wait on it to sign in and read the
/// DM; an `accountDeleted` is cleaning up after a decision made.
async fn queue(conn: &mut AsyncPgConnection, deliveries: Vec<Delivery>) -> crate::Result<()> {
    let jobs = deliveries
        .into_iter()
        .map(|delivery| {
            let class = match delivery.about {
                About::DmJoined { .. } => JobClass::Interactive,
                About::AccountDeleted | About::Unknown => JobClass::Normal,
            };
            NewJob::new(JobKind::DeliverNotice, class, &delivery)
        })
        .collect::<crate::Result<Vec<_>>>()?;
    enqueue_many(conn, jobs).await
}

/// Queues telling the homes of the users of `joined` from elsewhere that `by` put them in the
/// DM `channel` here, in the transaction that put them there. Nothing is queued where
/// federation is off or nobody in `joined` is from elsewhere.
pub async fn queue_dm(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    channel: ChannelId,
    joined: &[UserId],
    by: UserId,
) -> crate::Result<()> {
    if own_domain(&state.config.federation).is_none() {
        return Ok(());
    }
    let foreign: Vec<(Option<Domain>, Option<Uuid>)> = user::table
        .select((user::home_domain, user::home_id))
        .filter(user::id.eq_any(joined))
        .filter(user::id.ne(by))
        .filter(user::home_domain.is_not_null())
        .load(conn)
        .await?;
    if foreign.is_empty() {
        return Ok(());
    }
    let (name, display_name): (String, Option<String>) = user::table
        .select((user::name, user::display_name))
        .find(by)
        .first(conn)
        .await?;
    let deliveries = foreign
        .into_iter()
        .filter_map(|(home, home_id)| Some((home?, home_id?)))
        .map(|(to, sub)| Delivery {
            to,
            sub,
            about: About::DmJoined {
                channel,
                by: Person {
                    name: name.clone(),
                    display_name: display_name.clone(),
                },
            },
        })
        .collect();
    queue(conn, deliveries).await
}

/// Queues telling every other deployment `user_id`, one of this deployment's own, has used that
/// their account is gone, in the transaction that retires it (`app::user::retire`).
pub async fn queue_deleted(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
) -> crate::Result<()> {
    if own_domain(&state.config.federation).is_none() {
        return Ok(());
    }
    let used: Vec<String> = user_foreign_deployment::table
        .select(user_foreign_deployment::domain)
        .filter(user_foreign_deployment::user.eq(user_id))
        .load(conn)
        .await?;
    let deliveries = used
        .iter()
        .filter_map(|domain| Domain::parse(domain).ok())
        .map(|to| Delivery {
            to,
            sub: user_id.0,
            about: About::AccountDeleted,
        })
        .collect();
    queue(conn, deliveries).await
}

/// One attempt of a `deliverNotice` job: signs the notice and POSTs it to its receiver. Taken
/// (`2xx`), or refused (`4xx`, which will not change on its own), the job is done; otherwise
/// it fails, to be tried again after the jobs' backoff, until on its last attempt it is given
/// up with a warning. Without federation, or a signing key, there is nothing to deliver.
pub async fn deliver_step(state: &GlobalServerContext, job: &Claimed) -> crate::Result<Outcome> {
    let Delivery { to, sub, about } = job.payload()?;
    let Some(here) = own_domain(&state.config.federation) else {
        return Ok(Outcome::Done);
    };
    let Some(key) = signing_key(state.connection_pool.get().await?.as_mut()).await? else {
        return Ok(Outcome::Done);
    };
    let now = Utc::now();
    let notice = jws::sign(
        NOTICE_TYPE,
        key.id,
        &key.pair,
        &Notice {
            iss: here,
            aud: to.clone(),
            sub,
            iat: now.timestamp(),
            exp: (now + NOTICE_LIFETIME).timestamp(),
            jti: Uuid::new_v4(),
            about,
        },
    );
    let url = format!("https://{to}{}{NOTICES_PATH}", aspen_wire::API_PREFIX);
    let body = serde_json::json!({ "notice": notice });
    let failure = match state.federation_client.post(&url).json(&body).send().await {
        Ok(response) if response.status().is_success() => return Ok(Outcome::Done),
        Ok(response) if response.status().is_client_error() => {
            tracing::info!(%to, status = %response.status(), "a deployment refused a notice");
            return Ok(Outcome::Done);
        }
        Ok(response) => format!("answered {}", response.status()),
        Err(error) => error.to_string(),
    };
    if last_attempt(job) {
        tracing::warn!(%to, attempts = job.attempts, %failure, "gave up delivering a notice");
        return Ok(Outcome::Done);
    }
    tracing::info!(%to, %failure, "a deployment could not take a notice");
    Err(crate::Error::DeploymentUnreachable(t!(
        "federationNoAnswer",
        domain = to.as_str()
    )))
}

/// Takes a notice from another deployment: about one of this deployment's users, from a
/// deployment they use, or about one of that deployment's users signed in here, from their home.
/// A notice is taken from a deployment a gate admits, and an `accountDeleted` notice, which only
/// takes away, also from any whose key is pinned here, so a home closed off since its users
/// arrived may still retire them.
pub async fn receive_notice(state: &GlobalServerContext, token: &str) -> crate::Result<()> {
    const DIRECTIONS: &[Direction] = &[Direction::Emigration, Direction::Immigration];
    // The kind read before the signature is checked only chooses which senders to hear; the
    // claims acted on are those that verify, which are the same claims.
    let only_takes_away = jws::parse(token)
        .ok()
        .and_then(|unverified| unverified.peek::<Notice>().ok())
        .is_some_and(|notice| matches!(notice.about, About::AccountDeleted));
    let senders = if only_takes_away {
        Senders::AdmittedOrPinned(DIRECTIONS)
    } else {
        Senders::Admitted(DIRECTIONS)
    };
    let Received {
        claims,
        from,
        lists,
    } = receive::<Notice>(state, token, senders).await?;
    match claims.about {
        About::DmJoined { channel, by } => {
            // Taken, as every notice that verifies is, but not passed on.
            if let Err(error) = by.check() {
                tracing::info!(%from, %error, "ignored a notice naming someone in a way this deployment does not accept");
                return Ok(());
            }
            dm_joined(state, &from, &lists, claims.sub, channel, by).await
        }
        About::AccountDeleted => account_deleted(state, &from, claims.sub).await,
        About::Unknown => Ok(()),
    }
}

/// One of this deployment's users is in a DM on `from`: their devices hear of it while they
/// still use `from`.
async fn dm_joined(
    state: &GlobalServerContext,
    from: &Domain,
    lists: &[FederationList],
    sub: Uuid,
    channel: ChannelId,
    by: Person,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let found: Option<(UserId, bool)> = user::table
        .select((user::id, user::bot))
        .filter(user::id.eq(sub))
        .filter(user::home_domain.is_null())
        .filter(user::deleted_at.is_null())
        .first(&mut conn)
        .await
        .optional()?;
    let Some((user_id, bot)) = found else {
        return Ok(());
    };
    let subject = if bot { Subject::Bots } else { Subject::Users };
    let uses = diesel::select(diesel::dsl::exists(
        user_foreign_deployment::table
            .filter(user_foreign_deployment::user.eq(user_id))
            .filter(user_foreign_deployment::domain.eq(from.as_str())),
    ))
    .get_result::<bool>(&mut conn)
    .await?;
    if !uses
        || !admits(
            &state.settings().federation,
            subject,
            Direction::Emigration,
            lists,
        )
    {
        return Ok(());
    }
    publish_event(
        state,
        &mut conn,
        EventScope::User(user_id),
        &ServerEvent::ForeignDmJoined {
            domain: from.to_string(),
            channel,
            by_name: by.name,
            by_display_name: by.display_name,
        },
    )
    .await
}

/// A user from `from` deleted their account there: their user here is retired. Any home whose
/// key is pinned here may say so of its own users, whatever the gates now say, since it only
/// takes away.
async fn account_deleted(
    state: &GlobalServerContext,
    from: &Domain,
    sub: Uuid,
) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let found: Option<UserId> = user::table
        .select(user::id)
        .filter(user::home_domain.eq(from))
        .filter(user::home_id.eq(sub))
        .filter(user::deleted_at.is_null())
        .first(&mut conn)
        .await
        .optional()?;
    let Some(user_id) = found else {
        return Ok(());
    };
    let retired = conn
        .transaction(|conn| crate::user::retire(state, conn, user_id).scope_boxed())
        .await?;
    retired.finish(state).await;
    tracing::info!(%from, user = %user_id.0, "retired a user whose home account was deleted");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notices_of_every_known_kind_and_of_kinds_to_come_read() {
        let known: Notice = serde_json::from_str(include_str!(
            "../../../../spec/fixtures/federation/notice-dm-joined.json"
        ))
        .unwrap();
        assert!(matches!(known.about, About::DmJoined { .. }));
        let future: Notice = serde_json::from_str(include_str!(
            "../../../../spec/fixtures/federation/notice-from-newer.json"
        ))
        .unwrap();
        assert!(matches!(future.about, About::Unknown));
    }
}
