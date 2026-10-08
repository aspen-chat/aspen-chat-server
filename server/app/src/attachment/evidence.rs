//! The files of deleted messages, and attachments taken off their messages, kept as evidence for
//! reviewing reports and out of everyone else's reach.
//!
//! When the last message standing that holds an attachment is deleted ([`keep_deleted`]), or an
//! attachment is taken off its message and is in no other ([`keep_removed`], which records the
//! message as `removed_from`), the attachment becomes evidence (`evidence_at`), in the
//! transaction that makes the change: from then on it is read only in reviewing a report, its
//! preview is no longer made, and no message may take it up again. Soon after, every API server's
//! mover ([`spawn_mover`]) moves its object and its preview from the anonymous read path to
//! `media_store::EVIDENCE_PREFIX`, which that path never serves, and records the new keys. Reviewers read
//! them through URLs signed for [`URL_LIFETIME`] ([`signed_urls`]) when a case shows the message
//! they belong to. Nothing deletes evidence but an operator's [`purge`]
//! (`aspen-chat-server attachments purge`), which the moderation log records.

use crate::context::GlobalServerContext;
use crate::media_store::{MediaStore, evidence_key};
use crate::moderation_log::{ModerationAction, log_operator_moderation};
use crate::{AttachmentId, ChannelId, MessageId};
use aspen_schema::{attachment, channel, message, message_attachment};
use diesel::prelude::*;
use diesel::sql_types::{Array, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use std::time::Duration;

/// How long a reviewer's URL to a piece of evidence reads it: long enough to look at a case,
/// short enough that a URL copied out of the dashboard soon stops working.
pub const URL_LIFETIME: Duration = Duration::from_secs(10 * 60);

/// How often each server looks for evidence still on the anonymous read path. A deleted
/// message's files stay readable there, by whoever already holds their URLs, until then.
const MOVE_EVERY: Duration = Duration::from_secs(5);

/// How many attachments one look moves at most before looking again.
const MOVE_BATCH: i64 = 50;

#[derive(QueryableByName)]
struct Marked {
    #[diesel(sql_type = PgUuid)]
    id: uuid::Uuid,
}

/// Locks the rows of `ids`, so two changes that each leave an attachment in no message standing
/// cannot both miss that the other did: the second waits, then reads what the first committed.
async fn lock(conn: &mut AsyncPgConnection, ids: &[uuid::Uuid]) -> crate::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    attachment::table
        .select(attachment::id)
        .filter(attachment::id.eq_any(ids.iter().copied().map(AttachmentId)))
        .order(attachment::id)
        .for_update()
        .load::<AttachmentId>(conn)
        .await?;
    Ok(())
}

/// Makes evidence of those among `ids` in no message standing, recording `removed_from`, and
/// drops their preview jobs.
async fn mark(
    conn: &mut AsyncPgConnection,
    ids: &[uuid::Uuid],
    removed_from: Option<MessageId>,
) -> crate::Result<Vec<AttachmentId>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    lock(conn, ids).await?;
    let marked: Vec<Marked> = diesel::sql_query(
        "UPDATE attachment a SET evidence_at = now(), \
             removed_from = COALESCE($2, a.removed_from) \
         WHERE a.id = ANY($1) AND a.evidence_at IS NULL AND a.ready_at IS NOT NULL \
           AND NOT EXISTS ( \
               SELECT 1 FROM message_attachment ma JOIN message m ON m.id = ma.message_id \
               WHERE ma.attachment_id = a.id AND m.deleted_at IS NULL) \
         RETURNING a.id",
    )
    .bind::<Array<PgUuid>, _>(ids)
    .bind::<diesel::sql_types::Nullable<PgUuid>, _>(removed_from.map(|m| m.0))
    .load(conn)
    .await?;
    let marked: Vec<AttachmentId> = marked.into_iter().map(|m| AttachmentId(m.id)).collect();
    if !marked.is_empty() {
        diesel::delete(aspen_schema::attachment_preview_job::table)
            .filter(aspen_schema::attachment_preview_job::attachment_id.eq_any(&marked))
            .execute(conn)
            .await?;
    }
    Ok(marked)
}

