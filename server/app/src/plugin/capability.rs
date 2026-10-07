//! People's private URLs to plugins' routes (`capabilities`): a plugin answering one of its
//! routes gives the caller a path of theirs by a name of its own (`feed:{channel}`), which reaches
//! the plugin's route `aspen/capabilities/{name}` as that person without signing in, for what
//! cannot sign in, such as a calendar app following a feed. The path is the same each time it is
//! asked for, until the plugin revokes it or the person secures their account (changing or
//! resetting their password, or signing out everywhere else, which end every path of theirs:
//! [`revoke_all`]); a request to it answers only what the person may still see, and nothing once
//! they are banned or their account is gone.

use super::route::{self, Answer};
use crate::UserId;
use aspen_schema::{plugin_capability, user};
use base64::Engine;
use base64::prelude::BASE64_URL_SAFE_NO_PAD;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use rand::RngExt;

/// The longest a capability's name may be.
const MAX_NAME: usize = 200;

/// Where the plugin's capabilities are served, beneath the API.
pub fn path_of(plugin: &str, secret: &str) -> String {
    format!(
        "{}/plugins/{plugin}/capabilities/{secret}",
        aspen_wire::API_PREFIX
    )
}

/// `caller`'s path for `name` of `plugin`, made the first time it is asked for.
pub async fn path(
    conn: &mut AsyncPgConnection,
    plugin: &str,
    caller: UserId,
    name: &str,
) -> crate::Result<String> {
    if name.is_empty() || name.len() > MAX_NAME {
        return Err(crate::Error::Validation(
            format!("a capability's name is 1 to {MAX_NAME} bytes").into(),
        ));
    }
    let fresh = BASE64_URL_SAFE_NO_PAD
        .encode(crate::CHACHA_RNG.with(|rng| rng.borrow_mut().random::<[u8; 32]>()));
    diesel::insert_into(plugin_capability::table)
        .values((
            plugin_capability::secret.eq(&fresh),
            plugin_capability::plugin.eq(plugin),
            plugin_capability::user.eq(caller),
            plugin_capability::name.eq(name),
        ))
        .on_conflict((
            plugin_capability::plugin,
            plugin_capability::user,
            plugin_capability::name,
        ))
        .do_nothing()
        .execute(conn)
        .await?;
    let secret: String = plugin_capability::table
        .select(plugin_capability::secret)
        .filter(
            plugin_capability::plugin
                .eq(plugin)
                .and(plugin_capability::user.eq(caller))
                .and(plugin_capability::name.eq(name)),
        )
        .first(conn)
        .await?;
    Ok(path_of(plugin, &secret))
}

/// Ends `caller`'s path for `name` of `plugin`.
pub async fn revoke(
    conn: &mut AsyncPgConnection,
    plugin: &str,
    caller: UserId,
    name: &str,
) -> crate::Result<()> {
    diesel::delete(
        plugin_capability::table.filter(
            plugin_capability::plugin
                .eq(plugin)
                .and(plugin_capability::user.eq(caller))
                .and(plugin_capability::name.eq(name)),
        ),
    )
    .execute(conn)
    .await?;
    Ok(())
}

/// Ends every path of `user`'s, of every plugin, as securing their account does: a path is a
/// standing credential like a sign-in, and one copied by whoever took the account must stop
/// working with the rest. Plugins hand out fresh ones when next asked.
pub async fn revoke_all(conn: &mut AsyncPgConnection, user: UserId) -> crate::Result<()> {
    diesel::delete(plugin_capability::table.filter(plugin_capability::user.eq(user)))
        .execute(conn)
        .await?;
    Ok(())
}

/// The plugin's answer to someone following the path `secret`, as its owner, while they are
/// neither banned nor gone.
pub async fn follow(
    state: &crate::context::GlobalServerContext,
    plugin: &str,
    secret: &str,
    query: String,
) -> crate::Result<Answer> {
    let not_found = || crate::Error::Diesel(diesel::result::Error::NotFound);
    let mut conn = state.connection_pool.get().await?;
    let (owner, name): (UserId, String) = plugin_capability::table
        .inner_join(user::table)
        .select((plugin_capability::user, plugin_capability::name))
        .filter(
            plugin_capability::secret
                .eq(secret)
                .and(plugin_capability::plugin.eq(plugin))
                .and(user::deleted_at.is_null())
                .and(diesel::dsl::not(crate::user_ban::banned())),
        )
        .first(conn.as_mut())
        .await
        .optional()?
        .ok_or_else(not_found)?;
    drop(conn);
    let loaded = state.plugins.get(plugin).ok_or_else(not_found)?;
    if !loaded.holds(super::PluginPermission::Capabilities) {
        return Err(not_found());
    }
    route::answer_host(
        state,
        plugin,
        owner,
        route::Request {
            method: "GET".into(),
            path: format!("aspen/capabilities/{name}"),
            query,
            content_type: None,
            body: Vec::new(),
        },
    )
    .await
}
