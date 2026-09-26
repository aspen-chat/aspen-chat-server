use crate::api::error::PasswordRequirement;
use crate::api::message_enum::request::{UserCreateRequest, UserUpdateRequest};
use crate::api::message_enum::server_event::UserEvent;
use crate::api::user::UserOnlineStatus;
use crate::api::{GlobalServerContext, message_enum};
use crate::app;
use crate::app::icon::Icon;
use crate::app::login::{PASSWORD_MIN_LENGTH, hash_password};
use crate::app::react::validate_emoji;
use crate::app::{IconId, Loadable, MaybeLoaded, UserId, publish_event};
use crate::database::schema::{self, refresh_token, session, user};
use chrono::Utc;
use diesel::prelude::*;
use diesel::{BoolExpressionMethods, ExpressionMethods, Queryable, Selectable};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, RunQueryDsl};
use fred::prelude::KeysInterface;
use rust_i18n::t;

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
fn validate_profile(command: &UserUpdateRequest) -> Result<(), app::Error> {
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

/// Rejects usernames that are blank, padded with whitespace, or too long, and passwords that
/// fail the same length rule `try_change_password` enforces.
fn validate_registration(command: &UserCreateRequest) -> Result<(), app::Error> {
    if command.name.is_empty() || command.name.trim() != command.name {
        return Err(app::Error::Validation(t!("usernameBlankOrPadded")));
    }
    if command.name.chars().count() > USERNAME_MAX_LENGTH {
        return Err(app::Error::Validation(t!(
            "usernameTooLong",
            max = USERNAME_MAX_LENGTH
        )));
    }
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
    let password_hash = hash_password(command.password.to_string()).await?;
    let new_user_id = UserId::new();
    let now = chrono::Utc::now();
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
        })
        .execute(conn.as_mut())
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
    if requesting_user != id {
        return Err(app::Error::Unauthorized);
    }
    validate_profile(&command)?;
    let mut conn = state.connection_pool.get().await?;
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
                &message_enum::server_event::ServerEvent::User(UserEvent::Update {
                    id,
                    name: command.name,
                    icon: command.icon,
                    display_name: command.display_name,
                    pronouns: command.pronouns,
                    bio: command.bio,
                    status: command.status,
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

pub(crate) async fn delete_user(
    state: GlobalServerContext,
    requesting_user: UserId,
    id: UserId,
) -> Result<(), app::Error> {
    if requesting_user != id {
        return Err(app::Error::Unauthorized);
    }
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let deleted = diesel::update(user::table)
                .set(user::deleted_at.eq(diesel::dsl::now))
                .filter(user::id.eq(id).and(user::deleted_at.is_null()))
                .execute(conn.as_mut())
                .await?;
            if deleted == 0 {
                return Err(app::Error::Diesel(diesel::result::Error::NotFound));
            }
            publish_event(
                &state,
                &message_enum::server_event::ServerEvent::User(UserEvent::Delete { id }),
            )
            .await?;
            Ok(())
        }
        .scope_boxed()
    })
    .await
}

/// Resolves a session token to its user. `None` means the token is unknown, expired, or belongs
/// to a deleted user.
pub async fn user_for_token(
    state: &GlobalServerContext,
    token: &str,
) -> crate::app::Result<Option<UserPg>> {
    let mut conn = state.connection_pool.get().await?;
    let now = Utc::now().naive_utc();
    let user = schema::user::table
        .select(UserPg::as_select())
        .inner_join(refresh_token::table.inner_join(session::table))
        .filter(
            session::dsl::token
                .eq(&token)
                .and(session::dsl::expires.ge(now))
                .and(refresh_token::dsl::expires.ge(now))
                .and(schema::user::deleted_at.is_null()),
        )
        .first(conn.as_mut())
        .await
        .optional()?;
    Ok(user)
}

pub async fn user_online_status(
    state: &GlobalServerContext,
    user_id: UserId,
) -> crate::app::Result<UserOnlineStatus> {
    let online_value: Option<i64> = state
        .valkey
        .get(app::user_status::online_key(user_id))
        .await?;
    Ok(match online_value {
        Some(1) => UserOnlineStatus::Online,
        _ => UserOnlineStatus::Offline,
    })
}

pub async fn users_online_status(
    state: &GlobalServerContext,
    user_ids: Vec<UserId>,
) -> crate::app::Result<Vec<(UserId, UserOnlineStatus)>> {
    // MGET with no keys is a protocol error, so an empty batch is answered locally.
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }
    let online_values: Vec<Option<i64>> = state
        .valkey
        .mget(
            user_ids
                .iter()
                .map(|user_id| app::user_status::online_key(*user_id))
                .collect::<Vec<_>>(),
        )
        .await?;
    Ok(user_ids
        .into_iter()
        .zip(
            online_values
                .into_iter()
                .map(|online_value| match online_value {
                    Some(1) => UserOnlineStatus::Online,
                    _ => UserOnlineStatus::Offline,
                }),
        )
        .collect())
}

pub fn mark_user_online(state: &GlobalServerContext, user: &UserPg) {
    let valkey = state.valkey.clone();
    let nats = state.nats_context.clone();
    let user_id = user.id;
    tokio::spawn(async move {
        use fred::prelude::KeysInterface;
        let key = app::user_status::online_key(user_id);
        // SET NX: only succeeds if the key doesn't exist (user was offline)
        let became_online: bool = match valkey
            .set::<fred::types::Value, _, i64>(
                &key,
                1,
                Some(fred::types::Expiration::EX(60)),
                Some(fred::types::SetOptions::NX),
                false,
            )
            .await
        {
            Ok(v) => !v.is_null(),
            Err(e) => {
                tracing::warn!(error = %e, "failed to update user online status in Valkey");
                return;
            }
        };
        if became_online {
            // User just came online — publish event
            if let Err(e) = crate::app::user_status::publish_online(&nats, user_id).await {
                tracing::warn!(error = %e, "failed to publish user online event");
            }
        } else {
            // Already online — just refresh the TTL
            if let Err(e) = valkey
                .set::<(), _, _>(
                    &key,
                    "1",
                    Some(fred::types::Expiration::EX(60)),
                    None,
                    false,
                )
                .await
            {
                tracing::warn!(error = %e, "failed to refresh user online TTL in Valkey");
            }
        }
    });
}