/// Makes evidence of the attachments of `message`, which the caller's transaction has just
/// deleted, that no message standing holds.
pub async fn keep_deleted(
    conn: &mut AsyncPgConnection,
    message: MessageId,
) -> crate::Result<Vec<AttachmentId>> {
    let ids: Vec<AttachmentId> = message_attachment::table
        .select(message_attachment::attachment_id)
        .filter(message_attachment::message_id.eq(message))
        .load(conn)
        .await?;
    let ids: Vec<uuid::Uuid> = ids.into_iter().map(|id| id.0).collect();
    mark(conn, &ids, None).await
}

/// As [`keep_deleted`], for every one of `messages` at once.
pub async fn keep_deleted_many(
    conn: &mut AsyncPgConnection,
    messages: &[MessageId],
) -> crate::Result<Vec<AttachmentId>> {
    let ids: Vec<AttachmentId> = message_attachment::table
        .select(message_attachment::attachment_id)
        .filter(message_attachment::message_id.eq_any(messages))
        .load(conn)
        .await?;
    let ids: Vec<uuid::Uuid> = ids.into_iter().map(|id| id.0).collect();
    mark(conn, &ids, None).await
}

/// Makes evidence of `removed`, which the caller's transaction has just taken off `message`,
/// those that no message standing holds, recording the message they were taken off.
pub async fn keep_removed(
    conn: &mut AsyncPgConnection,
    message: MessageId,
    removed: &[AttachmentId],
) -> crate::Result<Vec<AttachmentId>> {
    let ids: Vec<uuid::Uuid> = removed.iter().map(|id| id.0).collect();
    mark(conn, &ids, Some(message)).await
}

#[derive(QueryableByName)]
struct Unmoved {
    #[diesel(sql_type = PgUuid)]
    id: uuid::Uuid,
    #[diesel(sql_type = diesel::sql_types::Text)]
    storage_key: String,
    #[diesel(sql_type = diesel::sql_types::Nullable<diesel::sql_types::Text>)]
    preview_storage_key: Option<String>,
}

/// Moves the objects of evidence still on the anonymous read path under `media_store::EVIDENCE_PREFIX`,
/// answering how many attachments it moved. Servers may run it at once: a move is a copy and a
/// deletion, and the keys are recorded only where they are still the ones moved from.
pub async fn move_unmoved(state: &GlobalServerContext) -> crate::Result<usize> {
    let mut total = 0;
    loop {
        let unmoved: Vec<Unmoved> = diesel::sql_query(
            "SELECT id, storage_key, preview_storage_key FROM attachment \
             WHERE evidence_at IS NOT NULL AND storage_key NOT LIKE 'evidence/%' \
             ORDER BY evidence_at LIMIT $1",
        )
        .bind::<diesel::sql_types::BigInt, _>(MOVE_BATCH)
        .load(state.connection_pool.get().await?.as_mut())
        .await?;
        let count = unmoved.len();
        let mut moved = 0;
        for row in unmoved {
            match move_one(state, &row).await {
                Ok(true) => moved += 1,
                Ok(false) => {}
                Err(e) => tracing::warn!(
                    attachment = %row.id,
                    error = %e,
                    "could not move evidence off the public read path; trying again soon"
                ),
            }
        }
        total += moved;
        // A batch that moved nothing is failing, and is tried again at the next look.
        if count < MOVE_BATCH as usize || moved == 0 {
            return Ok(total);
        }
    }
}

async fn move_one(state: &GlobalServerContext, row: &Unmoved) -> crate::Result<bool> {
    let store = &state.media_store;
    let storage_key = evidence_key(&row.storage_key);
    store.move_object(&row.storage_key, &storage_key).await?;
    let preview_storage_key = match &row.preview_storage_key {
        Some(key) => {
            let to = evidence_key(key);
            store.move_object(key, &to).await?;
            Some(to)
        }
        None => None,
    };
    let updated = diesel::update(attachment::table)
        .filter(
            attachment::id
                .eq(AttachmentId(row.id))
                .and(attachment::storage_key.eq(&row.storage_key)),
        )
        .set((
            attachment::storage_key.eq(&storage_key),
            attachment::preview_storage_key.eq(&preview_storage_key),
        ))
        .execute(state.connection_pool.get().await?.as_mut())
        .await?;
    Ok(updated == 1)
}

