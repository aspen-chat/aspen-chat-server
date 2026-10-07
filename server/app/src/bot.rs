//! Bots: users that sign in only with a token, made by a person, who owns and manages them.
//!
//! A bot is an ordinary row of `user` with `bot` set, so everything a person may do through
//! the API a bot may do too, under the same permissions and limits. It has no password, no
//! second factor, and no sessions: its one token (`bot_token`, kept only as a SHA-256 digest)
//! is sent as the bearer token of every request and of the event stream, and stands until its
//! owner issues another. Its owner (`user.bot_owner`) renames it, issues its token, hands it on,
//! and deletes it. An owner who deletes their account leaves their bots working and ownerless,
//! and a holder of Manage deployment settings may delete those.

use crate::context::GlobalServerContext;
use crate::deployment::DeploymentPermission;
use crate::permissions::{Permissions, require_member};
use crate::t;
use crate::user::{User, UserPg, validate_new_username, validate_profile, with_online_status};
use crate::{CommunityId, EventScope, UserId, publish_event};
use aspen_schema::{bot_token, community_user, user};
use aspen_wire::message_enum;
use aspen_wire::message_enum::server_event::{ServerEvent, UserEvent};
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use chrono::Utc;
use diesel::prelude::*;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use rand::RngExt;
use sha2::{Digest, Sha256};

/// How every bot token begins, so the server can tell one from a session token without a
/// lookup, and a token found in a log or a repository is recognisable for what it is.
pub const TOKEN_PREFIX: &str = "aspenbot_";

/// Whether `token` is shaped like a bot's.
pub fn is_bot_token(token: &str) -> bool {
    token.starts_with(TOKEN_PREFIX)
}

fn new_token() -> String {
    let bytes = crate::CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 32]>());
    format!("{TOKEN_PREFIX}{}", BASE64_URL_SAFE_NO_PAD.encode(bytes))
}

fn digest(token: &str) -> Vec<u8> {
    Sha256::digest(token.as_bytes()).to_vec()
}

/// The bot a token signs in, with a caller that has no session behind it: nothing to sign out
/// of, and no verification to give. `None` for a token no live bot holds, and while the bot is
/// banned from the deployment, which keeps its tokens for when the ban is lifted.
pub async fn user_for_token(
    state: &GlobalServerContext,
    token: &str,
) -> crate::Result<Option<(UserPg, crate::two_factor::Caller)>> {
    let mut conn = state.connection_pool.get().await?;
    let found: Option<UserPg> = user::table
        .inner_join(bot_token::table.on(bot_token::bot.eq(user::id)))
        .select(UserPg::as_select())
        .filter(
            bot_token::digest
                .eq(digest(token))
                .and(user::bot)
                .and(user::deleted_at.is_null())
                .and(diesel::dsl::not(crate::user_ban::shut_out())),
        )
        .first(conn.as_mut())
        .await
        .optional()?;
    Ok(found.map(|bot| {
        let caller = crate::two_factor::Caller {
            user: bot.id,
            session_digest: String::new(),
            refresh_digest: String::new(),
            verified_at: chrono::DateTime::<Utc>::MIN_UTC,
            has_second_factor: false,
            bot: true,
            method: crate::login::SignInMethod::Token,
            foreign: false,
            email_unverified: false,
        };
        (bot, caller)
    }))
}

/// Whether `owner` owns the live bot `bot`.
pub async fn owns(conn: &mut AsyncPgConnection, owner: UserId, bot: UserId) -> crate::Result<bool> {
    Ok(diesel::select(diesel::dsl::exists(
        user::table.filter(
            user::id
                .eq(bot)
                .and(user::bot_owner.eq(owner))
                .and(user::deleted_at.is_null()),
        ),
    ))
    .get_result(conn)
    .await?)
}

/// How many live bots `owner` owns.
async fn owned_count(conn: &mut AsyncPgConnection, owner: UserId) -> crate::Result<i64> {
    Ok(user::table
        .filter(user::bot_owner.eq(owner).and(user::deleted_at.is_null()))
        .count()
        .get_result(conn)
        .await?)
}

/// Refuses someone who may not own another bot: a bot, or a person already at the cap.
async fn ensure_may_own(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    owner: UserId,
    at_limit: impl FnOnce(u32) -> crate::Error,
) -> crate::Result<()> {
    let (is_bot, foreign): (bool, bool) = user::table
        .select((user::bot, user::home_domain.is_not_null()))
        .filter(user::id.eq(owner).and(user::deleted_at.is_null()))
        .first(conn)
        .await?;
    if is_bot {
        return Err(crate::Error::Validation(t!("botOwnerMustBePerson")));
    }
    // A bot belongs to its owner's home, where the owner manages it.
    if foreign {
        return Err(crate::Error::Validation(t!("botOwnerMustBeLocal")));
    }
    let max = state.settings().bots_max_per_user;
    if owned_count(conn, owner).await? >= i64::from(max) {
        return Err(at_limit(max));
    }
    Ok(())
}

