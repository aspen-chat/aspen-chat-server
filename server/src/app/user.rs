use crate::api::error::PasswordRequirement;
use crate::api::message_enum;
use crate::api::message_enum::request::{UserCreateRequest, UserUpdateRequest};
use crate::api::message_enum::server_event::UserEvent;
use crate::api::user::UserOnlineStatus;
use crate::app;
use crate::app::context::GlobalServerContext;
use crate::app::icon::Icon;
use crate::app::login::{PASSWORD_MIN_LENGTH, hash_password};
use crate::app::react::validate_emoji;
use crate::app::registration_invite;
use crate::app::{IconId, Loadable, MaybeLoaded, UserId, publish_event};
use crate::database::schema::{self, bot_token, refresh_token, session, user};
use crate::t;
use chrono::Utc;
use diesel::prelude::*;
use diesel::{BoolExpressionMethods, ExpressionMethods, Queryable, Selectable};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use fred::prelude::KeysInterface;

#[derive(Debug, Clone, Queryable, QueryableByName, Selectable, Insertable)]
#[diesel(table_name = user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserPg {
    pub id: UserId,
    pub name: String,
    pub icon: Option<MaybeLoaded<Icon>>,
    pub password_hash: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_seen_at: chrono::DateTime<chrono::Utc>,
    pub deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    pub display_name: Option<String>,
    pub pronouns: Option<String>,
    pub bio: Option<String>,
    pub status_text: Option<String>,
    pub status_emoji: Option<String>,
    /// Whether this is a bot, which signs in only with a token (`app::bot`).
    pub bot: bool,
    /// Whether this is the deployment's own account (`app::system_account`).
    pub system: bool,
    /// Who made the bot and manages it; `None` for a person, or a bot whose maker is gone.
    pub bot_owner: Option<UserId>,
    /// Whether anyone allowed to add bots to a community may add this one.
    pub bot_public: bool,
    /// For a foreign user, their home deployment and their id there; `None` for this
    /// deployment's own users (`app::federation::abroad`).
    pub home_domain: Option<crate::app::federation::Domain>,
    pub home_id: Option<uuid::Uuid>,
    /// For a foreign user, the home's id of the avatar `icon` is this deployment's copy of.
    pub home_icon: Option<uuid::Uuid>,
}

impl UserPg {
    /// Whether this is a user of another deployment.
    pub fn foreign(&self) -> bool {
        self.home_domain.is_some()
    }
}

pub struct User {
    pub user_pg: UserPg,
    pub online_status: UserOnlineStatus,
}

#[derive(Debug, Clone, AsChangeset)]
#[diesel(table_name = user)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct UserChangeset {
    pub name: Option<String>,
    pub icon: Option<Option<IconId>>,
    pub password_hash: Option<String>,
    pub display_name: Option<Option<String>>,
    pub pronouns: Option<Option<String>>,
    pub bio: Option<Option<String>>,
    pub status_text: Option<Option<String>>,
    pub status_emoji: Option<Option<String>>,
}

impl Loadable for User {
    type Id = UserId;

    async fn load_from_db(state: &GlobalServerContext, id: Self::Id) -> crate::app::Result<Self> {
        let user_pg = user::table
            .select(UserPg::as_select())
            .filter(user::dsl::id.eq(id).and(user::dsl::deleted_at.is_null()))
            .first(&mut state.connection_pool.get().await?)
            .await?;
        Ok(User {
            user_pg,
            online_status: user_online_status(state, id).await?,
        })
    }

    fn id(&self) -> &Self::Id {
        &self.user_pg.id
    }
}

/// Longest username accepted at registration. Long enough for any reasonable handle, short
/// enough to render in member lists without truncation.
pub const USERNAME_MAX_LENGTH: usize = 32;
/// Profile bounds. A display name is shown wherever the username would be, so it shares the
/// username's limit; a status is one line; a bio is a short paragraph.
pub const DISPLAY_NAME_MAX_CHARS: usize = USERNAME_MAX_LENGTH;
pub const PRONOUNS_MAX_CHARS: usize = 40;
pub const BIO_MAX_CHARS: usize = 1000;
pub const STATUS_MAX_CHARS: usize = 200;

/// Checks a profile field that must be non-blank and short enough when set; `None` clears it.
fn validate_profile_text(
    value: Option<&str>,
    max: usize,
    error: &'static str,
) -> Result<(), app::Error> {
    if let Some(text) = value
        && (text.trim().is_empty() || text.chars().count() > max)
    {
        return Err(app::Error::Validation(t!(error, max = max)));
    }
    Ok(())
}

