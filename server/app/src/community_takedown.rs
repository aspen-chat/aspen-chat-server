//! Taking a community down from the dashboard (`POST /admin/communities/{community}/takedown`):
//! deleting it at once, as its owner's deletion does, and then, as the moderator asks, banning
//! every member from the deployment and erasing everything it held.
//!
//! The deletion is the request's own: the community is marked deleted and announced in its
//! transaction, so it is gone for everyone the moment the request answers. Banning and erasing
//! grow with the community, so the request saves a `takeDownCommunity` job ([`step`]) that does
//! them in bounded steps: the bans first, while the memberships that name the members still
//! stand, then the erasure.
//!
//! Erasing deletes the community's rows outright, where deleting only hides them: its messages
//! with their reactions, attachments and their files, link previews' pictures, and pins; its
//! threads, channels, and categories with everything each kept per person; its polls, roles,
//! memberships, bans, invites, and emoji. Kept are the community's own row, so the moderation
//! log and report cases still name it; every message a report case is about, deleted as any
//! message is (its files kept as evidence, `app::attachment::evidence`); and the channels that
//! hold those messages, deleted. See `docs/architecture/administration/community-takedowns.md`.

use crate::ban::{BanRequest, validate};
use crate::context::GlobalServerContext;
use crate::deployment::{DeploymentAccess, DeploymentPermission, deployment_access};
use crate::jobs::{Claimed, JobClass, JobKind, NewJob, Outcome, checkpoint};
use crate::media_store::evidence_key;
use crate::moderation_log::{ModerationAction, log_moderation};
use crate::user_ban::{BANNED_SQL, UserBanned};
use crate::{ChannelId, CommunityId, MessageId, UserId};
use aspen_schema::community;
use chrono::{DateTime, Utc};
use diesel::prelude::*;
use diesel::sql_types::{Array, BigInt, Bool, Integer, Nullable, Text, Uuid as PgUuid};
use diesel_async::scoped_futures::ScopedFutureExt;
use diesel_async::{AsyncConnection, AsyncPgConnection, RunQueryDsl};
use serde::{Deserialize, Serialize};

/// How many members one step bans: each ban is a few statements and an event of its own.
const BAN_BATCH: i64 = 50;
/// How many messages one step erases. Each may hold up to `message::MAX_ATTACHMENTS` files,
/// whose objects the next step deletes.
const MESSAGE_BATCH: i64 = 50;
/// How many rows one statement deletes from a table that holds a data-driven number per
/// message, channel, category, or member (reactions, saves, read positions, memberships).
const CHUNK: i64 = 1000;

/// What a takedown asks for beyond deleting the community.
#[derive(Debug, Clone, Default)]
pub struct TakedownRequest {
    /// Whether everything the community held is erased rather than only hidden.
    pub erase: bool,
    /// A ban from the deployment for every member it reaches ([`Reach::banned`]), with the
    /// reason, end, and deletion window of a ban of one person.
    pub ban: Option<BanRequest>,
}

/// Whom a takedown of a community reaches, counted as it stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reach {
    /// Its members whose accounts stand.
    pub members: i64,
    /// Those of them a ban by the caller would reach: not the caller, the system account, anyone
    /// banned already, or anyone holding a deployment role at or above the caller's highest.
    pub banned: i64,
}

/// The job's payload, fixed when the takedown was asked for.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Takedown {
    community: CommunityId,
    /// Who asked for it, whose deployment access each ban is checked against again.
    by: UserId,
    ban: Option<MemberBan>,
    erase: bool,
}

/// The ban every member is given.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MemberBan {
    reason: Option<String>,
    /// When it ends, the same moment for everyone, fixed when it was asked for.
    until: Option<DateTime<Utc>>,
    delete_messages_seconds: Option<u32>,
}

