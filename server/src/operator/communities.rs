//! `communities set-owner` and `unowned`: naming a community's owner from the terminal.

use super::{database, operator};
use crate::aspen_config::AspenConfig;
use anyhow::{Result, bail};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum CommunitiesCommand {
    /// Make a member the owner of a community. Its members see the change when they next load
    /// it.
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
            let updated = diesel::update(
                community::table.filter(community::id.eq(id).and(community::deleted_at.is_null())),
            )
            .set(community::owner.eq(Some(member)))
            .execute(&mut conn)
            .await?;
            if updated == 0 {
                bail!("no community has the id {id}");
            }
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
