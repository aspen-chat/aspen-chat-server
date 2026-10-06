//! Signing in abroad. A user's home deployment signs a short-lived assertion naming them, how
//! they signed in, and their profile, for one other deployment ([`issue`]); that deployment
//! verifies it against the home's pinned key and signs them in as a foreign user, a local row
//! naming their home ([`sign_in`]).
//!
//! The home checks its emigration gate before signing, and the host its immigration gate
//! before contacting or believing anything. A host that requires two factors admits only
//! sign-ins that proved more than a password at home. Each assertion is used once. The profile
//! is the home's: it is written from each assertion, read-only here, and the avatar is copied
//! into this deployment's storage, fetched from the home itself ([`HOME_ICON_PATH`]).

use crate::api::message_enum::request::UserUpdateRequest;
use crate::api::message_enum::server_event::{ServerEvent, UserEvent};
use crate::app::context::GlobalServerContext;
use crate::app::federation::keys::signing_key;
use crate::app::federation::received::{Received, Senders, Statement, invalid, receive, refused};
use crate::app::federation::{Direction, Domain, Subject, admits, jws, lists_of, own_domain};
use crate::app::login::{Session, SignInMethod, issue_session};
use crate::app::two_factor::Caller;
use crate::app::user::UserPg;
use crate::app::{self, CommunityId, EventScope, IconId, UserId, publish_event};
use crate::database::schema::{icon, user, user_foreign_deployment};
use crate::t;
use chrono::{DateTime, Duration, Utc};
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use futures_util::StreamExt as _;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The `typ` of an assertion.
const ASSERTION_TYPE: &str = "aspen-assertion+jwt";
/// How long an assertion this deployment signs lasts: long enough to hand to the other
/// deployment, and short enough that one intercepted is soon useless.
const ASSERTION_LIFETIME: Duration = Duration::minutes(2);
/// Where a home serves its users' avatars to the deployments they sign in to, under the API.
pub const HOME_ICON_PATH: &str = "/federation/icons";
/// The largest avatar copied from a home.
const MAX_AVATAR_BYTES: u64 = app::icon::MAX_BYTES;

/// What a home says of one of its users to one other deployment.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Assertion {
    /// The home.
    iss: Domain,
    /// The one deployment it is for.
    aud: Domain,
    /// The user's id at home.
    sub: Uuid,
    iat: i64,
    exp: i64,
    /// Its id, which the host remembers until it expires so it is used once.
    jti: Uuid,
    method: SignInMethod,
    profile: Profile,
}

impl Statement for Assertion {
    const TYPE: &'static str = ASSERTION_TYPE;
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

/// A user's profile as their home keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    name: String,
    display_name: Option<String>,
    pronouns: Option<String>,
    bio: Option<String>,
    /// The home's id of their avatar, served at [`HOME_ICON_PATH`].
    icon: Option<Uuid>,
    bot: bool,
    /// The email address they show on their profile, if they show one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    public_email: Option<String>,
}

/// An assertion this deployment signed, for its user to hand to `audience`.
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct Issued {
    pub assertion: String,
    #[schema(value_type = String)]
    pub audience: Domain,
    pub expires_at: DateTime<Utc>,
}

/// Signs an assertion for `caller`, who is `user`, to sign in at `audience`, and remembers that
/// they use it.
pub async fn issue(
    state: &GlobalServerContext,
    caller: &Caller,
    user: &UserPg,
    audience: &Domain,
) -> app::Result<Issued> {
    let config = &state.config.federation;
    let home = own_domain(config).ok_or(app::Error::FederationRefused(t!("federationOff")))?;
    if user.foreign() {
        return Err(app::Error::FederationRefused(t!("federationNotYourHome")));
    }
    if *audience == home {
        return Err(app::Error::Validation(t!("federationOwnDomain")));
    }
    let mut conn = state.connection_pool.get().await?;
    let lists = lists_of(&mut conn, std::slice::from_ref(audience))
        .await?
        .remove(audience)
        .unwrap_or_default();
    let subject = if user.bot {
        Subject::Bots
    } else {
        Subject::Users
    };
    if !admits(
        &state.settings().federation,
        subject,
        Direction::Emigration,
        &lists,
    ) {
        return Err(app::Error::FederationRefused(t!(
            "federationEmigrationClosed",
            domain = audience.as_str()
        )));
    }
    let key = signing_key(&mut conn)
        .await?
        .ok_or(app::Error::FederationRefused(t!("federationOff")))?;
    let now = Utc::now();
    let expires_at = now + ASSERTION_LIFETIME;
    let assertion = jws::sign(
        ASSERTION_TYPE,
        key.id,
        &key.pair,
        &Assertion {
            iss: home,
            aud: audience.clone(),
            sub: user.id.0,
            iat: now.timestamp(),
            exp: expires_at.timestamp(),
            jti: Uuid::new_v4(),
            method: caller.method,
            profile: Profile {
                name: user.name.clone(),
                display_name: user.display_name.clone(),
                pronouns: user.pronouns.clone(),
                bio: user.bio.clone(),
                icon: user.icon.as_ref().map(|icon| icon.id().0),
                bot: user.bot,
                public_email: user.public_email.clone(),
            },
        },
    );
    diesel::insert_into(user_foreign_deployment::table)
        .values((
            user_foreign_deployment::user.eq(user.id),
            user_foreign_deployment::domain.eq(audience.as_str()),
        ))
        .on_conflict((
            user_foreign_deployment::user,
            user_foreign_deployment::domain,
        ))
        .do_update()
        .set(user_foreign_deployment::last_used_at.eq(diesel::dsl::now))
        .execute(&mut conn)
        .await?;
    Ok(Issued {
        assertion,
        audience: audience.clone(),
        expires_at,
    })
}