/// How far the job has come.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "stage")]
enum Stage {
    /// Members after `after`, in order of id, are still to be banned.
    Ban {
        after: Option<UserId>,
    },
    /// Messages of `channel` after `after` are still to be erased, then those of the channels
    /// after it, in order of id. `objects` are the files the last batch let go of, deleted from
    /// storage before the next batch.
    Messages {
        channel: Option<ChannelId>,
        after: Option<MessageId>,
        #[serde(default)]
        objects: Vec<String>,
    },
    /// Channels after `after` are still to be erased: threads first, since a channel cannot go
    /// while a thread of it stands, then the rest.
    Channels {
        threads: bool,
        after: Option<ChannelId>,
    },
    Categories,
    Members,
    Roles,
    Invites,
    Emoji,
    Finish,
}

impl Stage {
    /// Where the erasure begins.
    fn erasing() -> Self {
        Self::Messages {
            channel: None,
            after: None,
            objects: Vec::new(),
        }
    }
}

/// Whether the `"user"` row in a query is someone a ban by `$2`, whose highest deployment role
/// is at `$3`, reaches: an account that stands, not the system account, not the banner, not
/// banned already, and not holding a deployment role at or above the banner's. Deployment roles
/// count only for the deployment's own people, as `deployment::deployment_access` counts them.
fn bannable_sql() -> String {
    format!(
        r#"("user".deleted_at IS NULL AND NOT "user".system AND "user".id <> $2
            AND NOT {BANNED_SQL}
            AND NOT ("user".bot = false AND "user".home_domain IS NULL AND EXISTS (
                SELECT 1 FROM user_deployment_role udr
                JOIN deployment_role dr ON dr.id = udr.role
                WHERE udr."user" = "user".id AND dr.position >= $3)))"#
    )
}

#[derive(QueryableByName)]
struct Counted {
    #[diesel(sql_type = BigInt)]
    members: i64,
    #[diesel(sql_type = BigInt)]
    banned: i64,
}

async fn count_reach(
    conn: &mut AsyncPgConnection,
    access: &DeploymentAccess,
    community: CommunityId,
) -> crate::Result<Reach> {
    let counted: Counted = diesel::sql_query(format!(
        r#"SELECT count(*) AS members, count(*) FILTER (WHERE {bannable}) AS banned
           FROM community_user cu JOIN "user" ON "user".id = cu."user"
           WHERE cu.community = $1 AND "user".deleted_at IS NULL"#,
        bannable = bannable_sql()
    ))
    .bind::<PgUuid, _>(community.0)
    .bind::<PgUuid, _>(access.user.0)
    .bind::<Integer, _>(access.rank())
    .get_result(conn)
    .await?;
    Ok(Reach {
        members: counted.members,
        banned: counted.banned,
    })
}

/// Whom taking `community` down would reach, for the dashboard to say before it is asked for.
/// Takes Moderate any community. The count reads every membership of the community, which is
/// the answer it gives.
pub async fn reach(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    community: CommunityId,
) -> crate::Result<Reach> {
    access.require(DeploymentPermission::ModerateCommunities)?;
    let mut conn = state.connection_pool.get().await?;
    let live: Option<CommunityId> = community::table
        .select(community::id)
        .filter(community::id.eq(community))
        .filter(community::deleted_at.is_null())
        .first(conn.as_mut())
        .await
        .optional()?;
    if live.is_none() {
        return Err(crate::Error::Diesel(diesel::result::Error::NotFound));
    }
    count_reach(conn.as_mut(), access, community).await
}

