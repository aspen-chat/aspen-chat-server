use argon2::{
    PasswordHash, PasswordVerifier as _,
    password_hash::{Salt, SaltString},
};
use base64::{Engine, prelude::BASE64_STANDARD};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use diesel::{BoolExpressionMethods, ExpressionMethods as _, QueryDsl, SelectableHelper};
use diesel_async::{
    AsyncConnection, AsyncPgConnection, RunQueryDsl, scoped_futures::ScopedFutureExt,
};
use rand::RngExt;
use tracing::error;

use crate::PasswordRequirement;
use crate::context::GlobalServerContext;
use crate::ephemeral_token;
use crate::events::Publishing;
use crate::two_factor::{self, SecondFactor, SecondFactorMethods};
use crate::user::UserPg;
use crate::{CHACHA_RNG, UserId};
use aspen_schema as schema;
use aspen_wire::message_enum::server_event::ServerEvent;
use diesel::OptionalExtension;
use serde::{Deserialize, Serialize};

const REFRESH_TOKEN_LIFETIME: Duration = Duration::weeks(52);
const SESSION_TOKEN_LIFETIME: Duration = Duration::hours(3);
pub const PASSWORD_MIN_LENGTH: usize = 8;

/// How a sign-in proved who its user is. A foreign user's sign-in records what their home
/// deployment said of theirs (`app::federation::abroad`).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    utoipa::ToSchema,
    schemars::JsonSchema,
    diesel::FromSqlRow,
    diesel::AsExpression,
)]
#[serde(rename_all = "camelCase")]
#[diesel(sql_type = diesel::sql_types::Text)]
pub enum SignInMethod {
    /// A password alone.
    Password,
    /// A password and a second factor.
    SecondFactor,
    /// A passkey, which proves possession and the person's presence on its own.
    Passkey,
    /// A bot's token.
    Token,
}

crate::wire_name_traits!(SignInMethod);
crate::text_sql_traits!(SignInMethod);

impl SignInMethod {
    /// Whether it proved more than a password does, as a deployment that requires two factors
    /// asks of everyone.
    pub fn strong(self) -> bool {
        matches!(self, Self::SecondFactor | Self::Passkey)
    }
}

/// Credentials issued by a completed sign-in.
pub struct Session {
    pub user_id: UserId,
    pub refresh_token: String,
    pub session_token: String,
    pub session_token_expires: DateTime<Utc>,
    /// The server requires a second factor this account does not have yet; until it adds one,
    /// the session may do nothing else.
    pub enrollment_required: bool,
}

pub enum LoginOutcome {
    SignedIn(Session),
    /// The password was right and the account has two-factor sign-in on. `ticket` stands for
    /// the half-finished sign-in in `complete_second_factor` or a passkey sign-in.
    SecondFactorRequired {
        ticket: String,
        methods: SecondFactorMethods,
    },
    InvalidCredentials,
}

/// A sign-in waiting for its second factor.
#[derive(Serialize, Deserialize)]
struct Ticket {
    user: UserId,
}

const TICKET_PREFIX: &str = "auth:ticket";

/// Names a waiting sign-in without being usable as its ticket: the Valkey key it waits under,
/// which holds a digest of the ticket. What waits on a ticket elsewhere in Valkey (a passkey
/// ceremony that is its second factor) keeps this rather than the ticket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketKey(String);

impl TicketKey {
    pub fn of(ticket: &str) -> Self {
        Self(ephemeral_token::token_key(TICKET_PREFIX, ticket))
    }
}
/// Long enough to find a phone and open an authenticator app.
const TICKET_LIFETIME_SECONDS: i64 = 5 * 60;

/// The server's allowance for password work: Argon2 takes a large block of memory and most of a
/// core for a noticeable time per hash, so it runs on a fixed number of blocking threads and
/// work beyond them queues for a bounded time. A burst of sign-ins (everyone reconnecting after
/// a restart) then costs a known amount of memory and leaves the rest of the server its CPU,
/// and the requests that cannot be served soon are refused with `serverBusy` rather than all
/// slowing down together.
struct PasswordWork {
    permits: tokio::sync::Semaphore,
    wait: std::time::Duration,
}

static PASSWORD_WORK: std::sync::OnceLock<PasswordWork> = std::sync::OnceLock::new();

/// The blocking threads password work gets when nothing configured them: one per logical CPU.
pub fn default_password_hashing_threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get())
}