/// Another deployment this deployment's user has signed in to.
#[derive(Debug, Serialize, ToSchema, Queryable)]
#[serde(rename_all = "camelCase")]
pub struct ForeignDeployment {
    pub domain: String,
    pub first_used_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
}

/// The other deployments `user` has signed in to, the most recently used first.
pub async fn foreign_deployments(
    state: &GlobalServerContext,
    user_id: UserId,
) -> app::Result<Vec<ForeignDeployment>> {
    Ok(user_foreign_deployment::table
        .select((
            user_foreign_deployment::domain,
            user_foreign_deployment::first_used_at,
            user_foreign_deployment::last_used_at,
        ))
        .filter(user_foreign_deployment::user.eq(user_id))
        .order(user_foreign_deployment::last_used_at.desc())
        .load(&mut state.connection_pool.get().await?)
        .await?)
}

/// Forgets that `user_id` uses `domain`, so their other devices stop signing in there. Their
/// account there is untouched. Forgetting one they never used is not an error.
pub async fn forget_foreign_deployment(
    state: &GlobalServerContext,
    user_id: UserId,
    domain: &Domain,
) -> app::Result<()> {
    diesel::delete(
        user_foreign_deployment::table
            .filter(user_foreign_deployment::user.eq(user_id))
            .filter(user_foreign_deployment::domain.eq(domain.as_str())),
    )
    .execute(&mut state.connection_pool.get().await?)
    .await?;
    Ok(())
}

/// Signs in the foreign user `token` asserts, making their local row the first time.
/// `invite_code` is a registration invite, which a first arrival needs when
/// `immigration_invite_required` says so.
pub async fn sign_in(
    state: &GlobalServerContext,
    token: &str,
    invite_code: Option<&str>,
) -> app::Result<Session> {
    let settings = state.settings();
    let policy = &settings.federation;
    let Received {
        claims,
        from: home,
        lists,
        ..
    } = receive::<Assertion>(state, token, Senders::Admitted(&[Direction::Immigration])).await?;
    let subject = if claims.profile.bot {
        Subject::Bots
    } else {
        Subject::Users
    };
    if !admits(policy, subject, Direction::Immigration, &lists) {
        return Err(refused(&home, Direction::Immigration));
    }
    // A bot has no second factor anywhere; a person must have proved more than a password.
    let proved = if claims.profile.bot {
        claims.method == SignInMethod::Token
    } else {
        claims.method != SignInMethod::Token
    };
    if !proved {
        return Err(invalid(
            Some(&home),
            t!("statementInconsistent", domain = home.as_str()),
        ));
    }
    if settings.require_two_factor && !claims.profile.bot && !claims.method.strong() {
        return Err(app::Error::StrongerSignInRequired);
    }
    check_profile(&claims.profile).map_err(|error| {
        let detail = match error {
            app::Error::Validation(reason) => reason,
            other => other.to_string().into(),
        };
        invalid(
            Some(&home),
            t!("statementProfile", domain = home.as_str(), detail = detail),
        )
    })?;
    let rules = policy.rules(subject);
    let mut conn = state.connection_pool.get().await?;
    let (user, previous_icon, joined) = arrive(
        state,
        &mut conn,
        &home,
        &claims,
        rules.immigration_invite_required,
        invite_code,
    )
    .await?;
    if let Some(community) = joined {
        app::everyone_limit::after_join(state, community).await;
    }
    let session = issue_session(state, &mut conn, user, claims.method, true).await?;
    if claims.profile.icon != previous_icon {
        let state = state.clone();
        let icon = claims.profile.icon;
        tokio::spawn(async move {
            if let Err(error) = copy_avatar(&state, user, &home, icon).await {
                tracing::warn!(%home, %error, "could not copy a foreign user's avatar");
            }
        });
    }
    Ok(session)
}

