//! Commands an operator runs against a deployment with its own credentials, rather than through
//! the API, for powers no account should hold.
//!
//! `limits suspend` lifts rate limits for a while (see `aspen_limits::suspension`), `limits
//! resume` restores them, and `limits status` says which is in force. `bench seed` writes a
//! benchmark population straight into the database and `bench purge` removes one
//! (`app::benchmark`). `admin grant` and `admin revoke` decide who may open the Administration
//! Dashboard, which nothing over the API can do. `invites create` makes a registration invite,
//! which is how an invite-only deployment gets its first account (`app::registration_invite`).
//! `federation` manages the directory of other deployments and this deployment's key
//! (`app::federation`), as Manage federation does from the dashboard, and can also replace the
//! key, which no account can.

use crate::aspen_config::AspenConfig;
use anyhow::{Context, Result, anyhow, bail};
use aspen_limits::suspension::{self, Scope, Suspension};
use clap::{Subcommand, ValueEnum};
use std::time::Duration;

#[derive(Subcommand, Debug)]
pub enum LimitsCommand {
    /// Suspend rate limits until `--for` has passed. They come back by themselves.
    Suspend {
        /// How long, as `90s`, `30m`, `2h`, `1d`, or `1h 30m`; at most `max_suspension_seconds`.
        #[clap(long = "for", default_value = "2h")]
        duration: humantime::Duration,
        /// `networks`: requests from `--network` skip the limits that count by address, and
        /// every other limit stays. `all`: every limit is lifted for everyone.
        #[clap(long, value_enum, default_value_t = ScopeArg::Networks)]
        scope: ScopeArg,
        /// A network (CIDR) or address exempted by `--scope networks`; repeatable.
        #[clap(long = "network")]
        networks: Vec<String>,
        /// Why, for the servers' logs.
        #[clap(long)]
        reason: String,
    },
    /// End a suspension now.
    Resume,
    /// Say whether a suspension is in force.
    Status,
}

#[derive(Subcommand, Debug)]
pub enum BenchCommand {
    /// Write the population a seed plan describes, and the manifest of what was made.
    Seed {
        /// The plan, as `aspen-bench plan` writes it (JSON).
        #[clap(long)]
        plan: std::path::PathBuf,
        /// Where to write the manifest (JSON).
        #[clap(long)]
        out: std::path::PathBuf,
    },
    /// Remove a run's population and everything that came to depend on it.
    Purge {
        #[clap(long)]
        run: String,
    },
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeArg {
    Networks,
    All,
}

/// Who is running the command, for the servers' logs.
fn operator() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "unknown".into());
    let host = std::fs::read_to_string("/etc/hostname")
        .map(|h| h.trim().to_string())
        .unwrap_or_else(|_| "unknown".into());
    format!("{user}@{host}")
}

fn describe(record: &Suspension, max: Duration) -> String {
    let ends = record.effective_until(max);
    let remaining = ends.saturating_sub(suspension::now_ms()) / 1000;
    let scope = match &record.scope {
        Scope::All => "every limit, for everyone".to_string(),
        Scope::Networks { networks } => {
            format!("limits by address, for {}", networks.join(", "))
        }
    };
    format!(
        "rate limits suspended: {scope}; {remaining}s left; by {}; reason: {}",
        record.by, record.reason
    )
}

pub async fn limits(config: &AspenConfig, command: LimitsCommand) -> Result<()> {
    let client = async_nats::connect_with_options(
        &config.nats_url,
        async_nats::ConnectOptions::new().token(config.nats_auth_token.clone()),
    )
    .await
    .context("could not connect to NATS")?;
    let store = suspension::bucket(client).await.map_err(|e| anyhow!(e))?;
    let max = Duration::from_secs(config.rate_limits.max_suspension_seconds);
    match command {
        LimitsCommand::Suspend {
            duration,
            scope,
            networks,
            reason,
        } => {
            let duration = Duration::from(duration);
            let started_at = suspension::now_ms();
            let record = Suspension {
                started_at,
                until: started_at + u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
                scope: match scope {
                    ScopeArg::All => Scope::All,
                    ScopeArg::Networks => Scope::Networks { networks },
                },
                reason,
                by: operator(),
            };
            record.validate(max).map_err(|e| anyhow!(e))?;
            suspension::write(&store, &record)
                .await
                .map_err(|e| anyhow!(e))?;
            println!("{}", describe(&record, max));
        }
        LimitsCommand::Resume => {
            suspension::clear(&store).await.map_err(|e| anyhow!(e))?;
            println!("rate limits are in force");
        }
        LimitsCommand::Status => {
            match suspension::read(&store)
                .await
                .map_err(|e| anyhow!(e))?
                .filter(|record| suspension::now_ms() < record.effective_until(max))
            {
                Some(record) => println!("{}", describe(&record, max)),
                None => println!("rate limits are in force"),
            }
        }
    }
    Ok(())
}