/// Sets the password work allowance; the first call wins. Called once at startup from `[auth]`;
/// a process that never calls it (an operator command) gets one thread per CPU and a ten
/// second queue.
pub fn configure_password_work(threads: usize, wait: std::time::Duration) {
    let _ = PASSWORD_WORK.set(PasswordWork {
        permits: tokio::sync::Semaphore::new(threads.max(1)),
        wait,
    });
}

/// Runs `work` on a blocking thread once the allowance has room, or fails with `Busy` if it
/// has none within the wait.
async fn password_work<T: Send + 'static>(
    work: impl FnOnce() -> T + Send + 'static,
) -> crate::Result<T> {
    let allowance = PASSWORD_WORK.get_or_init(|| PasswordWork {
        permits: tokio::sync::Semaphore::new(default_password_hashing_threads()),
        wait: std::time::Duration::from_secs(10),
    });
    allowance.run(work).await
}

impl PasswordWork {
    async fn run<T: Send + 'static>(
        &self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> crate::Result<T> {
        let _permit = tokio::time::timeout(self.wait, self.permits.acquire())
            .await
            .map_err(|_| crate::Error::Busy)?
            .map_err(|_| crate::Error::Busy)?;
        Ok(tokio::task::spawn_blocking(work).await?)
    }
}

pub async fn hash_password(password: String) -> crate::Result<String> {
    password_work(|| {
        let argon2 = argon2::Argon2::default();
        CHACHA_RNG.with(|rng| {
            let bytes = rng.borrow_mut().random::<[u8; Salt::RECOMMENDED_LENGTH]>();
            SaltString::encode_b64(&bytes).and_then(|salt| {
                PasswordHash::generate(argon2, password, &salt).map(|h| h.to_string())
            })
        })
    })
    .await?
    .map_err(Into::into)
}

pub async fn check_password(password: String, entry_password_hash: String) -> crate::Result<bool> {
    password_work(move || {
        let argon2 = argon2::Argon2::default();
        let entry_hash = match PasswordHash::try_from(entry_password_hash.as_str()) {
            Ok(v) => v,
            Err(e) => {
                error!("user entry password hash malformed in database {e}");
                return false;
            }
        };
        argon2
            .verify_password(password.as_bytes(), &entry_hash)
            .is_ok()
    })
    .await
}

fn make_token() -> String {
    BASE64_STANDARD.encode(CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 32]>()))
}

pub enum TokenRefreshOutcome {
    Ok {
        session_token: String,
        session_token_expires: DateTime<Utc>,
    },
    InvalidToken,
}

pub async fn try_login(
    state: &GlobalServerContext,
    username: &str,
    password: &str,
) -> Result<LoginOutcome, crate::Error> {
    use schema::user::dsl::*;

    let mut conn = state.connection_pool.get().await?;
    let conn = conn.as_mut();
    let user_entry: Option<UserPg> = user
        .select(UserPg::as_select())
        .filter(crate::user::named(username.to_owned()))
        .first(conn)
        .await
        .optional()?;
    // A bot has no password; it signs in only with its token.
    let Some(u) = user_entry.filter(|u| !u.bot) else {
        return Ok(LoginOutcome::InvalidCredentials);
    };
    if !check_password(password.to_string(), u.password_hash).await? {
        return Ok(LoginOutcome::InvalidCredentials);
    }
    // A banned account learns so once its password is right, before a second factor is asked
    // for that could not be used.
    crate::user_ban::check_not_banned(conn, u.id).await?;
    let methods = two_factor::methods(conn, u.id).await?;
    if methods.any_factor() {
        let ticket = make_token();
        ephemeral_token::put_token(
            state,
            TICKET_PREFIX,
            &ticket,
            &Ticket { user: u.id },
            TICKET_LIFETIME_SECONDS,
        )
        .await?;
        return Ok(LoginOutcome::SecondFactorRequired { ticket, methods });
    }
    Ok(LoginOutcome::SignedIn(
        issue_session(state, conn, u.id, SignInMethod::Password, false).await?,
    ))
}

/// Starts a sign-in for `user_id`, made by `method`: a refresh token, verified now, and its first
/// session token. A `foreign` user's sign-in abroad owes this deployment no second factor: it
/// was admitted only if it proved enough at home. Every sign-in passes through here, so an
/// account banned from the deployment is refused here (`app::user_ban`).
pub async fn issue_session(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    method: SignInMethod,
    foreign: bool,
) -> crate::Result<Session> {
    issue(state, conn, user_id, method, foreign, Utc::now()).await
}

