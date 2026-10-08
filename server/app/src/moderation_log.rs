//! The moderation log (`moderation_log`): each use of Moderate any community that the
//! community's own permissions would not have allowed, every reading of a DM by someone not in
//! it, each ban from the deployment and its lifting, each warning, deletion, nickname cleared,
//! and profile reset a report's review gave (`app::report`), and how the log is read back with
//! what its ids name.

use crate::context::GlobalServerContext;
use crate::message::MessageKind;
use crate::{AttachmentId, ChannelId, CommunityId, MessageId, PollId, UserId};
use aspen_schema::moderation_log;
use diesel::prelude::*;
use diesel_async::{AsyncPgConnection, RunQueryDsl};
use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;

/// One action for the moderation log, named there as `spec/moderation_actions.json` lists it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::IntoStaticStr, strum::VariantArray, strum::EnumString,
)]
#[strum(serialize_all = "camelCase")]
pub enum ModerationAction {
    ReadDm,
    DeleteMessage,
    RemoveAttachment,
    RemoveReaction,
    RemoveMember,
    BanMember,
    LiftBan,
    /// A ban's deletion of the person's recent messages; the subject is the person.
    DeleteRecentMessages,
    RenameChannel,
    DeleteChannel,
    RenameCommunity,
    DeleteCommunity,
    RemoveWriteIn,
    /// A poll closed before its deadline; the subject is the poll.
    ClosePoll,
    /// A ban from the deployment (`app::user_ban`); the subject is the person.
    BanUser,
    LiftUserBan,
    /// A warning sent in reviewing a report; the subject is the person warned.
    WarnUser,
    /// A profile reset in reviewing a report; the subject is the person.
    ResetProfile,
    /// A member's nickname cleared, in their community or in reviewing a report of it; the
    /// subject is the person.
    ClearNickname,
    /// The messages around a reported message in a DM, read in reviewing the report; the
    /// subject is the reported message.
    ReadReportContext,
    /// Someone's DMs listed, to open one; the subject is the person.
    ListDms,
    /// Evidence deleted outright from the terminal (`aspen-chat-server attachments purge`,
    /// `app::attachment::evidence::purge`); the subject is the message it was in or taken off
    /// and the attachment, and the entry has no actor.
    PurgeAttachment,
}

/// Writes a moderator's action to the moderation log, and to the server's own log.
/// `subject` names what was acted on beyond the community and channel: a message, a user.
pub async fn log_moderation(
    conn: &mut AsyncPgConnection,
    actor: UserId,
    action: ModerationAction,
    community: Option<CommunityId>,
    channel: Option<ChannelId>,
    subject: Option<String>,
) -> crate::Result<()> {
    let action: &'static str = action.into();
    tracing::info!(
        actor = %actor.0,
        action,
        community = ?community.map(|c| c.0),
        channel = ?channel.map(|c| c.0),
        subject = ?subject,
        "deployment moderation"
    );
    diesel::insert_into(moderation_log::table)
        .values((
            moderation_log::id.eq(uuid::Uuid::now_v7()),
            moderation_log::actor.eq(Some(actor)),
            moderation_log::action.eq(action),
            moderation_log::community.eq(community),
            moderation_log::channel.eq(channel),
            moderation_log::subject.eq(subject),
        ))
        .execute(conn)
        .await?;
    Ok(())
}

/// Writes an operator's action from the terminal to the moderation log, with no actor, since
/// no account took it, and to the server's own log.
pub async fn log_operator_moderation(
    conn: &mut AsyncPgConnection,
    action: ModerationAction,
    community: Option<CommunityId>,
    channel: Option<ChannelId>,
    subject: Option<String>,
) -> crate::Result<()> {
    let action: &'static str = action.into();
    tracing::info!(
        action,
        community = ?community.map(|c| c.0),
        channel = ?channel.map(|c| c.0),
        subject = ?subject,
        "deployment moderation from the terminal"
    );
    diesel::insert_into(moderation_log::table)
        .values((
            moderation_log::id.eq(uuid::Uuid::now_v7()),
            moderation_log::actor.eq(None::<UserId>),
            moderation_log::action.eq(action),
            moderation_log::community.eq(community),
            moderation_log::channel.eq(channel),
            moderation_log::subject.eq(subject),
        ))
        .execute(conn)
        .await?;
    Ok(())
}

