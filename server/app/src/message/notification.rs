//! What a notification says of a message, written out by the server for a reader that holds
//! nothing to resolve the message's ids with or shows it where nothing can be revealed: a
//! phone's notification code and the web client's system notification, which show the text
//! as it is given (`spec/push.md`). Read with `include=notificationTexts` on a message read.

use super::tokens::{self, Token};
use crate::context::GlobalServerContext;
use crate::message::MessageKind;
use crate::t;
use crate::{AttachmentId, ChannelId, CommunityId, CustomEmojiId, MessageId, RoleId, UserId};
use aspen_schema::{attachment, channel, community_role, custom_emoji, user};
use aspen_wire::message_enum;
use diesel::prelude::*;
use diesel_async::RunQueryDsl;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use utoipa::ToSchema;

mod spoilers;

/// What a notification says of one message, ready to show.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NotificationText {
    pub message: MessageId,
    /// Plain text in the language the request asked for. For a message with text of its own,
    /// that text with each person and role it tags as `@name`, each of its community's emoji
    /// as `:name:`, `@…` or `:…:` for one that names nothing the reader could be shown, and
    /// each spoiler as a word for one in place of what it hides;
    /// for one without, what its kind records (a poll, a poll's closing, a join, a community
    /// become large), or else what it holds: a picture, several, other files, or nothing.
    pub text: String,
}

/// What stands for a tag or an emoji whose name the notification cannot give.
const UNNAMED: &str = "…";

/// The names the messages of one read are written out with.
#[derive(Default)]
struct Names {
    /// By display name, or else username.
    users: HashMap<UserId, String>,
    roles: HashMap<RoleId, String>,
    /// Each emoji's name, with the community it belongs to.
    emoji: HashMap<CustomEmojiId, (CommunityId, String)>,
    /// The community of each channel that has one.
    communities: HashMap<ChannelId, CommunityId>,
    /// Which attachments of the messages without text are pictures, by their declared type.
    pictures: HashSet<AttachmentId>,
}

impl Names {
    /// What `token` reads as in `message`. A person or role is named only when the message
    /// tags it (`mentions`, which holds those its author was allowed to tag, in the channel's
    /// own community) or is its author, and an emoji only in its own community, so no
    /// message names what its reader could not otherwise see.
    fn name(&self, message: &message_enum::Message, token: Token) -> String {
        match token {
            Token::User(id) => {
                let name = (id == message.author || message.mentions.users.contains(&id))
                    .then(|| self.users.get(&id))
                    .flatten();
                format!("@{}", name.map_or(UNNAMED, String::as_str))
            }
            Token::Role(id) => {
                let name = message
                    .mentions
                    .roles
                    .contains(&id)
                    .then(|| self.roles.get(&id))
                    .flatten();
                format!("@{}", name.map_or(UNNAMED, String::as_str))
            }
            Token::Emoji(id) => {
                let here = self.communities.get(&message.channel_id);
                let name = self
                    .emoji
                    .get(&id)
                    .filter(|(community, _)| Some(community) == here)
                    .map(|(_, name)| name.as_str());
                format!(":{}:", name.unwrap_or(UNNAMED))
            }
        }
    }
}