/// Starts a sign-in given by another of the user's sign-ins (`app::device_link`): it proved what
/// that one did, `method`, and was last verified when that one was, so a device signed in this
/// way is no stronger than the one that gave it and makes no security change without verifying
/// again.
pub async fn issue_linked_session(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    method: SignInMethod,
    verified_at: DateTime<Utc>,
) -> crate::Result<Session> {
    issue(state, conn, user_id, method, false, verified_at).await
}

async fn issue(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    method: SignInMethod,
    foreign: bool,
    verified_at: DateTime<Utc>,
) -> crate::Result<Session> {
    use aspen_schema::{refresh_token, session};
    let session_token = make_token();
    let refresh_token = make_token();
    let now = Utc::now();
    let session_token_expires = now + SESSION_TOKEN_LIFETIME;
    crate::user_ban::check_not_banned(conn, user_id).await?;
    conn.transaction(|conn| {
        let (refresh_token, session_token) =
            (token_digest(&refresh_token), token_digest(&session_token));
        async move {
            diesel::insert_into(refresh_token::table)
                .values((
                    refresh_token::dsl::token.eq(&refresh_token),
                    refresh_token::dsl::user.eq(user_id),
                    refresh_token::dsl::expires.eq((now + REFRESH_TOKEN_LIFETIME).naive_utc()),
                    refresh_token::dsl::verified_at.eq(verified_at),
                    refresh_token::dsl::method.eq(method),
                ))
                .execute(conn)
                .await?;
            diesel::insert_into(session::table)
                .values((
                    session::dsl::token.eq(session_token),
                    session::dsl::refresh_token.eq(&refresh_token),
                    session::dsl::expires.eq(session_token_expires.naive_utc()),
                ))
                .execute(conn)
                .await?;
            crate::Result::Ok(())
        }
        .scope_boxed()
    })
    .await?;
    Ok(Session {
        user_id,
        refresh_token,
        session_token,
        session_token_expires,
        enrollment_required: state.settings().require_two_factor
            && method == SignInMethod::Password
            && !foreign,
    })
}

pub enum SecondFactorOutcome {
    SignedIn(Session),
    /// The ticket is unknown, expired, or already used.
    InvalidTicket,
    /// The code was wrong or already used.
    Rejected,
}

/// Finishes a sign-in that `try_login` left waiting for a second factor.
pub async fn complete_second_factor(
    state: &GlobalServerContext,
    ticket: &str,
    factor: &SecondFactor,
) -> crate::Result<SecondFactorOutcome> {
    let ticket = TicketKey::of(ticket);
    let Some(user) = ticket_user(state, &ticket).await? else {
        return Ok(SecondFactorOutcome::InvalidTicket);
    };
    if !two_factor::verify(state, user, factor).await? {
        return Ok(SecondFactorOutcome::Rejected);
    }
    finish_ticket(state, &ticket, Some(user))
        .await
        .map(|session| {
            session.map_or(
                SecondFactorOutcome::InvalidTicket,
                SecondFactorOutcome::SignedIn,
            )
        })
}

/// The user a waiting sign-in belongs to, without using it up.
pub async fn ticket_user(
    state: &GlobalServerContext,
    ticket: &TicketKey,
) -> crate::Result<Option<UserId>> {
    Ok(
        ephemeral_token::get_at::<Ticket>(state, ticket.0.clone(), false)
            .await?
            .map(|waiting| waiting.user),
    )
}

/// Uses up a waiting sign-in whose second factor has been verified and issues its session.
/// `None` when the ticket was used or expired meanwhile, or belongs to someone other than
/// `expected_user`.
pub async fn finish_ticket(
    state: &GlobalServerContext,
    ticket: &TicketKey,
    expected_user: Option<UserId>,
) -> crate::Result<Option<Session>> {
    let Some(waiting) = ephemeral_token::get_at::<Ticket>(state, ticket.0.clone(), true).await?
    else {
        return Ok(None);
    };
    if expected_user.is_some_and(|expected| expected != waiting.user) {
        return Ok(None);
    }
    let mut conn = state.connection_pool.get().await?;
    Ok(Some(
        issue_session(
            state,
            &mut conn,
            waiting.user,
            SignInMethod::SecondFactor,
            false,
        )
        .await?,
    ))
}