/// The same checks a profile edited here passes.
fn check_profile(profile: &Profile) -> app::Result<()> {
    app::user::validate_username(&profile.name)?;
    app::user::validate_profile(&UserUpdateRequest {
        display_name: Some(profile.display_name.clone()),
        pronouns: Some(profile.pronouns.clone()),
        bio: Some(profile.bio.clone()),
        ..UserUpdateRequest::default()
    })?;
    if let Some(address) = &profile.public_email {
        app::email::parse_address(address)?;
    }
    Ok(())
}

/// The foreign user `claims` names, made on their first arrival and otherwise brought up to
/// date with their profile, the home avatar their local one copies, and the community a first
/// arrival with a dual invite joined.
async fn arrive(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    home: &Domain,
    claims: &Assertion,
    invite_required: bool,
    invite_code: Option<&str>,
) -> app::Result<(UserId, Option<Uuid>, Option<CommunityId>)> {
    let invite_code = invite_code.map(str::trim).filter(|code| !code.is_empty());
    conn.transaction(|conn| {
        async move {
            // Two first sign-ins of one user at once make one row: the second waits here and
            // then finds it.
            diesel::sql_query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind::<diesel::sql_types::Text, _>(format!(
                    "federation:user:{home}:{}",
                    claims.sub
                ))
                .execute(conn)
                .await?;
            let existing: Option<UserPg> = user::table
                .select(UserPg::as_select())
                .filter(user::home_domain.eq(home))
                .filter(user::home_id.eq(claims.sub))
                .for_update()
                .first(conn)
                .await
                .optional()?;
            let profile = &claims.profile;
            let Some(existing) = existing else {
                if invite_required && invite_code.is_none() {
                    return Err(app::Error::RegistrationInviteRequired);
                }
                // As at registration, an invite that works is recorded, and where invites are
                // optional one that does not is ignored.
                let (registered_with, community_invite) = match invite_code {
                    Some(code) => match app::registration_invite::redeem(conn, code).await {
                        Ok(community_invite) => (Some(code.to_string()), community_invite),
                        Err(app::Error::RegistrationInviteInvalid) if !invite_required => {
                            (None, None)
                        }
                        Err(e) => return Err(e),
                    },
                    None => (None, None),
                };
                let id = UserId::new();
                let now = Utc::now();
                diesel::insert_into(user::table)
                    .values(UserPg {
                        id,
                        name: profile.name.clone(),
                        icon: None,
                        // No password verifies against an empty hash, and sign-in by password
                        // looks only at this deployment's own users.
                        password_hash: String::new(),
                        created_at: now,
                        last_seen_at: now,
                        deleted_at: None,
                        display_name: profile.display_name.clone(),
                        pronouns: profile.pronouns.clone(),
                        bio: profile.bio.clone(),
                        status_text: None,
                        status_emoji: None,
                        bot: profile.bot,
                        system: false,
                        bot_owner: None,
                        bot_public: false,
                        name_hue: None,
                        home_domain: Some(home.clone()),
                        home_id: Some(claims.sub),
                        home_icon: None,
                        plugin: None,
                        public_email: profile.public_email.clone(),
                    })
                    .execute(conn)
                    .await?;
                diesel::update(user::table.find(id))
                    .set((
                        user::registered_with.eq(registered_with),
                        user::home_confirmed_at.eq(diesel::dsl::now),
                    ))
                    .execute(conn)
                    .await?;
                let joined = match community_invite {
                    Some(community_invite) => {
                        app::registration_invite::join_invited(state, conn, id, &community_invite)
                            .await?
                    }
                    None => None,
                };
                tracing::info!(%home, user = %id.0, "a foreign user arrived");
                return Ok((id, None, joined));
            };
            if existing.deleted_at.is_some() {
                return Err(app::Error::FederationRefused(t!("federationAccountClosed")));
            }
            let banned: bool = user::table
                .select(user::banned_at.is_not_null())
                .find(existing.id)
                .first(conn)
                .await?;
            if banned {
                return Err(app::Error::FederationRefused(t!("federationBanned")));
            }
            diesel::update(user::table.find(existing.id))
                .set(user::home_confirmed_at.eq(diesel::dsl::now))
                .execute(conn)
                .await?;
            let changed =
                |now: &Option<String>, then: &Option<String>| (now != then).then(|| now.clone());
            let event = UserEvent::Update {
                id: existing.id,
                name: (profile.name != existing.name).then(|| profile.name.clone()),
                icon: None,
                display_name: changed(&profile.display_name, &existing.display_name),
                pronouns: changed(&profile.pronouns, &existing.pronouns),
                bio: changed(&profile.bio, &existing.bio),
                status: None,
                bot_owner: None,
                bot_public: None,
                name_hue: None,
                public_email: (profile.public_email != existing.public_email)
                    .then(|| profile.public_email.clone()),
            };
            if let UserEvent::Update {
                name: None,
                display_name: None,
                pronouns: None,
                bio: None,
                public_email: None,
                ..
            } = event
            {
                return Ok((existing.id, existing.home_icon, None));
            }
            diesel::update(user::table.find(existing.id))
                .set((
                    user::name.eq(&profile.name),
                    user::display_name.eq(&profile.display_name),
                    user::pronouns.eq(&profile.pronouns),
                    user::bio.eq(&profile.bio),
                    user::public_email.eq(&profile.public_email),
                ))
                .execute(conn)
                .await?;
            publish_event(
                state,
                conn,
                EventScope::UserEverywhere(existing.id),
                &ServerEvent::User(event),
            )
            .await?;
            Ok((existing.id, existing.home_icon, None))
        }
        .scope_boxed()
    })
    .await
}