/// Deletes `community` and saves the job that does the rest of `request`, answering whom it
/// reaches as counted in the same transaction. Takes Moderate any community; banning its members
/// takes Ban users too, and their deletion window Remove content, as a ban of one person does.
/// Written to the moderation log: the deletion, the erasure, and each ban as the job makes it.
pub async fn take_down(
    state: &GlobalServerContext,
    access: &DeploymentAccess,
    community: CommunityId,
    request: &TakedownRequest,
) -> crate::Result<Reach> {
    access.require(DeploymentPermission::ModerateCommunities)?;
    let ban = match &request.ban {
        Some(ban) => {
            access.require(DeploymentPermission::BanUsers)?;
            if ban.delete_messages_seconds.is_some() {
                access.require(DeploymentPermission::RemoveContent)?;
            }
            let (reason, until) = validate(ban)?;
            Some(MemberBan {
                reason,
                until,
                delete_messages_seconds: ban.delete_messages_seconds,
            })
        }
        None => None,
    };
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        let ban = ban.clone();
        async move {
            // Held as every way in holds it (`community::add_member`), so nobody joins between
            // the count and the deletion.
            community::table
                .select(community::id)
                .filter(community::id.eq(community))
                .filter(community::deleted_at.is_null())
                .for_update()
                .first::<CommunityId>(conn.as_mut())
                .await?;
            let reach = count_reach(conn.as_mut(), access, community).await?;
            log_moderation(
                conn.as_mut(),
                access.user,
                ModerationAction::DeleteCommunity,
                Some(community),
                None,
                None,
            )
            .await?;
            crate::community::mark_deleted(state, conn.as_mut(), community).await?;
            if request.erase {
                log_moderation(
                    conn.as_mut(),
                    access.user,
                    ModerationAction::EraseCommunity,
                    Some(community),
                    None,
                    None,
                )
                .await?;
            }
            if ban.is_some() || request.erase {
                crate::jobs::enqueue(
                    conn.as_mut(),
                    NewJob::new(
                        JobKind::TakeDownCommunity,
                        JobClass::Normal,
                        &Takedown {
                            community,
                            by: access.user,
                            ban,
                            erase: request.erase,
                        },
                    )?
                    .keyed(community.0.to_string()),
                )
                .await?;
            }
            Ok(reach)
        }
        .scope_boxed()
    })
    .await
}

/// One step of `takeDownCommunity`.
pub async fn step(state: &GlobalServerContext, job: &Claimed) -> crate::Result<Outcome> {
    let takedown: Takedown = job.payload()?;
    let stage = match job.progress::<Stage>()? {
        Some(stage) => stage,
        None if takedown.ban.is_some() => Stage::Ban { after: None },
        None => Stage::erasing(),
    };
    let community = takedown.community;
    match stage {
        Stage::Ban { after } => ban_step(state, job, &takedown, after).await,
        Stage::Messages {
            channel,
            after,
            objects,
        } => {
            if !objects.is_empty() {
                delete_objects(state, &objects).await;
                return progress(&Stage::Messages {
                    channel,
                    after,
                    objects: Vec::new(),
                });
            }
            messages_step(state, job, community, channel, after).await
        }
        Stage::Channels { threads, after } => {
            channels_step(state, job, community, threads, after).await
        }
        Stage::Categories => categories_step(state, community).await,
        Stage::Members => members_step(state, community).await,
        Stage::Roles => roles_step(state, community).await,
        Stage::Invites => invites_step(state, community).await,
        Stage::Emoji => emoji_step(state, community).await,
        Stage::Finish => finish_step(state, community).await,
    }
}

fn progress(stage: &Stage) -> crate::Result<Outcome> {
    Ok(Outcome::Progress(serde_json::to_value(stage)?))
}

#[derive(QueryableByName)]
struct Member {
    #[diesel(sql_type = PgUuid)]
    user: UserId,
    #[diesel(sql_type = Bool)]
    bannable: bool,
}