pub async fn bench(config: &AspenConfig, command: BenchCommand) -> Result<()> {
    use diesel_async::AsyncConnection;
    let mut conn = diesel_async::AsyncPgConnection::establish(&config.database_url)
        .await
        .context("could not connect to the database")?;
    match command {
        BenchCommand::Seed { plan, out } => {
            let plan: aspen_bench_protocol::SeedPlan = serde_json::from_slice(
                &std::fs::read(&plan)
                    .with_context(|| format!("could not read {}", plan.display()))?,
            )
            .context("the plan is not a seed plan")?;
            let started = std::time::Instant::now();
            let manifest = crate::app::benchmark::seed(
                &mut conn,
                &plan,
                config.limits.max_communities_per_user,
            )
            .await
            .map_err(|e| anyhow!("{e}"))?;
            std::fs::write(&out, serde_json::to_vec(&manifest)?)
                .with_context(|| format!("could not write {}", out.display()))?;
            println!(
                "seeded run {}: {} users, {} communities in {:.1}s; manifest in {}",
                manifest.run,
                manifest.users.len(),
                manifest.communities.len(),
                started.elapsed().as_secs_f64(),
                out.display()
            );
        }
        BenchCommand::Purge { run } => {
            let media = crate::app::media_store::MediaStore::new(config)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            let report = crate::app::benchmark::purge(&mut conn, &media, &run)
                .await
                .map_err(|e| match e {
                    crate::app::Error::Diesel(diesel::result::Error::NotFound) => {
                        anyhow!("there is no benchmark run {run:?}")
                    }
                    other => anyhow!("{other}"),
                })?;
            let rows: usize = report.rows.values().sum();
            println!("purged run {run}: {rows} rows, {} files", report.objects);
            for (table, count) in &report.rows {
                println!("  {table}: {count}");
            }
            if !report.failed_objects.is_empty() {
                println!("files that could not be deleted:");
                for key in &report.failed_objects {
                    println!("  {key}");
                }
            }
        }
    }
    Ok(())
}

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
        /// A deployment permission's name: `viewDashboard`, `manageRegistrationInvites`,
        /// `manageVoiceServers`, `manageDeploymentRoles`, or `moderateCommunities`.
        permission: crate::app::deployment::DeploymentPermission,
    },
    /// Stop the deployment's top role doing something.
    Deny {
        /// A deployment permission's name, as for `allow`.
        permission: crate::app::deployment::DeploymentPermission,
    },
}

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
    /// Revoke a registration invite; accounts it made are kept.
    Revoke {
        /// The invite's code.
        code: String,
    },
}

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
    /// Replace this deployment's key. Every deployment that pinned the old one refuses the new
    /// one until its administrators accept it, so this is for a key that has leaked.
    RotateKey {
        /// Confirms the replacement.
        #[clap(long)]
        yes: bool,
    },
}

async fn database(config: &AspenConfig) -> Result<diesel_async::AsyncPgConnection> {
    use diesel_async::AsyncConnection;
    diesel_async::AsyncPgConnection::establish(&config.database_url)
        .await
        .context("could not connect to the database")
}

