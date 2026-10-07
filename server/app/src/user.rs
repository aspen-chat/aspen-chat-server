use crate::context::GlobalServerContext;
use crate::icon::Icon;
use crate::login::hash_password;
use crate::react::validate_emoji;
use crate::registration_invite;
use crate::t;
use crate::{IconId, Loadable, MaybeLoaded, UserId, publish_event};
use aspen_schema::{self as schema, bot_token, refresh_token, session, user};
use aspen_wire::message_enum;
use aspen_wire::message_enum::request::{UserCreateRequest, UserUpdateRequest};
use aspen_wire::message_enum::server_event::UserEvent;
use aspen_wire::user::UserOnlineStatus;
use chrono::Utc;
use diesel::prelude::*;
use diesel::{BoolExpressionMethods, ExpressionMethods, Queryable, Selectable};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

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
    pub home_domain: Option<crate::federation::Domain>,
    pub home_id: Option<uuid::Uuid>,
    /// For a foreign user, the home's id of the avatar `icon` is this deployment's copy of.
    pub home_icon: Option<uuid::Uuid>,
    /// The hue of the highest deployment role they hold that has one (`app::deployment_role`).
    pub name_hue: Option<i16>,
    /// For a plugin's principal, the plugin's id (`app::plugin::principal`).
    pub plugin: Option<String>,
    /// The email address the profile shows (`app::email::set_public_email`).
    pub public_email: Option<String>,
}

impl UserPg {
    /// Whether this is a user of another deployment.
    pub fn foreign(&self) -> bool {
        self.home_domain.is_some()
    }
}

#[derive(Debug, Clone)]
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

/// Loads the user with their presence answered as `offline`: who may learn it depends on who
/// asks (`app::user_status::statuses_for`), which loading by id does not know. `read_user`
/// gives it as the caller may see it.
impl Loadable for User {
    type Id = UserId;