/// Bans the next [`BAN_BATCH`] members, each as a ban of one person bans them, and moves past
/// them in the same transaction. The banner's access is read again each step, so a banner who
/// has lost Ban users (or Remove content, for a deletion window) or rank bans nobody more; the
/// job then goes on to erase, if it was asked to, since that took only Moderate any community,
/// checked when it was asked for.
async fn ban_step(
    state: &GlobalServerContext,
    job: &Claimed,
    takedown: &Takedown,
    after: Option<UserId>,
) -> crate::Result<Outcome> {
    let Some(ban) = &takedown.ban else {
        return next_after_bans(takedown);
    };
    let mut conn = state.connection_pool.get().await?;
    let access = deployment_access(&conn, takedown.by).await?;
    let still_may = access.has(DeploymentPermission::BanUsers)
        && (ban.delete_messages_seconds.is_none()
            || access.has(DeploymentPermission::RemoveContent));
    if !still_may {
        tracing::warn!(
            community = %takedown.community.0,
            banner = %takedown.by.0,
            "a community takedown's banner may no longer ban; its remaining members are not banned"
        );
        return next_after_bans(takedown);
    }
    let access = &access;
    conn.transaction(|conn| {
        async move {
            let members: Vec<Member> = diesel::sql_query(format!(
                r#"SELECT cu."user", {bannable} AS bannable
                   FROM community_user cu JOIN "user" ON "user".id = cu."user"
                   WHERE cu.community = $1 AND cu."user" > $4
                   ORDER BY cu."user" LIMIT $5"#,
                bannable = bannable_sql()
            ))
            .bind::<PgUuid, _>(takedown.community.0)
            .bind::<PgUuid, _>(access.user.0)
            .bind::<Integer, _>(access.rank())
            .bind::<PgUuid, _>(after.map_or(uuid::Uuid::nil(), |u| u.0))
            .bind::<BigInt, _>(BAN_BATCH)
            .load(conn)
            .await?;
            let Some(last) = members.last().map(|m| m.user) else {
                return next_after_bans(takedown);
            };
            let mut outcome = UserBanned::default();
            for member in members.iter().filter(|m| m.bannable) {
                // Each in a savepoint of its own: someone whose rank or account changed since
                // they were read is passed over rather than failing the batch.
                let banned = conn
                    .transaction(|conn| {
                        let outcome = &mut outcome;
                        async move {
                            crate::user_ban::ban_one(
                                state,
                                conn,
                                access,
                                member.user,
                                &ban.reason,
                                ban.until,
                                ban.delete_messages_seconds,
                                outcome,
                            )
                            .await
                        }
                        .scope_boxed()
                    })
                    .await;
                match banned {
                    Ok(_) => {}
                    Err(crate::Error::Forbidden(_) | crate::Error::Validation(_)) => {}
                    Err(crate::Error::Diesel(diesel::result::Error::NotFound)) => {}
                    Err(e) => return Err(e),
                }
            }
            checkpoint(conn, job.id, &Stage::Ban { after: Some(last) }).await?;
            Ok(Outcome::Continue)
        }
        .scope_boxed()
    })
    .await
}

/// Where the job goes once nobody is left to ban.
fn next_after_bans(takedown: &Takedown) -> crate::Result<Outcome> {
    if takedown.erase {
        progress(&Stage::erasing())
    } else {
        Ok(Outcome::Done)
    }
}

/// The community's first channel after `after` in order of id, threads and deleted channels
/// among them, through `channel_by_community`.
async fn next_channel(
    conn: &mut AsyncPgConnection,
    community: CommunityId,
    after: Option<ChannelId>,
) -> crate::Result<Option<ChannelId>> {
    use aspen_schema::channel;
    let mut query = channel::table
        .select(channel::id)
        .filter(channel::community.eq(community))
        .order(channel::id)
        .into_boxed();
    if let Some(after) = after {
        query = query.filter(channel::id.gt(after));
    }
    Ok(query.first(conn).await.optional()?)
}

/// Deletes up to [`CHUNK`] rows of `table` that `filter` picks, `filter` reading `$1` as an
/// array of ids. Answers whether a full chunk went, so more may be left.
async fn chunk_by_ids(
    conn: &mut AsyncPgConnection,
    table: &str,
    filter: &str,
    ids: &[uuid::Uuid],
) -> crate::Result<bool> {
    let deleted = diesel::sql_query(format!(
        "DELETE FROM {table} WHERE ctid IN (SELECT ctid FROM {table} WHERE {filter} LIMIT $2)"
    ))
    .bind::<Array<PgUuid>, _>(ids)
    .bind::<BigInt, _>(CHUNK)
    .execute(conn)
    .await?;
    Ok(deleted as i64 >= CHUNK)
}

/// As [`chunk_by_ids`], with `filter` reading `$1` as one id.
async fn chunk_by_id(
    conn: &mut AsyncPgConnection,
    table: &str,
    filter: &str,
    id: uuid::Uuid,
) -> crate::Result<bool> {
    let deleted = diesel::sql_query(format!(
        "DELETE FROM {table} WHERE ctid IN (SELECT ctid FROM {table} WHERE {filter} LIMIT $2)"
    ))
    .bind::<PgUuid, _>(id)
    .bind::<BigInt, _>(CHUNK)
    .execute(conn)
    .await?;
    Ok(deleted as i64 >= CHUNK)
}