/// A stretch of a message's text, by how it is shown to a reader outside the app, who holds
/// nothing to resolve its ids with and can reveal nothing: a notification's, an email's.
#[derive(Debug, PartialEq)]
pub(crate) enum Part<'a> {
    /// Text, whose tokens are written as names.
    Text(&'a str),
    /// A code span or block, markup included, where a message shows its text as written and
    /// names nothing.
    Code(&'a str),
    /// A spoiler, markers included, which is not shown.
    Spoiler,
}

/// The parts of `content`, in order. A spoiler hides the code inside it.
pub(crate) fn parts<'a>(content: &'a str) -> Vec<Part<'a>> {
    let reading = spoilers::read(content);
    let mut parts = Vec::new();
    let mut code = reading.code.iter().peekable();
    // Adds what is shown of `range`, which holds no spoiler: its code, and the text between.
    let mut shown = |parts: &mut Vec<Part<'a>>, range: Range<usize>| {
        let mut at = range.start;
        while let Some(span) = code.peek().filter(|span| span.start < range.end) {
            let (start, end) = (span.start.max(at), span.end.min(range.end));
            if start < end {
                if at < start {
                    parts.push(Part::Text(&content[at..start]));
                }
                parts.push(Part::Code(&content[start..end]));
                at = end;
            }
            if span.end > range.end {
                break;
            }
            code.next();
        }
        if at < range.end {
            parts.push(Part::Text(&content[at..range.end]));
        }
    };
    let mut at = 0;
    for spoiler in reading.spoilers {
        shown(&mut parts, at..spoiler.start);
        parts.push(Part::Spoiler);
        at = spoiler.end;
    }
    shown(&mut parts, at..content.len());
    parts
}

/// `parts` as a reader outside the app is shown them, trimmed: each token of the text as
/// `name` gives it, or as written when it gives nothing, and each spoiler as `spoiler` in
/// place of its markers and all it holds. The Markdown of what is shown stays as written.
/// Every such reader's text is written here, so none shows what a spoiler hides.
pub(crate) fn written_out(
    parts: &[Part<'_>],
    spoiler: &str,
    mut name: impl FnMut(Token) -> Option<String>,
) -> String {
    let mut out = String::new();
    for part in parts {
        match part {
            Part::Text(text) => out.push_str(&tokens::rewritten(text, &mut name)),
            Part::Code(code) => out.push_str(code),
            Part::Spoiler => out.push_str(spoiler),
        }
    }
    out.trim().to_string()
}

/// What a notification says of a message with no text of its own, in the request's language:
/// what its kind records, or for a kind that records nothing to put in words, what it holds.
fn recorded(message: &message_enum::Message, names: &Names) -> String {
    match message.kind {
        MessageKind::Poll => t!("notificationPoll").into_owned(),
        MessageKind::PollClosed => t!("notificationPollClosed").into_owned(),
        MessageKind::MemberJoined => t!("notificationMemberJoined").into_owned(),
        MessageKind::CommunityGrew => {
            let grew = t!("notificationCommunityGrew");
            if message.growth.is_some_and(|g| g.mention_everyone_taken) {
                format!("{grew} {}", t!("notificationCommunityGrewMentionEveryone"))
            } else {
                grew.into_owned()
            }
        }
        MessageKind::Standard
        | MessageKind::ThreadEcho
        | MessageKind::Call
        | MessageKind::MissedCall
        | MessageKind::Command
        | MessageKind::Warning => held(message, names).into_owned(),
    }
}

/// What a notification says of a message by its files alone: one picture, several, files that
/// are not all pictures, or none.
fn held(message: &message_enum::Message, names: &Names) -> Cow<'static, str> {
    let pictures = message
        .attachments
        .iter()
        .all(|attachment| names.pictures.contains(attachment));
    match (message.attachments.len(), pictures) {
        (0, _) => t!("notificationNewMessage"),
        (1, true) => t!("notificationPicture"),
        (_, true) => t!("notificationPictures"),
        (_, false) => t!("notificationAttachment"),
    }
}

