//! Whether the users from elsewhere signed in here are still in good standing at home. About
//! every `[federation] standing_interval_seconds`, this deployment asks each home, in one signed
//! request per home (`aspen-standing-request+jwt`, POSTed to `/api/v1/federation/standing`),
//! about its users with a session here, and the home answers with a signed statement
//! (`aspen-standing+jwt`) saying, for each:
//! - `good`: the account exists and its home still lets it use this deployment; it is confirmed.
//! - `gone`: the account was deleted; its user here is retired.
//! - `refused`: its home no longer lets it use this deployment (the user left it, the home's
//!   emigration gate closed to it, or the home banned them, `app::user_ban`); its sessions here
//!   end, and it may sign in again if that changes.
//!
//! A home answers only for users who signed in at the asker (`user_foreign_deployment`), and
//! says `refused` of anyone else, so an answer reveals nothing of accounts the asker was never
//! given.
//!
//! A standing this deployment does not know it takes as `refused`. Sessions also end when this
//! deployment's own immigration gate no longer admits the home, at once when the gate or a list
//! changes ([`shut_out`]) and at each pass; when the home is suspended for presenting a key
//! nothing vouches for, at once ([`shut_out_home`]) and at each pass; and when the home has gone
//! unreached for `standing_grace_seconds`.
//!
//! One server does each pass, under an advisory lock, while any gate is open. It reads again
//! the documents of the deployments in use that a gate admits ([`refresh_documents`]), so a key
//! one replaced stops verifying here within an interval, and beside that, while an immigration
//! gate admits anyone, asks the homes, [`CONCURRENCY`] at a time and each within [`BUDGET`], so
//! slow homes delay no one else; then it forgets deployments contacted once and never used
//! (`directory::prune_unused`). A deployment that fails is left alone for a while, the wait doubling with
//! each failure in a row up to an interval; its users still lose their sessions once unconfirmed
//! for the grace.

use crate::UserId;
use crate::context::GlobalServerContext;
use crate::events::Publishing;
use crate::federation::contact::{ContactOutcome, contact};
use crate::federation::keys::{SigningKey, signing_key};
use crate::federation::received::{Received, Senders, Statement, receive};
use crate::federation::{
    Direction, Domain, FederationList, FederationPolicy, Subject, admits, jws, lists_of, own_domain,
};
use aspen_schema::{federated_deployment, refresh_token, user, user_foreign_deployment};
use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use futures_util::StreamExt as _;
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
) -> crate::Result<String> {
    let key = signing_key(conn).await?.ok_or_else(|| {
        crate::Error::Config(config::ConfigError::Message("no federation key yet".into()))
    })?;
    Ok(jws::sign(typ, key.id, &key.pair, claims))
}

// ---------------------------------------------------------------------------
// At home: answering for this deployment's users
// ---------------------------------------------------------------------------