/// Makes `user`'s avatar a copy of `home`'s icon `home_icon`, or takes it away with `None`.
async fn copy_avatar(
    state: &GlobalServerContext,
    user_id: UserId,
    home: &Domain,
    home_icon: Option<Uuid>,
) -> app::Result<()> {
    let copied = match home_icon {
        Some(home_icon) => Some(fetch_avatar(state, home, home_icon).await?),
        None => None,
    };
    let local = match copied {
        Some((bytes, mime_type)) => {
            let id = IconId::new();
            let key = app::icon::storage_key(id);
            state.media_store.put_bytes(&key, bytes, &mime_type).await?;
            Some(app::icon::Icon {
                id,
                mime_type,
                timestamp: Utc::now(),
                storage_key: key,
                ready_at: Some(Utc::now()),
            })
        }
        None => None,
    };
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            if let Some(local) = &local {
                diesel::insert_into(icon::table)
                    .values(local)
                    .execute(conn)
                    .await?;
            }
            let icon_id = local.as_ref().map(|local| local.id);
            diesel::update(user::table.find(user_id))
                .set((user::icon.eq(icon_id), user::home_icon.eq(home_icon)))
                .execute(conn)
                .await?;
            publish_event(
                state,
                conn,
                EventScope::UserEverywhere(user_id),
                &ServerEvent::User(UserEvent::Update {
                    id: user_id,
                    name: None,
                    icon: Some(icon_id),
                    display_name: None,
                    pronouns: None,
                    bio: None,
                    status: None,
                    bot_owner: None,
                    bot_public: None,
                    name_hue: None,
                    public_email: None,
                }),
            )
            .await
        }
        .scope_boxed()
    })
    .await
}

/// The avatar `icon` of one of this deployment's own users, with its type, for a deployment
/// they sign in to: nothing else is served this way, and only a picture of one of
/// [`app::icon::IMAGE_TYPES`].
pub async fn home_avatar(
    state: &GlobalServerContext,
    icon: IconId,
) -> app::Result<(Vec<u8>, String)> {
    let mut conn = state.connection_pool.get().await?;
    let row: app::icon::Icon = icon::table
        .select(app::icon::Icon::as_select())
        .filter(icon::id.eq(icon))
        .filter(icon::ready_at.is_not_null())
        .filter(icon::icon_mime_type.eq_any(app::icon::IMAGE_TYPES))
        .filter(diesel::dsl::exists(
            user::table.filter(
                user::icon
                    .eq(icon)
                    .and(user::home_domain.is_null())
                    .and(user::deleted_at.is_null()),
            ),
        ))
        .first(&mut conn)
        .await?;
    drop(conn);
    let bytes = state
        .media_store
        .get_bytes(&row.storage_key, app::icon::MAX_BYTES)
        .await?
        .ok_or(app::Error::Diesel(diesel::result::Error::NotFound))?;
    Ok((bytes, row.mime_type))
}

/// An avatar's bytes and type, as its home serves them; only a picture of one of
/// [`app::icon::IMAGE_TYPES`] is taken.
async fn fetch_avatar(
    state: &GlobalServerContext,
    home: &Domain,
    icon: Uuid,
) -> app::Result<(Vec<u8>, String)> {
    let unreachable =
        || app::Error::DeploymentUnreachable(t!("federationNoAnswer", domain = home.as_str()));
    let response = state
        .federation_client
        .get(format!(
            "https://{home}{}{HOME_ICON_PATH}/{icon}",
            crate::api::API_PREFIX
        ))
        .send()
        .await
        .map_err(|error| super::fetch::failure(home, &error))?;
    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mime_type = mime_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_string();
    if !response.status().is_success() || !app::icon::IMAGE_TYPES.contains(&mime_type.as_str()) {
        return Err(unreachable());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| unreachable())?;
        if (bytes.len() + chunk.len()) as u64 > MAX_AVATAR_BYTES {
            return Err(unreachable());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, mime_type))
}
