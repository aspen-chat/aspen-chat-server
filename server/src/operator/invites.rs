//! `invites create`, `list`, and `revoke`: registration invites (`app::registration_invite`).

use super::{database, operator, publisher};
use crate::aspen_config::AspenConfig;
use anyhow::{Result, anyhow};
use clap::Subcommand;
use std::time::Duration;

#[derive(Subcommand, Debug)]
pub enum InvitesCommand {
    /// Make a registration invite and print its code.
    Create {
        /// How many accounts it may create.
        #[clap(long, default_value_t = 1)]
        uses: i32,
        /// How long it lasts, as `90s`, `30m`, `2h`, or `7d`; for good when left out.
        #[clap(long)]
        expires: Option<humantime::Duration>,
        /// What it is for, shown beside it in the dashboard.
        #[clap(long)]
        note: Option<String>,
    },
    /// List the newest registration invites.
    List,
    /// Revoke a registration invite, and a dual invite's community invite with it; accounts it
    /// made are kept.
    Revoke {
        /// The invite's code.
        code: String,
    },
}

pub async fn invites(config: &AspenConfig, command: InvitesCommand) -> Result<()> {
    use crate::app::registration_invite;
    let mut conn = database(config).await?;
    match command {
        InvitesCommand::Create {
            uses,
            expires,
            note,
        } => {
            let expires_in = expires.map(|d| {
                chrono::Duration::from_std(Duration::from(d)).unwrap_or(chrono::Duration::MAX)
            });
            let terms = registration_invite::Terms {
                max_uses: uses,
                expires_in,
                note,
            };
            let invite = registration_invite::create(&mut conn, None, terms)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            tracing::info!(code = %invite.code, operator = operator(), "made a registration invite");
            println!("{}", invite.code);
            let settings = crate::app::deployment_settings::load(&mut conn)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            if !settings.registration_invite_required {
                eprintln!(
                    "note: registering takes no invite on this deployment; \
                     `aspen-chat-server settings set --registration-invite-required true` \
                     makes it take one"
                );
            }
        }
        InvitesCommand::List => {
            let now = chrono::Utc::now();
            for invite in registration_invite::list(&mut conn, true)
                .await
                .map_err(|e| anyhow!("{e}"))?
            {
                let state = if invite.revoked_at.is_some() {
                    "revoked"
                } else if invite.usable(now) {
                    "usable"
                } else {
                    "spent"
                };
                let joins = invite
                    .community_invite
                    .map(|code| format!("  joins with {code}"))
                    .unwrap_or_default();
                println!(
                    "{}  {}/{} used  {state}{joins}  {}",
                    invite.code,
                    invite.uses,
                    invite.max_uses,
                    invite.note.unwrap_or_default()
                );
            }
        }
        InvitesCommand::Revoke { code } => {
            let publisher = publisher(config).await?;
            registration_invite::revoke(&publisher, &mut conn, &code)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            tracing::info!(%code, operator = operator(), "revoked a registration invite");
            println!("revoked {code}");
        }
    }
    Ok(())
}