/// Answers another deployment's request about this deployment's users signed in there.
pub async fn answer(state: &GlobalServerContext, token: &str) -> crate::Result<String> {
    let Received {
        claims,
        from,
        lists,
    } = receive::<StandingRequest>(state, token, Senders::Admitted(&[Direction::Emigration]))
        .await?;
    let here = own_domain(&state.config.federation)
        .ok_or_else(|| crate::Error::FederationRefused(crate::t!("federationOff")))?;
    // A request asking about more users is answered for the first of them, so the answer fits
    // in a statement; the rest go unanswered, which the asker takes as no news of them.
    let asked: Vec<Uuid> = claims.users.into_iter().take(MAX_USERS).collect();
    let mut conn = state.connection_pool.get().await?;
    // Only users who signed in at the asker are answered for, so an answer tells the asker
    // nothing of an account it was never given: whether it exists, was deleted, or is banned.
    // Everyone else is `refused`, whatever is true of them, which is also what one who left
    // the asker is.
    let found: Vec<(UserId, bool, bool, bool)> = user::table
        .select((
            user::id,
            user::bot,
            user::deleted_at.is_not_null(),
            crate::user_ban::banned(),
        ))
        .filter(user::id.eq_any(&asked))
        .filter(user::home_domain.is_null())
        .filter(diesel::dsl::exists(
            user_foreign_deployment::table
                .filter(user_foreign_deployment::user.eq(user::id))
                .filter(user_foreign_deployment::domain.eq(from.as_str())),
        ))
        .load(&mut conn)
        .await?;
    let policy = state.settings().federation;
    let users = asked
        .into_iter()
        .map(|sub| {
            let standing = match found.iter().find(|(id, ..)| id.0 == sub) {
                None => Standing::Refused,
                Some((_, _, true, _)) => Standing::Gone,
                Some((_, bot, false, banned)) => {
                    let subject = if *bot { Subject::Bots } else { Subject::Users };
                    if !banned && admits(&policy, subject, Direction::Emigration, &lists) {
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

/// How many homes a pass asks at once, and how many deployments it reads the documents of at
/// once, so a few slow ones hold up no one else.
const CONCURRENCY: usize = 16;
/// The longest a pass waits on one deployment: for all of a home's answers, or for its
/// document. A home that takes longer is taken as unreached.
const BUDGET: std::time::Duration = std::time::Duration::from_secs(20);
/// The longest a pass spends reading documents again, however many are due.
const REFRESH_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

/// Runs a pass whenever one may be due, for as long as the server runs, while any gate is open:
/// the documents of the deployments a gate admits are read again, so a key they replaced stops
/// verifying, and, while an immigration gate admits anyone, the homes of the users from
/// elsewhere are asked about them.
pub fn spawn_confirmer(state: GlobalServerContext) {
    tokio::spawn(async move {
        let interval = state.config.federation.standing_interval_seconds.max(1);
        let every = std::time::Duration::from_secs(interval.min(300));
        let mut tick = tokio::time::interval(every);
        let longest = std::time::Duration::from_secs(interval);
        let mut backoff = Backoffs {
            documents: Backoff::new(every, longest),
            homes: Backoff::new(every, longest),
        };
        loop {
            tick.tick().await;
            if !state.settings().federation.enabled() {
                continue;
            }
            let (passed, noted) = crate::events::noting(pass(&state, &mut backoff)).await;
            crate::events::settle(&state, noted, passed.is_err()).await;
            if let Err(error) = passed {
                tracing::warn!(%error, "checking foreign users' standing failed");
            }
        }
    });
}

/// Who a pass leaves alone for now: deployments whose documents could not be read, and homes
/// that did not answer.
struct Backoffs {
    documents: Backoff,
    homes: Backoff,
}

/// The deployments that did not answer the last time they were tried, and when each may be
/// tried again: the wait doubles from `first` with each failure in a row, up to `longest`. It
/// belongs to the server doing passes; another server taking over starts afresh.
struct Backoff {
    first: std::time::Duration,
    longest: std::time::Duration,
    failing: HashMap<Domain, (u32, tokio::time::Instant)>,
}

impl Backoff {
    fn new(first: std::time::Duration, longest: std::time::Duration) -> Self {
        Backoff {
            first,
            longest: longest.max(first),
            failing: HashMap::new(),
        }
    }

    /// Whether `domain` may be tried now.
    fn ready(&self, domain: &Domain) -> bool {
        self.failing
            .get(domain)
            .is_none_or(|(_, after)| *after <= tokio::time::Instant::now())
    }

    /// How many times in a row `domain` has failed, which orders who is tried first.
    fn failures(&self, domain: &Domain) -> u32 {
        self.failing
            .get(domain)
            .map_or(0, |(failures, _)| *failures)
    }

    fn record(&mut self, domain: &Domain, reached: bool) {
        if reached {
            self.failing.remove(domain);
            return;
        }
        let failures = self.failures(domain).saturating_add(1);
        let wait = self
            .first
            .saturating_mul(2u32.saturating_pow(failures - 1))
            .min(self.longest);
        self.failing.insert(
            domain.clone(),
            (failures, tokio::time::Instant::now() + wait),
        );
    }
}

/// Does what is due, if no other server is doing so: reads again the documents of the
/// deployments in use that a gate admits while it asks every home about its users here who are
/// due, then forgets the deployments recorded on first contact that went unused
/// (`directory::prune_unused`).
async fn pass(state: &GlobalServerContext, backoff: &mut Backoffs) -> crate::Result<()> {
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
            // The two run side by side, so however many documents are due, homes are asked
            // about their users within the pass.
            let Backoffs { documents, homes } = backoff;
            let confirming = async {
                if state.settings().federation.admits_anyone() {
                    confirm_users(state, homes).await
                } else {
                    Ok(())
                }
            };
            let (refreshed, confirmed) =
                tokio::join!(refresh_documents(state, documents), confirming);
            refreshed?;
            confirmed?;
            let pruned = super::directory::prune_unused(lock.as_mut()).await?;
            if pruned > 0 {
                tracing::info!(pruned, "forgot deployments contacted once and never used");
            }
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Reads again the document of every deployment in use (`directory::IN_USE_SQL`) that a gate
/// admits, whose key is pinned and that was last contacted more than `standing_interval_seconds`
/// ago, as [`contact`] does: a key handed over to is followed, so the key it replaced stops
/// verifying here, and a key nothing vouches for suspends the deployment and signs its users out
/// (`contact::record_contact`). A deployment no gate admits, or not in use, is not contacted.
/// The longest contacted first, and all within [`REFRESH_BUDGET`]: those not reached by then
/// are still due at the next pass, and do not count as failing.
async fn refresh_documents(
    state: &GlobalServerContext,
    backoff: &mut Backoff,
) -> crate::Result<()> {
    let config = &state.config.federation;
    let due_before = Utc::now()
        - Duration::seconds(i64::try_from(config.standing_interval_seconds).unwrap_or(i64::MAX));
    let mut conn = state.connection_pool.get().await?;
    let due: Vec<Domain> = federated_deployment::table
        .select(federated_deployment::domain)
        .filter(federated_deployment::public_key.is_not_null())
        .filter(
            federated_deployment::last_contact_at
                .is_null()
                .or(federated_deployment::last_contact_at.lt(due_before)),
        )
        .filter(super::directory::in_use())
        .order(federated_deployment::last_contact_at.asc().nulls_first())
        .load(&mut conn)
        .await?;
    let lists = lists_of(&mut conn, &due).await?;
    drop(conn);
    let policy = state.settings().federation;
    let mut due: Vec<Domain> = due
        .into_iter()
        .filter(|domain| {
            let on = lists.get(domain).map(Vec::as_slice).unwrap_or_default();
            [Subject::Users, Subject::Bots].into_iter().any(|subject| {
                [Direction::Emigration, Direction::Immigration]
                    .into_iter()
                    .any(|direction| admits(&policy, subject, direction, on))
            })
        })
        .filter(|domain| backoff.ready(domain))
        .collect();
    // A stable sort, so among those failing as often the longest contacted stay first.
    due.sort_by_key(|domain| backoff.failures(domain));
    let reached: Vec<(Domain, bool)> = futures_util::stream::iter(due)
        .map(|domain| async move {
            let reached = match tokio::time::timeout(BUDGET, contact(state, &domain)).await {
                Ok(Ok((_, ContactOutcome::KeyChanged))) => false,
                Ok(Ok(_)) => true,
                Ok(Err(error)) => {
                    tracing::info!(%domain, %error, "could not read a deployment's document again");
                    false
                }
                Err(_) => {
                    tracing::info!(%domain, "a deployment took too long to serve its document");
                    false
                }
            };
            (domain, reached)
        })
        .buffer_unordered(CONCURRENCY)
        .take_until(tokio::time::sleep(REFRESH_BUDGET))
        .collect()
        .await;
    for (domain, reached) in reached {
        backoff.record(&domain, reached);
    }
    Ok(())
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

/// What one pass knows of every home, read once for them all.
struct PassContext<'a> {
    state: &'a GlobalServerContext,
    here: Domain,
    key: SigningKey,
    policy: FederationPolicy,
    lists: HashMap<Domain, Vec<FederationList>>,
    /// Homes that presented a key nothing vouches for, refused until it is accepted.
    suspended: Vec<Domain>,
}

/// Asks every home about its users here who are due, [`CONCURRENCY`] homes at a time, the homes
/// that answered last time first and those that keep failing left alone for a while.
async fn confirm_users(state: &GlobalServerContext, backoff: &mut Backoff) -> crate::Result<()> {
    let config = &state.config.federation;
    let Some(here) = own_domain(config) else {
        return Ok(());
    };
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
    if by_home.is_empty() {
        return Ok(());
    }
    let homes: Vec<Domain> = by_home.keys().cloned().collect();
    let lists = lists_of(&mut conn, &homes).await?;
    let suspended: Vec<Domain> = federated_deployment::table
        .select(federated_deployment::domain)
        .filter(federated_deployment::domain.eq_any(&homes))
        .filter(federated_deployment::offered_key.is_not_null())
        .load(&mut conn)
        .await?;
    let Some(key) = signing_key(&mut conn).await? else {
        return Ok(());
    };
    drop(conn);
    let context = PassContext {
        state,
        here,
        key,
        policy: state.settings().federation,
        lists,
        suspended,
    };
    let mut homes: Vec<(Domain, Vec<Due>, bool)> = by_home
        .into_iter()
        .map(|(home, users)| {
            let ready = backoff.ready(&home);
            (home, users, ready)
        })
        .collect();
    homes.sort_by_key(|(home, _, _)| backoff.failures(home));
    let context = &context;
    let reached: Vec<(Domain, Option<bool>)> = futures_util::stream::iter(homes)
        .map(|(home, users, ready)| async move {
            let reached = match confirm_home(context, &home, users, ready).await {
                Ok(reached) => reached,
                Err(error) => {
                    tracing::info!(%home, %error, "could not confirm a home's users");
                    Some(false)
                }
            };
            (home, reached)
        })
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
    for (home, reached) in reached {
        if let Some(reached) = reached {
            backoff.record(&home, reached);
        }
    }
    Ok(())
}

/// Confirms `home`'s users here: those this deployment's own gate no longer admits, or all of
/// them when the home is suspended, have their sessions ended; the rest are asked about when
/// `ask` says the home may be tried now, and are otherwise taken as unreached. Returns whether
/// the home answered, or `None` when it was not asked. No connection is held while the home is
/// asked, nor for longer than [`BUDGET`] in all.
async fn confirm_home(
    context: &PassContext<'_>,
    home: &Domain,
    users: Vec<Due>,
    ask_now: bool,
) -> crate::Result<Option<bool>> {
    let state = context.state;
    let deadline = tokio::time::Instant::now() + BUDGET;
    let on = context
        .lists
        .get(home)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let suspended = context.suspended.contains(home);
    // Those this deployment's own gate no longer admits are not asked about.
    let (admitted, closed): (Vec<Due>, Vec<Due>) = users.into_iter().partition(|due| {
        let subject = if due.bot {
            Subject::Bots
        } else {
            Subject::Users
        };
        !suspended && admits(&context.policy, subject, Direction::Immigration, on)
    });
    if !closed.is_empty() {
        let mut conn = state.connection_pool.get().await?;
        for due in &closed {
            end_stay(state, &mut conn, due.id).await?;
            tracing::info!(%home, user = %due.id.0, "ended the sessions of a user whose home this deployment no longer admits");
        }
    }
    if admitted.is_empty() {
        return Ok(None);
    }
    let mut reached = ask_now;
    for chunk in admitted.chunks(MAX_USERS) {
        let answer = if reached {
            let now = Utc::now();
            let request = jws::sign(
                REQUEST_TYPE,
                context.key.id,
                &context.key.pair,
                &StandingRequest {
                    iss: context.here.clone(),
                    aud: home.clone(),
                    iat: now.timestamp(),
                    exp: (now + LIFETIME).timestamp(),
                    jti: Uuid::new_v4(),
                    users: chunk.iter().map(|due| due.home_id).collect(),
                },
            );
            match tokio::time::timeout_at(deadline, ask(state, home, request)).await {
                Ok(Ok(answer)) => Some(answer),
                Ok(Err(error)) => {
                    tracing::info!(%home, %error, "could not ask a home about its users");
                    None
                }
                Err(_) => {
                    tracing::info!(%home, "a home took too long to answer about its users");
                    None
                }
            }
        } else {
            None
        };
        let mut conn = state.connection_pool.get().await?;
        match answer {
            Some(answer) => apply(state, &mut conn, home, chunk, answer).await?,
            None => {
                // Once a home fails, the rest of its users are not asked about this pass.
                reached = false;
                unreached(state, &mut conn, home, chunk).await?;
            }
        }
    }
    Ok(ask_now.then_some(reached))
}

/// Ends the sessions of those of `home`'s users who have gone unconfirmed for longer than
/// `standing_grace_seconds`, for a home that did not answer or was not asked.
async fn unreached(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    home: &Domain,
    users: &[Due],
) -> crate::Result<()> {
    let grace = Duration::seconds(
        i64::try_from(state.config.federation.standing_grace_seconds).unwrap_or(i64::MAX),
    );
    for due in users {
        if due.confirmed_at.is_none_or(|at| at < Utc::now() - grace) {
            end_stay(state, conn, due.id).await?;
            tracing::info!(%home, user = %due.id.0, "ended the sessions of a user whose home has gone unreached");
        }
    }
    Ok(())
}

/// Sends a request to `home` and verifies its answer.
async fn ask(
    state: &GlobalServerContext,
    home: &Domain,
    request: String,
) -> crate::Result<StandingAnswer> {
    #[derive(Deserialize)]
    struct Answer {
        standing: String,
    }
    let unreachable = || {
        crate::Error::DeploymentUnreachable(crate::t!("federationNoAnswer", domain = home.as_str()))
    };
    let response = state
        .federation_client
        .post(format!(
            "https://{home}{}{STANDING_PATH}",
            aspen_wire::API_PREFIX
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
    let Received { claims, from, .. } = receive::<StandingAnswer>(
        state,
        &standing,
        Senders::Admitted(&[Direction::Immigration]),
    )
    .await?;
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
) -> crate::Result<()> {
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
                    .transaction(|conn| crate::user::retire(state, conn, due.id).scope_boxed())
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
) -> crate::Result<()> {
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

/// Ends the sessions of every user of `home` signed in here, for a home that is suspended
/// because it presented a key nothing vouches for (`contact::record_contact`). It reads only
/// users with a session, so a second run finds nothing left to do.
pub async fn shut_out_home(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    home: &Domain,
) -> crate::Result<()> {
    let staying: Vec<UserId> = user::table
        .select(user::id)
        .filter(user::home_domain.eq(home))
        .filter(user::deleted_at.is_null())
        .filter(diesel::dsl::exists(
            refresh_token::table
                .filter(refresh_token::user.eq(user::id))
                .filter(refresh_token::expires.gt(diesel::dsl::now)),
        ))
        .load(conn)
        .await?;
    for id in staying {
        end_stay(state, conn, id).await?;
        tracing::info!(%home, user = %id.0, "ended the sessions of a user whose home presented a key nothing vouches for");
    }
    Ok(())
}

/// Ends a foreign user's sessions here, closing their event streams, and takes them out of
/// their calls.
async fn end_stay(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    user: UserId,
) -> crate::Result<()> {
    crate::login::revoke_all_sessions(state, conn, user).await?;
    if let Err(e) = crate::voice::kick_everywhere(state, conn, user).await {
        tracing::warn!(user = %user.0, error = %e, "could not take a user out of their calls");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FederationKeyId;
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
