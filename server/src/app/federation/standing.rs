//! Whether the users from elsewhere signed in here are still in good standing at home. About
//! every `[federation] standing_interval_seconds`, this deployment asks each home, in one signed
//! request per home (`aspen-standing-request+jwt`, POSTed to `/api/v1/federation/standing`),
//! about its users with a session here, and the home answers with a signed statement
//! (`aspen-standing+jwt`) saying, for each:
//! - `good`: the account exists and its home still lets it use this deployment; it is confirmed.
//! - `gone`: there is no such account any more; its user here is retired.
//! - `refused`: its home no longer lets it use this deployment (the user left it, the home's
//!   emigration gate closed to it, or the home banned them, `app::user_ban`); its sessions here
//!   end, and it may sign in again if that changes.
//!
//! A standing this deployment does not know it takes as `refused`. Sessions also end when this
//! deployment's own immigration gate no longer admits the home, at once when the gate changes
//! ([`shut_out`]) and at each pass, and when the home has gone unreached for
//! `standing_grace_seconds`. One server does each pass, under an advisory lock, while an
//! immigration gate admits anyone.

use crate::app::context::GlobalServerContext;
use crate::app::events::Publishing;
use crate::app::federation::keys::signing_key;
use crate::app::federation::received::{Received, Statement, receive};
use crate::app::federation::{
    Direction, Domain, FederationPolicy, Subject, admits, jws, lists_of, own_domain,
};
use crate::app::{self, UserId};
use crate::database::schema::{refresh_token, user, user_foreign_deployment};
use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

const REQUEST_TYPE: &str = "aspen-standing-request+jwt";
const ANSWER_TYPE: &str = "aspen-standing+jwt";
/// Where a home answers for its users, under the API.
pub const STANDING_PATH: &str = "/federation/standing";
/// How long a request or an answer is good for once signed.
const LIFETIME: Duration = Duration::minutes(2);
/// The most users one request asks about, and the most one answer answers for: as many as fit,
/// request and answer alike, within the longest statement any version of Aspen reads
/// (`jws::MAX_LENGTH`). A user takes 68 bytes of an answer (`{"sub":"…","standing":"refused"},`)
/// and base64 makes that 91; the two domains, the other claims, the header, and the signature
/// take at most about 1.3 KiB more. At 128 users an answer is at most about 12.4 KiB, which
/// leaves room for standings a newer home names at greater length.
pub const MAX_USERS: usize = 128;
/// The largest answer read: its statement, and the JSON around it.
const MAX_ANSWER_BYTES: usize = jws::MAX_LENGTH + 1024;
/// Which advisory lock a pass holds, so one server does it at a time.
const PASS_LOCK: i64 = 0x6173_7065_6e5f_7374;

/// A deployment asks one home about the home's users signed in there.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StandingRequest {
    pub iss: Domain,
    pub aud: Domain,
    pub iat: i64,
    pub exp: i64,
    pub jti: Uuid,
    /// The users asked about, by their ids at home.
    pub users: Vec<Uuid>,
}