/// Checks the profile fields of an update. Absent fields are unchanged and need no check.
pub(crate) fn validate_profile(command: &UserUpdateRequest) -> Result<(), app::Error> {
    if let Some(display_name) = &command.display_name {
        validate_profile_text(
            display_name.as_deref(),
            DISPLAY_NAME_MAX_CHARS,
            "displayNameLength",
        )?;
    }
    if let Some(pronouns) = &command.pronouns {
        validate_profile_text(pronouns.as_deref(), PRONOUNS_MAX_CHARS, "pronounsLength")?;
    }
    if let Some(bio) = &command.bio {
        validate_profile_text(bio.as_deref(), BIO_MAX_CHARS, "bioLength")?;
    }
    if let Some(Some(status)) = &command.status {
        validate_profile_text(Some(&status.text), STATUS_MAX_CHARS, "statusLength")?;
        if let Some(emoji) = &status.emoji {
            validate_emoji(emoji)?;
        }
    }
    Ok(())
}

/// Rejects usernames that are blank, padded with whitespace, or too long; a bot's name follows
/// the same rules.
pub(crate) fn validate_username(name: &str) -> Result<(), app::Error> {
    if name.is_empty() || name.trim() != name {
        return Err(app::Error::Validation(t!("usernameBlankOrPadded")));
    }
    if name.chars().count() > USERNAME_MAX_LENGTH {
        return Err(app::Error::Validation(t!(
            "usernameTooLong",
            max = USERNAME_MAX_LENGTH
        )));
    }
    Ok(())
}

/// Rejects a bad username, and passwords that fail the same length rule `try_change_password`
/// enforces.
fn validate_registration(command: &UserCreateRequest) -> Result<(), app::Error> {
    validate_username(&command.name)?;
    if command.password.len() < PASSWORD_MIN_LENGTH {
        return Err(app::Error::PasswordRequirement(PasswordRequirement::Length));
    }
    Ok(())
}

pub async fn create_user(
    state: GlobalServerContext,
    command: &UserCreateRequest,
) -> Result<UserId, app::Error> {
    validate_registration(command)?;
    let mut conn = match state.connection_pool.get().await {
        Ok(conn) => conn,
        Err(e) => {
            return Err(e.into());
        }
    };
    let invite_required = state.config.registration.invite_required;
    let invite_code = command
        .invite_code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty());
    // Refused before the password is hashed, which is the expensive part.
    if invite_required && invite_code.is_none() {
        return Err(app::Error::RegistrationInviteRequired);
    }
    let password_hash = hash_password(command.password.to_string()).await?;
    let new_user_id = UserId::new();
    let now = chrono::Utc::now();
    conn.transaction(|conn| {
        async move {
            // The invite is used in the transaction that makes the account, so an invite with
            // one use left makes one account however many register with it at once. Where
            // invites are optional, one that no longer works is ignored rather than refused.
            let registered_with = match invite_code {
                Some(code) => match registration_invite::redeem(conn.as_mut(), code).await {
                    Ok(()) => Some(code.to_string()),
                    Err(app::Error::RegistrationInviteInvalid) if !invite_required => None,
                    Err(e) => return Err(e),
                },
                None => None,
            };
            diesel::insert_into(user::table)
                .values(UserPg {
                    id: new_user_id,
                    name: command.name.clone(),
                    icon: command.icon.map(MaybeLoaded::NotLoaded),
                    password_hash,
                    created_at: now,
                    last_seen_at: now,
                    deleted_at: None,
                    display_name: command.display_name.clone(),
                    pronouns: command.pronouns.clone(),
                    bio: command.bio.clone(),
                    status_text: command.status.as_ref().map(|s| s.text.clone()),
                    status_emoji: command.status.as_ref().and_then(|s| s.emoji.clone()),
                    bot: false,
                    system: false,
                    bot_owner: None,
                    bot_public: false,
                    home_domain: None,
                    home_id: None,
                    home_icon: None,
                })
                .execute(conn.as_mut())
                .await?;
            if registered_with.is_some() {
                diesel::update(user::table.filter(user::id.eq(new_user_id)))
                    .set(user::registered_with.eq(registered_with))
                    .execute(conn.as_mut())
                    .await?;
            }
            Ok::<_, app::Error>(())
        }
        .scope_boxed()
    })
    .await?;
    Ok(new_user_id)
}

pub async fn read_user(state: &GlobalServerContext, id: UserId) -> crate::app::Result<User> {
    let user = User::load_from_db(state, id).await?;
    Ok(user)
}

/// Loads every live user among `ids`, in no particular order. Ids of deleted or unknown users
/// are skipped rather than reported, because callers use this to sideload the authors of
/// records that may outlive their accounts.
pub async fn read_users(state: &GlobalServerContext, ids: &[UserId]) -> app::Result<Vec<User>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let users: Vec<UserPg> = user::table
        .select(UserPg::as_select())
        .filter(user::id.eq_any(ids).and(user::deleted_at.is_null()))
        .load(conn.as_mut())
        .await?;
    with_online_status(state, users).await
}