/// The live bot `bot`, which `caller` must own.
async fn owned_bot(
    conn: &mut AsyncPgConnection,
    caller: UserId,
    bot: UserId,
) -> crate::Result<UserPg> {
    let row: UserPg = user::table
        .select(UserPg::as_select())
        .filter(
            user::id
                .eq(bot)
                .and(user::bot)
                .and(user::deleted_at.is_null()),
        )
        .first(conn)
        .await?;
    if row.bot_owner != Some(caller) {
        return Err(crate::Error::Forbidden(t!("botNotYours")));
    }
    Ok(row)
}

/// Makes a bot owned by `owner`, named `name` (a username, unique among everyone's), with an
/// optional display name. Returns the bot and its token, which is never shown again.
pub async fn create(
    state: &GlobalServerContext,
    owner: UserId,
    name: String,
    display_name: Option<String>,
) -> crate::Result<(User, String)> {
    if !state.settings().bots_enabled {
        return Err(crate::Error::Forbidden(t!("botsDisabled")));
    }
    validate_new_username(&name)?;
    validate_profile(&aspen_wire::message_enum::request::UserUpdateRequest {
        display_name: Some(display_name.clone()),
        ..Default::default()
    })?;
    let mut conn = state.connection_pool.get().await?;
    let (is_bot, foreign): (bool, bool) = user::table
        .select((user::bot, user::home_domain.is_not_null()))
        .filter(user::id.eq(owner))
        .first(conn.as_mut())
        .await?;
    if is_bot {
        return Err(crate::Error::Forbidden(t!("botsMakeNoBots")));
    }
    if foreign {
        return Err(crate::Error::Forbidden(t!("foreignMakeNoBots")));
    }
    let token = new_token();
    let token_digest = digest(&token);
    let bot = conn
        .transaction(|conn| {
            async move {
                // The owner's row is locked so two creations cannot both see room for one more.
                user::table
                    .select(user::id)
                    .filter(user::id.eq(owner))
                    .for_update()
                    .first::<UserId>(conn.as_mut())
                    .await?;
                ensure_may_own(state, conn.as_mut(), owner, |max| {
                    crate::Error::Validation(t!("botLimit", max = max))
                })
                .await?;
                let now = Utc::now();
                let bot: UserPg = diesel::insert_into(user::table)
                    .values(UserPg {
                        id: UserId::new(),
                        name,
                        icon: None,
                        // No password verifies against an empty hash; sign-in refuses bots
                        // before trying.
                        password_hash: String::new(),
                        created_at: now,
                        last_seen_at: now,
                        deleted_at: None,
                        display_name,
                        pronouns: None,
                        bio: None,
                        status_text: None,
                        status_emoji: None,
                        bot: true,
                        system: false,
                        bot_owner: Some(owner),
                        bot_public: false,
                        name_hue: None,
                        home_domain: None,
                        home_id: None,
                        home_icon: None,
                        plugin: None,
                        public_email: None,
                    })
                    .returning(UserPg::as_returning())
                    .get_result(conn.as_mut())
                    .await?;
                diesel::insert_into(bot_token::table)
                    .values((
                        bot_token::bot.eq(bot.id),
                        bot_token::digest.eq(token_digest),
                    ))
                    .execute(conn.as_mut())
                    .await?;
                Ok::<_, crate::Error>(bot)
            }
            .scope_boxed()
        })
        .await?;
    let bot = with_online_status(state, owner, vec![bot]).await?.remove(0);
    Ok((bot, token))
}

/// The live bots `owner` owns, the oldest first.
pub async fn list_owned(state: &GlobalServerContext, owner: UserId) -> crate::Result<Vec<User>> {
    let mut conn = state.connection_pool.get().await?;
    let bots: Vec<UserPg> = user::table
        .select(UserPg::as_select())
        .filter(user::bot_owner.eq(owner).and(user::deleted_at.is_null()))
        .order_by(user::created_at.asc())
        .load(conn.as_mut())
        .await?;
    with_online_status(state, owner, bots).await
}

/// A bot `caller` owns.
pub async fn read_owned(
    state: &GlobalServerContext,
    caller: UserId,
    bot: UserId,
) -> crate::Result<User> {
    let mut conn = state.connection_pool.get().await?;
    let row = owned_bot(conn.as_mut(), caller, bot).await?;
    Ok(with_online_status(state, caller, vec![row])
        .await?
        .remove(0))
}