/// Clears `column` of up to [`CHUNK`] rows of `table` where it is `id`. Answers whether a full
/// chunk was cleared, so more may be left.
async fn chunk_clear(
    conn: &mut AsyncPgConnection,
    table: &str,
    column: &str,
    id: uuid::Uuid,
) -> crate::Result<bool> {
    let cleared = diesel::sql_query(format!(
        "UPDATE {table} SET {column} = NULL \
         WHERE ctid IN (SELECT ctid FROM {table} WHERE {column} = $1 LIMIT $2)"
    ))
    .bind::<PgUuid, _>(id)
    .bind::<BigInt, _>(CHUNK)
    .execute(conn)
    .await?;
    Ok(cleared as i64 >= CHUNK)
}

#[derive(QueryableByName)]
struct Batched {
    #[diesel(sql_type = PgUuid)]
    id: MessageId,
    /// Whether a report case is about it, so it is deleted as any message is rather than erased.
    #[diesel(sql_type = Bool)]
    kept: bool,
}

#[derive(QueryableByName)]
struct ObjectRow {
    #[diesel(sql_type = Text)]
    storage_key: String,
    #[diesel(sql_type = Nullable<Text>)]
    preview_storage_key: Option<String>,
}

#[derive(QueryableByName)]
struct PictureRow {
    #[diesel(sql_type = PgUuid)]
    image_id: crate::LinkPreviewImageId,
}