    async fn load_from_db(state: &GlobalServerContext, id: Self::Id) -> crate::Result<Self> {
        let user_pg = user::table
            .select(UserPg::as_select())
            .filter(user::dsl::id.eq(id).and(user::dsl::deleted_at.is_null()))
            .first(&mut state.connection_pool.get().await?)
            .await?;
        Ok(User {
            user_pg,
            online_status: UserOnlineStatus::Offline,
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
) -> Result<(), crate::Error> {
    if let Some(text) = value
        && (text.trim().is_empty() || text.chars().count() > max)
    {
        return Err(crate::Error::Validation(t!(error, max = max)));
    }
    Ok(())
}

/// Checks the profile fields of an update. Absent fields are unchanged and need no check.
pub fn validate_profile(command: &UserUpdateRequest) -> Result<(), crate::Error> {
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

diesel::define_sql_function! {
    /// PostgreSQL's `lower`, by which `user_name_key` keeps usernames unique.
    fn lower(text: diesel::sql_types::Text) -> diesel::sql_types::Text;
}

/// The one user of this deployment named `username`, in whatever case it is written: the account
/// a sign-in or an operator command names. Its terms match `user_name_key`, so the index answers
/// it. Bots are among them; the system account and deleted users are not.
#[diesel::dsl::auto_type]
pub fn named(username: String) -> _ {
    lower(user::name)
        .eq(lower(username))
        .and(user::home_domain.is_null())
        .and(diesel::dsl::not(user::system))
        .and(user::deleted_at.is_null())
}

/// Rejects usernames that are blank, padded with whitespace, or too long; a bot's name follows
/// the same rules.
pub fn validate_username(name: &str) -> Result<(), crate::Error> {
    if name.is_empty() || name.trim() != name {
        return Err(crate::Error::Validation(t!("usernameBlankOrPadded")));
    }
    if name.chars().count() > USERNAME_MAX_LENGTH {
        return Err(crate::Error::Validation(t!(
            "usernameTooLong",
            max = USERNAME_MAX_LENGTH
        )));
    }
    Ok(())
}

/// Rejects what [`validate_username`] does, and the names no one may take for an account of
/// their own: the system account's (`app::system_account::USERNAME`), in any case, which would
/// pass for the deployment speaking. For registration, renames, and bots; a foreign user's name
/// is their home's.
pub fn validate_new_username(name: &str) -> Result<(), crate::Error> {
    validate_username(name)?;
    if name.to_lowercase() == crate::system_account::USERNAME {
        return Err(crate::Error::Validation(t!(
            "usernameReserved",
            name = name
        )));
    }
    Ok(())
}

/// Rejects a bad username, a password `app::login::check_new_password` refuses, and a profile
/// `update_user` would refuse.
fn validate_registration(command: &UserCreateRequest) -> Result<(), crate::Error> {
    validate_new_username(&command.name)?;
    crate::login::check_new_password(&command.password)
        .map_err(crate::Error::PasswordRequirement)?;
    validate_profile(&UserUpdateRequest {
        display_name: Some(command.display_name.clone()),
        pronouns: Some(command.pronouns.clone()),
        bio: Some(command.bio.clone()),
        status: Some(command.status.clone()),
        ..Default::default()
    })?;
    // A picture must be one the account uploaded itself (`app::icon::require_own`), and an
    // account being made has uploaded none.
    if command.icon.is_some() {
        return Err(crate::Error::Validation(t!("iconMissing")));
    }
    Ok(())
}

pub async fn create_user(
    state: GlobalServerContext,
    command: &UserCreateRequest,
) -> Result<UserId, crate::Error> {
    validate_registration(command)?;
    let email = crate::email::check_registration(&state, command.email.as_deref())?;
    let newsletter = command.newsletter.unwrap_or(false);
    let invite_required = state.settings().registration_invite_required;
    let invite_code = command
        .invite_code
        .as_deref()
        .map(str::trim)
        .filter(|code| !code.is_empty());
    // Refused before the password is hashed, which is the expensive part.
    if invite_required && invite_code.is_none() {
        return Err(crate::Error::RegistrationInviteRequired);
    }
    // The connection is taken once the password is hashed (see `hash_password`).
    let password_hash = hash_password(command.password.to_string()).await?;
    let mut conn = state.connection_pool.get().await?;
    let new_user_id = UserId::new();
    let now = chrono::Utc::now();
    let state = &state;
    let joined = conn
        .transaction(|conn| {
            async move {
                // The invite is used in the transaction that makes the account, so an invite with
                // one use left makes one account however many register with it at once. Where
                // invites are optional, one that no longer works is ignored rather than refused.
                let (registered_with, community_invite) = match invite_code {
                    Some(code) => match registration_invite::redeem(conn.as_mut(), code).await {
                        Ok(community_invite) => (Some(code.to_string()), community_invite),
                        Err(crate::Error::RegistrationInviteInvalid) if !invite_required => {
                            (None, None)
                        }
                        Err(e) => return Err(e),
                    },
                    None => (None, None),
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
                        name_hue: None,
                        bot: false,
                        system: false,
                        bot_owner: None,
                        bot_public: false,
                        home_domain: None,
                        home_id: None,
                        home_icon: None,
                        plugin: None,
                        public_email: None,
                    })
                    .execute(conn.as_mut())
                    .await?;
                if let Some(address) = &email {
                    crate::email::register(state, conn.as_mut(), new_user_id, address, newsletter)
                        .await?;
                }
                if registered_with.is_some() {
                    diesel::update(user::table.filter(user::id.eq(new_user_id)))
                        .set(user::registered_with.eq(registered_with))
                        .execute(conn.as_mut())
                        .await?;
                }
                match community_invite {
                    Some(community_invite) => {
                        registration_invite::join_invited(
                            state,
                            conn.as_mut(),
                            new_user_id,
                            &community_invite,
                        )
                        .await
                    }
                    None => Ok::<_, crate::Error>(None),
                }
            }
            .scope_boxed()
        })
        .await?;
    if let Some(community) = joined {
        crate::everyone_limit::after_join(state, community).await;
    }
    crate::email::wake(state).await;
    Ok(new_user_id)
}

/// A live user, with their presence as `viewer` may learn it.
pub async fn read_user(
    state: &GlobalServerContext,
    viewer: UserId,
    id: UserId,
) -> crate::Result<User> {
    let user = User::load_from_db(state, id).await?;
    Ok(with_online_status(state, viewer, vec![user.user_pg])
        .await?
        .remove(0))
}

/// Loads every live user among `ids`, in no particular order, with their presence as `viewer`
/// may learn it. Ids of deleted or unknown users are skipped rather than reported, because
/// callers use this to sideload the authors of records that may outlive their accounts.
pub async fn read_users(
    state: &GlobalServerContext,
    viewer: UserId,
    ids: &[UserId],
) -> crate::Result<Vec<User>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut conn = state.connection_pool.get().await?;
    let users: Vec<UserPg> = user::table
        .select(UserPg::as_select())
        .filter(user::id.eq_any(ids).and(user::deleted_at.is_null()))
        .load(conn.as_mut())
        .await?;
    drop(conn);
    with_online_status(state, viewer, users).await
}

/// Pairs each row with its online status as `viewer` may learn it
/// (`app::user_status::statuses_for`).
pub async fn with_online_status(
    state: &GlobalServerContext,
    viewer: UserId,
    users: Vec<UserPg>,
) -> crate::Result<Vec<User>> {
    let online_status =
        crate::user_status::statuses_for(state, viewer, users.iter().map(|u| u.id).collect())
            .await?;
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
) -> Result<Vec<crate::community::Community>, crate::Error> {
    use aspen_schema::{community, community_user};

    let mut conn = state.connection_pool.get().await?;
    let communities = community_user::table
        .inner_join(community::table)
        .select(crate::community::Community::as_select())
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

pub async fn update_user(
    state: GlobalServerContext,
    requesting_user: UserId,
    id: UserId,
    command: UserUpdateRequest,
) -> Result<User, crate::Error> {
    let mut conn = state.connection_pool.get().await?;
    // A bot's profile is its owner's to change too.
    if requesting_user != id && !crate::bot::owns(conn.as_mut(), requesting_user, id).await? {
        return Err(crate::Error::Unauthorized);
    }
    validate_profile(&command)?;
    // A foreign user's profile is their home's, written from each sign-in; only their status
    // is this deployment's.
    let (foreign, current_icon): (bool, Option<IconId>) = user::table
        .select((user::home_domain.is_not_null(), user::icon))
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
        return Err(crate::Error::Forbidden(t!("foreignProfileAtHome")));
    }
    if let Some(name) = &command.name {
        validate_new_username(name)?;
    }
    // A new picture must be one the caller uploaded: their own, or for a bot, its owner's.
    if let Some(Some(icon)) = command.icon
        && Some(icon) != current_icon
    {
        crate::icon::require_own(conn.as_mut(), requesting_user, icon, t!("iconMissing")).await?;
    }
    drop(conn);
    let updated = apply_profile_update(state.clone(), id, command).await?;
    Ok(
        with_online_status(&state, requesting_user, vec![updated.user_pg])
            .await?
            .remove(0),
    )
}

/// Writes a checked profile update to `id`'s account and announces it to everyone who shares a
/// community with them: the owner's own update, or a moderator's reset (`app::report`).
pub async fn apply_profile_update(
    state: GlobalServerContext,
    id: UserId,
    command: UserUpdateRequest,
) -> Result<User, crate::Error> {
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
                return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
            };
            publish_event(
                &state,
                conn.as_mut(),
                crate::EventScope::UserEverywhere(id),
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
                    name_hue: None,
                    public_email: None,
                }),
            )
            .await?;
            // Presence is the caller's to fill in, as whoever reads it may learn it.
            Ok(User {
                user_pg,
                online_status: UserOnlineStatus::Offline,
            })
        }
        .scope_boxed()
    })
    .await
}

