//! `admin grant`, `revoke`, `list`, `allow`, and `deny`: who holds the deployment's top role and
//! what it allows (`app::deployment_role`).

use super::{database, operator, publisher};
use crate::app::events::{Publisher, noting, settle_in};
use crate::aspen_config::AspenConfig;
use anyhow::{Result, anyhow, bail};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum AdminCommand {
    /// Give a user the deployment's top role, making an Administrator role first when there is
    /// none.
    Grant {
        /// Their username.
        username: String,
    },
    /// Take every deployment role from a user.
    Revoke {
        /// Their username.
        username: String,
    },
    /// List who holds which deployment roles.
    List,
    /// Let the deployment's top role do something more, such as `moderateCommunities`, which
    /// its holders may then give to the roles below it.
    Allow {
        /// A deployment permission's name.
        #[clap(value_enum)]
        permission: crate::app::deployment::DeploymentPermission,
    },
    /// Stop the deployment's top role doing something.
    Deny {
        /// A deployment permission's name.
        #[clap(value_enum)]
        permission: crate::app::deployment::DeploymentPermission,
    },
}

/// The terminal names deployment permissions as the API does, and lists them in its help.
impl clap::ValueEnum for crate::app::deployment::DeploymentPermission {
    fn value_variants<'a>() -> &'a [Self] {
        <Self as strum::VariantArray>::VARIANTS
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        Some(clap::builder::PossibleValue::new(<&'static str>::from(
            self,
        )))
    }
}

pub async fn admin(config: &AspenConfig, command: AdminCommand) -> Result<()> {
    let mut conn = database(config).await?;
    let publisher = publisher(config).await?;
    let (result, noted) = noting(run(&publisher, &mut conn, command)).await;
    settle_in(&publisher, &mut conn, noted, result.is_err()).await;
    result
}

async fn run(
    publisher: &Publisher,
    conn: &mut diesel_async::AsyncPgConnection,
    command: AdminCommand,
) -> Result<()> {
    use crate::app::deployment_role;
    use crate::database::schema::user;
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    let find = |username: String| {
        user::table
            .select(user::id)
            .filter(crate::app::user::named(username))
    };
    match command {
        AdminCommand::Grant { username } => {
            let Some(id) = find(username.clone())
                .first::<crate::app::UserId>(conn)
                .await
                .optional()?
            else {
                bail!("no user is named {username:?}");
            };
            let role = deployment_role::grant_top_role(publisher, conn, id)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            tracing::info!(%username, %role, operator = operator(), "granted the top deployment role");
            println!("{username} now holds {role}, the deployment's top role");
        }
        AdminCommand::Revoke { username } => {
            let Some(id) = find(username.clone())
                .first::<crate::app::UserId>(conn)
                .await
                .optional()?
            else {
                bail!("no user is named {username:?}");
            };
            let taken = deployment_role::revoke_all(publisher, conn, id)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            tracing::info!(%username, taken, operator = operator(), "revoked deployment roles");
            println!("{username} no longer holds any deployment role");
        }
        AdminCommand::Allow { permission } => top_role(publisher, conn, permission, true).await?,
        AdminCommand::Deny { permission } => top_role(publisher, conn, permission, false).await?,
        AdminCommand::List => {
            let holders = deployment_role::holders(conn)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            if holders.is_empty() {
                println!("nobody holds a deployment role; grant one with `admin grant <username>`");
            }
            for (name, role) in holders {
                println!("{name}  {role}");
            }
        }
    }
    Ok(())
}

/// Allows `permission` to the deployment's top role, or denies it.
async fn top_role(
    publisher: &Publisher,
    conn: &mut diesel_async::AsyncPgConnection,
    permission: crate::app::deployment::DeploymentPermission,
    allow: bool,
) -> Result<()> {
    use crate::app::deployment_role;
    let role = deployment_role::set_top_role_permission(publisher, conn, permission, allow)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    tracing::info!(%permission, allow, %role, operator = operator(), "changed the top deployment role");
    println!(
        "{role} {} {permission}",
        if allow {
            "now allows"
        } else {
            "no longer allows"
        }
    );
    Ok(())
}