/// Erases the next [`MESSAGE_BATCH`] messages of `channel`, or moves on to the next channel when
/// it has none left, or to the channels themselves after the last.
///
/// What each message holds a data-driven number of (reactions, saves) goes first, a chunk a
/// step; then, in one transaction, every reference to the batch is cleared, its pins,
/// attachments in no other message, and the messages go, and the place moves past them with
/// the files they let go of, which the next step deletes from storage. The files go after the
/// rows, so a failed step leaves nothing pointing at a missing file, and deleting one twice is
/// harmless.
async fn messages_step(
    state: &GlobalServerContext,
    job: &Claimed,
    community: CommunityId,
    channel: Option<ChannelId>,
    after: Option<MessageId>,
) -> crate::Result<Outcome> {
    let mut conn = state.connection_pool.get().await?;
    let Some(channel) = (match channel {
        Some(channel) => Some(channel),
        None => next_channel(conn.as_mut(), community, None).await?,
    }) else {
        return progress(&Stage::Channels {
            threads: true,
            after: None,
        });
    };
    let batch: Vec<Batched> = diesel::sql_query(
        r#"SELECT m.id, EXISTS (SELECT 1 FROM report_case c WHERE c.message = m.id) AS kept
           FROM message m WHERE m.channel = $1 AND m.id > $2 ORDER BY m.id LIMIT $3"#,
    )
    .bind::<PgUuid, _>(channel.0)
    .bind::<PgUuid, _>(after.map_or(uuid::Uuid::nil(), |m| m.0))
    .bind::<BigInt, _>(MESSAGE_BATCH)
    .load(conn.as_mut())
    .await?;
    let Some(last) = batch.last().map(|m| m.id) else {
        let next = next_channel(conn.as_mut(), community, Some(channel)).await?;
        return progress(&match next {
            Some(next) => Stage::Messages {
                channel: Some(next),
                after: None,
                objects: Vec::new(),
            },
            None => Stage::Channels {
                threads: true,
                after: None,
            },
        });
    };
    let doomed: Vec<uuid::Uuid> = batch.iter().filter(|m| !m.kept).map(|m| m.id.0).collect();
    let kept: Vec<MessageId> = batch.iter().filter(|m| m.kept).map(|m| m.id).collect();
    for (table, filter) in [
        ("react", "message = ANY($1)"),
        ("saved_message", "message = ANY($1)"),
        ("mention", "message = ANY($1)"),
    ] {
        if chunk_by_ids(conn.as_mut(), table, filter, &doomed).await? {
            return progress(&Stage::Messages {
                channel: Some(channel),
                after,
                objects: Vec::new(),
            });
        }
    }
    conn.transaction(|conn| {
        async move {
            crate::message::soft_delete_many(state, conn, &kept).await?;
            let mut objects = Vec::new();
            if !doomed.is_empty() {
                for clear in [
                    "UPDATE message SET reply_to = NULL WHERE reply_to = ANY($1)",
                    "UPDATE message SET echo_of = NULL WHERE echo_of = ANY($1)",
                    "UPDATE message SET echo = NULL WHERE echo = ANY($1)",
                    "UPDATE channel SET starter_message = NULL WHERE starter_message = ANY($1)",
                    "DELETE FROM pin WHERE message_id = ANY($1)",
                ] {
                    diesel::sql_query(clear)
                        .bind::<Array<PgUuid>, _>(&doomed)
                        .execute(conn)
                        .await?;
                }
                // Attachments in the batch, or taken off its messages, that no other message
                // holds, locked in order of id as `attachment::evidence` locks them.
                let freed: Vec<ObjectRow> = diesel::sql_query(
                    r#"WITH named AS (
                           DELETE FROM message_attachment WHERE message_id = ANY($1)
                           RETURNING attachment_id AS id
                       ), candidates AS (
                           SELECT id FROM named
                           UNION SELECT id FROM attachment WHERE removed_from = ANY($1)
                       ), freed AS (
                           SELECT a.id FROM attachment a JOIN candidates USING (id)
                           WHERE NOT EXISTS (
                               SELECT 1 FROM message_attachment other
                               WHERE other.attachment_id = a.id
                                 AND NOT other.message_id = ANY($1))
                           ORDER BY a.id FOR UPDATE OF a
                       )
                       DELETE FROM attachment WHERE id IN (SELECT id FROM freed)
                       RETURNING storage_key, preview_storage_key"#,
                )
                .bind::<Array<PgUuid>, _>(&doomed)
                .load(conn)
                .await?;
                for row in freed {
                    objects.push(evidence_key(&row.storage_key));
                    objects.push(row.storage_key);
                    if let Some(preview) = row.preview_storage_key {
                        objects.push(evidence_key(&preview));
                        objects.push(preview);
                    }
                }
                let pictures: Vec<PictureRow> = diesel::sql_query(
                    "SELECT image_id FROM message_link_preview \
                     WHERE message_id = ANY($1) AND image_id IS NOT NULL",
                )
                .bind::<Array<PgUuid>, _>(&doomed)
                .load(conn)
                .await?;
                objects.extend(
                    pictures
                        .into_iter()
                        .map(|p| crate::link_preview::image_storage_key(p.image_id)),
                );
                diesel::sql_query("DELETE FROM message WHERE id = ANY($1)")
                    .bind::<Array<PgUuid>, _>(&doomed)
                    .execute(conn)
                    .await?;
            }
            objects.sort();
            objects.dedup();
            checkpoint(
                conn,
                job.id,
                &Stage::Messages {
                    channel: Some(channel),
                    after: Some(last),
                    objects,
                },
            )
            .await?;
            Ok(Outcome::Continue)
        }
        .scope_boxed()
    })
    .await
}

/// Deletes each of `keys` from storage, logging the ones that would not go: the rows that named
/// them are gone, so nothing serves them.
async fn delete_objects(state: &GlobalServerContext, keys: &[String]) {
    for key in keys {
        if let Err(e) = state.media_store.delete(key).await {
            tracing::warn!(key, error = %e, "could not delete an erased community's file");
        }
    }
}

#[derive(QueryableByName)]
struct Found {
    #[diesel(sql_type = PgUuid)]
    id: uuid::Uuid,
}