/// Issues a new token for a bot `caller` owns; the old one stops working at once, and the event
/// streams opened with it close.
pub async fn rotate_token(
    state: &GlobalServerContext,
    caller: UserId,
    bot: UserId,
) -> crate::Result<String> {
    let mut conn = state.connection_pool.get().await?;
    owned_bot(conn.as_mut(), caller, bot).await?;
    let token = new_token();
    let digested = digest(&token);
    conn.transaction(|conn| {
        async move {
            diesel::insert_into(bot_token::table)
                .values((bot_token::bot.eq(bot), bot_token::digest.eq(&digested)))
                .on_conflict(bot_token::bot)
                .do_update()
                .set((
                    bot_token::digest.eq(&digested),
                    bot_token::created_at.eq(diesel::dsl::now),
                ))
                .execute(conn.as_mut())
                .await?;
            crate::login::revoke_all_sessions(state, conn.as_mut(), bot).await
        }
        .scope_boxed()
    })
    .await?;
    Ok(token)
}

/// Hands a bot `caller` owns to `new_owner`, a person who may own another.
pub async fn transfer(
    state: &GlobalServerContext,
    caller: UserId,
    bot: UserId,
    new_owner: UserId,
) -> crate::Result<User> {
    if new_owner == caller {
        return Err(crate::Error::Validation(t!("botAlreadyYours")));
    }
    let mut conn = state.connection_pool.get().await?;
    let row = conn
        .transaction(|conn| {
            async move {
                // Both owners' rows are locked, in id order, so a transfer and a creation for
                // the same person cannot both fit under the cap.
                let mut owners = [caller, new_owner];
                owners.sort();
                for owner in owners {
                    user::table
                        .select(user::id)
                        .filter(user::id.eq(owner))
                        .for_update()
                        .first::<UserId>(conn.as_mut())
                        .await?;
                }
                owned_bot(conn.as_mut(), caller, bot).await?;
                ensure_may_own(state, conn.as_mut(), new_owner, |_| {
                    crate::Error::Validation(t!("botNewOwnerAtLimit"))
                })
                .await?;
                let row: UserPg = diesel::update(user::table.filter(user::id.eq(bot)))
                    .set(user::bot_owner.eq(new_owner))
                    .returning(UserPg::as_returning())
                    .get_result(conn.as_mut())
                    .await?;
                publish_owner(state, conn.as_mut(), bot, Some(new_owner)).await?;
                Ok::<_, crate::Error>(row)
            }
            .scope_boxed()
        })
        .await?;
    Ok(with_online_status(state, caller, vec![row])
        .await?
        .remove(0))
}

/// Deletes a bot: its owner may, and so may a holder of Manage deployment settings once its
/// owner is gone. A plugin's account goes only with its plugin (`app::plugin::install`).
pub async fn delete(state: &GlobalServerContext, caller: UserId, bot: UserId) -> crate::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let (owner, plugin): (Option<UserId>, Option<String>) = user::table
        .select((user::bot_owner, user::plugin))
        .filter(
            user::id
                .eq(bot)
                .and(user::bot)
                .and(user::deleted_at.is_null()),
        )
        .first(conn.as_mut())
        .await?;
    if plugin.is_some() {
        return Err(crate::Error::Validation(t!("botIsPlugin")));
    }
    match owner {
        Some(owner) if owner == caller => {}
        Some(_) => return Err(crate::Error::Forbidden(t!("botNotYours"))),
        None => {
            crate::deployment::access_of(state, caller)
                .await?
                .require(DeploymentPermission::ManageDeploymentSettings)?;
        }
    }
    let retired = conn
        .transaction(|conn| crate::user::retire(state, conn.as_mut(), bot).scope_boxed())
        .await?;
    drop(conn);
    retired.finish(state).await;
    crate::federation::notices::announce_deleted(state, bot);
    Ok(())
}

/// Makes a bot `caller` owns public, so anyone allowed to add bots to a community may add it,
/// or private, so only its owner may.
pub async fn set_public(
    state: &GlobalServerContext,
    caller: UserId,
    bot: UserId,
    public: bool,
) -> crate::Result<User> {
    let mut conn = state.connection_pool.get().await?;
    let row = conn
        .transaction(|conn| {
            async move {
                owned_bot(conn.as_mut(), caller, bot).await?;
                let row: UserPg = diesel::update(user::table.filter(user::id.eq(bot)))
                    .set(user::bot_public.eq(public))
                    .returning(UserPg::as_returning())
                    .get_result(conn.as_mut())
                    .await?;
                publish_event(
                    state,
                    conn.as_mut(),
                    EventScope::UserEverywhere(bot),
                    &ServerEvent::User(UserEvent::Update {
                        id: bot,
                        name: None,
                        icon: None,
                        display_name: None,
                        pronouns: None,
                        bio: None,
                        status: None,
                        bot_owner: None,
                        bot_public: Some(public),
                        name_hue: None,
                        public_email: None,
                    }),
                )
                .await?;
                Ok::<_, crate::Error>(row)
            }
            .scope_boxed()
        })
        .await?;
    Ok(with_online_status(state, caller, vec![row])
        .await?
        .remove(0))
}