/// What a notification says of `message`, whose text is `parts`: that text written out, each
/// spoiler as a word for one in the request's language, or what the message records. Text
/// that is all spoilers is still text.
fn text_of(message: &message_enum::Message, parts: &[Part<'_>], names: &Names) -> String {
    let text = written_out(parts, &t!("notificationSpoiler"), |token| {
        Some(names.name(message, token))
    });
    if text.is_empty() {
        recorded(message, names)
    } else {
        text
    }
}

/// What a notification says of each of `messages`, which the caller may read: one batched
/// read per kind of name however many messages there are.
pub async fn read_notification_texts(
    state: &GlobalServerContext,
    messages: &[message_enum::Message],
) -> crate::Result<Vec<NotificationText>> {
    let mut users: HashSet<UserId> = HashSet::new();
    let mut roles: HashSet<RoleId> = HashSet::new();
    let mut emoji: HashSet<CustomEmojiId> = HashSet::new();
    let mut channels: HashSet<ChannelId> = HashSet::new();
    let mut files: HashSet<AttachmentId> = HashSet::new();
    let read: Vec<Vec<Part<'_>>> = messages
        .iter()
        .map(|message| parts(&message.content))
        .collect();
    for (message, parts) in messages.iter().zip(&read) {
        files.extend(&message.attachments);
        for part in parts {
            let Part::Text(text) = part else {
                continue;
            };
            tokens::each(text, |token| match token {
                Token::User(id) => {
                    users.insert(id);
                }
                Token::Role(id) => {
                    roles.insert(id);
                }
                Token::Emoji(id) => {
                    emoji.insert(id);
                    channels.insert(message.channel_id);
                }
            });
        }
    }
    let mut names = Names::default();
    if !(users.is_empty() && roles.is_empty() && emoji.is_empty() && files.is_empty()) {
        let mut conn = state.connection_pool.get().await?;
        if !files.is_empty() {
            names.pictures = attachment::table
                .select((attachment::id, attachment::mime_type))
                .filter(attachment::id.eq_any(&files))
                .load::<(AttachmentId, String)>(conn.as_mut())
                .await?
                .into_iter()
                .filter(|(_, mime_type)| crate::attachment::pictured(mime_type))
                .map(|(id, _)| id)
                .collect();
        }
        if !users.is_empty() {
            names.users = user::table
                .select((user::id, user::name, user::display_name))
                .filter(user::id.eq_any(&users).and(user::deleted_at.is_null()))
                .load::<(UserId, String, Option<String>)>(conn.as_mut())
                .await?
                .into_iter()
                .map(|(id, name, display_name)| {
                    let shown = display_name.filter(|display| !display.is_empty());
                    (id, shown.unwrap_or(name))
                })
                .collect();
        }
        if !roles.is_empty() {
            names.roles = community_role::table
                .select((community_role::id, community_role::name))
                .filter(community_role::id.eq_any(&roles))
                .load::<(RoleId, String)>(conn.as_mut())
                .await?
                .into_iter()
                .collect();
        }
        if !emoji.is_empty() {
            names.emoji = custom_emoji::table
                .select((
                    custom_emoji::id,
                    custom_emoji::community,
                    custom_emoji::name,
                ))
                .filter(
                    custom_emoji::id
                        .eq_any(&emoji)
                        .and(custom_emoji::deleted_at.is_null()),
                )
                .load::<(CustomEmojiId, CommunityId, String)>(conn.as_mut())
                .await?
                .into_iter()
                .map(|(id, community, name)| (id, (community, name)))
                .collect();
            names.communities = channel::table
                .select((channel::id, channel::community.assume_not_null()))
                .filter(
                    channel::id
                        .eq_any(&channels)
                        .and(channel::community.is_not_null()),
                )
                .load::<(ChannelId, CommunityId)>(conn.as_mut())
                .await?
                .into_iter()
                .collect();
        }
    }
    Ok(messages
        .iter()
        .zip(&read)
        .map(|(message, parts)| NotificationText {
            message: message.id,
            text: text_of(message, parts, &names),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mention::Mentions;

    const ANN: u128 = 1;
    const BOB: u128 = 2;
    const MODS: u128 = 3;
    const PARTY: u128 = 4;
    const ELSEWHERE: u128 = 5;
    const HERE: u128 = 10;
    const CHANNEL: u128 = 11;
    const PICTURE: u128 = 20;
    const OTHER_PICTURE: u128 = 21;
    const FILE: u128 = 22;

    fn id(n: u128) -> uuid::Uuid {
        uuid::Uuid::from_u128(n)
    }

    fn names() -> Names {
        Names {
            users: HashMap::from([
                (UserId(id(ANN)), "Ann".to_string()),
                (UserId(id(BOB)), "Bob".to_string()),
            ]),
            roles: HashMap::from([(RoleId(id(MODS)), "Mods".to_string())]),
            emoji: HashMap::from([
                (
                    CustomEmojiId(id(PARTY)),
                    (CommunityId(id(HERE)), "party".to_string()),
                ),
                (
                    CustomEmojiId(id(ELSEWHERE)),
                    (CommunityId(id(99)), "secret".to_string()),
                ),
            ]),
            communities: HashMap::from([(ChannelId(id(CHANNEL)), CommunityId(id(HERE)))]),
            pictures: HashSet::from([AttachmentId(id(PICTURE)), AttachmentId(id(OTHER_PICTURE))]),
        }
    }

    /// A message by Ann in the community's channel, tagging Bob and the Mods.
    fn message(kind: MessageKind, content: &str) -> message_enum::Message {
        message_enum::Message {
            id: MessageId(id(100)),
            channel_id: ChannelId(id(CHANNEL)),
            author: UserId(id(ANN)),
            timestamp: chrono::DateTime::UNIX_EPOCH,
            edited_at: None,
            content: content.to_string(),
            attachments: Vec::new(),
            link_previews: Vec::new(),
            kind,
            notifies: kind.notifies(),
            poll: None,
            thread: None,
            echo_of: None,
            mentions: Mentions {
                users: vec![UserId(id(BOB))],
                roles: vec![RoleId(id(MODS))],
                everyone: false,
            },
            call_seconds: None,
            command_bot: None,
            linked_messages: Vec::new(),
            warning: None,
            growth: None,
            altered_by: Vec::new(),
            card: None,
            echo: None,
            reply_to: None,
        }
    }

    /// What a notification says of `message`, read as a read of it alone would.
    fn said(message: &message_enum::Message) -> String {
        text_of(message, &parts(&message.content), &names())
    }

    fn text(content: &str) -> String {
        said(&message(MessageKind::Standard, content))
    }

    #[test]
    fn tags_and_emoji_read_as_names() {
        let content = format!(
            "  hi <@{}> and <@&{}> <:{}>, from <@{}>\n",
            id(BOB),
            id(MODS),
            id(PARTY),
            id(ANN)
        );
        assert_eq!(
            text(&content).as_str(),
            "hi @Bob and @Mods :party:, from @Ann"
        );
    }

    /// The scenario of a message that is one emoji and nothing else: its name, not its id.
    #[test]
    fn a_message_that_is_only_an_emoji_reads_as_its_name() {
        assert_eq!(text(&format!("<:{}>", id(PARTY))).as_str(), ":party:");
    }

    /// Someone the message does not tag, a role it does not, an emoji of another community,
    /// and ids nothing is known by all read as unnamed, never as an id or another place's name.
    #[test]
    fn what_the_message_may_not_name_reads_as_unnamed() {
        let mut untagged = message(
            MessageKind::Standard,
            &format!(
                "<@{}> <@&{}> <:{}> <:{}> <@{}>",
                id(BOB),
                id(MODS),
                id(ELSEWHERE),
                id(77),
                id(78)
            ),
        );
        untagged.mentions = Mentions::default();
        assert_eq!(said(&untagged).as_str(), "@… @… :…: :…: @…");
        // In a DM, which has no community, no emoji is named.
        let mut dm = message(MessageKind::Standard, &format!("<:{}>", id(PARTY)));
        dm.channel_id = ChannelId(id(12));
        assert_eq!(said(&dm).as_str(), ":…:");
    }

    /// Code shows its text as written, and Markdown is left for the reader as it is.
    #[test]
    fn code_names_nothing_and_markdown_stays() {
        let bob = id(BOB);
        let content =
            format!("**see** `<@{bob}>` <@{bob}>\n\n```\n<@{bob}>\n```\n\n> <@{bob}> _hi_");
        assert_eq!(
            text(&content).as_str(),
            format!("**see** `<@{bob}>` @Bob\n\n```\n<@{bob}>\n```\n\n> @Bob _hi_").as_str()
        );
        assert_eq!(text("a < b <@nope> c ж").as_str(), "a < b <@nope> c ж");
    }

    /// A spoiler reads as a word for one, with whatever it holds: text, tags, and code. A
    /// marker in code is none, and neither is one nothing closes.
    #[test]
    fn spoilers_are_not_shown() {
        let bob = id(BOB);
        assert_eq!(
            text(&format!(
                "the killer is ||**the butler** <@{bob}>|| <@{bob}>"
            ))
            .as_str(),
            "the killer is [spoiler] @Bob"
        );
        assert_eq!(
            text(">!one!< and || `two` || and `||three||` and ||four").as_str(),
            "[spoiler] and [spoiler] and `||three||` and ||four"
        );
        assert_eq!(
            text("||a||||b||\n\n```\n||c||\n```").as_str(),
            "[spoiler][spoiler]\n\n```\n||c||\n```"
        );
        assert_eq!(text("a ||b\nc|| d").as_str(), "a [spoiler] d");
        assert_eq!(text("a ||b\n\nc|| d").as_str(), "a ||b\n\nc|| d");
    }

    /// Text that is nothing but spoilers is still the message's text, whatever it holds.
    #[test]
    fn a_message_of_spoilers_alone_says_so() {
        assert_eq!(text(" ||secret|| ").as_str(), "[spoiler]");
        let mut pictured = message(MessageKind::Standard, "||secret||");
        pictured.attachments = vec![AttachmentId(id(PICTURE))];
        assert_eq!(said(&pictured).as_str(), "[spoiler]");
    }

    /// Code inside a spoiler is hidden with it, and no part is empty.
    #[test]
    fn parts_tell_text_from_code_and_spoilers() {
        assert_eq!(
            parts("a `b` ||c `d`|| e"),
            [
                Part::Text("a "),
                Part::Code("`b`"),
                Part::Text(" "),
                Part::Spoiler,
                Part::Text(" e"),
            ]
        );
        assert_eq!(parts(""), []);
    }

    /// A message with no text says what its kind records.
    #[test]
    fn a_message_without_text_says_what_it_records() {
        let of = |kind| said(&message(kind, " \n"));
        assert_eq!(of(MessageKind::Poll).as_str(), "Posted a poll");
        assert_eq!(of(MessageKind::PollClosed).as_str(), "A poll has closed");
        assert_eq!(
            of(MessageKind::MemberJoined).as_str(),
            "Joined the community"
        );
        let grew = of(MessageKind::CommunityGrew);
        assert!(grew.starts_with("This community is now large."), "{grew}");
        let mut taken = message(MessageKind::CommunityGrew, "");
        taken.growth = Some(crate::large_community::CommunityGrowth {
            members: 1000,
            mention_everyone_taken: true,
        });
        let text = said(&taken);
        assert!(text.starts_with(&grew) && text.len() > grew.len(), "{text}");
        // Text of its own is shown whatever the kind.
        assert_eq!(
            said(&message(MessageKind::Poll, "Lunch?")).as_str(),
            "Lunch?"
        );
    }

    /// A message with no text whose kind records nothing in words says what it holds.
    #[test]
    fn a_message_of_files_alone_says_what_they_are() {
        let holding = |files: &[u128]| {
            let mut message = message(MessageKind::Standard, "");
            message.attachments = files.iter().map(|&n| AttachmentId(id(n))).collect();
            said(&message)
        };
        assert_eq!(holding(&[]), "New message");
        assert_eq!(holding(&[PICTURE]), "Sent an image");
        assert_eq!(holding(&[PICTURE, OTHER_PICTURE]), "Sent images");
        assert_eq!(holding(&[FILE]), "Sent an attachment");
        assert_eq!(holding(&[PICTURE, FILE]), "Sent an attachment");
        // Text of its own is shown whatever the message holds.
        let mut captioned = message(MessageKind::Standard, "look");
        captioned.attachments = vec![AttachmentId(id(PICTURE))];
        assert_eq!(said(&captioned), "look");
    }
}