/// Issues a new session token from a live refresh token. A sign-in of an account that has been
/// deleted, or is banned from the deployment, gets none: its refresh token answers as invalid,
/// so the app signs out, and signing in again tells of the ban.
pub async fn try_token_refresh(
    mut conn: impl AsMut<AsyncPgConnection>,
    refresh_token_value: &str,
) -> Result<TokenRefreshOutcome, crate::Error> {
    use schema::{refresh_token, session, user};
    let conn = conn.as_mut();
    let refresh_digest = token_digest(refresh_token_value);
    let found: Option<(NaiveDateTime, UserId)> = refresh_token::table
        .inner_join(user::table)
        .select((refresh_token::expires, refresh_token::user))
        .filter(refresh_token::dsl::token.eq(&refresh_digest))
        .filter(user::deleted_at.is_null())
        .first(conn)
        .await
        .optional()?;
    let Some((expires, owner)) = found else {
        return Ok(TokenRefreshOutcome::InvalidToken);
    };
    if expires.and_utc() < Utc::now() {
        return Ok(TokenRefreshOutcome::InvalidToken);
    }
    match crate::user_ban::check_not_banned(conn, owner).await {
        Ok(()) => {}
        Err(crate::Error::DeploymentBanned { .. }) => return Ok(TokenRefreshOutcome::InvalidToken),
        Err(e) => return Err(e),
    }
    // If we got here then the token is valid. Issue a refresh.
    let new_token = make_token();
    let session_token_expires = Utc::now() + SESSION_TOKEN_LIFETIME;
    diesel::insert_into(session::table)
        .values((
            session::dsl::token.eq(token_digest(&new_token)),
            session::dsl::expires.eq(session_token_expires.naive_utc()),
            session::dsl::refresh_token.eq(&refresh_digest),
        ))
        .execute(conn)
        .await?;

    Ok(TokenRefreshOutcome::Ok {
        session_token: new_token,
        session_token_expires,
    })
}

