//! The deployment's own account, which sends people notices from the deployment itself, such as
//! a community owner's when `app::everyone_limit` turns Mention everyone off.
//!
//! It is one row of `user` with `system` set, made the first time a notice is sent, called by
//! `[system_account] display_name`, and labelled as the system wherever it is named. It has no
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

/// The system account, made the first time it is needed and called by what the configuration
/// says now. Those holding its record see a new name when they next read it: it belongs to no
/// community, so no event of its profile reaches anyone.
pub async fn id(state: &GlobalServerContext, conn: &mut AsyncPgConnection) -> app::Result<UserId> {
    let display_name = state.config.system_account.display_name.trim().to_string();
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
            home_domain: None,
            home_id: None,
            home_icon: None,
        })
        .on_conflict(user::system)
        .filter_target(user::system.eq(true))
        .do_nothing()
        .execute(conn)
        .await?;
    let (id, shown): (UserId, Option<String>) = user::table
        .select((user::id, user::display_name))
        .filter(user::system)
        .first(conn)
        .await?;
    if shown.as_deref() != Some(display_name.as_str()) {
        diesel::update(user::table.filter(user::id.eq(id)))
            .set(user::display_name.eq(&display_name))
            .execute(conn)
            .await?;
    }
    Ok(id)
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