/// A home answers for its users.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StandingAnswer {
    pub iss: Domain,
    pub aud: Domain,
    pub iat: i64,
    pub exp: i64,
    pub jti: Uuid,
    pub users: Vec<UserStanding>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserStanding {
    /// The user's id at home.
    pub sub: Uuid,
    pub standing: Standing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Standing {
    Good,
    Gone,
    Refused,
    /// A standing this deployment does not know, from a newer home; taken as `refused`.
    #[serde(other)]
    Unknown,
}

macro_rules! statement {
    ($type:ty, $typ:expr) => {
        impl Statement for $type {
            const TYPE: &'static str = $typ;
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
    };
}
statement!(StandingRequest, REQUEST_TYPE);
statement!(StandingAnswer, ANSWER_TYPE);

async fn sign<T: Serialize>(
    conn: &mut AsyncPgConnection,
    typ: &str,
    claims: &T,
) -> app::Result<String> {
    let key = signing_key(conn).await?.ok_or_else(|| {
        app::Error::Config(config::ConfigError::Message("no federation key yet".into()))
    })?;
    Ok(jws::sign(typ, key.id, &key.pair, claims))
}

// ---------------------------------------------------------------------------
// At home: answering for this deployment's users
// ---------------------------------------------------------------------------

/// Answers another deployment's request about this deployment's users signed in there.
pub async fn answer(state: &GlobalServerContext, token: &str) -> app::Result<String> {
    let Received {
        claims,
        from,
        lists,
    } = receive::<StandingRequest>(state, token, &[Direction::Emigration]).await?;
    let here = own_domain(&state.config.federation)
        .ok_or_else(|| app::Error::FederationRefused(crate::t!("federationOff")))?;
    // A request asking about more users is answered for the first of them, so the answer fits
    // in a statement; the rest go unanswered, which the asker takes as no news of them.
    let asked: Vec<Uuid> = claims.users.into_iter().take(MAX_USERS).collect();
    let mut conn = state.connection_pool.get().await?;
    let found: Vec<(UserId, bool, bool)> = user::table
        .select((user::id, user::bot, crate::app::user_ban::banned()))
        .filter(user::id.eq_any(&asked))
        .filter(user::home_domain.is_null())
        .filter(user::deleted_at.is_null())
        .load(&mut conn)
        .await?;
    let using: Vec<UserId> = user_foreign_deployment::table
        .select(user_foreign_deployment::user)
        .filter(user_foreign_deployment::user.eq_any(&asked))
        .filter(user_foreign_deployment::domain.eq(from.as_str()))
        .load(&mut conn)
        .await?;
    let policy = state.settings().federation;
    let users = asked
        .into_iter()
        .map(|sub| {
            let standing = match found.iter().find(|(id, _, _)| id.0 == sub) {
                None => Standing::Gone,
                Some((id, bot, banned)) => {
                    let subject = if *bot { Subject::Bots } else { Subject::Users };
                    if !banned
                        && using.contains(id)
                        && admits(&policy, subject, Direction::Emigration, &lists)
                    {
                        Standing::Good
                    } else {
                        Standing::Refused
                    }
                }
            };
            UserStanding { sub, standing }
        })
        .collect();
    let now = Utc::now();
    sign(
        &mut conn,
        ANSWER_TYPE,
        &StandingAnswer {
            iss: here,
            aud: from,
            iat: now.timestamp(),
            exp: (now + LIFETIME).timestamp(),
            jti: Uuid::new_v4(),
            users,
        },
    )
    .await
}

// ---------------------------------------------------------------------------
// As a host: asking about the users from elsewhere signed in here
// ---------------------------------------------------------------------------

/// Runs a pass whenever one may be due, for as long as the server runs, while an immigration gate
/// admits anyone.
pub fn spawn_confirmer(state: GlobalServerContext) {
    tokio::spawn(async move {
        let interval = state.config.federation.standing_interval_seconds.max(1);
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(interval.min(300)));
        loop {
            tick.tick().await;
            if !state.settings().federation.admits_anyone() {
                continue;
            }
            let (passed, noted) = app::events::noting(pass(&state)).await;
            app::events::settle(&state, noted, passed.is_err()).await;
            if let Err(error) = passed {
                tracing::warn!(%error, "checking foreign users' standing failed");
            }
        }
    });
}

/// A foreign user with a session here, due to be asked about.
struct Due {
    id: UserId,
    home_id: Uuid,
    bot: bool,
    confirmed_at: Option<DateTime<Utc>>,
}

/// A foreign user as the pass reads them: id, home, id at home, bot, when last confirmed.
type DueRow = (
    UserId,
    Option<Domain>,
    Option<Uuid>,
    bool,
    Option<DateTime<Utc>>,
);

/// Asks every home about its users here who are due, if no other server is doing so.
pub async fn pass(state: &GlobalServerContext) -> app::Result<()> {
    // The lock belongs to a transaction, so it is let go however the pass ends: done, failed,
    // or dropped part way, when the pool discards the connection rather than recycling it
    // still in the transaction.
    let mut lock = state.connection_pool.get().await?;
    lock.transaction(|lock| {
        async move {
            let locked: bool = diesel::select(diesel::dsl::sql::<diesel::sql_types::Bool>(
                &format!("pg_try_advisory_xact_lock({PASS_LOCK})"),
            ))
            .get_result(lock.as_mut())
            .await?;
            if !locked {
                return Ok(());
            }
            pass_locked(state).await
        }
        .scope_boxed()
    })
    .await
}

async fn pass_locked(state: &GlobalServerContext) -> app::Result<()> {
    let config = &state.config.federation;
    let due_before = Utc::now()
        - Duration::seconds(i64::try_from(config.standing_interval_seconds).unwrap_or(i64::MAX));
    let mut conn = state.connection_pool.get().await?;
    let rows: Vec<DueRow> = user::table
        .select((
            user::id,
            user::home_domain,
            user::home_id,
            user::bot,
            user::home_confirmed_at,
        ))
        .filter(user::home_domain.is_not_null())
        .filter(user::deleted_at.is_null())
        .filter(
            user::home_confirmed_at
                .is_null()
                .or(user::home_confirmed_at.lt(due_before)),
        )
        .filter(diesel::dsl::exists(
            refresh_token::table
                .filter(refresh_token::user.eq(user::id))
                .filter(refresh_token::expires.gt(diesel::dsl::now)),
        ))
        .load(&mut conn)
        .await?;
    let mut by_home: HashMap<Domain, Vec<Due>> = HashMap::new();
    for (id, home, home_id, bot, confirmed_at) in rows {
        if let (Some(home), Some(home_id)) = (home, home_id) {
            by_home.entry(home).or_default().push(Due {
                id,
                home_id,
                bot,
                confirmed_at,
            });
        }
    }
    drop(conn);
    for (home, users) in by_home {
        if let Err(error) = confirm_home(state, &home, users).await {
            tracing::info!(%home, %error, "could not confirm a home's users");
        }
    }
    Ok(())
}

async fn confirm_home(
    state: &GlobalServerContext,
    home: &Domain,
    users: Vec<Due>,
) -> app::Result<()> {
    let config = &state.config.federation;
    let policy = state.settings().federation;
    let mut conn = state.connection_pool.get().await?;
    let lists = lists_of(&mut conn, std::slice::from_ref(home))
        .await?
        .remove(home)
        .unwrap_or_default();
    // Those this deployment's own gate no longer admits are not asked about.
    let (admitted, closed): (Vec<Due>, Vec<Due>) = users.into_iter().partition(|due| {
        let subject = if due.bot {
            Subject::Bots
        } else {
            Subject::Users
        };
        admits(&policy, subject, Direction::Immigration, &lists)
    });
    for due in &closed {
        end_stay(state, &mut conn, due.id).await?;
        tracing::info!(%home, user = %due.id.0, "ended the sessions of a user whose home this deployment no longer admits");
    }
    let Some(here) = own_domain(config) else {
        return Ok(());
    };
    for chunk in admitted.chunks(MAX_USERS) {
        let now = Utc::now();
        let request = sign(
            &mut conn,
            REQUEST_TYPE,
            &StandingRequest {
                iss: here.clone(),
                aud: home.clone(),
                iat: now.timestamp(),
                exp: (now + LIFETIME).timestamp(),
                jti: Uuid::new_v4(),
                users: chunk.iter().map(|due| due.home_id).collect(),
            },
        )
        .await?;
        match ask(state, home, request).await {
            Ok(answer) => apply(state, &mut conn, home, chunk, answer).await?,
            Err(error) => {
                tracing::info!(%home, %error, "could not ask a home about its users");
                let grace = Duration::seconds(
                    i64::try_from(config.standing_grace_seconds).unwrap_or(i64::MAX),
                );
                for due in chunk {
                    if due.confirmed_at.is_none_or(|at| at < Utc::now() - grace) {
                        end_stay(state, &mut conn, due.id).await?;
                        tracing::info!(%home, user = %due.id.0, "ended the sessions of a user whose home has gone unreached");
                    }
                }
            }
        }
    }
    Ok(())
}

/// Sends a request to `home` and verifies its answer.
async fn ask(
    state: &GlobalServerContext,
    home: &Domain,
    request: String,
) -> app::Result<StandingAnswer> {
    #[derive(Deserialize)]
    struct Answer {
        standing: String,
    }
    let unreachable = || {
        app::Error::DeploymentUnreachable(crate::t!("federationNoAnswer", domain = home.as_str()))
    };
    let response = state
        .federation_client
        .post(format!(
            "https://{home}{}{STANDING_PATH}",
            crate::api::API_PREFIX
        ))
        .json(&serde_json::json!({ "request": request }))
        .send()
        .await
        .map_err(|error| super::fetch::failure(home, &error))?;
    if !response.status().is_success() {
        return Err(unreachable());
    }
    let body = super::fetch::read_capped(home, response, MAX_ANSWER_BYTES)
        .await?
        .ok_or_else(unreachable)?;
    let Answer { standing } = serde_json::from_slice(&body).map_err(|_| unreachable())?;
    let Received { claims, from, .. } =
        receive::<StandingAnswer>(state, &standing, &[Direction::Immigration]).await?;
    if from != *home {
        return Err(unreachable());
    }
    Ok(claims)
}

async fn apply(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    home: &Domain,
    asked: &[Due],
    answer: StandingAnswer,
) -> app::Result<()> {
    for due in asked {
        let Some(said) = answer.users.iter().find(|u| u.sub == due.home_id) else {
            continue;
        };
        match said.standing {
            Standing::Good => {
                diesel::update(user::table.find(due.id))
                    .set(user::home_confirmed_at.eq(diesel::dsl::now))
                    .execute(conn)
                    .await?;
            }
            Standing::Gone => {
                let retired = conn
                    .transaction(|conn| app::user::retire(state, conn, due.id).scope_boxed())
                    .await?;
                retired.finish(state).await;
                tracing::info!(%home, user = %due.id.0, "retired a user their home says is gone");
            }
            Standing::Refused | Standing::Unknown => {
                end_stay(state, conn, due.id).await?;
                tracing::info!(%home, user = %due.id.0, "ended the sessions of a user their home no longer lets be here");
            }
        }
    }
    Ok(())
}

/// Ends the sessions of the users from elsewhere whose homes `policy` no longer admits, for a
/// change to the immigration gates that has committed. It reads only users with a session, so a
/// second run finds nothing left to do.
pub async fn shut_out(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    policy: &FederationPolicy,
) -> app::Result<()> {
    let staying: Vec<(UserId, Option<Domain>, bool)> = user::table
        .select((user::id, user::home_domain, user::bot))
        .filter(user::home_domain.is_not_null())
        .filter(user::deleted_at.is_null())
        .filter(diesel::dsl::exists(
            refresh_token::table
                .filter(refresh_token::user.eq(user::id))
                .filter(refresh_token::expires.gt(diesel::dsl::now)),
        ))
        .load(conn)
        .await?;
    let homes: Vec<Domain> = staying
        .iter()
        .filter_map(|(_, home, _)| home.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let lists = lists_of(conn, &homes).await?;
    for (id, home, bot) in staying {
        let Some(home) = home else { continue };
        let subject = if bot { Subject::Bots } else { Subject::Users };
        let on = lists.get(&home).map(Vec::as_slice).unwrap_or_default();
        if !admits(policy, subject, Direction::Immigration, on) {
            end_stay(state, conn, id).await?;
            tracing::info!(%home, user = %id.0, "ended the sessions of a user whose home this deployment no longer admits");
        }
    }
    Ok(())
}

/// Ends a foreign user's sessions here, closing their event streams, and takes them out of
/// their calls.
async fn end_stay(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    user: UserId,
) -> app::Result<()> {
    app::login::revoke_all_sessions(state, conn, user).await?;
    if let Err(e) = app::voice::kick_everywhere(state, conn, user).await {
        tracing::warn!(user = %user.0, error = %e, "could not take a user out of their calls");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::FederationKeyId;
    use ring::rand::SystemRandom;
    use ring::signature::Ed25519KeyPair;

    /// The longest domain there can be: a 253-byte name and a five-digit port.
    fn longest_domain(first: char) -> Domain {
        let label = |c: char, n: usize| std::iter::repeat_n(c, n).collect::<String>();
        Domain::parse(&format!(
            "{}.{}.{}.{}:65535",
            label(first, 63),
            label('b', 63),
            label('c', 63),
            label('d', 61)
        ))
        .unwrap()
    }

    #[test]
    fn the_largest_request_and_answer_fit_in_a_statement() {
        let document = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
        let key = Ed25519KeyPair::from_pkcs8(document.as_ref()).unwrap();
        let (asker, home) = (longest_domain('a'), longest_domain('e'));
        assert_eq!(asker.as_str().len(), 259);
        let users: Vec<Uuid> = (0..MAX_USERS).map(|_| Uuid::new_v4()).collect();
        let now = Utc::now();
        let request = jws::sign(
            REQUEST_TYPE,
            FederationKeyId::new(),
            &key,
            &StandingRequest {
                iss: asker.clone(),
                aud: home.clone(),
                iat: now.timestamp(),
                exp: (now + LIFETIME).timestamp(),
                jti: Uuid::new_v4(),
                users: users.clone(),
            },
        );
        let answer = jws::sign(
            ANSWER_TYPE,
            FederationKeyId::new(),
            &key,
            &StandingAnswer {
                iss: home,
                aud: asker,
                iat: now.timestamp(),
                exp: (now + LIFETIME).timestamp(),
                jti: Uuid::new_v4(),
                users: users
                    .into_iter()
                    .map(|sub| UserStanding {
                        sub,
                        standing: Standing::Refused,
                    })
                    .collect(),
            },
        );
        assert!(jws::parse(&request).is_ok(), "{} bytes", request.len());
        assert!(jws::parse(&answer).is_ok(), "{} bytes", answer.len());
        // Room to spare for standings with names up to 16 bytes longer.
        assert!(answer.len() + MAX_USERS * 16 * 4 / 3 <= jws::MAX_LENGTH);
        let body = serde_json::to_vec(&serde_json::json!({ "standing": answer })).unwrap();
        assert!(body.len() <= MAX_ANSWER_BYTES);
    }

    #[test]
    fn a_standing_from_a_newer_home_reads_as_unknown() {
        let answer: StandingAnswer = serde_json::from_str(include_str!(
            "../../../../spec/fixtures/federation/standing-from-newer.json"
        ))
        .unwrap();
        assert_eq!(answer.users[0].standing, Standing::Good);
        assert_eq!(answer.users[1].standing, Standing::Unknown);
    }
}
