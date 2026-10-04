//! `federation …`: the directory of other deployments and this deployment's key
//! (`app::federation`).

use super::{database, operator};
use crate::aspen_config::AspenConfig;
use anyhow::{Result, anyhow, bail};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum FederationCommand {
    /// Show this deployment's domain, key, and gates.
    Status,
    /// List every deployment this one knows, with the lists each is on.
    List,
    /// Add a deployment and contact it, pinning its key.
    Add {
        /// Its domain, with `:port` when it is not served on 443.
        domain: crate::app::federation::Domain,
        /// What it is, shown beside it in the dashboard.
        #[clap(long)]
        note: Option<String>,
    },
    /// Forget a deployment: its pinned key and the lists it is on.
    Remove {
        domain: crate::app::federation::Domain,
    },
    /// Read a known deployment's document now and check its key against the pinned one.
    Contact {
        domain: crate::app::federation::Domain,
    },
    /// Accept the key a deployment offered in place of its pinned one.
    AcceptKey {
        domain: crate::app::federation::Domain,
        /// The offered key's fingerprint, as `federation list` shows it, confirmed with the
        /// deployment's administrators.
        #[clap(long)]
        fingerprint: String,
    },
    /// Put a known deployment on a list, such as `usersEmigrationAllow`.
    ListAdd {
        domain: crate::app::federation::Domain,
        list: crate::app::federation::FederationList,
    },
    /// Take a deployment off a list.
    ListRemove {
        domain: crate::app::federation::Domain,
        list: crate::app::federation::FederationList,
    },
    /// Replace this deployment's key, as planned or because it may have leaked.
    RotateKey {
        /// The old key signs a handover to the new one, which the deployments that pinned it
        /// accept on their own.
        #[clap(
            long,
            conflicts_with = "compromised",
            required_unless_present = "compromised"
        )]
        planned: bool,
        /// The old key may be in someone else's hands and vouches for nothing: every deployment
        /// that pinned it refuses the new one until its administrators accept it.
        #[clap(long)]
        compromised: bool,
    },
}

fn print_deployment(
    policy: &crate::app::federation::FederationPolicy,
    listed: &crate::app::federation::Listed,
) {
    use crate::app::federation::{self, Direction, Subject};
    let d = &listed.deployment;
    let key = d
        .public_key
        .as_deref()
        .map_or_else(|| "not contacted".to_string(), federation::fingerprint);
    let admitted: Vec<&str> = [
        (Subject::Users, Direction::Emigration, "users go"),
        (Subject::Users, Direction::Immigration, "users come"),
        (Subject::Bots, Direction::Emigration, "bots go"),
        (Subject::Bots, Direction::Immigration, "bots come"),
    ]
    .into_iter()
    .filter(|(subject, direction, _)| {
        federation::admits(policy, *subject, *direction, &listed.lists)
    })
    .map(|(_, _, name)| name)
    .collect();
    println!(
        "{}  {key}  {}",
        d.domain,
        d.note.as_deref().unwrap_or_default()
    );
    if let Some(offered) = &d.offered_key {
        println!(
            "  offers a new key {}, refused until accepted",
            federation::fingerprint(offered)
        );
    }
    if let Some(protocol) = d.protocol() {
        let software = match (&d.software_name, &d.software_version) {
            (Some(name), Some(version)) => format!("{name} {version}"),
            (Some(name), None) => name.clone(),
            _ => "unnamed software".into(),
        };
        println!(
            "  runs {software}, protocol {}..={}{}",
            protocol.minimum,
            protocol.version,
            if protocol
                .common_version(&federation::protocol::Protocol::ours())
                .is_none()
            {
                ", no version in common with this deployment"
            } else {
                ""
            }
        );
    }
    if !listed.lists.is_empty() {
        let lists: Vec<String> = listed.lists.iter().map(ToString::to_string).collect();
        println!("  on {}", lists.join(", "));
    }
    println!(
        "  admits: {}",
        if admitted.is_empty() {
            "no one".to_string()
        } else {
            admitted.join(", ")
        }
    );
}