/// Erases the next channel: threads in the first pass, the rest in the second. What it keeps
/// per person or per poll goes a chunk a step; then its polls, pins, call, and the channel
/// itself, unless a message a report case is about, or a thread of it, still stands in it, in
/// which case it stays, deleted.
async fn channels_step(
    state: &GlobalServerContext,
    job: &Claimed,
    community: CommunityId,
    threads: bool,
    after: Option<ChannelId>,
) -> crate::Result<Outcome> {
    let mut conn = state.connection_pool.get().await?;
    let found: Option<Found> = diesel::sql_query(
        "SELECT id FROM channel WHERE community = $1 AND id > $2 \
         AND (parent_channel IS NOT NULL) = $3 ORDER BY id LIMIT 1",
    )
    .bind::<PgUuid, _>(community.0)
    .bind::<PgUuid, _>(after.map_or(uuid::Uuid::nil(), |c| c.0))
    .bind::<Bool, _>(threads)
    .get_result(conn.as_mut())
    .await
    .optional()?;
    let Some(Found { id: channel }) = found else {
        return progress(&if threads {
            Stage::Channels {
                threads: false,
                after: None,
            }
        } else {
            Stage::Categories
        });
    };
    for (table, filter) in [
        ("read_state", "channel = $1"),
        ("mention", "channel = $1"),
        ("thread_follow", "thread = $1"),
        ("channel_mute", "channel = $1"),
        ("notification_setting", "channel = $1"),
        ("held_message", "channel = $1"),
        ("plugin_notice", "channel = $1"),
        (
            "poll_vote",
            "poll IN (SELECT id FROM poll WHERE channel = $1)",
        ),
        ("poll", "channel = $1"),
    ] {
        if chunk_by_id(conn.as_mut(), table, filter, channel).await? {
            return progress(&Stage::Channels { threads, after });
        }
    }
    // The file transfer record and the moderation log keep their entries, no longer naming
    // the channel.
    for (table, column) in [("file_offer", "channel"), ("moderation_log", "channel")] {
        if chunk_clear(conn.as_mut(), table, column, channel).await? {
            return progress(&Stage::Channels { threads, after });
        }
    }
    conn.transaction(|conn| {
        async move {
            for statement in [
                "DELETE FROM pin WHERE channel = $1",
                "DELETE FROM voice_session WHERE channel = $1",
                "UPDATE community SET default_channel = NULL WHERE default_channel = $1",
                "UPDATE channel SET deleted_at = now() WHERE id = $1 AND deleted_at IS NULL",
                r#"DELETE FROM channel WHERE id = $1
                   AND NOT EXISTS (SELECT 1 FROM message WHERE channel = $1)
                   AND NOT EXISTS (SELECT 1 FROM message WHERE thread = $1)
                   AND NOT EXISTS (SELECT 1 FROM channel child WHERE child.parent_channel = $1)"#,
            ] {
                diesel::sql_query(statement)
                    .bind::<PgUuid, _>(channel)
                    .execute(conn)
                    .await?;
            }
            checkpoint(
                conn,
                job.id,
                &Stage::Channels {
                    threads,
                    after: Some(ChannelId(channel)),
                },
            )
            .await?;
            Ok(Outcome::Continue)
        }
        .scope_boxed()
    })
    .await
}

/// Erases the community's categories, one a step, with what each kept per person; a channel
/// left standing for a report case is taken out of its category first.
async fn categories_step(
    state: &GlobalServerContext,
    community: CommunityId,
) -> crate::Result<Outcome> {
    let mut conn = state.connection_pool.get().await?;
    let found: Option<Found> =
        diesel::sql_query("SELECT id FROM category WHERE community = $1 LIMIT 1")
            .bind::<PgUuid, _>(community.0)
            .get_result(conn.as_mut())
            .await
            .optional()?;
    let Some(Found { id: category }) = found else {
        return progress(&Stage::Members);
    };
    for (table, filter) in [
        ("category_collapse", "category = $1"),
        ("category_mute", "category = $1"),
        ("notification_setting", "category = $1"),
    ] {
        if chunk_by_id(conn.as_mut(), table, filter, category).await? {
            return progress(&Stage::Categories);
        }
    }
    conn.transaction(|conn| {
        async move {
            for statement in [
                "UPDATE channel SET parent_category = NULL WHERE parent_category = $1",
                "DELETE FROM category WHERE id = $1",
            ] {
                diesel::sql_query(statement)
                    .bind::<PgUuid, _>(category)
                    .execute(conn)
                    .await?;
            }
            Ok::<_, crate::Error>(())
        }
        .scope_boxed()
    })
    .await?;
    progress(&Stage::Categories)
}