/// One entry of the moderation log, with what its ids name.
#[derive(Debug, Clone)]
pub struct ModerationEntry {
    pub id: uuid::Uuid,
    pub actor: Option<UserId>,
    pub action: String,
    pub community: Option<CommunityId>,
    pub channel: Option<ChannelId>,
    pub subject: Option<String>,
    pub at: chrono::DateTime<chrono::Utc>,
    pub details: ModerationDetails,
}

/// What an entry of the moderation log names, as it stands when the log is read, so its reader
/// sees names rather than ids. Whatever is no longer found is left out, and the entry's own ids
/// remain.
#[derive(Debug, Clone, Default, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModerationDetails {
    pub community: Option<LoggedCommunity>,
    pub channel: Option<LoggedChannel>,
    /// The person acted on: a member removed, someone banned from the deployment or let back,
    /// warned, or whose profile was reset, or whoever's reaction was removed.
    pub user: Option<UserId>,
    /// The message acted on or read: deleted, stripped of an attachment or a reaction, or the
    /// poll a write-in was taken from.
    pub message: Option<LoggedMessage>,
    /// The reaction removed.
    pub emoji: Option<String>,
    /// The removed attachment's file name.
    pub attachment: Option<String>,
    /// The removed write-in's text.
    pub write_in: Option<String>,
    /// The name a community or channel was given.
    pub renamed_to: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoggedCommunity {
    pub name: String,
    pub deleted: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoggedChannel {
    pub name: String,
    pub ty: crate::channel::ChannelType,
    pub deleted: bool,
    /// The channel a thread belongs to.
    pub parent_channel: Option<ChannelId>,
    /// A DM's people, who name it, since a DM has no name of its own.
    pub recipients: Vec<UserId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoggedMessage {
    pub id: MessageId,
    pub channel: ChannelId,
    pub author: UserId,
    pub deleted: bool,
}

/// What an entry's `subject` names, which depends on its action.
#[derive(Debug, Clone, PartialEq)]
enum Subject {
    User(UserId),
    Message(MessageId),
    Attachment(MessageId, AttachmentId),
    Reaction(MessageId, String, UserId),
    WriteIn(PollId, i32),
    /// A poll itself, named in the log by the message it is shown in.
    Poll(PollId),
    Name(String),
}

/// Reads a `subject` as `log_moderation`'s callers write it for `action`.
fn subject_of(action: &str, subject: &str) -> Option<Subject> {
    let id = |text: &str| uuid::Uuid::parse_str(text).ok();
    match action.parse::<ModerationAction>().ok()? {
        ModerationAction::RemoveMember
        | ModerationAction::BanMember
        | ModerationAction::LiftBan
        | ModerationAction::DeleteRecentMessages
        | ModerationAction::BanUser
        | ModerationAction::LiftUserBan
        | ModerationAction::WarnUser
        | ModerationAction::ResetProfile
        | ModerationAction::ClearNickname
        | ModerationAction::ListDms => Some(Subject::User(UserId(id(subject)?))),
        ModerationAction::DeleteMessage
        | ModerationAction::ReadDm
        | ModerationAction::ReadReportContext => Some(Subject::Message(MessageId(id(subject)?))),
        ModerationAction::RemoveAttachment | ModerationAction::PurgeAttachment => {
            let (message, attachment) = subject.split_once('/')?;
            Some(Subject::Attachment(
                MessageId(id(message)?),
                AttachmentId(id(attachment)?),
            ))
        }
        ModerationAction::RemoveReaction => {
            let (message, rest) = subject.split_once('/')?;
            let (emoji, author) = rest.rsplit_once('/')?;
            Some(Subject::Reaction(
                MessageId(id(message)?),
                emoji.to_string(),
                UserId(id(author)?),
            ))
        }
        ModerationAction::ClosePoll => Some(Subject::Poll(PollId(id(subject)?))),
        ModerationAction::RemoveWriteIn => {
            let (poll, option) = subject.split_once('/')?;
            Some(Subject::WriteIn(PollId(id(poll)?), option.parse().ok()?))
        }
        ModerationAction::RenameChannel | ModerationAction::RenameCommunity => {
            Some(Subject::Name(subject.to_string()))
        }
        ModerationAction::DeleteChannel | ModerationAction::DeleteCommunity => None,
    }
}

/// The newest entries of the moderation log, before `before` when given, each with what its
/// ids name, read one query per kind of record for the whole page.
pub async fn read_moderation_log(
    state: &GlobalServerContext,
    before: Option<uuid::Uuid>,
    limit: i64,
) -> crate::Result<Vec<ModerationEntry>> {
    use aspen_schema::{attachment, channel, community, dm_recipient, message, poll_option};
    type Row = (
        uuid::Uuid,
        Option<UserId>,
        String,
        Option<CommunityId>,
        Option<ChannelId>,
        Option<String>,
        chrono::DateTime<chrono::Utc>,
    );
    let mut conn = state.connection_pool.get().await?;
    let mut query = moderation_log::table
        .select((
            moderation_log::id,
            moderation_log::actor,
            moderation_log::action,
            moderation_log::community,
            moderation_log::channel,
            moderation_log::subject,
            moderation_log::at,
        ))
        .order(moderation_log::id.desc())
        .limit(limit)
        .into_boxed();
    if let Some(before) = before {
        query = query.filter(moderation_log::id.lt(before));
    }
    let rows: Vec<Row> = query.load(conn.as_mut()).await?;
    let subjects: Vec<Option<Subject>> = rows
        .iter()
        .map(|(_, _, action, _, _, subject, _)| {
            subject.as_deref().and_then(|s| subject_of(action, s))
        })
        .collect();

    let community_ids: Vec<CommunityId> = rows.iter().filter_map(|r| r.3).collect();
    let communities: HashMap<CommunityId, LoggedCommunity> = community::table
        .select((
            community::id,
            community::name,
            community::deleted_at.is_not_null(),
        ))
        .filter(community::id.eq_any(&community_ids))
        .load::<(CommunityId, String, bool)>(conn.as_mut())
        .await?
        .into_iter()
        .map(|(id, name, deleted)| (id, LoggedCommunity { name, deleted }))
        .collect();

    let mut message_ids: Vec<MessageId> = Vec::new();
    let mut poll_ids: Vec<PollId> = Vec::new();
    let mut attachment_ids: Vec<AttachmentId> = Vec::new();
    for subject in subjects.iter().flatten() {
        match subject {
            Subject::Message(id) | Subject::Reaction(id, _, _) => message_ids.push(*id),
            Subject::Attachment(id, attachment) => {
                message_ids.push(*id);
                attachment_ids.push(*attachment);
            }
            Subject::WriteIn(poll, _) | Subject::Poll(poll) => poll_ids.push(*poll),
            Subject::User(_) | Subject::Name(_) => {}
        }
    }
    let messages: Vec<(MessageId, ChannelId, UserId, bool, Option<PollId>)> = message::table
        .select((
            message::id,
            message::channel,
            message::author,
            message::deleted_at.is_not_null(),
            message::poll,
        ))
        .filter(
            // A poll's entry links the message it is shown in, never its `poll_closed` notice,
            // which `message_poll_shown_once` finds by this very condition.
            message::id.eq_any(&message_ids).or(message::kind
                .eq(MessageKind::Poll)
                .and(message::poll.eq_any(&poll_ids))),
        )
        .load(conn.as_mut())
        .await?;
    let logged = |(id, channel, author, deleted, _): &(
        MessageId,
        ChannelId,
        UserId,
        bool,
        Option<PollId>,
    )| {
        LoggedMessage {
            id: *id,
            channel: *channel,
            author: *author,
            deleted: *deleted,
        }
    };
    let by_id: HashMap<MessageId, LoggedMessage> =
        messages.iter().map(|m| (m.0, logged(m))).collect();
    let by_poll: HashMap<PollId, LoggedMessage> = messages
        .iter()
        .filter_map(|m| m.4.map(|poll| (poll, logged(m))))
        .collect();
    let attachments: HashMap<AttachmentId, String> = attachment::table
        .select((attachment::id, attachment::file_name))
        .filter(attachment::id.eq_any(&attachment_ids))
        .load(conn.as_mut())
        .await?
        .into_iter()
        .collect();
    let write_ins: HashMap<(PollId, i32), String> = poll_option::table
        .select((poll_option::poll, poll_option::index, poll_option::label))
        .filter(poll_option::poll.eq_any(&poll_ids))
        .load::<(PollId, i32, String)>(conn.as_mut())
        .await?
        .into_iter()
        .map(|(poll, index, label)| ((poll, index), label))
        .collect();

    let channel_ids: Vec<ChannelId> = rows.iter().filter_map(|r| r.4).collect();
    let channels: Vec<(
        ChannelId,
        String,
        crate::channel::ChannelType,
        bool,
        Option<ChannelId>,
    )> = channel::table
        .select((
            channel::id,
            channel::name,
            channel::ty,
            channel::deleted_at.is_not_null(),
            channel::parent_channel,
        ))
        .filter(channel::id.eq_any(&channel_ids))
        .load(conn.as_mut())
        .await?;
    let mut recipients: HashMap<ChannelId, Vec<UserId>> = HashMap::new();
    for (channel, user) in dm_recipient::table
        .select((dm_recipient::channel, dm_recipient::user))
        .filter(dm_recipient::channel.eq_any(&channel_ids))
        .order((dm_recipient::channel, dm_recipient::joined_at))
        .load::<(ChannelId, UserId)>(conn.as_mut())
        .await?
    {
        recipients.entry(channel).or_default().push(user);
    }
    let channels: HashMap<ChannelId, LoggedChannel> = channels
        .into_iter()
        .map(|(id, name, ty, deleted, parent_channel)| {
            let channel = LoggedChannel {
                name,
                ty,
                deleted,
                parent_channel,
                recipients: recipients.remove(&id).unwrap_or_default(),
            };
            (id, channel)
        })
        .collect();

    Ok(rows
        .into_iter()
        .zip(subjects)
        .map(
            |((id, actor, action, community, channel, subject, at), named)| {
                let mut details = ModerationDetails {
                    community: community.and_then(|c| communities.get(&c).cloned()),
                    channel: channel.and_then(|c| channels.get(&c).cloned()),
                    ..ModerationDetails::default()
                };
                match named {
                    Some(Subject::User(user)) => details.user = Some(user),
                    Some(Subject::Message(message)) => {
                        details.message = by_id.get(&message).cloned()
                    }
                    Some(Subject::Attachment(message, attachment)) => {
                        details.message = by_id.get(&message).cloned();
                        details.attachment = attachments.get(&attachment).cloned();
                    }
                    Some(Subject::Reaction(message, emoji, author)) => {
                        details.message = by_id.get(&message).cloned();
                        details.emoji = Some(emoji);
                        details.user = Some(author);
                    }
                    Some(Subject::Name(name)) => details.renamed_to = Some(name),
                    Some(Subject::WriteIn(poll, option)) => {
                        details.message = by_poll.get(&poll).cloned();
                        details.write_in = write_ins.get(&(poll, option)).cloned();
                    }
                    Some(Subject::Poll(poll)) => {
                        details.message = by_poll.get(&poll).cloned();
                    }
                    None => {}
                }
                ModerationEntry {
                    id,
                    actor,
                    action,
                    community,
                    channel,
                    subject,
                    at,
                    details,
                }
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each action's subject reads back as what its caller wrote, an emoji with a slash in its
    /// neighbours included, and a subject that does not parse names nothing.
    #[test]
    fn subjects_read_back_by_action() {
        let a = "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
        let b = "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5c";
        let id = |text: &str| uuid::Uuid::parse_str(text).unwrap();
        assert_eq!(
            subject_of("removeMember", a),
            Some(Subject::User(UserId(id(a))))
        );
        assert_eq!(
            subject_of("liftUserBan", a),
            Some(Subject::User(UserId(id(a))))
        );
        assert_eq!(
            subject_of("readDm", a),
            Some(Subject::Message(MessageId(id(a))))
        );
        assert_eq!(
            subject_of("removeAttachment", &format!("{a}/{b}")),
            Some(Subject::Attachment(MessageId(id(a)), AttachmentId(id(b))))
        );
        assert_eq!(
            subject_of("removeReaction", &format!("{a}/👍🏽/{b}")),
            Some(Subject::Reaction(
                MessageId(id(a)),
                "👍🏽".into(),
                UserId(id(b))
            ))
        );
        assert_eq!(
            subject_of("removeWriteIn", &format!("{a}/3")),
            Some(Subject::WriteIn(PollId(id(a)), 3))
        );
        assert_eq!(subject_of("deleteMessage", "not an id"), None);
        assert_eq!(
            subject_of("renameChannel", "a new / name"),
            Some(Subject::Name("a new / name".into()))
        );
        assert_eq!(subject_of("deleteChannel", a), None);
        assert_eq!(subject_of("somethingNew", a), None);
    }

    /// The log's action names are the ones `spec/moderation_actions.json` lists, which the
    /// client names in its dashboard.
    #[test]
    fn moderation_actions_match_the_spec() {
        #[derive(serde::Deserialize)]
        struct Spec {
            actions: Vec<String>,
        }
        let spec: Spec =
            serde_json::from_str(include_str!("../../../spec/moderation_actions.json")).unwrap();
        let names: Vec<String> = <ModerationAction as strum::VariantArray>::VARIANTS
            .iter()
            .map(|action| <&str>::from(*action).to_string())
            .collect();
        assert_eq!(names, spec.actions);
    }
}