pub async fn federation(config: &AspenConfig, command: FederationCommand) -> Result<()> {
    use crate::app::federation::{self, ContactOutcome, Origin};
    let mut conn = database(config).await?;
    let fail = |e: crate::app::Error| anyhow!("{e}");
    let policy = crate::app::deployment_settings::load(&mut conn)
        .await
        .map_err(fail)?
        .federation;
    let contact = async |conn: &mut diesel_async::AsyncPgConnection,
                         domain: &federation::Domain|
           -> Result<()> {
        let client = federation::fetch::client(&config.federation).map_err(fail)?;
        let document = federation::fetch_document(&config.federation, &client, domain)
            .await
            .map_err(fail)?;
        let (listed, outcome) = federation::record_contact(conn, domain, &document)
            .await
            .map_err(fail)?;
        tracing::info!(%domain, ?outcome, operator = operator(), "contacted a deployment");
        match outcome {
            ContactOutcome::Pinned => println!("pinned {domain}'s key"),
            ContactOutcome::Confirmed => println!("{domain} presented its pinned key"),
            ContactOutcome::HandedOver => {
                println!("{domain} handed over to a new key, which is now pinned")
            }
            ContactOutcome::KeyChanged => {
                println!("{domain} presented a different key, which is refused until accepted")
            }
        }
        print_deployment(&policy, &listed);
        Ok(())
    };
    match command {
        FederationCommand::Status => {
            match federation::own_domain(&config.federation) {
                Some(domain) => println!("domain: {domain}"),
                None => println!("domain: none; this deployment takes no part in federation"),
            }
            match federation::current_key(&mut conn).await.map_err(fail)? {
                Some(key) => println!(
                    "key: {} (made {})",
                    federation::fingerprint(&key.public_key),
                    key.created_at
                ),
                None => println!("key: none yet; the server makes it when it starts with a domain"),
            }
            for (name, rules) in [("users", &policy.users), ("bots", &policy.bots)] {
                println!(
                    "{name}: emigration {}, immigration {}{}",
                    rules.emigration,
                    rules.immigration,
                    if rules.shared_list {
                        ", one shared list"
                    } else {
                        ""
                    }
                );
            }
            let lists: Vec<String> = federation::FederationList::all_in_force(&policy)
                .iter()
                .map(ToString::to_string)
                .collect();
            if !lists.is_empty() {
                println!("lists in force: {}", lists.join(", "));
            }
        }
        FederationCommand::List => {
            for listed in federation::list_all(&mut conn).await.map_err(fail)? {
                print_deployment(&policy, &listed);
            }
        }
        FederationCommand::Add { domain, note } => {
            federation::add(
                &config.federation,
                &mut conn,
                &domain,
                Origin::Terminal,
                None,
                note,
            )
            .await
            .map_err(fail)?;
            tracing::info!(%domain, operator = operator(), "added a deployment to the federation directory");
            println!("added {domain}");
            contact(&mut conn, &domain).await?;
        }
        FederationCommand::Remove { domain } => {
            federation::remove(&mut conn, &domain).await.map_err(fail)?;
            tracing::info!(%domain, operator = operator(), "forgot a deployment");
            println!("forgot {domain}");
        }
        FederationCommand::Contact { domain } => {
            federation::get(&mut conn, &domain).await.map_err(fail)?;
            contact(&mut conn, &domain).await?;
        }
        FederationCommand::AcceptKey {
            domain,
            fingerprint,
        } => {
            let listed = federation::get(&mut conn, &domain).await.map_err(fail)?;
            let Some(offered) = listed.deployment.offered_key else {
                bail!("{domain} offers no new key");
            };
            if federation::fingerprint(&offered) != fingerprint.trim() {
                bail!(
                    "{domain} offers {}, not {fingerprint}",
                    federation::fingerprint(&offered)
                );
            }
            let listed = federation::accept_key(&mut conn, &domain, &offered)
                .await
                .map_err(fail)?;
            tracing::warn!(%domain, operator = operator(), "accepted a deployment's new key");
            print_deployment(&policy, &listed);
        }
        FederationCommand::ListAdd { domain, list } => {
            let added = federation::set_listed(&mut conn, &domain, list, true, None)
                .await
                .map_err(fail)?;
            if added {
                tracing::info!(%domain, %list, operator = operator(), "put a deployment on a list");
            }
            print_deployment(
                &policy,
                &federation::get(&mut conn, &domain).await.map_err(fail)?,
            );
        }
        FederationCommand::ListRemove { domain, list } => {
            if federation::set_listed(&mut conn, &domain, list, false, None)
                .await
                .map_err(fail)?
            {
                tracing::info!(%domain, %list, operator = operator(), "took a deployment off a list");
            }
            print_deployment(
                &policy,
                &federation::get(&mut conn, &domain).await.map_err(fail)?,
            );
        }
        FederationCommand::RotateKey { planned, .. } => {
            let rotation = if planned {
                federation::Rotation::Planned
            } else {
                federation::Rotation::Compromised
            };
            let domain = federation::own_domain(&config.federation);
            let key = federation::rotate_key(&mut conn, domain.as_ref(), rotation)
                .await
                .map_err(fail)?;
            tracing::warn!(
                ?rotation,
                operator = operator(),
                "replaced this deployment's federation key"
            );
            if rotation == federation::Rotation::Compromised {
                eprintln!(
                    "every deployment that pinned the old key will refuse this one until its \
                     administrators accept it; tell them its fingerprint"
                );
            }
            println!("new key: {}", federation::fingerprint(&key.public_key));
        }
    }
    Ok(())
}