/// Pairs each row with its live online status in one round trip to Valkey.
pub(crate) async fn with_online_status(
    state: &GlobalServerContext,
    users: Vec<UserPg>,
) -> app::Result<Vec<User>> {
    let online_status = users_online_status(state, users.iter().map(|u| u.id).collect()).await?;
    Ok(users
        .into_iter()
        .zip(online_status)
        .map(|(user_pg, (_id, online_status))| User {
            user_pg,
            online_status,
        })
        .collect())
}

pub async fn read_user_communities(
    state: GlobalServerContext,
    user_id: UserId,
) -> Result<Vec<app::community::Community>, app::Error> {
    use crate::database::schema::{community, community_user};

    let mut conn = state.connection_pool.get().await?;
    let communities = community_user::table
        .inner_join(community::table)
        .select(app::community::Community::as_select())
        .filter(
            community_user::user
                .eq(user_id)
                .and(community::deleted_at.is_null()),
        )
        .order_by(community::name.asc())
        .load(conn.as_mut())
        .await?;
    Ok(communities)
}

pub(crate) async fn update_user(
    state: GlobalServerContext,
    requesting_user: UserId,
    id: UserId,
    command: UserUpdateRequest,
) -> Result<User, app::Error> {
    let mut conn = state.connection_pool.get().await?;
    // A bot's profile is its owner's to change too.
    if requesting_user != id && !app::bot::owns(conn.as_mut(), requesting_user, id).await? {
        return Err(app::Error::Unauthorized);
    }
    validate_profile(&command)?;
    // A foreign user's profile is their home's, written from each sign-in; only their status
    // is this deployment's.
    let foreign: bool = user::table
        .select(user::home_domain.is_not_null())
        .filter(user::id.eq(id))
        .first(conn.as_mut())
        .await?;
    if foreign
        && (command.name.is_some()
            || command.icon.is_some()
            || command.display_name.is_some()
            || command.pronouns.is_some()
            || command.bio.is_some())
    {
        return Err(app::Error::Forbidden(t!("foreignProfileAtHome")));
    }
    if let Some(name) = &command.name {
        validate_username(name)?;
    }
    conn.transaction(|conn| {
        async move {
            // A status is stored as its two columns; setting or clearing it writes both.
            let (status_text, status_emoji) = match &command.status {
                None => (None, None),
                Some(None) => (Some(None), Some(None)),
                Some(Some(status)) => (Some(Some(status.text.clone())), Some(status.emoji.clone())),
            };
            let Some(user_pg) = diesel::update(user::table)
                .set(UserChangeset {
                    name: command.name.clone(),
                    icon: command.icon,
                    password_hash: None,
                    display_name: command.display_name.clone(),
                    pronouns: command.pronouns.clone(),
                    bio: command.bio.clone(),
                    status_text,
                    status_emoji,
                })
                .filter(user::id.eq(id).and(user::deleted_at.is_null()))
                .returning(UserPg::as_select())
                .load(conn.as_mut())
                .await?
                .into_iter()
                .next()
            else {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            };
            publish_event(
                &state,
                conn.as_mut(),
                app::EventScope::UserEverywhere(id),
                &message_enum::server_event::ServerEvent::User(UserEvent::Update {
                    id,
                    name: command.name,
                    icon: command.icon,
                    display_name: command.display_name,
                    pronouns: command.pronouns,
                    bio: command.bio,
                    status: command.status,
                    bot_owner: None,
                    bot_public: None,
                }),
            )
            .await?;
            Ok(User {
                user_pg,
                online_status: user_online_status(&state, id).await?,
            })
        }
        .scope_boxed()
    })
    .await
}

/// Deletes the caller's account, which is a change to security settings and so needs a recent
/// verification. Its credentials go with it.
pub(crate) async fn delete_user(
    state: GlobalServerContext,
    caller: &app::two_factor::Caller,
    id: UserId,
) -> Result<(), app::Error> {
    if caller.user != id {
        return Err(app::Error::Unauthorized);
    }
    caller.ensure_recently_verified(&state.config.auth)?;
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| retire(&state, conn.as_mut(), id).scope_boxed())
        .await?;
    app::federation::notices::announce_deleted(&state, id);
    Ok(())
}

