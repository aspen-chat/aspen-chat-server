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
    /// How many members a community gains before it is large: its everyone role loses Mention
    /// everyone, and its channels notify of tags rather than every message. With 0 no
    /// community crosses it, and every community's channels notify of tags.
    #[clap(long)]
    large_community_members: Option<u32>,
    /// The most custom emoji one community may hold.
    #[clap(long)]
    custom_emoji_limit: Option<u32>,
    /// How many GiB one person may upload in any 24 hours; 0 sets no limit.
    #[clap(long)]
    upload_quota_gib: Option<u32>,
    /// The most mana any channel recharges a second, and the rate of channels that set none.
    #[clap(long)]
    mana_max_recharge: Option<u32>,
    /// The most mana a channel holds where it sets none, and the most it may set.
    #[clap(long)]
    mana_max: Option<u32>,
    /// What an attachment costs, in the mana a glyph costs.
    #[clap(long)]
    mana_attachment_cost: Option<u32>,
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
    /// How many MiB of NATS's memory the events of the last minute may take, at least 16, or
    /// `auto` to size their stream to what NATS has.
    #[clap(long)]
    event_stream_mib: Option<StreamSize>,
    /// The same for the voice servers' reports not yet applied.
    #[clap(long)]
    voice_report_stream_mib: Option<StreamSize>,
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

/// A stream's size as the terminal takes and prints it: a count of MiB, or `auto`.
#[derive(Debug, Clone, Copy)]
struct StreamSize(Option<u32>);

const AUTOMATIC: &str = "auto";

impl std::str::FromStr for StreamSize {
    type Err = std::num::ParseIntError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text == AUTOMATIC {
            return Ok(Self(None));
        }
        text.parse().map(|mib| Self(Some(mib)))
    }
}

impl Serialize for StreamSize {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.0 {
            Some(mib) => serializer.serialize_u32(mib),
            None => serializer.serialize_str(AUTOMATIC),
        }
    }
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
            large_community_members: args.large_community_members,
            custom_emoji_limit: args.custom_emoji_limit,
            upload_quota_gib: args.upload_quota_gib,
            mana_max_recharge: args.mana_max_recharge,
            mana_max: args.mana_max,
            mana_attachment_cost: args.mana_attachment_cost,
            evidence_retention_days: args.evidence_retention_days,
            file_transfers: args.file_transfers,
            email_required: args.email_required,
            email_verification_required: args.email_verification_required,
            newsletter_enabled: args.newsletter_enabled,
            event_stream_mib: args.event_stream_mib.map(|size| size.0),
            voice_report_stream_mib: args.voice_report_stream_mib.map(|size| size.0),
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
            large_community_members: Some(settings.large_community_members),
            custom_emoji_limit: Some(settings.custom_emoji_limit),
            upload_quota_gib: Some(settings.upload_quota_gib),
            mana_max_recharge: Some(settings.mana_max_recharge),
            mana_max: Some(settings.mana_max),
            mana_attachment_cost: Some(settings.mana_attachment_cost),
            evidence_retention_days: Some(settings.evidence_retention_days),
            file_transfers: Some(settings.file_transfers),
            email_required: Some(settings.email_required),
            email_verification_required: Some(settings.email_verification_required),
            newsletter_enabled: Some(settings.newsletter_enabled),
            event_stream_mib: Some(StreamSize(settings.event_stream_mib)),
            voice_report_stream_mib: Some(StreamSize(settings.voice_report_stream_mib)),
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
            let monitor = aspen_app::nats_memory::Monitor::new(config).map_err(fail)?;
            let (changed, noted) = noting(deployment_settings::update(
                &publisher,
                &mut conn,
                config.email.is_some(),
                &monitor,
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
