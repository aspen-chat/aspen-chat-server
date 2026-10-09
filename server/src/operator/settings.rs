//! `settings show` and `settings set`: the deployment's settings (`app::deployment_settings`),
//! every one of them, as the Administration Dashboard changes them under Manage deployment
//! settings and Manage federation. An invite-only deployment is made one here before anyone
//! has an account.

use super::{database, operator, publisher};
use anyhow::{Result, anyhow};
use aspen_app::aspen_config::AspenConfig;
use aspen_app::deployment_settings::{self, DeploymentSettings, SettingsChange};
use aspen_app::events::{noting, settle_in};
use aspen_app::federation::{Gate, own_domain};
use clap::Subcommand;
use serde::Serialize;

#[derive(Subcommand, Debug)]
pub enum SettingsCommand {
    /// Show every setting.
    Show,
    /// Change settings, for every server at once; those not named are left as they are.
    Set(SettingsArgs),
}

/// The settings, each named as its flag is. `settings show` prints them under the same names.
#[derive(clap::Args, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub struct SettingsArgs {
    /// What the deployment calls itself: on its sign-in screens, as its system account, and to
    /// its users' authenticator apps and passkey prompts. Empty for none, which they show as
    /// "Aspen".
    #[clap(long)]
    display_name: Option<String>,
    /// Whether creating an account takes a registration invite (`invites create`).
    #[clap(long)]
    registration_invite_required: Option<bool>,
    /// Whether every person's account must have a second factor.
    #[clap(long)]
    require_two_factor: Option<bool>,
    /// Whether people may make bots. Bots already made keep working either way.
    #[clap(long)]
    bots_enabled: Option<bool>,
    /// The most bots one person may own.
    #[clap(long)]
    bots_max_per_user: Option<u32>,
    /// How many members a community gains before Mention everyone is taken from its everyone
    /// role; 0 never.
    #[clap(long)]
    everyone_mention_limit: Option<u32>,
    /// The most custom emoji one community may hold.
    #[clap(long)]
    custom_emoji_limit: Option<u32>,
    /// How many GiB one person may upload in any 24 hours; 0 sets no limit.
    #[clap(long)]
    upload_quota_gib: Option<u32>,
    /// How many days the files of deleted messages are kept for reviewing reports, past any
    /// report case about them; 0 keeps them for good.
    #[clap(long)]
    evidence_retention_days: Option<u32>,
    /// Whether people may offer files to one another in calls.
    #[clap(long)]
    file_transfers: Option<bool>,
    /// Whether registering takes an email address. Needs `[email]` in aspen.toml.
    #[clap(long)]
    email_required: Option<bool>,
    /// Whether an account must verify its email address before using the deployment.
    #[clap(long)]
    email_verification_required: Option<bool>,
    /// Whether the deployment has a newsletter its users may subscribe to.
    #[clap(long)]
    newsletter_enabled: Option<bool>,
    /// Whether this deployment's users may use others: closed, open, allowList, or blockList.
    #[clap(long)]
    users_emigration: Option<Gate>,
    /// Whether other deployments' users may use this one: closed, open, allowList, or
    /// blockList.
    #[clap(long)]
    users_immigration: Option<Gate>,
    /// Whether users' two gates read one list, which needs both to be the same kind of list.
    #[clap(long)]
    users_shared_list: Option<bool>,
    /// Whether another deployment's user arriving for the first time needs a registration
    /// invite.
    #[clap(long)]
    users_immigration_invite_required: Option<bool>,
    /// Whether this deployment's bots may use others.
    #[clap(long)]
    bots_emigration: Option<Gate>,
    /// Whether other deployments' bots may use this one.
    #[clap(long)]
    bots_immigration: Option<Gate>,
    /// Whether bots' two gates read one list.
    #[clap(long)]
    bots_shared_list: Option<bool>,
    /// Whether another deployment's bot arriving for the first time needs a registration
    /// invite.
    #[clap(long)]
    bots_immigration_invite_required: Option<bool>,
}