/// The form refresh and session tokens are kept in: the SHA-256 of the token, in hex. The
/// database holds no token itself, so a copy of it (a backup, a leaked dump) signs no one in;
/// a presented token is looked up by its digest. The tokens are 256 random bits, so an
/// unsalted, fast digest gives nothing to guess from.
pub fn token_digest(token: &str) -> String {
    use sha2::Digest;
    hex(&sha2::Sha256::digest(token.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Names a sign-in (a refresh token and the sessions issued from it) without revealing it: the
/// first half of its refresh token's digest (`token_digest`), from that digest. Event streams
/// know which sign-in they belong to by it, `signInsEnded` events name the sign-ins that ended
/// by it, and what waits in Valkey to act on a sign-in (a passkey ceremony, a device link)
/// keeps it rather than the sign-in's tokens.
pub fn sign_in_id(refresh_digest: &str) -> String {
    refresh_digest
        .get(..32)
        .unwrap_or(refresh_digest)
        .to_string()
}

/// `sign_in_id` of the `refresh_token` row in scope, in SQL, compared with the bound value that
/// follows it.
pub const SIGN_IN_ID_IS_SQL: &str = "substr(refresh_token.token, 1, 32) = ";

/// Tells `user`'s event streams that sign-ins ended: `ended` alone, or without it every one but
/// `kept`. The streams of those sign-ins close.
async fn announce_ended(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    user: UserId,
    ended: Option<String>,
    kept: Option<String>,
) -> crate::Result<()> {
    let at = database_clock(conn).await?;
    crate::publish_event(
        state,
        conn,
        crate::EventScope::User(user),
        &ServerEvent::SignInsEnded { ended, kept, at },
    )
    .await
}

/// The database's clock as it reads now, which a sign-in's `refresh_token.created_at` is set
/// by. Read after a transaction's changes, it is later than the start of every transaction they
/// saw committed, so every sign-in an end of sign-ins or a ban covered began before it.
pub async fn database_clock(conn: &mut AsyncPgConnection) -> crate::Result<DateTime<Utc>> {
    Ok(
        diesel::select(diesel::dsl::sql::<diesel::sql_types::Timestamptz>(
            "clock_timestamp()",
        ))
        .get_result(conn)
        .await?,
    )
}

/// Revokes a refresh token and all sessions issued from it, closing their event streams.
/// Returns whether a refresh token was actually removed; callers treat an unknown token as
/// already revoked.
pub async fn try_logout(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    refresh_token_value: &str,
) -> Result<bool, crate::Error> {
    use schema::{refresh_token, session};
    let refresh_digest = token_digest(refresh_token_value);
    conn.transaction(|conn| {
        async move {
            diesel::delete(session::table)
                .filter(session::dsl::refresh_token.eq(&refresh_digest))
                .execute(conn)
                .await?;
            let owner: Option<UserId> = diesel::delete(refresh_token::table)
                .filter(refresh_token::dsl::token.eq(&refresh_digest))
                .returning(refresh_token::user)
                .get_result(conn)
                .await
                .optional()?;
            if let Some(user) = owner {
                let ended = Some(sign_in_id(&refresh_digest));
                announce_ended(state, conn, user, ended, None).await?;
            }
            Ok(owner.is_some())
        }
        .scope_boxed()
    })
    .await
}

pub enum ChangePasswordOutcome {
    Ok,
    OldPasswordIncorrect,
    RequirementNotMet(PasswordRequirement),
}

/// Changes the caller's password and expires every other session and refresh token belonging
/// to them, and every plugin capability URL of theirs (`app::plugin::capability::revoke_all`), so
/// a stolen credential stops working the moment the owner rotates their password, and tells the
/// account's verified address. The session performing the change stays valid. The old password proves who the caller is
/// for an account without two-factor sign-in; one with it also needs a recent verification. A
/// wrong old password counts toward the user's failure limit (`two_factor::limited`), as one
/// given to re-verify does, so a stolen session cannot guess it.
pub async fn try_change_password(
    state: &GlobalServerContext,
    mut conn: impl AsMut<AsyncPgConnection>,
    caller: &two_factor::Caller,
    config: &crate::aspen_config::AuthConfig,
    old_password: &str,
    new_password: &str,
) -> Result<ChangePasswordOutcome, crate::Error> {
    caller.ensure_person()?;
    if caller.has_second_factor {
        caller.ensure_recently_verified(config)?;
    }
    let user_id = caller.user;
    let current_session = caller.session_digest.as_str();
    let conn = conn.as_mut();
    let entry_password_hash: String = schema::user::table
        .select(schema::user::password_hash)
        .filter(
            schema::user::id
                .eq(&user_id.0)
                .and(schema::user::deleted_at.is_null()),
        )
        .first(conn)
        .await?;
    let old_password = old_password.to_string();
    let right = two_factor::limited(state, user_id, async || {
        check_password(old_password, entry_password_hash).await
    })
    .await?;
    if !right {
        return Ok(ChangePasswordOutcome::OldPasswordIncorrect);
    }
    if new_password.len() < PASSWORD_MIN_LENGTH {
        return Ok(ChangePasswordOutcome::RequirementNotMet(
            PasswordRequirement::Length,
        ));
    }
    let new_password_hash = hash_password(new_password.to_string()).await?;
    let current_session = current_session.to_string();
    conn.transaction(|conn| {
        async move {
            diesel::update(
                schema::user::table.filter(
                    schema::user::id
                        .eq(&user_id)
                        .and(schema::user::deleted_at.is_null()),
                ),
            )
            .set(schema::user::password_hash.eq(new_password_hash))
            .execute(conn)
            .await?;
            revoke_other_sessions(state, conn, user_id, &current_session).await?;
            crate::plugin::capability::revoke_all(conn, user_id).await?;
            crate::email::outbox::notify(
                state,
                conn,
                user_id,
                &crate::email::outbox::Mail::PasswordChanged,
            )
            .await
        }
        .scope_boxed()
    })
    .await?;

    Ok(ChangePasswordOutcome::Ok)
}

/// Expires every session and sign-in of `user_id`, closing their event streams, as when their
/// home withdraws them from this deployment, its moderators ban them, or the account ends.
/// Their phones are no longer woken (`app::push`), which goes by live sign-ins.
pub async fn revoke_all_sessions(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
) -> crate::Result<()> {
    use schema::refresh_token;
    diesel::update(
        refresh_token::table
            .filter(refresh_token::user.eq(user_id))
            .filter(refresh_token::expires.gt(diesel::dsl::now)),
    )
    .set(refresh_token::expires.eq(diesel::dsl::now))
    .execute(conn)
    .await?;
    announce_ended(state, conn, user_id, None, None).await
}

/// Ends every sign-in of the caller but their own, and every plugin capability URL of theirs, as
/// signing out everywhere else does, so whatever a stolen credential opened stops working. It
/// needs a recently verified session, so a stolen session cannot sign the owner out.
pub async fn end_other_sign_ins(
    state: &GlobalServerContext,
    caller: &two_factor::Caller,
) -> crate::Result<()> {
    caller.ensure_recently_verified(&state.config.auth)?;
    let user_id = caller.user;
    let kept = caller.sign_in();
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            revoke_other_sign_ins(state, conn, user_id, &kept).await?;
            crate::plugin::capability::revoke_all(conn, user_id).await
        }
        .scope_boxed()
    })
    .await
}

