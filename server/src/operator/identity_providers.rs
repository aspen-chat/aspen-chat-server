//! `identity-providers list`, `add`, `set`, and `remove`: the OpenID Connect providers people
//! sign in through (`app::identity_provider`), as Manage identity providers keeps them from the
//! dashboard. A client secret is read from a file, or from standard input with `-`, never from
//! the command line, where other users of the machine and the shell's history would see it.

use super::database;
use anyhow::{Context, Result, anyhow, bail};
use aspen_app::IdentityProviderId;
use aspen_app::aspen_config::AspenConfig;
use aspen_app::identity_provider::{self as providers, ProviderChanges, ProviderSettings};
use clap::Subcommand;
use std::io::Read as _;
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum IdentityProvidersCommand {
    /// List every provider, and the redirect URI to register at each.
    List,
    /// Add a provider, once its discovery document names the issuer given.
    Add {
        /// What its sign-in button says.
        name: String,
        /// The issuer URL; `{issuer}/.well-known/openid-configuration` must name it.
        #[clap(long)]
        issuer: String,
        #[clap(long)]
        client_id: String,
        /// A file holding the client secret, or `-` for standard input.
        #[clap(long)]
        client_secret_file: PathBuf,
        /// Add it turned off.
        #[clap(long)]
        disabled: bool,
        /// A first sign-in makes an account without a registration invite where one is required.
        #[clap(long)]
        skip_invite: bool,
        /// Every sign-in there counts as two factors, whatever it reports.
        #[clap(long)]
        enforces_two_factor: bool,
        /// Do not take the addresses it reports as verified.
        #[clap(long)]
        distrust_email: bool,
    },
    /// Change a provider, named by its name or id. Its issuer and client id are fixed once it is
    /// added: to change either, add another provider and have its people link it.
    Set {
        provider: String,
        #[clap(long)]
        name: Option<String>,
        /// A file holding a new client secret, or `-` for standard input.
        #[clap(long)]
        client_secret_file: Option<PathBuf>,
        #[clap(long)]
        enabled: Option<bool>,
        #[clap(long)]
        skip_invite: Option<bool>,
        #[clap(long)]
        enforces_two_factor: Option<bool>,
        #[clap(long)]
        trust_email: Option<bool>,
    },
    /// Remove a provider, named by its name or id. Its links are deleted in the background, and
    /// accounts whose only way in it was can no longer sign in.
    Remove { provider: String },
}

fn print(provider: &ProviderSettings) {
    let linked = if provider.linked_accounts > providers::LINKED_COUNT_CAP {
        format!("{}+", providers::LINKED_COUNT_CAP)
    } else {
        provider.linked_accounts.to_string()
    };
    let mut flags = Vec::new();
    if !provider.enabled {
        flags.push("disabled");
    }
    if provider.skip_invite {
        flags.push("skips invites");
    }
    if provider.enforces_two_factor {
        flags.push("enforces two factors");
    }
    if !provider.trust_email {
        flags.push("email not trusted");
    }
    println!(
        "{}  {}  client {}  {linked} linked  id {}{}",
        provider.name,
        provider.issuer,
        provider.client_id,
        provider.id.0,
        if flags.is_empty() {
            String::new()
        } else {
            format!("  ({})", flags.join(", "))
        }
    );
}

fn read_secret(path: &PathBuf) -> Result<String> {
    let mut secret = String::new();
    if path.as_os_str() == "-" {
        std::io::stdin()
            .read_to_string(&mut secret)
            .context("reading the client secret from standard input")?;
    } else {
        secret = std::fs::read_to_string(path)
            .with_context(|| format!("reading the client secret from {}", path.display()))?;
    }
    let secret = secret.trim().to_string();
    if secret.is_empty() {
        bail!("the client secret is empty");
    }
    Ok(secret)
}

fn named<'a>(listed: &'a [ProviderSettings], provider: &str) -> Result<&'a ProviderSettings> {
    if let Ok(id) = provider.parse::<uuid::Uuid>()
        && let Some(found) = listed.iter().find(|p| p.id == IdentityProviderId::from(id))
    {
        return Ok(found);
    }
    let mut matching = listed.iter().filter(|p| p.name == provider);
    match (matching.next(), matching.next()) {
        (Some(found), None) => Ok(found),
        (Some(_), Some(_)) => bail!("more than one provider is called {provider}; name it by id"),
        (None, _) => Err(anyhow!(
            "no identity provider is called {provider}; `identity-providers list` lists them"
        )),
    }
}

pub async fn identity_providers(
    config: &AspenConfig,
    command: IdentityProvidersCommand,
) -> Result<()> {
    let fail = |e: aspen_app::Error| anyhow!("{e}");
    providers::discovery::prepare(config).map_err(fail)?;
    let mut conn = database(config).await?;
    let listed = providers::list_in(&mut conn).await.map_err(fail)?;
    match command {
        IdentityProvidersCommand::List => {
            listed.iter().for_each(print);
            let base = config.public_url.trim_end_matches('/');
            println!(
                "\nRegister this redirect URI at each provider: {base}{}\nand, where it offers back-channel logout, this logout URI: {base}{}",
                providers::CALLBACK_PATH,
                providers::BACK_CHANNEL_LOGOUT_PATH
            );
        }
        IdentityProvidersCommand::Add {
            name,
            issuer,
            client_id,
            client_secret_file,
            disabled,
            skip_invite,
            enforces_two_factor,
            distrust_email,
        } => {
            let added = providers::add_in(
                config,
                &mut conn,
                providers::NewProvider {
                    name,
                    issuer,
                    client_id,
                    client_secret: read_secret(&client_secret_file)?,
                    enabled: !disabled,
                    skip_invite,
                    enforces_two_factor,
                    trust_email: !distrust_email,
                },
            )
            .await
            .map_err(fail)?;
            tracing::info!(
                name = added.name,
                issuer = added.issuer,
                "added an identity provider"
            );
            print(&added);
        }
        IdentityProvidersCommand::Set {
            provider,
            name,
            client_secret_file,
            enabled,
            skip_invite,
            enforces_two_factor,
            trust_email,
        } => {
            let id = named(&listed, &provider)?.id;
            let changes = ProviderChanges {
                name,
                client_secret: client_secret_file.as_ref().map(read_secret).transpose()?,
                enabled,
                skip_invite,
                enforces_two_factor,
                trust_email,
            };
            let changed = providers::update_in(&mut conn, id, changes)
                .await
                .map_err(fail)?;
            tracing::info!(name = changed.name, "changed an identity provider");
            print(&changed);
        }
        IdentityProvidersCommand::Remove { provider } => {
            let found = named(&listed, &provider)?;
            providers::remove_in(&mut conn, found.id)
                .await
                .map_err(fail)?;
            tracing::info!(name = found.name, "removed an identity provider");
            println!(
                "Removed {}. Its links are deleted in the background; sign-ins it made last until they end.",
                found.name
            );
        }
    }
    Ok(())
}