/// Adds a bot to a community, as its link offers: `caller` must be allowed to add bots there,
/// and the bot must be public or theirs. With `permissions`, the bot is given a role of its own
/// holding them, which takes Manage roles and Assign roles, and only permissions `caller` holds.
/// Returns the bot's membership and whether this call made it.
pub async fn add_to_community(
    state: &GlobalServerContext,
    caller: UserId,
    community: CommunityId,
    bot: UserId,
    permissions: Permissions,
) -> crate::Result<(message_enum::UserCommunity, bool)> {
    let mut conn = state.connection_pool.get().await?;
    let added = conn
        .transaction(|conn| {
            async move {
                let access = require_member(conn.as_mut(), caller, community).await?;
                access.require(Permissions::ADD_BOTS)?;
                let row: UserPg = user::table
                    .select(UserPg::as_select())
                    .filter(user::id.eq(bot).and(user::deleted_at.is_null()))
                    .first(conn.as_mut())
                    .await?;
                if !row.bot {
                    return Err(crate::Error::Validation(t!("notABot")));
                }
                if !row.bot_public && row.bot_owner != Some(caller) {
                    return Err(crate::Error::Forbidden(t!("botPrivate")));
                }
                // Who is already a member, and their nickname there.
                let member: Option<Option<String>> = community_user::table
                    .select(community_user::nickname)
                    .filter(
                        community_user::community
                            .eq(community)
                            .and(community_user::user.eq(bot)),
                    )
                    .first(conn.as_mut())
                    .await
                    .optional()?;
                if let Some(nickname) = member {
                    let roles = crate::role::roles_of_members(conn.as_mut(), &[(community, bot)])
                        .await?
                        .remove(&(community, bot))
                        .unwrap_or_default();
                    return Ok((
                        message_enum::UserCommunity {
                            community,
                            user: bot,
                            sort_index: None,
                            roles,
                            nickname,
                        },
                        false,
                    ));
                }
                let permissions = permissions.valid();
                let mut roles = Vec::new();
                if !permissions.is_empty() {
                    access.require(Permissions::MANAGE_ROLES)?;
                    access.require(Permissions::ASSIGN_ROLES)?;
                    access.require_holds(permissions)?;
                    let name = row.display_name.unwrap_or(row.name);
                    let role = crate::role::insert_role(
                        state,
                        conn.as_mut(),
                        community,
                        crate::role::NewRole {
                            name,
                            permissions,
                            hue: None,
                            hoist: false,
                            bot: Some(bot),
                        },
                    )
                    .await?;
                    roles.push(role.id);
                }
                crate::community::ensure_room_for_another(state, conn.as_mut(), bot).await?;
                let mut membership =
                    crate::community::add_member(state, conn.as_mut(), bot, community, &roles)
                        .await?;
                // The list position is the bot's own business.
                membership.sort_index = None;
                Ok((membership, true))
            }
            .scope_boxed()
        })
        .await?;
    if added.1 {
        crate::everyone_limit::after_join(state, community).await;
    }
    Ok(added)
}

/// Leaves every bot `owner` owned working and ownerless, as their account goes, inside the
/// caller's transaction.
pub async fn orphan_bots_of(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    owner: UserId,
) -> crate::Result<()> {
    let bots: Vec<UserId> = diesel::update(
        user::table.filter(user::bot_owner.eq(owner).and(user::deleted_at.is_null())),
    )
    .set(user::bot_owner.eq(None::<UserId>))
    .returning(user::id)
    .get_results(conn)
    .await?;
    for bot in bots {
        publish_owner(state, conn, bot, None).await?;
    }
    Ok(())
}

/// Tells everyone who sees the bot who owns it now.
async fn publish_owner(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    bot: UserId,
    owner: Option<UserId>,
) -> crate::Result<()> {
    publish_event(
        state,
        conn,
        EventScope::UserEverywhere(bot),
        &ServerEvent::User(UserEvent::Update {
            id: bot,
            name: None,
            icon: None,
            display_name: None,
            pronouns: None,
            bio: None,
            status: None,
            bot_owner: Some(owner),
            bot_public: None,
            name_hue: None,
            public_email: None,
        }),
    )
    .await
}