/// Expires every sign-in of `user_id` and its sessions except the sign-in named `kept`
/// (`sign_in_id`), as adding a first second factor and signing out everywhere else do, so a
/// stolen credential stops working once its owner secures the account.
pub async fn revoke_other_sign_ins(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    kept: &str,
) -> crate::Result<()> {
    let other = format!("NOT ({SIGN_IN_ID_IS_SQL}$1)");
    diesel::sql_query(format!(
        "
        UPDATE session
        SET expires = now()
        FROM refresh_token
        WHERE refresh_token.token = session.refresh_token
            AND refresh_token.user = $2
            AND session.expires > now()
            AND {other};
    "
    ))
    .bind::<diesel::sql_types::Text, _>(kept)
    .bind::<diesel::sql_types::Uuid, _>(&user_id)
    .execute(conn)
    .await?;
    diesel::sql_query(format!(
        "
        UPDATE refresh_token
        SET expires = now()
        WHERE refresh_token.user = $2
            AND refresh_token.expires > now()
            AND {other};
    "
    ))
    .bind::<diesel::sql_types::Text, _>(kept)
    .bind::<diesel::sql_types::Uuid, _>(&user_id)
    .execute(conn)
    .await?;
    announce_ended(state, conn, user_id, None, Some(kept.to_string())).await
}

/// Expires every session and sign-in of `user_id` except the one the session whose digest is
/// `current_session` (`token_digest`) belongs to, so a stolen credential stops working once its
/// owner secures the account.
pub async fn revoke_other_sessions(
    state: &impl Publishing,
    conn: &mut AsyncPgConnection,
    user_id: UserId,
    current_session: &str,
) -> crate::Result<()> {
    diesel::sql_query(
        "
        UPDATE session
        SET expires = now()
        FROM refresh_token
        WHERE refresh_token.token = session.refresh_token
            AND session.token != $1
            AND refresh_token.user = $2
            AND session.expires > now();
    ",
    )
    .bind::<diesel::sql_types::Text, _>(current_session)
    .bind::<diesel::sql_types::Uuid, _>(&user_id)
    .execute(conn)
    .await?;
    diesel::sql_query(
        "
        UPDATE refresh_token
        SET expires = now()
        WHERE refresh_token.user = $2
            AND refresh_token.expires > now()
            AND refresh_token.token != (
                SELECT session.refresh_token FROM session WHERE session.token = $1
            );
    ",
    )
    .bind::<diesel::sql_types::Text, _>(current_session)
    .bind::<diesel::sql_types::Uuid, _>(&user_id)
    .execute(conn)
    .await?;
    let kept: Option<String> = schema::session::table
        .select(schema::session::refresh_token)
        .filter(schema::session::token.eq(current_session))
        .first(conn)
        .await
        .optional()?;
    announce_ended(state, conn, user_id, None, kept.as_deref().map(sign_in_id)).await
}

#[cfg(test)]
mod token_tests {
    use super::*;

    #[test]
    fn a_sign_in_id_is_the_first_half_of_its_refresh_tokens_digest() {
        let digest = token_digest("abc");
        assert_eq!(
            digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(sign_in_id(&digest), "ba7816bf8f01cfea414140de5dae2223");
    }
}

#[cfg(test)]
mod password_work_tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn work_beyond_the_allowance_waits_and_then_is_refused() {
        let allowance = std::sync::Arc::new(PasswordWork {
            permits: tokio::sync::Semaphore::new(1),
            wait: Duration::from_millis(100),
        });
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let holder = tokio::spawn({
            let allowance = allowance.clone();
            async move {
                allowance
                    .run(move || {
                        let _ = started_tx.send(());
                        let _ = release_rx.recv();
                    })
                    .await
            }
        });
        started_rx.await.unwrap();
        assert!(matches!(
            allowance.run(|| ()).await,
            Err(crate::Error::Busy)
        ));
        release_tx.send(()).unwrap();
        holder.await.unwrap().unwrap();
        assert_eq!(allowance.run(|| 7).await.unwrap(), 7);
    }
}