/// Erases the community's memberships (their roles go with them), what members set for it, its
/// bans, and its call mutes, a chunk a step. Its members were told it is gone when it was
/// deleted, and no read counts a deleted community's memberships (`community::live`).
async fn members_step(
    state: &GlobalServerContext,
    community: CommunityId,
) -> crate::Result<Outcome> {
    let mut conn = state.connection_pool.get().await?;
    for (table, filter) in [
        ("community_user", "community = $1"),
        ("notification_setting", "community = $1"),
        ("community_ban", "community = $1"),
        ("voice_mute", "community = $1"),
    ] {
        if chunk_by_id(conn.as_mut(), table, filter, community.0).await? {
            return progress(&Stage::Members);
        }
    }
    progress(&Stage::Roles)
}

/// Erases the community's roles, once the tags of them are gone.
async fn roles_step(state: &GlobalServerContext, community: CommunityId) -> crate::Result<Outcome> {
    let mut conn = state.connection_pool.get().await?;
    if chunk_by_id(
        conn.as_mut(),
        "mention",
        "target_role IN (SELECT id FROM community_role WHERE community = $1)",
        community.0,
    )
    .await?
    {
        return progress(&Stage::Roles);
    }
    diesel::sql_query(
        "DELETE FROM community_member_role \
         WHERE role IN (SELECT id FROM community_role WHERE community = $1)",
    )
    .bind::<PgUuid, _>(community.0)
    .execute(conn.as_mut())
    .await?;
    diesel::sql_query("DELETE FROM community_role WHERE community = $1")
        .bind::<PgUuid, _>(community.0)
        .execute(conn.as_mut())
        .await?;
    progress(&Stage::Invites)
}

/// Erases the community's invites, revoked and expired ones included, a chunk a step.
async fn invites_step(
    state: &GlobalServerContext,
    community: CommunityId,
) -> crate::Result<Outcome> {
    let mut conn = state.connection_pool.get().await?;
    if chunk_by_id(conn.as_mut(), "invite", "community = $1", community.0).await? {
        return progress(&Stage::Invites);
    }
    progress(&Stage::Emoji)
}

/// Deletes the community's emoji and saves the `purgeCustomEmoji` job of each, which takes
/// their reactions off everywhere and deletes their pictures.
async fn emoji_step(state: &GlobalServerContext, community: CommunityId) -> crate::Result<Outcome> {
    let mut conn = state.connection_pool.get().await?;
    conn.transaction(|conn| {
        async move {
            let deleted: Vec<Found> = diesel::sql_query(
                "UPDATE custom_emoji SET deleted_at = now() WHERE id IN \
                 (SELECT id FROM custom_emoji WHERE community = $1 AND deleted_at IS NULL \
                  LIMIT $2) RETURNING id",
            )
            .bind::<PgUuid, _>(community.0)
            .bind::<BigInt, _>(CHUNK)
            .load(conn)
            .await?;
            let full = deleted.len() as i64 >= CHUNK;
            let jobs = deleted
                .into_iter()
                .map(|Found { id }| {
                    let emoji = crate::CustomEmojiId(id);
                    Ok(NewJob::new(
                        JobKind::PurgeCustomEmoji,
                        JobClass::Normal,
                        &crate::custom_emoji::ToPurge { emoji },
                    )?
                    .keyed(id.to_string()))
                })
                .collect::<crate::Result<Vec<_>>>()?;
            crate::jobs::enqueue_many(conn, jobs).await?;
            progress(&if full { Stage::Emoji } else { Stage::Finish })
        }
        .scope_boxed()
    })
    .await
}

/// Takes the community's plugins' settings and its icon away, which lets the icon go to
/// `forgetIcon`, and ends the job.
async fn finish_step(
    state: &GlobalServerContext,
    community: CommunityId,
) -> crate::Result<Outcome> {
    let mut conn = state.connection_pool.get().await?;
    diesel::sql_query("DELETE FROM community_plugin WHERE community = $1")
        .bind::<PgUuid, _>(community.0)
        .execute(conn.as_mut())
        .await?;
    diesel::update(community::table.find(community))
        .set((
            community::icon.eq(None::<crate::IconId>),
            community::default_channel.eq(None::<ChannelId>),
        ))
        .execute(conn.as_mut())
        .await?;
    Ok(Outcome::Done)
}