pub async fn admin(config: &AspenConfig, command: AdminCommand) -> Result<()> {
    use crate::app::deployment;
    use crate::database::schema::user;
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    let mut conn = database(config).await?;
    let find = |username: String| {
        user::table
            .select(user::id)
            .filter(user::name.eq(username).and(user::deleted_at.is_null()))
    };
    match command {
        AdminCommand::Grant { username } => {
            let Some(id) = find(username.clone())
                .first::<crate::app::UserId>(&mut conn)
                .await
                .optional()?
            else {
                bail!("no user is named {username:?}");
            };
            let role = deployment::grant_top_role(&mut conn, id)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            tracing::info!(%username, %role, operator = operator(), "granted the top deployment role");
            println!("{username} now holds {role}, the deployment's top role");
        }
        AdminCommand::Revoke { username } => {
            let Some(id) = find(username.clone())
                .first::<crate::app::UserId>(&mut conn)
                .await
                .optional()?
            else {
                bail!("no user is named {username:?}");
            };
            let taken = deployment::revoke_all(&mut conn, id)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            tracing::info!(%username, taken, operator = operator(), "revoked deployment roles");
            println!("{username} no longer holds any deployment role");
        }
        AdminCommand::Allow { permission } => top_role(&mut conn, permission, true).await?,
        AdminCommand::Deny { permission } => top_role(&mut conn, permission, false).await?,
        AdminCommand::List => {
            let holders = deployment::holders(&mut conn)
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
    conn: &mut diesel_async::AsyncPgConnection,
    permission: crate::app::deployment::DeploymentPermission,
    allow: bool,
) -> Result<()> {
    use crate::app::deployment;
    let role = deployment::set_top_role_permission(conn, permission, allow)
        .await
        .map_err(|e| anyhow!("{e}"))?;
    tracing::info!(%permission, allow, %role, operator = operator(), "changed the top deployment role");
    println!(
        "{role} {} {permission}; its holders see the change when their clients next load",
        if allow {
            "now allows"
        } else {
            "no longer allows"
        }
    );
    Ok(())
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
                        .and(user::name.eq(&username))
                        .and(user::deleted_at.is_null()),
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
            let invite = registration_invite::create(&mut conn, None, uses, expires_in, note)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            tracing::info!(code = %invite.code, operator = operator(), "made a registration invite");
            println!("{}", invite.code);
            if !config.registration.invite_required {
                eprintln!(
                    "note: [registration] invite_required is off, so this server does not ask for it"
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
                println!(
                    "{}  {}/{} used  {state}  {}",
                    invite.code,
                    invite.uses,
                    invite.max_uses,
                    invite.note.unwrap_or_default()
                );
            }
        }
        InvitesCommand::Revoke { code } => {
            registration_invite::revoke(&mut conn, &code)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            tracing::info!(%code, operator = operator(), "revoked a registration invite");
            println!("revoked {code}");
        }
    }
    Ok(())
}

fn print_deployment(config: &AspenConfig, listed: &crate::app::federation::Listed) {
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
        federation::admits(&config.federation, *subject, *direction, &listed.lists)
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
    let contact = async |conn: &mut diesel_async::AsyncPgConnection,
                         domain: &federation::Domain|
           -> Result<()> {
        let client = federation::fetch::client(&config.federation).map_err(fail)?;
        let key = federation::fetch_key(&config.federation, &client, domain)
            .await
            .map_err(fail)?;
        let (listed, outcome) = federation::record_contact(conn, domain, key)
            .await
            .map_err(fail)?;
        tracing::info!(%domain, ?outcome, operator = operator(), "contacted a deployment");
        match outcome {
            ContactOutcome::Pinned => println!("pinned {domain}'s key"),
            ContactOutcome::Confirmed => println!("{domain} presented its pinned key"),
            ContactOutcome::KeyChanged => {
                println!("{domain} presented a different key, which is refused until accepted")
            }
        }
        print_deployment(config, &listed);
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
            for (name, rules) in [
                ("users", &config.federation.users),
                ("bots", &config.federation.bots),
            ] {
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
            let lists: Vec<String> = federation::FederationList::all_in_force(&config.federation)
                .iter()
                .map(ToString::to_string)
                .collect();
            if !lists.is_empty() {
                println!("lists in force: {}", lists.join(", "));
            }
        }
        FederationCommand::List => {
            for listed in federation::list_all(&mut conn).await.map_err(fail)? {
                print_deployment(config, &listed);
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
            print_deployment(config, &listed);
        }
        FederationCommand::ListAdd { domain, list } => {
            let added = federation::set_listed(&mut conn, &domain, list, true, None)
                .await
                .map_err(fail)?;
            if added {
                tracing::info!(%domain, %list, operator = operator(), "put a deployment on a list");
            }
            print_deployment(
                config,
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
                config,
                &federation::get(&mut conn, &domain).await.map_err(fail)?,
            );
        }
        FederationCommand::RotateKey { yes } => {
            if !yes {
                bail!(
                    "every deployment that pinned this deployment's key will refuse the new one \
                     until its administrators accept it; pass --yes to replace it"
                );
            }
            let key = federation::rotate_key(&mut conn).await.map_err(fail)?;
            tracing::warn!(
                operator = operator(),
                "replaced this deployment's federation key"
            );
            println!("new key: {}", federation::fingerprint(&key.public_key));
        }
    }
    Ok(())
}