/// Deletes the caller's account, which is a change to security settings and so needs a recent
/// verification. Its credentials go with it.
pub async fn delete_user(
    state: GlobalServerContext,
    caller: &crate::two_factor::Caller,
    id: UserId,
) -> Result<(), crate::Error> {
    if caller.user != id {
        return Err(crate::Error::Unauthorized);
    }
    caller.ensure_recently_verified(&state.config.auth)?;
    let mut conn = state.connection_pool.get().await?;
    let retired = conn
        .transaction(|conn| retire(&state, conn.as_mut(), id).scope_boxed())
        .await?;
    drop(conn);
    retired.finish(&state).await;
    crate::federation::notices::announce_deleted(&state, id);
    Ok(())
}

/// Deletes an account inside the caller's transaction: marks it deleted, ends its sign-ins and
/// takes its credentials and any bot token, hands each community it owned to the member ranked
/// highest there (`successor`), takes a bot out of its communities, leaves the bots it owned
/// working but ownerless, and says it is gone. What is left to do once the transaction commits
/// is returned, for `Retired::finish`.
pub async fn retire(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    id: UserId,
) -> Result<Retired, crate::Error> {
    let retired: Option<(String, Option<String>, bool)> = diesel::update(user::table)
        .set(user::deleted_at.eq(diesel::dsl::now))
        .filter(user::id.eq(id).and(user::deleted_at.is_null()))
        .returning((user::name, user::display_name, user::bot))
        .get_result(conn)
        .await
        .optional()?;
    let Some((name, display_name, bot)) = retired else {
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    };
    let mut handovers = Vec::new();
    let owned: Vec<(crate::CommunityId, String)> = schema::community::table
        .select((schema::community::id, schema::community::name))
        .filter(
            schema::community::owner
                .eq(Some(id))
                .and(schema::community::deleted_at.is_null()),
        )
        .load(conn)
        .await?;
    for (community, community_name) in owned {
        let next = successor(conn, community, id).await?;
        crate::role::set_owner(state, conn, community, next).await?;
        if let Some(next) = next {
            handovers.push((community_name, next));
        }
    }
    if bot {
        for community in crate::events::memberships(conn, id).await? {
            crate::community::end_membership(state, conn, id, community).await?;
        }
    }
    crate::login::revoke_all_sessions(state, conn, id).await?;
    crate::two_factor::remove_all(conn, id).await?;
    crate::bot::orphan_bots_of(state, conn, id).await?;
    // Their address and the mail waiting for it go with them.
    diesel::delete(schema::email_outbox::table.filter(schema::email_outbox::user.eq(id)))
        .execute(conn)
        .await?;
    diesel::delete(schema::user_email::table.filter(schema::user_email::user.eq(id)))
        .execute(conn)
        .await?;
    diesel::update(user::table.filter(user::id.eq(id)))
        .set(user::public_email.eq(None::<String>))
        .execute(conn)
        .await?;
    // What plugins kept about them goes with them.
    crate::plugin::storage::forget(conn, crate::plugin::storage::Scope::User(id)).await?;
    diesel::delete(bot_token::table.filter(bot_token::bot.eq(id)))
        .execute(conn)
        .await?;
    publish_event(
        state,
        conn,
        crate::EventScope::UserEverywhere(id),
        &message_enum::server_event::ServerEvent::User(UserEvent::Delete { id }),
    )
    .await?;
    Ok(Retired {
        name: display_name.unwrap_or(name),
        handovers,
    })
}