impl From<SettingsArgs> for SettingsChange {
    fn from(args: SettingsArgs) -> Self {
        Self {
            display_name: args
                .display_name
                .map(|name| Some(name).filter(|name| !name.trim().is_empty())),
            icon: None,
            registration_invite_required: args.registration_invite_required,
            require_two_factor: args.require_two_factor,
            bots_enabled: args.bots_enabled,
            bots_max_per_user: args.bots_max_per_user,
            everyone_mention_limit: args.everyone_mention_limit,
            custom_emoji_limit: args.custom_emoji_limit,
            upload_quota_gib: args.upload_quota_gib,
            evidence_retention_days: args.evidence_retention_days,
            file_transfers: args.file_transfers,
            email_required: args.email_required,
            email_verification_required: args.email_verification_required,
            newsletter_enabled: args.newsletter_enabled,
            users_emigration: args.users_emigration,
            users_immigration: args.users_immigration,
            users_shared_list: args.users_shared_list,
            users_immigration_invite_required: args.users_immigration_invite_required,
            bots_emigration: args.bots_emigration,
            bots_immigration: args.bots_immigration,
            bots_shared_list: args.bots_shared_list,
            bots_immigration_invite_required: args.bots_immigration_invite_required,
        }
    }
}

impl From<&DeploymentSettings> for SettingsArgs {
    fn from(settings: &DeploymentSettings) -> Self {
        let users = &settings.federation.users;
        let bots = &settings.federation.bots;
        Self {
            display_name: settings.display_name.clone(),
            registration_invite_required: Some(settings.registration_invite_required),
            require_two_factor: Some(settings.require_two_factor),
            bots_enabled: Some(settings.bots_enabled),
            bots_max_per_user: Some(settings.bots_max_per_user),
            everyone_mention_limit: Some(settings.everyone_mention_limit),
            custom_emoji_limit: Some(settings.custom_emoji_limit),
            upload_quota_gib: Some(settings.upload_quota_gib),
            evidence_retention_days: Some(settings.evidence_retention_days),
            file_transfers: Some(settings.file_transfers),
            email_required: Some(settings.email_required),
            email_verification_required: Some(settings.email_verification_required),
            newsletter_enabled: Some(settings.newsletter_enabled),
            users_emigration: Some(users.emigration),
            users_immigration: Some(users.immigration),
            users_shared_list: Some(users.shared_list),
            users_immigration_invite_required: Some(users.immigration_invite_required),
            bots_emigration: Some(bots.emigration),
            bots_immigration: Some(bots.immigration),
            bots_shared_list: Some(bots.shared_list),
            bots_immigration_invite_required: Some(bots.immigration_invite_required),
        }
    }
}

/// Prints every setting, one per line, under its flag's name.
fn print(settings: &DeploymentSettings) -> Result<()> {
    let serde_json::Value::Object(fields) = serde_json::to_value(SettingsArgs::from(settings))?
    else {
        unreachable!("the settings serialize as an object");
    };
    for (name, value) in fields {
        match value {
            serde_json::Value::Null => println!("{name}: none"),
            serde_json::Value::String(text) => println!("{name}: {text}"),
            other => println!("{name}: {other}"),
        }
    }
    if let Some(domain) = &settings.federation_domain {
        println!("federation domain: {domain}");
    }
    Ok(())
}

pub async fn settings(config: &AspenConfig, command: SettingsCommand) -> Result<()> {
    let fail = |e: aspen_app::Error| anyhow!("{e}");
    let mut conn = database(config).await?;
    match command {
        SettingsCommand::Show => {
            print(&deployment_settings::load(&mut conn).await.map_err(fail)?)?;
        }
        SettingsCommand::Set(args) => {
            // A gate opens only with a domain, which may be named before any server has run.
            deployment_settings::pin_domain(&mut conn, own_domain(&config.federation).as_ref())
                .await
                .map_err(fail)?;
            let publisher = publisher(config).await?;
            let (changed, noted) = noting(deployment_settings::update(
                &publisher,
                &mut conn,
                config.email.is_some(),
                args.into(),
            ))
            .await;
            settle_in(&publisher, &mut conn, noted, changed.is_err()).await;
            let changed = changed.map_err(fail)?;
            tracing::info!(
                revision = changed.revision,
                operator = operator(),
                "changed the deployment's settings"
            );
            print(&changed)?;
        }
    }
    Ok(())
}