/// Looks for evidence to move every [`MOVE_EVERY`], on every server, for as long as it runs.
pub fn spawn_mover(state: GlobalServerContext) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(MOVE_EVERY).await;
            match move_unmoved(&state).await {
                Ok(0) => {}
                Ok(moved) => tracing::info!(moved, "moved evidence off the public read path"),
                Err(e) => tracing::warn!(error = %e, "could not look for evidence to move"),
            }
        }
    });
}

/// URLs reading an attachment kept as evidence and its preview, signed for [`URL_LIFETIME`],
/// for a reviewer. They name its keys as they stand, before its move or after.
pub async fn signed_urls(
    state: &GlobalServerContext,
    row: &super::Attachment,
) -> crate::Result<(String, Option<String>)> {
    let store = &state.media_store;
    let original = store.presign_get(&row.storage_key, URL_LIFETIME).await?;
    let preview = match &row.preview_storage_key {
        Some(key) => Some(store.presign_get(key, URL_LIFETIME).await?),
        None => None,
    };
    Ok((original, preview))
}

/// The confirmed attachments among `ids`, evidence included, for a review, which alone reads
/// evidence.
pub async fn read_for_review(
    state: &GlobalServerContext,
    ids: &[AttachmentId],
) -> crate::Result<Vec<super::Attachment>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    Ok(attachment::table
        .select(super::Attachment::as_select())
        .filter(
            attachment::id
                .eq_any(ids)
                .and(attachment::ready_at.is_not_null()),
        )
        .load(state.connection_pool.get().await?.as_mut())
        .await?)
}

/// The evidence taken off each of `messages`, by message.
pub async fn removed_from(
    conn: &mut AsyncPgConnection,
    messages: &[MessageId],
) -> crate::Result<Vec<(MessageId, AttachmentId)>> {
    if messages.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<(Option<MessageId>, AttachmentId)> = attachment::table
        .select((attachment::removed_from, attachment::id))
        .filter(attachment::removed_from.eq_any(messages))
        .filter(attachment::evidence_at.is_not_null())
        .order(attachment::id)
        .load(conn)
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(message, id)| Some((message?, id)))
        .collect())
}

/// What an operator's purge is of.
#[derive(Debug, Clone, Copy)]
pub enum PurgeTarget {
    /// Every piece of evidence of a message: its attachments, once it is deleted, and those
    /// taken off it.
    Message(MessageId),
    Attachment(AttachmentId),
}

/// One attachment purged.
#[derive(Debug, Clone)]
pub struct Purged {
    pub id: AttachmentId,
    pub file_name: String,
    /// The message it was in, or taken off.
    pub message: Option<MessageId>,
    /// Objects that could not be deleted, which the operator deletes by hand.
    pub failed_objects: Vec<String>,
}

/// Why nothing was purged.
#[derive(Debug, thiserror::Error)]
pub enum PurgeError {
    #[error("there is no such attachment or message")]
    NotFound,
    /// The attachment is in a message standing, so it is not evidence.
    #[error(
        "that attachment is in a message that stands, so it is not evidence; delete the message \
         or take the attachment off it first"
    )]
    NotEvidence,
    #[error("{0}")]
    App(#[from] crate::Error),
}

impl From<diesel::result::Error> for PurgeError {
    fn from(error: diesel::result::Error) -> Self {
        Self::App(error.into())
    }
}

