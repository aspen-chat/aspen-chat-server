//! `communities set-owner` and `unowned`: naming a community's owner from the terminal.

use super::{database, operator, publisher};
use crate::aspen_config::AspenConfig;
use anyhow::{Result, bail};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum CommunitiesCommand {
    /// Make a member the owner of a community, announced to its members as a handover is.
    SetOwner {
        /// The community's id.
        community: uuid::Uuid,
        /// The username of the member who becomes its owner.
        username: String,
    },
    /// List the communities that have no owner, with their member counts.
    Unowned,
}

pub async fn communities(config: &AspenConfig, command: CommunitiesCommand) -> Result<()> {
    use crate::database::schema::{community, community_user, user};
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    let mut conn = database(config).await?;
    match command {
        CommunitiesCommand::SetOwner {
            community: id,
            username,
        } => {
            let member: Option<uuid::Uuid> = community_user::table
                .inner_join(user::table)
                .select(user::id)
                .filter(
                    community_user::community
                        .eq(id)
                        .and(crate::app::user::named(username.clone())),
                )
                .first(&mut conn)
                .await
                .optional()?;
            let Some(member) = member else {
                bail!("{username:?} is not a member of community {id}");
            };
            let live: bool = diesel::select(diesel::dsl::exists(
                community::table.filter(community::id.eq(id).and(community::deleted_at.is_null())),
            ))
            .get_result(&mut conn)
            .await?;
            if !live {
                bail!("no community has the id {id}");
            }
            let publisher = publisher(config).await?;
            let community_id = crate::app::CommunityId(id);
            let set = {
                use diesel_async::AsyncConnection;
                use diesel_async::scoped_futures::ScopedFutureExt;
                let publisher = &publisher;
                let conn = &mut conn;
                crate::app::events::noting(conn.transaction(|conn| {
                    async move {
                        crate::app::role::set_owner(
                            publisher,
                            conn,
                            community_id,
                            Some(crate::app::UserId(member)),
                        )
                        .await
                    }
                    .scope_boxed()
                }))
                .await
            };
            let (set, noted) = set;
            crate::app::events::settle_in(&publisher, &mut conn, noted, set.is_err()).await;
            set.map_err(|e| anyhow::anyhow!("{e}"))?;
            tracing::info!(%id, %username, operator = operator(), "set a community's owner");
            println!("{username} now owns community {id}");
        }
        CommunitiesCommand::Unowned => {
            let rows: Vec<(uuid::Uuid, String, i64)> = community::table
                .left_join(community_user::table)
                .group_by((community::id, community::name))
                .select((
                    community::id,
                    community::name,
                    diesel::dsl::count(community_user::user.nullable()),
                ))
                .filter(
                    community::owner
                        .is_null()
                        .and(community::deleted_at.is_null()),
                )
                .order(community::name)
                .load(&mut conn)
                .await?;
            if rows.is_empty() {
                println!("every community has an owner");
            }
            for (id, name, members) in rows {
                println!("{id}  {members:>6} members  {name}");
            }
        }
    }
    Ok(())
}