/// Who a community goes to when its owner's account is deleted: the member whose highest role
/// ranks highest, the earliest to join among equals, leaving out bots, the system account, and
/// anyone deleted or banned from the deployment. `None` when nobody is left.
async fn successor(
    conn: &mut AsyncPgConnection,
    community: crate::CommunityId,
    leaving: UserId,
) -> crate::Result<Option<UserId>> {
    #[derive(QueryableByName)]
    struct Row {
        #[diesel(sql_type = diesel::sql_types::Uuid)]
        user: UserId,
    }
    let query = format!(
        r#"
        SELECT cu."user" AS "user"
        FROM community_user cu
        JOIN "user" ON "user".id = cu."user"
        LEFT JOIN community_member_role cmr
            ON cmr.community = cu.community AND cmr."user" = cu."user"
        LEFT JOIN community_role r ON r.id = cmr.role
        WHERE cu.community = $1
          AND cu."user" <> $2
          AND "user".deleted_at IS NULL
          AND NOT "user".bot
          AND NOT "user".system
          AND NOT {banned}
        GROUP BY cu."user", cu.joined_at
        ORDER BY COALESCE(MAX(r.position), 0) DESC, cu.joined_at, cu."user"
        LIMIT 1
        "#,
        banned = crate::user_ban::BANNED_SQL,
    );
    let row: Option<Row> = diesel::sql_query(query)
        .bind::<diesel::sql_types::Uuid, _>(community.0)
        .bind::<diesel::sql_types::Uuid, _>(leaving.0)
        .get_result(conn)
        .await
        .optional()?;
    Ok(row.map(|row| row.user))
}

/// What retiring an account leaves to do once its transaction has committed.
#[must_use = "a retired account's new owners wait on `finish`"]
pub struct Retired {
    /// The name the account went by, for the notices.
    name: String,
    /// Each community handed on, by name, and who it went to.
    handovers: Vec<(String, UserId)>,
}

impl Retired {
    /// Tells each new owner by the system account what they now own. The account is gone
    /// either way, so failures are logged. Its calls end as its deletion settles
    /// (`app::events::rechecks_of`).
    pub async fn finish(self, state: &GlobalServerContext) {
        for (community, owner) in self.handovers {
            let notice = crate::t!(
                "ownershipInheritedNotice",
                previous = self.name.as_str(),
                community = community.as_str()
            );
            if let Err(e) = crate::system_account::notify(state, owner, notice.to_string()).await {
                tracing::warn!(owner = %owner.0, error = %e, "could not tell a new owner of their community");
            }
        }
    }
}

