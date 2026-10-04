//! The deployment's own account, which sends people notices from the deployment itself, such as
//! a community owner's when `app::everyone_limit` turns Mention everyone off.
//!
//! It is one row of `user` with `system` set, made the first time a notice is sent, called what
//! the deployment is called (`app::deployment_settings`), and labelled as the system wherever it is named. It has no
//! password or token, so nothing signs in as it, and its username is outside the ones people
//! sign in and are found by (`user_name_key` leaves it out). Its notices are ordinary messages
//! of a one-to-one DM, which it alone may start and which needs no shared community; the person
//! reads them and cannot answer (`ChannelAccess::from_system`), block it, or bring it into
//! another DM.

use crate::app::context::GlobalServerContext;
use crate::app::user::UserPg;
use crate::app::{self, ChannelId, UserId};
use crate::database::schema::{channel, user};
use chrono::Utc;
use diesel::prelude::*;
use diesel::upsert::DecoratableTarget;
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};

/// The system account's username, which no one else's can clash with.
pub const USERNAME: &str = "system";

/// The system account, made the first time it is needed and called what the deployment is
/// called. Renaming the deployment renames it (`app::deployment_settings::update`).
pub async fn id(state: &GlobalServerContext, conn: &mut AsyncPgConnection) -> app::Result<UserId> {
    let display_name = state.settings().name().to_string();
    let now = Utc::now();
    diesel::insert_into(user::table)
        .values(UserPg {
            id: UserId::new(),
            name: USERNAME.to_string(),
            icon: None,
            // No password verifies against an empty hash, and sign-in looks past this account.
            password_hash: String::new(),
            created_at: now,
            last_seen_at: now,
            deleted_at: None,
            display_name: Some(display_name.clone()),
            pronouns: None,
            bio: None,
            status_text: None,
            status_emoji: None,
            bot: false,
            system: true,
            bot_owner: None,
            bot_public: false,
            name_hue: None,
            home_domain: None,
            home_id: None,
            home_icon: None,
            plugin: None,
        })
        .on_conflict(user::system)
        .filter_target(user::system.eq(true))
        .do_nothing()
        .execute(conn)
        .await?;
    let id: UserId = user::table
        .select(user::id)
        .filter(user::system)
        .first(conn)
        .await?;
    Ok(id)
}

/// Whether `user` is the system account.
pub async fn is(conn: &mut AsyncPgConnection, user_id: UserId) -> app::Result<bool> {
    Ok(user::table
        .select(user::system)
        .filter(user::id.eq(user_id))
        .first::<bool>(conn)
        .await
        .optional()?
        .unwrap_or(false))
}

/// Sends `recipient` a notice from the system account, in their DM with it, made on first use.
pub async fn notify(
    state: &GlobalServerContext,
    recipient: UserId,
    content: String,
) -> app::Result<()> {
    let mut conn = state.connection_pool.get().await?;
    let (system, dm) = conn
        .transaction(|conn| {
            async move {
                let system = id(state, conn).await?;
                let key = app::dm::pair_key(system, recipient);
                let existing: Option<ChannelId> = channel::table
                    .select(channel::id)
                    .filter(channel::dm_key.eq(&key))
                    .first(conn)
                    .await
                    .optional()?;
                let dm = match existing {
                    Some(dm) => dm,
                    None => {
                        app::dm::insert_dm(state, conn, system, &[recipient])
                            .await?
                            .0
                            .id
                    }
                };
                Ok::<_, app::Error>((system, dm))
            }
            .scope_boxed()
        })
        .await?;
    drop(conn);
    app::message::create_message(
        state,
        system,
        dm,
        content,
        Vec::new(),
        false,
        app::message::Posting::Text,
    )
    .await?;
    Ok(())
}