/// Deletes an account inside the caller's transaction: marks it deleted, takes its credentials
/// and any bot token, leaves the bots it owned working but ownerless, and says it is gone.
pub(crate) async fn retire(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    id: UserId,
) -> Result<(), app::Error> {
    let deleted = diesel::update(user::table)
        .set(user::deleted_at.eq(diesel::dsl::now))
        .filter(user::id.eq(id).and(user::deleted_at.is_null()))
        .execute(conn)
        .await?;
    if deleted == 0 {
        return Err(app::Error::Diesel(diesel::result::Error::NotFound));
    }
    app::two_factor::remove_all(conn, id).await?;
    app::bot::orphan_bots_of(state, conn, id).await?;
    diesel::delete(bot_token::table.filter(bot_token::bot.eq(id)))
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        app::EventScope::UserEverywhere(id),
        &message_enum::server_event::ServerEvent::User(UserEvent::Delete { id }),
    )
    .await?;
    Ok(())
}

/// Resolves a session token, or a bot's token, to its user and the sign-in it belongs to.
/// `None` means the token is unknown, expired, or belongs to a deleted user.
pub async fn user_for_token(
    state: &GlobalServerContext,
    token: &str,
) -> crate::app::Result<Option<(UserPg, app::two_factor::Caller)>> {
    if app::bot::is_bot_token(token) {
        return app::bot::user_for_token(state, token).await;
    }
    let mut conn = state.connection_pool.get().await?;
    let now = Utc::now().naive_utc();
    let found = schema::user::table
        .inner_join(refresh_token::table.inner_join(session::table))
        .select((
            UserPg::as_select(),
            refresh_token::token,
            refresh_token::verified_at,
            refresh_token::method,
            diesel::dsl::sql::<diesel::sql_types::Bool>(app::two_factor::HAS_SECOND_FACTOR_SQL),
        ))
        .filter(
            session::dsl::token
                .eq(&token)
                .and(session::dsl::expires.ge(now))
                .and(refresh_token::dsl::expires.ge(now))
                .and(schema::user::deleted_at.is_null()),
        )
        .first::<(
            UserPg,
            String,
            chrono::DateTime<Utc>,
            app::login::SignInMethod,
            bool,
        )>(conn.as_mut())
        .await
        .optional()?;
    Ok(found.map(
        |(user, refresh_token, verified_at, method, has_second_factor)| {
            let caller = app::two_factor::Caller {
                user: user.id,
                session_token: token.to_string(),
                refresh_token,
                verified_at,
                has_second_factor,
                // A bot holds a session only abroad, from its home's assertion.
                bot: user.bot,
                method,
                foreign: user.foreign(),
            };
            (user, caller)
        },
    ))
}

pub async fn user_online_status(
    state: &GlobalServerContext,
    user_id: UserId,
) -> crate::app::Result<UserOnlineStatus> {
    Ok(users_online_status(state, vec![user_id])
        .await?
        .pop()
        .map_or(UserOnlineStatus::Offline, |(_, status)| status))
}

/// The presence of each user (`app::user_status`), read in one round trip.
pub async fn users_online_status(
    state: &GlobalServerContext,
    user_ids: Vec<UserId>,
) -> crate::app::Result<Vec<(UserId, UserOnlineStatus)>> {
    // MGET with no keys is a protocol error, so an empty batch is answered locally.
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let keys: Vec<String> = user_ids
        .iter()
        .flat_map(|user_id| {
            [
                app::user_status::online_key(*user_id),
                app::user_status::active_key(*user_id),
            ]
        })
        .collect();
    let values: Vec<Option<i64>> = state.valkey.mget(keys).await?;
    Ok(user_ids
        .into_iter()
        .zip(
            values
                .chunks(2)
                .map(|pair| app::user_status::status(pair[0], pair.get(1).copied().flatten())),
        )
        .collect())
}

/// Records that the user has a connection for `ONLINE_TTL_SECONDS`; the event stream calls this
/// when they connect and while they stay, and so does every authenticated request. Fire and
/// forget: presence is best effort.
pub fn mark_user_online(state: &GlobalServerContext, user: &UserPg) {
    mark_user_online_id(state, user.id, user.bot);
}

/// As `mark_user_online`, for a user known by id. A bot is never away: it uses Aspen through
/// the API rather than as a person does, so being connected is being active, and both of its
/// keys are set together.
pub fn mark_user_online_id(state: &GlobalServerContext, user: UserId, bot: bool) {
    use fred::interfaces::KeysInterface;
    let valkey = state.valkey.clone();
    let mut keys = vec![app::user_status::online_key(user)];
    if bot {
        keys.push(app::user_status::active_key(user));
    }
    tokio::spawn(async move {
        for key in keys {
            if let Err(e) = valkey
                .set::<(), _, i64>(
                    key,
                    1,
                    Some(fred::types::Expiration::EX(ONLINE_TTL_SECONDS)),
                    None,
                    false,
                )
                .await
            {
                tracing::warn!(error = %e, "failed to record the user as online");
            }
        }
    });
}

/// How long a presence key lives; the event stream refreshes it while the user is connected.
pub const ONLINE_TTL_SECONDS: i64 = 60;
