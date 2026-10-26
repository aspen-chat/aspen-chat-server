//! `documents show`, `set`, and `withdraw`: the deployment's Privacy Policy and Terms of Use
//! (`app::deployment_document`), as Manage deployment settings keeps them from the dashboard.

use super::{database, operator};
use anyhow::{Context, Result, anyhow};
use aspen_app::aspen_config::AspenConfig;
use aspen_app::deployment_document::{self, DocumentKind};
use clap::Subcommand;
use std::io::Read;
use std::path::PathBuf;

#[derive(Subcommand, Debug)]
pub enum DocumentsCommand {
    /// Print a document's Markdown, or say the deployment has not published it.
    Show {
        /// `privacyPolicy` or `termsOfUse`.
        document: DocumentKind,
    },
    /// Publish a document from a Markdown file, in place of the one published before.
    Set {
        /// `privacyPolicy` or `termsOfUse`.
        document: DocumentKind,
        /// The Markdown file; `-` reads it from standard input.
        file: PathBuf,
    },
    /// Withdraw a document, leaving the deployment without one.
    Withdraw {
        /// `privacyPolicy` or `termsOfUse`.
        document: DocumentKind,
    },
}

pub async fn documents(config: &AspenConfig, command: DocumentsCommand) -> Result<()> {
    let mut conn = database(config).await?;
    match command {
        DocumentsCommand::Show { document } => {
            match deployment_document::load(&mut conn, document)
                .await
                .map_err(|e| anyhow!("{e}"))?
            {
                Some(published) => {
                    eprintln!("published {}", published.updated_at.to_rfc3339());
                    println!("{}", published.body);
                }
                None => eprintln!("this deployment has not published its {document}"),
            }
        }
        DocumentsCommand::Set { document, file } => {
            let body = if file.as_os_str() == "-" {
                let mut body = String::new();
                std::io::stdin()
                    .read_to_string(&mut body)
                    .context("reading the document from standard input")?;
                body
            } else {
                std::fs::read_to_string(&file)
                    .with_context(|| format!("reading {}", file.display()))?
            };
            deployment_document::publish(&mut conn, document, body)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            tracing::info!(%document, operator = operator(), "published a deployment document");
        }
        DocumentsCommand::Withdraw { document } => {
            if deployment_document::withdraw(&mut conn, document)
                .await
                .map_err(|e| anyhow!("{e}"))?
            {
                tracing::info!(%document, operator = operator(), "withdrew a deployment document");
            } else {
                eprintln!("this deployment had not published its {document}");
            }
        }
    }
    Ok(())
}
