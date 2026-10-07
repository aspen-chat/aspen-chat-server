//! `attachments purge`: deletes evidence outright (`app::attachment::evidence::purge`), the
//! files of deleted messages and attachments taken off their messages, which are otherwise kept
//! for reviewing reports.

use super::{database, operator};
use anyhow::{Result, anyhow};
use aspen_app::aspen_config::AspenConfig;
use aspen_app::attachment::evidence::{self, PurgeError, PurgeTarget};
use aspen_app::{AttachmentId, MessageId};
use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum AttachmentsCommand {
    /// Delete for good, rows and files, what is kept of a deleted message's attachments, or of
    /// attachments taken off a message, which reviewers of reports could otherwise still read.
    /// Logged in the moderation log.
    Purge {
        /// Every kept attachment of this message: its own, once it is deleted, and those taken
        /// off it.
        #[clap(
            long,
            conflicts_with = "attachment",
            required_unless_present = "attachment"
        )]
        message: Option<uuid::Uuid>,
        /// This kept attachment alone.
        #[clap(long)]
        attachment: Option<uuid::Uuid>,
    },
}

pub async fn attachments(config: &AspenConfig, command: AttachmentsCommand) -> Result<()> {
    match command {
        AttachmentsCommand::Purge {
            message,
            attachment,
        } => {
            let target = match (message, attachment) {
                (Some(message), _) => PurgeTarget::Message(MessageId(message)),
                (None, Some(attachment)) => PurgeTarget::Attachment(AttachmentId(attachment)),
                (None, None) => return Err(anyhow!("give --message or --attachment")),
            };
            let mut conn = database(config).await?;
            let store = aspen_app::media_store::MediaStore::new(config)
                .await
                .map_err(|e| anyhow!("{e}"))?;
            let purged = evidence::purge(&mut conn, &store, target)
                .await
                .map_err(|e| match e {
                    PurgeError::App(e) => anyhow!("{e}"),
                    other => anyhow!("{other}"),
                })?;
            if purged.is_empty() {
                println!("nothing is kept for that message");
                return Ok(());
            }
            for item in &purged {
                tracing::info!(
                    attachment = %item.id.0,
                    message = ?item.message.map(|m| m.0),
                    operator = operator(),
                    "purged an attachment kept as evidence"
                );
                println!("purged {} ({})", item.id.0, item.file_name);
                for key in &item.failed_objects {
                    println!(
                        "  could not delete the object {key}; delete it from the storage by hand"
                    );
                }
            }
            Ok(())
        }
    }
}