/// Deletes evidence outright, rows and objects, writing each attachment to the moderation log
/// as purged from the terminal. The rows go in one transaction; the objects after it, each
/// tried at every key it may be at, so evidence not yet moved goes too.
pub async fn purge(
    conn: &mut AsyncPgConnection,
    store: &MediaStore,
    target: PurgeTarget,
) -> Result<Vec<Purged>, PurgeError> {
    let rows = conn
        .transaction::<_, PurgeError, _>(|conn| {
            async move {
                let ids: Vec<AttachmentId> = match target {
                    PurgeTarget::Attachment(id) => {
                        let evidence: Option<bool> = attachment::table
                            .select(attachment::evidence_at.is_not_null())
                            .filter(attachment::id.eq(id))
                            .first(conn)
                            .await
                            .optional()?;
                        match evidence {
                            None => return Err(PurgeError::NotFound),
                            Some(false) => return Err(PurgeError::NotEvidence),
                            Some(true) => vec![id],
                        }
                    }
                    PurgeTarget::Message(message_id) => {
                        let exists: Option<MessageId> = message::table
                            .select(message::id)
                            .filter(message::id.eq(message_id))
                            .first(conn)
                            .await
                            .optional()?;
                        if exists.is_none() {
                            return Err(PurgeError::NotFound);
                        }
                        // What it holds and what was taken off it, each found through its
                        // own index, then those of them kept as evidence.
                        let mut named: Vec<AttachmentId> = message_attachment::table
                            .select(message_attachment::attachment_id)
                            .filter(message_attachment::message_id.eq(message_id))
                            .load(conn)
                            .await?;
                        named.extend(
                            attachment::table
                                .select(attachment::id)
                                .filter(attachment::removed_from.eq(message_id))
                                .load::<AttachmentId>(conn)
                                .await?,
                        );
                        attachment::table
                            .select(attachment::id)
                            .filter(attachment::evidence_at.is_not_null())
                            .filter(attachment::id.eq_any(&named))
                            .load(conn)
                            .await?
                    }
                };
                let raw: Vec<uuid::Uuid> = ids.iter().map(|id| id.0).collect();
                lock(conn, &raw).await?;
                // Where each was, for the log: the message it is in, or was taken off.
                let in_messages: Vec<(AttachmentId, MessageId)> = message_attachment::table
                    .select((
                        message_attachment::attachment_id,
                        message_attachment::message_id,
                    ))
                    .filter(message_attachment::attachment_id.eq_any(&ids))
                    .load(conn)
                    .await?;
                let removed_from: std::collections::HashMap<AttachmentId, MessageId> =
                    attachment::table
                        .select((attachment::id, attachment::removed_from))
                        .filter(attachment::id.eq_any(&ids))
                        .load::<(AttachmentId, Option<MessageId>)>(conn)
                        .await?
                        .into_iter()
                        .filter_map(|(id, message)| Some((id, message?)))
                        .collect();
                diesel::delete(message_attachment::table)
                    .filter(message_attachment::attachment_id.eq_any(&ids))
                    .execute(conn)
                    .await?;
                let deleted: Vec<super::Attachment> = diesel::delete(attachment::table)
                    .filter(attachment::id.eq_any(&ids))
                    .filter(attachment::evidence_at.is_not_null())
                    .returning(super::Attachment::as_returning())
                    .load(conn)
                    .await?;
                let mut purged = Vec::new();
                for row in deleted {
                    let message = in_messages
                        .iter()
                        .find(|(attachment, _)| *attachment == row.id)
                        .map(|(_, message)| *message)
                        .or_else(|| removed_from.get(&row.id).copied());
                    let place = match message {
                        Some(message) => place_of(conn, message).await?,
                        None => (None, None),
                    };
                    log_operator_moderation(
                        conn,
                        ModerationAction::PurgeAttachment,
                        place.0,
                        place.1,
                        Some(match message {
                            Some(message) => format!("{}/{}", message.0, row.id.0),
                            None => format!("-/{}", row.id.0),
                        }),
                    )
                    .await?;
                    purged.push((row, message));
                }
                Ok(purged)
            }
            .scope_boxed()
        })
        .await?;
    let mut purged = Vec::new();
    for (row, message) in rows {
        let mut keys = vec![row.storage_key.clone(), evidence_key(&row.storage_key)];
        if let Some(key) = &row.preview_storage_key {
            keys.extend([key.clone(), evidence_key(key)]);
        }
        keys.dedup();
        let mut failed_objects = Vec::new();
        for key in keys {
            if let Err(e) = store.delete(&key).await {
                tracing::warn!(key, error = %e, "could not delete purged evidence");
                failed_objects.push(key);
            }
        }
        purged.push(Purged {
            id: row.id,
            file_name: row.file_name,
            message,
            failed_objects,
        });
    }
    Ok(purged)
}

/// The community and channel of `message`, for the log.
async fn place_of(
    conn: &mut AsyncPgConnection,
    message: MessageId,
) -> crate::Result<(Option<crate::CommunityId>, Option<ChannelId>)> {
    let place: Option<(ChannelId, Option<crate::CommunityId>)> = message::table
        .inner_join(channel::table.on(channel::id.eq(message::channel)))
        .select((channel::id, channel::community))
        .filter(message::id.eq(message))
        .first(conn)
        .await
        .optional()?;
    Ok(match place {
        Some((channel, community)) => (community, Some(channel)),
        None => (None, None),
    })
}