/// When the caller's sign-in began and when it ends by itself.
pub struct SignInTimes {
    /// For closing what it holds open then (an event stream). A bot's token, which has no
    /// refresh token, never expires: `None`. A sign-in that is gone already ends now.
    pub expires: Option<chrono::DateTime<Utc>>,
    /// `refresh_token.created_at`, or for a bot the making of its current token; `None` when
    /// gone.
    pub began: Option<chrono::DateTime<Utc>>,
}

/// When `caller`'s sign-in began and when it ends by itself.
pub async fn sign_in_times(
    state: &GlobalServerContext,
    caller: &crate::two_factor::Caller,
) -> crate::Result<SignInTimes> {
    let mut conn = state.connection_pool.get().await?;
    if caller.refresh_digest.is_empty() {
        let began = schema::bot_token::table
            .select(schema::bot_token::created_at)
            .filter(schema::bot_token::bot.eq(caller.user))
            .first(conn.as_mut())
            .await
            .optional()?;
        return Ok(SignInTimes {
            expires: None,
            began,
        });
    }
    let found: Option<(chrono::NaiveDateTime, chrono::DateTime<Utc>)> = refresh_token::table
        .select((refresh_token::expires, refresh_token::created_at))
        .filter(refresh_token::token.eq(&caller.refresh_digest))
        .first(conn.as_mut())
        .await
        .optional()?;
    Ok(SignInTimes {
        expires: Some(found.map_or_else(Utc::now, |(expires, _)| expires.and_utc())),
        began: found.map(|(_, began)| began),
    })
}

/// Resolves a session token, or a bot's token, to its user and the sign-in it belongs to.
/// `None` means the token is unknown, expired, or belongs to a deleted or banned user.
pub async fn user_for_token(
    state: &GlobalServerContext,
    token: &str,
) -> crate::Result<Option<(UserPg, crate::two_factor::Caller)>> {
    if crate::bot::is_bot_token(token) {
        return crate::bot::user_for_token(state, token).await;
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
            diesel::dsl::sql::<diesel::sql_types::Bool>(crate::two_factor::HAS_SECOND_FACTOR_SQL),
            diesel::dsl::sql::<diesel::sql_types::Bool>(crate::two_factor::EMAIL_UNVERIFIED_SQL),
        ))
        .filter(
            session::dsl::token
                .eq(crate::login::token_digest(token))
                .and(session::dsl::expires.ge(now))
                .and(refresh_token::dsl::expires.ge(now))
                .and(schema::user::deleted_at.is_null())
                .and(diesel::dsl::not(crate::user_ban::banned())),
        )
        .first::<(
            UserPg,
            String,
            chrono::DateTime<Utc>,
            crate::login::SignInMethod,
            bool,
            bool,
        )>(conn.as_mut())
        .await
        .optional()?;
    Ok(found.map(
        |(user, refresh_digest, verified_at, method, has_second_factor, email_unverified)| {
            let caller = crate::two_factor::Caller {
                user: user.id,
                session_digest: crate::login::token_digest(token),
                refresh_digest,
                verified_at,
                has_second_factor,
                // A bot holds a session only abroad, from its home's assertion.
                bot: user.bot,
                method,
                foreign: user.foreign(),
                email_unverified,
            };
            (user, caller)
        },
    ))
}

impl From<User> for aspen_wire::message_enum::User {
    fn from(user: User) -> Self {
        aspen_wire::message_enum::User {
            id: user.user_pg.id,
            name: user.user_pg.name,
            icon: user.user_pg.icon.map(|i| *i.id()),
            online_status: user.online_status,
            display_name: user.user_pg.display_name,
            pronouns: user.user_pg.pronouns,
            bio: user.user_pg.bio,
            status: user
                .user_pg
                .status_text
                .map(|text| aspen_wire::user::CustomStatus {
                    text,
                    emoji: user.user_pg.status_emoji,
                }),
            bot: user.user_pg.bot,
            system: user.user_pg.system,
            bot_owner: user.user_pg.bot_owner,
            bot_public: user.user_pg.bot_public,
            home_domain: user.user_pg.home_domain.map(String::from),
            home_id: user.user_pg.home_id,
            name_hue: user.user_pg.name_hue,
            plugin: user.user_pg.plugin,
            public_email: user.user_pg.public_email,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_accounts_name_is_reserved_in_any_case() {
        for name in ["system", "System", "SYSTEM"] {
            assert!(matches!(
                validate_new_username(name),
                Err(crate::Error::Validation(_))
            ));
        }
        assert!(validate_new_username("systems").is_ok());
        assert!(validate_username("system").is_ok());
    }
}
