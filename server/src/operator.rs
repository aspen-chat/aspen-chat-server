//! Commands an operator runs against a deployment with its own credentials, rather than through
//! the API, for powers no account should hold.
//!
//! `limits suspend` lifts rate limits for a while (see `aspen_limits::suspension`), `limits
//! resume` restores them, and `limits status` says which is in force. `bench seed` writes a
//! benchmark population straight into the database and `bench purge` removes one
//! (`app::benchmark`). `admin grant` and `admin revoke` decide who may open the Administration
//! Dashboard, which nothing over the API can do. `invites create` makes a registration invite,
//! which is how an invite-only deployment gets its first account (`app::registration_invite`).

use crate::aspen_config::AspenConfig;
use anyhow::{Context, Result, anyhow, bail};
use aspen_limits::suspension::{self, Scope, Suspension};
use clap::{Subcommand, ValueEnum};
use std::time::Duration;

#[derive(Subcommand, Debug)]
pub enum LimitsCommand {
    /// Suspend rate limits until `--for` has passed. They come back by themselves.
    Suspend {
        /// How long, as `90s`, `30m`, `2h`, or `1d`; at most `max_suspension_seconds`.
        #[clap(long = "for", default_value = "2h")]
        duration: String,
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

/// Parses `90s`, `30m`, `2h`, `1d`, or a bare number of seconds.
pub fn parse_duration(text: &str) -> Result<Duration> {
    let text = text.trim();
    let (number, unit) = text.split_at(
        text.find(|c: char| !c.is_ascii_digit())
            .unwrap_or(text.len()),
    );
    let value: u64 = number
        .parse()
        .map_err(|_| anyhow!("{text:?} is not a duration such as 90s, 30m, 2h, or 1d"))?;
    let seconds = match unit {
        "" | "s" => value,
        "m" => value * 60,
        "h" => value * 3600,
        "d" => value * 86_400,
        _ => bail!("{text:?} is not a duration such as 90s, 30m, 2h, or 1d"),
    };
    Ok(Duration::from_secs(seconds))
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
            let duration = parse_duration(&duration)?;
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
    /// Let a user open the Administration Dashboard.
    Grant {
        /// Their username.
        username: String,
    },
    /// Stop a user opening the Administration Dashboard.
    Revoke {
        /// Their username.
        username: String,
    },
    /// List the deployment's administrators.
    List,
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
        expires: Option<String>,
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

async fn database(config: &AspenConfig) -> Result<diesel_async::AsyncPgConnection> {
    use diesel_async::AsyncConnection;
    diesel_async::AsyncPgConnection::establish(&config.database_url)
        .await
        .context("could not connect to the database")
}

pub async fn admin(config: &AspenConfig, command: AdminCommand) -> Result<()> {
    use crate::database::schema::user;
    use diesel::prelude::*;
    use diesel_async::RunQueryDsl;
    let mut conn = database(config).await?;
    let set = |username: String, admin: bool| {
        diesel::update(user::table.filter(user::name.eq(username).and(user::deleted_at.is_null())))
            .set(user::admin.eq(admin))
    };
    match command {
        AdminCommand::Grant { username } => {
            if set(username.clone(), true).execute(&mut conn).await? == 0 {
                bail!("no user is named {username:?}");
            }
            tracing::info!(%username, operator = operator(), "granted administration");
            println!("{username} may now open the Administration Dashboard");
        }
        AdminCommand::Revoke { username } => {
            if set(username.clone(), false).execute(&mut conn).await? == 0 {
                bail!("no user is named {username:?}");
            }
            tracing::info!(%username, operator = operator(), "revoked administration");
            println!("{username} may no longer open the Administration Dashboard");
        }
        AdminCommand::List => {
            let names: Vec<String> = user::table
                .select(user::name)
                .filter(user::admin.eq(true).and(user::deleted_at.is_null()))
                .order(user::name)
                .load(&mut conn)
                .await?;
            if names.is_empty() {
                println!("no administrators; grant one with `admin grant <username>`");
            }
            for name in names {
                println!("{name}");
            }
        }
    }
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
            let expires_in = expires
                .as_deref()
                .map(parse_duration)
                .transpose()?
                .map(|d| chrono::Duration::from_std(d).unwrap_or(chrono::Duration::MAX));
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

#[cfg(test)]
mod tests {
    use super::parse_duration;
    use std::time::Duration;

    #[test]
    fn durations_parse_with_units() {
        assert_eq!(parse_duration("90s").unwrap(), Duration::from_secs(90));
        assert_eq!(parse_duration("30m").unwrap(), Duration::from_secs(1800));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86_400));
        assert_eq!(parse_duration("45").unwrap(), Duration::from_secs(45));
        assert!(parse_duration("2 hours").is_err());
        assert!(parse_duration("h").is_err());
    }
}
