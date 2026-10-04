//! A plugin's own account, its principal: a bot no person owns, marked with the plugin's id
//! (`user.plugin`), which signs in to nothing and acts only through the host. It is made when
//! the plugin is installed, joins a community when the community turns the plugin on (with a
//! role of its own holding what the person turning it on grants, as adding a bot does), and
//! leaves when it is turned off. It acts through the same app functions as anyone, so its
//! permissions, its rank, and everything that takes access from a member hold for it too.

use super::host::Deferred;
use super::registry::LoadedPlugin;
use crate::app::context::GlobalServerContext;
use crate::app::permissions::{CommunityAccess, Permissions};
use crate::app::{self, CommunityId, MessageId, UserId};
use crate::database::schema::{community_user, user};
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use std::sync::Arc;

/// Does `action` as `plugin`'s principal, refusing while the principal is banned from the
/// deployment. Whatever it publishes is settled as a request's is (`app::events::settle`).
pub(super) async fn act(
    state: &GlobalServerContext,
    plugin: &LoadedPlugin,
    action: Deferred,
) -> app::Result<Option<MessageId>> {
    let Some(principal) = plugin.principal else {
        return Err(app::Error::Forbidden(std::borrow::Cow::Borrowed(
            "the plugin has no principal",
        )));
    };
    {
        let mut conn = state.connection_pool.get().await?;
        let banned: bool = user::table
            .select(app::user_ban::banned())
            .filter(user::id.eq(principal))
            .first(conn.as_mut())
            .await?;
        if banned {
            return Err(app::Error::DeploymentBanned {
                reason: None,
                until: None,
            });
        }
    }
    let work = async {
        match action {
            Deferred::Send { channel, content } => app::message::create_message(
                state,
                principal,
                channel,
                content,
                Vec::new(),
                false,
                app::message::Posting::Text,
            )
            .await
            .map(|message| Some(message.id)),
            Deferred::SendCard {
                channel,
                content,
                card,
            } => app::message::create_message(
                state,
                principal,
                channel,
                content,
                Vec::new(),
                false,
                app::message::Posting::Card(card),
            )
            .await
            .map(|message| Some(message.id)),
            Deferred::UpdateCard { message, card } => {
                super::card::update(state, principal, message, card)
                    .await
                    .map(|_| None)
            }
            Deferred::Delete(id) => app::message::delete_message(state, principal, id)
                .await
                .map(|_| None),
            Deferred::React { message, emoji } => {
                app::react::create_react(state, principal, message, emoji)
                    .await
                    .map(|_| None)
            }
            Deferred::Remove { community, user } => {
                app::role::remove_member(state, principal, community, user)
                    .await
                    .map(|_| None)
            }
            Deferred::Ban {
                community,
                user,
                reason,
                seconds,
            } => app::ban::ban_member(
                state,
                principal,
                community,
                user,
                &app::ban::BanRequest {
                    reason,
                    duration_seconds: seconds.map(|s| u32::try_from(s).unwrap_or(u32::MAX)),
                    delete_messages_seconds: None,
                },
            )
            .await
            .map(|_| None),
        }
    };
    let (result, noted) = app::events::noting(work).await;
    app::events::settle(state, noted, result.is_err()).await;
    result
}

/// Runs what a plugin asked to do while intercepting, now that the hook has answered.
pub(super) fn run_deferred(
    state: &GlobalServerContext,
    plugin: Arc<LoadedPlugin>,
    actions: Vec<Deferred>,
) {
    if actions.is_empty() {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        for action in actions {
            if let Err(e) = act(&state, &plugin, action).await {
                tracing::info!(plugin = plugin.id, "an action it asked for failed: {e}");
            }
        }
    });
}

/// Whether `principal` belongs to `community`.
pub async fn is_member(
    conn: &mut AsyncPgConnection,
    principal: UserId,
    community: CommunityId,
) -> app::Result<bool> {
    Ok(diesel::select(diesel::dsl::exists(
        community_user::table.filter(
            community_user::community
                .eq(community)
                .and(community_user::user.eq(principal)),
        ),
    ))
    .get_result(conn)
    .await?)
}

/// Brings `principal` into `community` for the plugin `display_name` names, with a role of its
/// own holding `granted`, inside the caller's transaction. `access` is whoever turns the plugin
/// on, who needs Add bots, and to give it permissions, Manage roles and Assign roles, and may
/// grant only what they hold. Nothing happens when it is a member already.
pub async fn join(
    state: &GlobalServerContext,
    conn: &mut AsyncPgConnection,
    access: &CommunityAccess,
    principal: UserId,
    display_name: String,
    granted: Permissions,
) -> app::Result<()> {
    let community = access.community;
    if is_member(conn, principal, community).await? {
        return Ok(());
    }
    access.require(Permissions::ADD_BOTS)?;
    let granted = granted.valid();
    let mut roles = Vec::new();
    if !granted.is_empty() {
        access.require(Permissions::MANAGE_ROLES)?;
        access.require(Permissions::ASSIGN_ROLES)?;
        access.require_holds(granted)?;
        let role = app::role::insert_role(
            state,
            conn,
            community,
            app::role::NewRole {
                name: display_name,
                permissions: granted,
                hue: None,
                hoist: false,
                bot: Some(principal),
            },
        )
        .await?;
        roles.push(role.id);
    }
    app::community::add_member(state, conn, principal, community, &roles).await?;
    Ok(())
}

/// Takes `principal` out of `community`, and its role with it, inside the caller's
/// transaction.
pub async fn leave(
    state: &impl app::events::Publishing,
    conn: &mut AsyncPgConnection,
    principal: UserId,
    community: CommunityId,
) -> app::Result<()> {
    app::community::end_membership(state, conn, principal, community).await
}
